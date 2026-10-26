//! S1 step 1 (03-gc §3 and §4, H11; V9-D21): what the barrier slow path logs when the heap is
//! generational ([`crate::HeapConfig::generational`]).
//!
//! ```text
//! barrier_slow(host, slot, v), barrier_slow_shape(host, id)  (signatures and BARRIER_VERSION 2 kept)
//!   the store
//!   host outside every generational cage of this thread: done (S0, or no heap cell)
//!   base = host & PAGE_MASK; the header kind byte at base + 4:
//!     small page:    host BLACK -> GREY; push (host - base) on the page's remembered list
//!     large object:  card[(slot - cell) / 512] = 1; dirty = 1   (the host stays BLACK)
//!
//! small page header +16 (PAGE_REMEMBERED_OFFSET): { pointer, length, capacity }
//!     a growable array of u32 byte offsets, one per remembered (GREY) host of the page
//! large object header +32 (LARGE_CARDS_OFFSET): card table pointer, one byte per 512 B of
//!     the cell, made on the first logged store; +40 (LARGE_DIRTY_OFFSET): 1 if a card is dirty
//! ```
//!
//! | Rule | Why |
//! |---|---|
//! | A small host is logged once per GC cycle | GREY is above the GEN threshold, so a remembered host does not fire again until a GC sets it BLACK (the mark does) |
//! | A large host dirties a card at every logged store | It stays BLACK (03-gc §3, H11 "S1 logs"): the minor GC rescans only the dirty 512-byte cards of a large array |
//! | The lists and cards hold no `Value` and are not roots in step 1 | Cells never move in S1, and each collection empties them after its mark (every collection is still a full GC, which traces everything). The minor GC (M4-P3) will trace the remembered hosts and the dirty cards first |
//! | Rust memory, not heap cells | The slow path must not allocate on the GC heap or reach a GC point (H11); `std::alloc` does neither. A list is freed with its page, a card table with its large object, both when the heap is dropped |
//! | The cage check | `barrier_slow` gets only a `&Word0`. The thread-local list of generational cages shows that the host lies in a heap of this thread before the slow path reads a page header, and gives the cage base pointer whose provenance covers that header (no integer-to-pointer cast). A host outside (any heap in S0, a word on a test's stack) only stores |

use core::cell::RefCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, AtomicU8, AtomicU32, Ordering::Relaxed};
use std::alloc::Layout;

use ep9_value::Word0;

use crate::barrier::gc_state;
use crate::layout::{
    CARD_SIZE, LARGE_CELL_OFFSET, LargeHeader, PAGE_KIND_LARGE, PAGE_KIND_SMALL, PAGE_MAGIC,
    PAGE_MASK, PageHeader,
};

/// The remembered list of one small page (in its header at +16).
#[repr(C)]
pub(crate) struct Remembered {
    ptr: AtomicPtr<u32>,
    len: AtomicU32,
    cap: AtomicU32,
}

const _: () = assert!(size_of::<Remembered>() == 16);

/// The first capacity of a list.
const MIN_REMEMBERED: usize = 16;

impl Remembered {
    /// An empty list (no memory).
    pub(crate) const fn empty() -> Remembered {
        Remembered {
            ptr: AtomicPtr::new(core::ptr::null_mut()),
            len: AtomicU32::new(0),
            cap: AtomicU32::new(0),
        }
    }

    /// Adds the byte offset of a host. Not a GC point; may allocate Rust memory.
    pub(crate) fn push(&self, offset: u32) {
        let len = self.len.load(Relaxed);
        if len == self.cap.load(Relaxed) {
            self.grow();
        }
        // SAFETY: `len < cap` now, and `ptr` holds `cap` u32s (`grow`). Only the heap's thread
        // reaches this list, and no slice from `offsets` is live during a push (its contract).
        unsafe { self.ptr.load(Relaxed).add(len as usize).write(offset) };
        self.len.store(len + 1, Relaxed);
    }

    #[cold]
    #[inline(never)]
    fn grow(&self) {
        let cap = self.cap.load(Relaxed) as usize;
        let new_cap = (cap * 2).max(MIN_REMEMBERED);
        let new_layout = Layout::array::<u32>(new_cap).expect("a remembered list fits");
        let old = self.ptr.load(Relaxed);
        let p = if old.is_null() {
            // SAFETY: the layout has a non-zero size.
            unsafe { std::alloc::alloc(new_layout) }
        } else {
            let old_layout = Layout::array::<u32>(cap).expect("the old layout was valid");
            // SAFETY: `old` came from `alloc`/`realloc` with `old_layout`; the new size is
            // non-zero and fits (checked by `Layout::array`).
            unsafe { std::alloc::realloc(old.cast(), old_layout, new_layout.size()) }
        };
        if p.is_null() {
            std::alloc::handle_alloc_error(new_layout);
        }
        self.ptr.store(p.cast(), Relaxed);
        self.cap.store(new_cap as u32, Relaxed);
    }

    pub(crate) fn len(&self) -> usize {
        self.len.load(Relaxed) as usize
    }

    /// The offsets logged since the last [`Remembered::clear`].
    ///
    /// # Safety
    /// No push may run while the slice is live (the GC reads it; only the mutator pushes).
    pub(crate) unsafe fn offsets(&self) -> &[u32] {
        let len = self.len();
        if len == 0 {
            return &[];
        }
        // SAFETY: `ptr` holds `len` initialized u32s; no push reallocates it while the slice
        // lives (caller contract).
        unsafe { core::slice::from_raw_parts(self.ptr.load(Relaxed), len) }
    }

    /// Empties the list and keeps its memory.
    pub(crate) fn clear(&self) {
        self.len.store(0, Relaxed);
    }

    /// Frees the list's memory (its page is freed, or the heap is dropped).
    pub(crate) fn release(&self) {
        let p = self.ptr.swap(core::ptr::null_mut(), Relaxed);
        let cap = self.cap.swap(0, Relaxed) as usize;
        self.len.store(0, Relaxed);
        if !p.is_null() {
            let layout = Layout::array::<u32>(cap).expect("the layout was valid");
            // SAFETY: `p` came from `alloc`/`realloc` with this layout and is no longer
            // reachable from the header.
            unsafe { std::alloc::dealloc(p.cast(), layout) };
        }
    }
}

/// Cards of a large object of `size` bytes.
pub(crate) fn card_count(size: usize) -> usize {
    size.div_ceil(CARD_SIZE).max(1)
}

/// Marks card `card` of the large object `h` dirty, making its card table on first use.
fn dirty_card(h: &LargeHeader, card: usize) {
    let n = card_count(h.size);
    debug_assert!(card < n, "card {card} of {n}");
    let card = card.min(n - 1);
    let mut p = h.cards.load(Relaxed);
    if p.is_null() {
        p = new_cards(n);
        h.cards.store(p, Relaxed);
    }
    // SAFETY: the table holds `n` bytes and `card < n`; it is only accessed as atomics.
    unsafe { AtomicU8::from_ptr(p.add(card)) }.store(1, Relaxed);
    h.dirty.store(1, Relaxed);
}

#[cold]
#[inline(never)]
fn new_cards(n: usize) -> *mut u8 {
    let layout = Layout::array::<u8>(n).expect("a card table fits");
    // SAFETY: `n >= 1`, so the layout has a non-zero size.
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    if p.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    p
}

/// The card table of `h` (one byte per card, 1 = dirty), or `None` before the first logged
/// store.
pub(crate) fn cards(h: &LargeHeader) -> Option<&[AtomicU8]> {
    let p = h.cards.load(Relaxed);
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null table holds `card_count(size)` bytes, accessed only as atomics, and
    // lives until `release_cards` (which needs the object to be freed or the heap dropped).
    Some(unsafe { core::slice::from_raw_parts(p.cast::<AtomicU8>(), card_count(h.size)) })
}

/// Cleans every card of `h` and its dirty byte; keeps the table.
pub(crate) fn clear_cards(h: &LargeHeader) {
    if h.dirty.swap(0, Relaxed) == 0 {
        return;
    }
    if let Some(cards) = cards(h) {
        for c in cards {
            c.store(0, Relaxed);
        }
    }
}

/// Frees the card table of `h` (the object is freed, or the heap is dropped).
pub(crate) fn release_cards(h: &LargeHeader) {
    let p = h.cards.swap(core::ptr::null_mut(), Relaxed);
    h.dirty.store(0, Relaxed);
    if !p.is_null() {
        let layout = Layout::array::<u8>(card_count(h.size)).expect("the layout was valid");
        // SAFETY: `p` came from `alloc_zeroed` with this layout and is no longer reachable.
        unsafe { std::alloc::dealloc(p, layout) };
    }
}

std::thread_local! {
    /// The generational heaps alive on this thread: each cage's base (with the provenance of
    /// the whole reservation) and its size in bytes.
    static CAGES: RefCell<Vec<(NonNull<u8>, usize)>> = const { RefCell::new(Vec::new()) };
}

/// The registration of one generational heap's cage on its thread; dropping it removes it.
#[derive(Debug)]
pub(crate) struct GenerationalCage {
    base: NonNull<u8>,
}

impl GenerationalCage {
    /// `base`: the reservation's base pointer; `size`: its bytes.
    pub(crate) fn register(base: NonNull<u8>, size: usize) -> GenerationalCage {
        CAGES.with_borrow_mut(|c| c.push((base, size)));
        GenerationalCage { base }
    }
}

impl Drop for GenerationalCage {
    fn drop(&mut self) {
        // The thread may be exiting (its locals already gone): nothing left to remove then.
        let _ = CAGES.try_with(|c| c.borrow_mut().retain(|&(base, _)| base != self.base));
    }
}

/// The base of the generational cage of this thread that holds `addr`, if one does.
fn generational_cage_base(addr: usize) -> Option<NonNull<u8>> {
    CAGES
        .try_with(|c| {
            c.borrow().iter().find_map(|&(base, size)| {
                addr.checked_sub(base.addr().get())
                    .is_some_and(|off| off < size)
                    .then_some(base)
            })
        })
        .ok()
        .flatten()
}

/// The S1 log of a store into `host` at `slot_addr` (module doc). Not a GC point; never
/// allocates on the GC heap.
#[inline]
pub(crate) fn log_store(host: &Word0, slot_addr: usize) {
    let addr = core::ptr::from_ref(host).addr();
    let Some(cage) = generational_cage_base(addr) else {
        return;
    };
    let base = addr & PAGE_MASK;
    // The header, from the cage base: its provenance covers the whole reservation.
    let header = cage.with_addr(core::num::NonZeroUsize::new(base).expect("a cage address"));
    // SAFETY: `addr` lies in the cage of a live heap of this thread, and `host` is a valid
    // reference there, so the chunk that holds it is committed from its base on: a small page
    // (its header at the base) or the first chunk of a large object (the cell starts at
    // base + 64). Both headers start with the magic (u32) and the kind byte at +4.
    let (magic, kind) = unsafe { (header.cast::<u32>().read(), header.add(4).read()) };
    debug_assert_eq!(
        magic, PAGE_MAGIC,
        "S1 log: host {addr:#x} has no page header"
    );
    if magic != PAGE_MAGIC {
        return;
    }
    match kind {
        PAGE_KIND_SMALL => {
            if host.gc_state() != gc_state::BLACK {
                return;
            }
            host.set_gc_state(gc_state::GREY);
            // SAFETY: a small page header (checked above); its S1 fields are atomics.
            let h = unsafe { header.cast::<PageHeader>().as_ref() };
            h.remembered.push((addr - base) as u32);
        }
        PAGE_KIND_LARGE => {
            // SAFETY: a large object header (checked above); its S1 fields are atomics.
            let h = unsafe { header.cast::<LargeHeader>().as_ref() };
            let cell = base + LARGE_CELL_OFFSET;
            debug_assert!(
                (cell..cell + h.size).contains(&slot_addr),
                "S1 log: slot {slot_addr:#x} outside its large host {cell:#x}"
            );
            dirty_card(h, slot_addr.saturating_sub(cell) / CARD_SIZE);
        }
        _ => debug_assert!(false, "S1 log: page kind {kind} at {base:#x}"),
    }
}
