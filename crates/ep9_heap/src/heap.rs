//! The heap of one isolate (D14: isolate per thread): allocation, stop-the-world non-moving
//! mark-sweep (S0), and the root API.
//!
//! ```text
//! alloc(kind, size)
//!   size <= 8 KiB:  TLAB {cursor, limit} bump (inline) --miss--> alloc_slow
//!   size >  8 KiB:  alloc_slow --> LOS: own chunk run, commit to the OS page after the cell
//! alloc_slow: GC-stress count / budget --> collect(); then
//!   small (<= 256 B):  next hole: current page -> recyclable pages -> lazy sweep -> fresh page
//!   medium (> 256 B):  current hole if it fits, else the overflow TLAB (fresh pages only)
//! every new cell: word0 = {shape 0, WHITE, kind, flags 0}; every other word = undefined;
//!                 VO bit set (small pages)
//!
//! collect(): spill registers (ep9_platform) -> retire TLABs -> finish the last lazy sweep
//!   -> mark: CSS [sp, stack high) + handles + scratch roots + root ranges + root sources
//!   -> drain (trace by kind) -> clear dead weak slots -> free dead LOS cells now
//!   -> queue every small page for lazy sweep (mark bits stay until the page is swept)
//! a callback (root source, trace, size, weak source) panics while marking: every mark bit
//!   is cleared and nothing is freed; while sweeping a page: that page's mark bits are cleared
//! ```
//!
//! [`Heap`] keeps the fields that JIT and AOT code read at fixed offsets first
//! ([`Heap::BARRIER_THRESHOLD_OFFSET`], [`Heap::TLAB_CURSOR_OFFSET`],
//! [`Heap::TLAB_LIMIT_OFFSET`]). An isolate embeds the heap and adds its own offset.

use core::cell::{Cell, RefCell};
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::Ordering::Relaxed;

use ep9_platform::CageConfig;
use ep9_value::encoding::VALUE_UNDEFINED;
use ep9_value::{ShapeId, Value, Word0};

use crate::barrier::{BarrierState, Slot, gc_state, threshold};
use crate::cell::{CellKind, CellPtr, CellSize, KindError, KindId, KindTable};
use crate::layout::{
    BITMAP_WORDS, FIRST_CELL_OFFSET, FIRST_LINE, LARGE_CELL_OFFSET, LINE_SIZE, LINES_PER_PAGE,
    Large, MAX_LINE_CELL, MAX_SMALL_SIZE, PAGE_CELL_BYTES, PAGE_MASK, PAGE_SIZE, Page, ZAP_WORD,
    cell_size_for,
};
use crate::memory::{HeapError, Memory};
use crate::roots::{GlobalHandle, Handles, RootId, RootStrength, Roots, ScratchRoots, WeakHandle};
use crate::trace::{Mode, Tracer, WeakVisitor, find_cell};

/// Heap settings. `Default` gives the plan values: a 64 GiB cage (H7), no limit below the
/// cage, an 8 MiB first budget, no GC-stress, zapping in debug builds.
#[derive(Clone, Debug)]
pub struct HeapConfig {
    /// The cage reservation (H7). Its alignment is raised to 256 KiB if lower.
    pub cage: CageConfig,
    /// Most bytes committed at one time. Allocation fails with
    /// [`AllocError::OutOfMemory`] beyond it (after one collection).
    pub max_heap_bytes: usize,
    /// Bytes allocated before the first GC. Later budgets are `max(this, live bytes)`, so the
    /// heap may double between collections.
    pub initial_budget: usize,
    /// GC-stress: collect before every Nth allocation. 0 = off, 1 = every allocation.
    pub gc_stress: u32,
    /// Run the heap verifier after every collection and panic on an error.
    pub verify_after_gc: bool,
    /// Fill freed cells with [`ZAP_WORD`].
    pub zap: bool,
    /// Keep the cells that the last conservative scan found ([`Heap::last_conservative_roots`]).
    pub record_conservative_roots: bool,
}

impl Default for HeapConfig {
    fn default() -> Self {
        HeapConfig {
            cage: CageConfig::default(),
            max_heap_bytes: usize::MAX,
            initial_budget: 8 << 20,
            gc_stress: 0,
            verify_after_gc: false,
            zap: cfg!(debug_assertions),
            record_conservative_roots: false,
        }
    }
}

/// Why an allocation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// No memory left after a full collection (cage or `max_heap_bytes`).
    OutOfMemory,
    /// The kind byte has no registration.
    UnknownKind(KindId),
    /// The size overflows the address space.
    TooLarge,
    /// The kind has a fixed size and the size, rounded by `cell_size_for`, is not it. The
    /// sweep and the verifier use the kind's size, so such a cell would overlap its neighbour.
    WrongSize(KindId),
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::OutOfMemory => f.write_str("out of GC heap memory"),
            AllocError::UnknownKind(k) => write!(f, "cell kind {} is not registered", k.get()),
            AllocError::TooLarge => f.write_str("allocation size overflows"),
            AllocError::WrongSize(k) => {
                write!(
                    f,
                    "size does not match the fixed size of cell kind {}",
                    k.get()
                )
            }
        }
    }
}

impl std::error::Error for AllocError {}

/// Counters. "last" values describe the most recent collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    pub collections: u64,
    pub last_marked_cells: usize,
    pub last_marked_bytes: usize,
    pub last_conservative_roots: usize,
    pub last_stack_words: usize,
    pub last_weak_cleared: usize,
    /// Cells freed by sweeps so far (small pages and LOS).
    pub freed_cells: u64,
    pub freed_bytes: u64,
    /// Bytes handed to allocation buffers and LOS cells so far.
    pub allocated_bytes: u64,
    pub committed_bytes: usize,
    pub small_pages: usize,
    pub large_cells: usize,
}

#[repr(C)]
struct Hot {
    barrier: BarrierState,
    _pad: [u8; 7],
    /// TLAB bump pointer (an address; 0 when there is no TLAB).
    cursor: Cell<usize>,
    /// TLAB end. 0 when there is no TLAB, and always 0 in GC-stress mode, so every
    /// allocation takes the slow path.
    limit: Cell<usize>,
}

/// The GC heap of one isolate.
///
/// `Heap` is neither `Send` nor `Sync`: one thread creates it, allocates from it and collects
/// it, and the conservative scan reads that thread's stack. Methods take `&self`; a call from
/// inside a trace function, root source or weak source panics.
#[repr(C)]
pub struct Heap {
    hot: Hot,
    base: NonNull<u8>,
    base_addr: usize,
    /// Per kind byte, for `alloc` without a borrow of `inner`: [`NOT_REGISTERED`],
    /// [`DYNAMIC_SIZE`], or the fixed size rounded by `cell_size_for`.
    kind_sizes: [Cell<usize>; 256],
    pub(crate) inner: RefCell<Inner>,
    pub(crate) roots: RefCell<Roots>,
    pub(crate) handles: RefCell<Handles>,
    pub(crate) scratch: RefCell<Vec<u64>>,
    pub(crate) scratch_depth: Cell<u32>,
    _not_send: PhantomData<*mut ()>,
}

const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(Heap, hot) == 0);
    assert!(offset_of!(Hot, barrier) + BarrierState::THRESHOLD_OFFSET == 0);
    assert!(offset_of!(Hot, cursor) == 8);
    assert!(offset_of!(Hot, limit) == 16);
};

/// `kind_sizes` entry of a kind byte with no registration.
const NOT_REGISTERED: usize = 0;
/// `kind_sizes` entry of a kind with a dynamic size. Not a multiple of 8, so no rounded size
/// equals it.
const DYNAMIC_SIZE: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub(crate) initial_budget: usize,
    pub(crate) gc_stress: u32,
    pub(crate) verify_after_gc: bool,
    pub(crate) zap: bool,
    pub(crate) record_css: bool,
}

/// Where the conservative scan reads.
#[derive(Clone, Copy, Debug)]
enum Scan {
    /// [sp, stack high) of the heap's thread, after the register spill.
    Stack(usize),
    /// A word range given by a test.
    #[cfg(test)]
    Words(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Swept {
    Free,
    Recyclable,
    Full,
}

/// Clears the mark bits of every small page and large object when dropped. `collect` holds
/// one while it marks and forgets it when marking is done. If a callback panics, the unwind
/// drops it: a cell left marked would not be traced by the next GC, so its children would be
/// freed. (Cells marked so far keep gc_state BLACK, as survivors do.)
///
/// Why a reset and not a poisoned heap: before the guard is forgotten nothing is freed, the
/// work lists are only dropped, and the weak entries cleared so far (only after a full mark)
/// held cells that are unreachable. So clean marks are the whole state to restore. The heap
/// stays usable after a caught panic, and the cost is on the panic path only.
struct UnmarkOnUnwind<'a>(&'a Memory);

impl Drop for UnmarkOnUnwind<'_> {
    fn drop(&mut self) {
        let mem = self.0;
        for idx in mem.small_pages() {
            clear_page_marks(mem.page(idx));
        }
        for idx in mem.large_heads() {
            // SAFETY: chunk `idx` is the committed head of a large object.
            let large = unsafe { Large::from_base(mem.chunk_base(idx)) };
            large.header().mark.store(0, Relaxed);
        }
    }
}

/// Clears the mark bits of one small page when dropped (`sweep_page` holds one).
struct UnmarkPageOnUnwind(Page);

impl Drop for UnmarkPageOnUnwind {
    fn drop(&mut self) {
        clear_page_marks(self.0);
    }
}

fn clear_page_marks(page: Page) {
    for w in 0..BITMAP_WORDS {
        page.mark_word(w).store(0, Relaxed);
    }
}

pub(crate) struct Inner {
    pub(crate) mem: Memory,
    pub(crate) kinds: KindTable,
    pub(crate) s: Settings,
    #[cfg(not(miri))]
    stack: ep9_platform::StackBounds,
    /// End of the main TLAB's hole (`Hot::limit` mirrors it unless GC-stress is on).
    tlab_end: usize,
    cur_page: Option<usize>,
    cur_line: usize,
    recyclable: Vec<usize>,
    sweep_queue: Vec<usize>,
    ov_cursor: usize,
    ov_end: usize,
    allocated_since_gc: usize,
    budget: usize,
    stress_count: u32,
    mark_stack: Vec<CellPtr>,
    weak_slots: Vec<NonNull<Slot>>,
    css_log: Vec<CellPtr>,
    pub(crate) stats: GcStats,
}

impl Heap {
    /// Offset of the barrier threshold byte from the start of the heap (H11 "fixed isolate
    /// offset", plus the heap's offset in the isolate).
    pub const BARRIER_THRESHOLD_OFFSET: usize = 0;
    /// Offset of the TLAB cursor (S1 inline allocation in JIT and AOT code).
    pub const TLAB_CURSOR_OFFSET: usize = 8;
    /// Offset of the TLAB limit.
    pub const TLAB_LIMIT_OFFSET: usize = 16;

    /// Reserves the cage and creates an empty heap for the current thread.
    pub fn new(config: HeapConfig) -> Result<Heap, HeapError> {
        let mem = Memory::new(&config.cage, config.max_heap_bytes)?;
        #[cfg(not(miri))]
        let stack = ep9_platform::current_thread_stack_bounds().map_err(HeapError::Stack)?;
        let base = mem.base();
        let base_addr = mem.base_addr();
        let s = Settings {
            initial_budget: config.initial_budget.max(PAGE_SIZE),
            gc_stress: config.gc_stress,
            verify_after_gc: config.verify_after_gc,
            zap: config.zap,
            record_css: config.record_conservative_roots,
        };
        Ok(Heap {
            hot: Hot {
                barrier: BarrierState::new(threshold::OFF),
                _pad: [0; 7],
                cursor: Cell::new(0),
                limit: Cell::new(0),
            },
            base,
            base_addr,
            kind_sizes: [const { Cell::new(NOT_REGISTERED) }; 256],
            inner: RefCell::new(Inner {
                mem,
                kinds: KindTable::new(),
                s,
                #[cfg(not(miri))]
                stack,
                tlab_end: 0,
                cur_page: None,
                cur_line: FIRST_LINE,
                recyclable: Vec::new(),
                sweep_queue: Vec::new(),
                ov_cursor: 0,
                ov_end: 0,
                allocated_since_gc: 0,
                budget: s.initial_budget,
                stress_count: 0,
                mark_stack: Vec::new(),
                weak_slots: Vec::new(),
                css_log: Vec::new(),
                stats: GcStats::default(),
            }),
            roots: RefCell::new(Roots::default()),
            handles: RefCell::new(Handles::default()),
            scratch: RefCell::new(Vec::new()),
            scratch_depth: Cell::new(0),
            _not_send: PhantomData,
        })
    }

    /// The barrier state that [`Slot::set`] and [`crate::set_shape`] take.
    #[inline(always)]
    pub fn barrier(&self) -> &BarrierState {
        &self.hot.barrier
    }

    /// Registers the trace and size functions of a cell kind. Needed before the first
    /// allocation of that kind.
    pub fn register_kind(&self, id: KindId, kind: CellKind) -> Result<(), KindError> {
        self.inner.borrow_mut().kinds.register(id, kind)?;
        let size = match kind.size {
            CellSize::Fixed(n) => cell_size_for(n).expect("register checked the fixed size"),
            CellSize::Dynamic(_) => DYNAMIC_SIZE,
        };
        self.kind_sizes[id.get() as usize].set(size);
        Ok(())
    }

    /// Allocates a cell of `kind` with `size` bytes (word0 included). The cell is WHITE, its
    /// ShapeId is 0, and every word after word0 holds `undefined`. The caller sets the
    /// ShapeId and the fields that the kind's size and trace functions read before the next
    /// GC point (no barrier needed: elision E2).
    ///
    /// For a kind with a fixed size, `size` must round to that size
    /// ([`AllocError::WrongSize`] otherwise).
    ///
    /// This is a GC point: it may collect before it returns.
    #[inline]
    pub fn alloc(&self, kind: KindId, size: usize) -> Result<CellPtr, AllocError> {
        let kind_size = self.kind_sizes[kind.get() as usize].get();
        if kind_size == NOT_REGISTERED {
            return Err(AllocError::UnknownKind(kind));
        }
        let Some(size) = cell_size_for(size) else {
            return Err(AllocError::TooLarge);
        };
        // Every path below (TLAB, overflow TLAB, LOS) reserves `size` bytes; the sweep uses the
        // kind's size, so they must agree.
        if kind_size != DYNAMIC_SIZE && kind_size != size {
            return Err(AllocError::WrongSize(kind));
        }
        if size <= MAX_SMALL_SIZE {
            let cur = self.hot.cursor.get();
            let end = cur.wrapping_add(size);
            if end <= self.hot.limit.get() && end > cur {
                self.hot.cursor.set(end);
                // SAFETY: [cur, end) lies in the current TLAB hole: committed, inside a small
                // page, holding no cell.
                return Ok(unsafe { self.init_cell(cur, kind, size, true) });
            }
        }
        self.alloc_slow(kind, size)
    }

    #[cold]
    #[inline(never)]
    fn alloc_slow(&self, kind: KindId, size: usize) -> Result<CellPtr, AllocError> {
        let trigger = self.inner.borrow_mut().gc_trigger(size);
        if trigger {
            self.collect();
        }
        let mut collected = trigger;
        loop {
            let placed = self.inner.borrow_mut().alloc_in(&self.hot, size);
            match placed {
                Some((addr, small)) => {
                    // SAFETY: `alloc_in` returned free, committed memory of this heap for a
                    // cell of `size`: a TLAB hole of a small page, or a new LOS cell area.
                    return Ok(unsafe { self.init_cell(addr, kind, size, small) });
                }
                None if !collected => {
                    self.collect();
                    collected = true;
                }
                None => return Err(AllocError::OutOfMemory),
            }
        }
    }

    /// Writes word0, pre-fills the other words with `undefined`, and sets the VO bit.
    ///
    /// # Safety
    /// `[addr, addr + size)` is committed memory of this heap that holds no cell: a TLAB hole
    /// of a small page (`small`) or the cell area of a new large object.
    #[inline(always)]
    unsafe fn init_cell(&self, addr: usize, kind: KindId, size: usize, small: bool) -> CellPtr {
        // SAFETY: `addr` is inside the reservation (caller contract), so the offset is in
        // bounds of the provenance of `base`.
        let p = unsafe { self.base.add(addr - self.base_addr) };
        let w = p.cast::<u64>();
        // SAFETY: the caller guarantees `size` bytes of unused, committed, 8-aligned memory.
        // No reference to it exists yet; the GC reads it only after the VO bit is set below.
        unsafe {
            w.cast::<Word0>()
                .write(Word0::new(ShapeId::new(0), gc_state::WHITE, kind.get(), 0));
            for i in 1..size / 8 {
                w.add(i).write(VALUE_UNDEFINED);
            }
        }
        if small {
            // SAFETY: a small cell lies in a committed small page; its base is the address
            // masked to 256 KiB, inside the reservation.
            let page =
                unsafe { Page::from_base(self.base.add((addr & PAGE_MASK) - self.base_addr)) };
            page.set_vo(addr);
        }
        // SAFETY: a new cell of this heap starts at `p`.
        unsafe { CellPtr::from_raw(p.cast()) }
    }

    /// A full stop-the-world collection. It spills the callee-saved registers and scans this
    /// thread's stack conservatively, so every cell that a Rust local holds stays alive.
    pub fn collect(&self) {
        #[cfg(not(miri))]
        ep9_platform::with_spilled_registers(|regs| self.collect_at(regs.sp()));
        #[cfg(miri)]
        self.collect_scanning(Scan::Stack(0));
    }

    #[cfg_attr(miri, allow(dead_code))]
    #[inline(never)]
    fn collect_at(&self, sp: usize) {
        self.collect_scanning(Scan::Stack(sp));
    }

    /// A collection whose conservative scan reads `words` instead of the stack (tests only:
    /// deterministic CSS cases).
    #[cfg(test)]
    pub(crate) fn collect_with_words(&self, words: &[usize]) {
        let lo = words.as_ptr().addr();
        self.collect_scanning(Scan::Words(lo, lo + words.len() * 8));
    }

    fn collect_scanning(&self, scan: Scan) {
        let verify = {
            let mut inner_ref = self.inner.borrow_mut();
            let inner = &mut *inner_ref;
            let mut roots = self.roots.borrow_mut();
            let mut handles = self.handles.borrow_mut();
            let scratch = self.scratch.borrow();

            // Retire the TLABs: the VO bits already cover every bump-allocated cell.
            self.hot.cursor.set(0);
            self.hot.limit.set(0);
            inner.tlab_end = 0;
            inner.cur_page = None;
            inner.ov_cursor = 0;
            inner.ov_end = 0;
            // Mark bits and VO bits must be exact before marking.
            inner.finish_sweeping();
            inner.recyclable.clear();

            // From here until the weak sources are done, a root source, trace, size or weak
            // function may panic. The unwind then drops this guard, which clears every mark
            // bit, so the next GC starts from clean marks. Nothing is freed before the guard
            // is forgotten.
            let unmark = UnmarkOnUnwind(&inner.mem);
            let mut log = core::mem::take(&mut inner.css_log);
            log.clear();
            let record = inner.s.record_css;
            let mut t = Tracer::new(
                &inner.mem,
                &inner.kinds,
                Mode::Mark,
                core::mem::take(&mut inner.mark_stack),
                core::mem::take(&mut inner.weak_slots),
            );

            let stack_words = match scan {
                #[cfg(not(miri))]
                Scan::Stack(sp) => {
                    assert!(
                        inner.stack.contains(sp),
                        "collect() on a thread other than the heap's"
                    );
                    // SAFETY: `sp` comes from the spill trampoline on this thread, so
                    // [sp, stack high) is the live, readable part of this thread's stack.
                    unsafe {
                        crate::css::scan_range(
                            &mut t,
                            sp,
                            inner.stack.high,
                            record.then_some(&mut log),
                        )
                    }
                }
                #[cfg(miri)]
                Scan::Stack(_) => {
                    let _ = record;
                    0
                }
                #[cfg(all(test, not(miri)))]
                Scan::Words(lo, hi) => {
                    // SAFETY: [lo, hi) is the test's word slice: aligned and readable.
                    unsafe { crate::css::scan_range(&mut t, lo, hi, record.then_some(&mut log)) }
                }
                #[cfg(all(test, miri))]
                Scan::Words(..) => 0,
            };
            let css_roots = t.pending();

            handles.visit_strong(&mut t);
            t.context = crate::trace::Context::Root("scratch roots");
            for &b in scratch.iter() {
                t.visit_bits(b);
            }
            roots.visit_strong(&mut t, false);
            t.drain();

            let mut weak_cleared = t.clear_weak_slots();
            let marked_cells = t.marked_cells;
            let marked_bytes = t.marked_bytes;
            let (stack, weak) = t.into_lists();
            inner.mark_stack = stack;
            inner.weak_slots = weak;
            inner.css_log = log;

            let mut w = WeakVisitor::new(&inner.mem);
            handles.clear_dead(&mut w);
            roots.visit_weak(&mut w);
            weak_cleared += w.cleared;
            core::mem::forget(unmark);

            inner.sweep_large();
            inner.sweep_queue = inner.mem.small_pages().collect();
            inner.sweep_queue.reverse(); // pop() takes low pages first

            inner.budget = inner.s.initial_budget.max(marked_bytes);
            inner.allocated_since_gc = 0;
            let st = &mut inner.stats;
            st.collections += 1;
            st.last_marked_cells = marked_cells;
            st.last_marked_bytes = marked_bytes;
            st.last_conservative_roots = css_roots;
            st.last_stack_words = stack_words;
            st.last_weak_cleared = weak_cleared;
            inner.s.verify_after_gc
        };
        if verify && let Err(e) = self.verify() {
            panic!("heap verifier failed after GC: {e}");
        }
    }

    /// Sweeps every page that the lazy sweep has not reached yet. After this, VO bits match
    /// the live cells exactly.
    pub fn finish_sweeping(&self) {
        self.inner.borrow_mut().finish_sweeping();
    }

    /// Turns GC-stress on (`n` > 0: collect before every nth allocation) or off (0).
    pub fn set_gc_stress(&self, n: u32) {
        let mut inner = self.inner.borrow_mut();
        inner.s.gc_stress = n;
        inner.stress_count = 0;
        inner.sync_limit(&self.hot);
    }

    pub fn set_verify_after_gc(&self, on: bool) {
        self.inner.borrow_mut().s.verify_after_gc = on;
    }

    pub fn stats(&self) -> GcStats {
        let inner = self.inner.borrow();
        let mut s = inner.stats.clone();
        s.committed_bytes = inner.mem.committed_bytes();
        s.small_pages = inner.mem.small_pages().count();
        s.large_cells = inner.mem.large_heads().count();
        s
    }

    /// Cells that the last conservative scan marked first (only with
    /// [`HeapConfig::record_conservative_roots`]).
    pub fn last_conservative_roots(&self) -> Vec<CellPtr> {
        self.inner.borrow().css_log.clone()
    }

    /// True if a live cell starts exactly at `addr`. Finishes the lazy sweep first.
    pub fn is_cell(&self, addr: usize) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.finish_sweeping();
        crate::trace::cell_start(&inner.mem, addr).is_some()
    }

    /// The live cell that contains `addr` (interior pointers too), as the conservative scan
    /// finds it. Finishes the lazy sweep first.
    pub fn find_cell(&self, addr: usize) -> Option<CellPtr> {
        let mut inner = self.inner.borrow_mut();
        inner.finish_sweeping();
        find_cell(&inner.mem, &inner.kinds, addr)
    }

    /// Allocated size of a cell.
    ///
    /// # Safety
    /// `cell` is a live cell of this heap.
    pub unsafe fn cell_size(&self, cell: CellPtr) -> usize {
        self.inner
            .borrow()
            .kinds
            .cell_size(cell)
            .expect("cell has an unregistered kind")
    }

    /// True if `addr` is inside this heap's reservation (the in-cage test).
    pub fn contains(&self, addr: usize) -> bool {
        self.inner.borrow().mem.contains(addr)
    }

    // ---- roots ----

    /// A strong handle (`Global<T>` storage).
    pub fn new_global(&self, v: Value) -> GlobalHandle {
        self.handles.borrow_mut().new_global(v)
    }

    pub fn global(&self, h: GlobalHandle) -> Value {
        self.handles.borrow().global(h)
    }

    /// Stores into a handle. No barrier: handles are roots (E3).
    pub fn set_global(&self, h: GlobalHandle, v: Value) {
        self.handles.borrow_mut().set_global(h, v);
    }

    pub fn drop_global(&self, h: GlobalHandle) {
        self.handles.borrow_mut().release(h.0, false);
    }

    /// A weak handle. It reads `None` after a GC in which its cell died.
    pub fn new_weak(&self, v: Value) -> WeakHandle {
        self.handles.borrow_mut().new_weak(v)
    }

    pub fn weak(&self, h: WeakHandle) -> Option<Value> {
        self.handles.borrow().weak(h)
    }

    pub fn drop_weak(&self, h: WeakHandle) {
        self.handles.borrow_mut().release(h.0, true);
    }

    /// Live handles (strong and weak).
    pub fn handle_count(&self) -> usize {
        self.handles.borrow().len()
    }

    /// Opens a scope of precise roots for Rust temporaries.
    pub fn scratch_roots(&self) -> ScratchRoots<'_> {
        let depth = self.scratch_depth.get() + 1;
        self.scratch_depth.set(depth);
        ScratchRoots {
            heap: self,
            base: self.scratch.borrow().len(),
            depth,
        }
    }

    /// Registers a precise root source (for example the VM register stack). Every collection
    /// calls it; the S2 fixpoint may call it more than once per collection.
    pub fn add_root_source(&self, f: impl FnMut(&mut Tracer<'_>) + 'static) -> RootId {
        self.roots.borrow_mut().add_source(Box::new(f))
    }

    /// Registers a weak source. It runs after marking; it clears dead entries through the
    /// [`WeakVisitor`].
    pub fn add_weak_source(&self, f: impl FnMut(&mut WeakVisitor<'_>) + 'static) -> RootId {
        self.roots.borrow_mut().add_weak_source(Box::new(f))
    }

    /// The root-range hook for unit RW data (03-gc §5): the slots of `slots`, visited strongly
    /// (link cells) or weakly (IC cells). Take the pointer from the whole slice
    /// (`NonNull::from(&cells[..])`), not from its first element.
    ///
    /// # Safety
    /// `slots` stays valid for reads and holds initialized `Slot`s until [`Heap::remove_root`]
    /// removes this range. No slot holds the bits 0.
    pub unsafe fn add_root_range(&self, slots: NonNull<[Slot]>, strength: RootStrength) -> RootId {
        self.roots.borrow_mut().add_range(slots, strength)
    }

    /// Removes a root source, weak source or range. False if the id is unknown.
    pub fn remove_root(&self, id: RootId) -> bool {
        self.roots.borrow_mut().remove(id)
    }
}

impl Inner {
    /// Called on every slow-path allocation: true if a GC must run first.
    fn gc_trigger(&mut self, size: usize) -> bool {
        if self.s.gc_stress > 0 {
            self.stress_count += 1;
            if self.stress_count >= self.s.gc_stress {
                self.stress_count = 0;
                return true;
            }
            return false;
        }
        self.allocated_since_gc + size > self.budget
    }

    fn sync_limit(&self, hot: &Hot) {
        hot.limit.set(if self.s.gc_stress > 0 {
            0
        } else {
            self.tlab_end
        });
    }

    /// Finds memory for a cell of `size` (already rounded). Returns (address, small).
    fn alloc_in(&mut self, hot: &Hot, size: usize) -> Option<(usize, bool)> {
        if size > MAX_SMALL_SIZE {
            return self.alloc_large(size).map(|a| (a, false));
        }
        let cur = hot.cursor.get();
        if cur != 0 && cur + size <= self.tlab_end {
            hot.cursor.set(cur + size);
            return Some((cur, true));
        }
        if size > MAX_LINE_CELL {
            if self.ov_cursor == 0 || self.ov_cursor + size > self.ov_end {
                let idx = self.fresh_page()?;
                let base = self.mem.page(idx).addr();
                self.ov_cursor = base + FIRST_CELL_OFFSET;
                self.ov_end = base + PAGE_SIZE;
                self.note_allocated(PAGE_CELL_BYTES);
            }
            let a = self.ov_cursor;
            self.ov_cursor += size;
            return Some((a, true));
        }
        let (start, end) = self.next_hole()?;
        self.note_allocated(end - start);
        debug_assert!(end - start >= LINE_SIZE && size <= LINE_SIZE);
        hot.cursor.set(start + size);
        self.tlab_end = end;
        self.sync_limit(hot);
        Some((start, true))
    }

    fn note_allocated(&mut self, bytes: usize) {
        self.allocated_since_gc += bytes;
        self.stats.allocated_bytes += bytes as u64;
    }

    fn alloc_large(&mut self, size: usize) -> Option<usize> {
        let total = LARGE_CELL_OFFSET.checked_add(size)?;
        let chunks = total.div_ceil(PAGE_SIZE);
        let committed = total.next_multiple_of(self.mem.os_page());
        let idx = self.mem.alloc_large(chunks, committed)?;
        let base = self.mem.chunk_base(idx);
        // SAFETY: the chunk run was just committed for this object and nothing else uses it.
        let large = unsafe { Large::init(base, idx as u32, chunks as u32, size, committed) };
        self.note_allocated(size);
        Some(large.cell_addr())
    }

    /// A new small page from the chunk pool, with fresh metadata.
    fn fresh_page(&mut self) -> Option<usize> {
        let idx = self.mem.alloc_small_page()?;
        self.mem.page(idx).init(idx as u32);
        Some(idx)
    }

    /// The next free-line run for the main TLAB.
    fn next_hole(&mut self) -> Option<(usize, usize)> {
        loop {
            if let Some(idx) = self.cur_page {
                if let Some(h) = self.find_hole(idx) {
                    return Some(h);
                }
                self.cur_page = None;
            }
            if let Some(idx) = self.recyclable.pop() {
                self.cur_page = Some(idx);
                self.cur_line = FIRST_LINE;
                continue;
            }
            if let Some(idx) = self.sweep_queue.pop() {
                if self.sweep_page(idx) == Swept::Recyclable {
                    self.cur_page = Some(idx);
                    self.cur_line = FIRST_LINE;
                }
                continue;
            }
            let idx = self.fresh_page()?;
            self.cur_page = Some(idx);
            self.cur_line = FIRST_LINE;
        }
    }

    fn find_hole(&mut self, idx: usize) -> Option<(usize, usize)> {
        let page = self.mem.page(idx);
        let mut l = self.cur_line;
        while l < LINES_PER_PAGE && page.line_used(l) {
            l += 1;
        }
        if l >= LINES_PER_PAGE {
            self.cur_line = LINES_PER_PAGE;
            return None;
        }
        let start = l;
        while l < LINES_PER_PAGE && !page.line_used(l) {
            l += 1;
        }
        self.cur_line = l;
        Some((page.addr() + start * LINE_SIZE, page.addr() + l * LINE_SIZE))
    }

    pub(crate) fn finish_sweeping(&mut self) {
        while let Some(idx) = self.sweep_queue.pop() {
            if self.sweep_page(idx) == Swept::Recyclable {
                self.recyclable.push(idx);
            }
        }
    }

    /// Lazy sweep of one small page: frees unmarked cells (VO bit cleared, zapped), clears the
    /// mark bits and recomputes the line marks from the live cells.
    ///
    /// The caller has taken the page off the sweep queue, so no allocation list holds it. If
    /// a size function panics, the unwind clears the page's mark bits; its dead cells that
    /// still have a VO bit are not zapped yet, and the next GC frees them.
    fn sweep_page(&mut self, idx: usize) -> Swept {
        let page = self.mem.page(idx);
        let unmark = UnmarkPageOnUnwind(page);
        // SAFETY: the line-mark array lies in the page's committed metadata area.
        unsafe { page.line_marks().write_bytes(0, LINES_PER_PAGE) };
        let mut live = 0usize;
        for w in 0..crate::layout::BITMAP_WORDS {
            let vo = page.vo_word(w).load(Relaxed);
            if vo == 0 {
                continue;
            }
            let mk = page.mark_word(w).load(Relaxed);
            debug_assert_eq!(mk & !vo, 0, "mark bit without a VO bit");
            let mut alive = vo & mk;
            while alive != 0 {
                let bit = alive.trailing_zeros() as usize;
                alive &= alive - 1;
                let addr = page.addr() + (w * 64 + bit) * 8;
                let size = self.size_at(addr);
                let first = (addr - page.addr()) / LINE_SIZE;
                let last = (addr + size - 1 - page.addr()) / LINE_SIZE;
                // SAFETY: lines first..=last are inside the line-mark array (a cell never
                // crosses the page end).
                unsafe {
                    page.line_marks()
                        .add(first)
                        .write_bytes(1, last - first + 1)
                };
                live += 1;
            }
            // Free the dead cells before zapping them: no VO bit is left on a zapped cell.
            page.vo_word(w).store(vo & mk, Relaxed);
            let mut dead = vo & !mk;
            while dead != 0 {
                let bit = dead.trailing_zeros() as usize;
                dead &= dead - 1;
                let addr = page.addr() + (w * 64 + bit) * 8;
                let size = self.size_at(addr);
                if self.s.zap {
                    let p = self.mem.ptr(addr).cast::<u64>();
                    for i in 0..size / 8 {
                        // SAFETY: the dead cell [addr, addr + size) is committed memory that no
                        // live cell or root reaches; nothing reads it after this sweep.
                        unsafe { p.add(i).write(ZAP_WORD) };
                    }
                }
                self.stats.freed_cells += 1;
                self.stats.freed_bytes += size as u64;
            }
            page.mark_word(w).store(0, Relaxed);
        }
        core::mem::forget(unmark);
        if live == 0 {
            self.mem.free_small_page(idx);
            return Swept::Free;
        }
        if (FIRST_LINE..LINES_PER_PAGE).any(|l| !page.line_used(l)) {
            Swept::Recyclable
        } else {
            Swept::Full
        }
    }

    /// Size of the cell at `addr` (a VO start). A cell with no registered kind is a heap
    /// corruption.
    fn size_at(&self, addr: usize) -> usize {
        // SAFETY: a VO bit marks a cell start whose word0 the allocator wrote.
        let c = unsafe { CellPtr::from_raw(self.mem.ptr(addr).cast()) };
        self.kinds
            .cell_size(c)
            .unwrap_or_else(|| panic!("heap corrupt: cell {c:?} has an unregistered kind"))
    }

    /// Frees every unmarked large object and clears the marks of the others.
    fn sweep_large(&mut self) {
        let heads: Vec<usize> = self.mem.large_heads().collect();
        for idx in heads {
            // SAFETY: chunk `idx` is the committed head of a large object.
            let large = unsafe { Large::from_base(self.mem.chunk_base(idx)) };
            let h = large.header();
            if h.mark.swap(0, Relaxed) != 0 {
                continue;
            }
            let (size, committed) = (h.size, h.committed);
            self.stats.freed_cells += 1;
            self.stats.freed_bytes += size as u64;
            // SAFETY: the object is unmarked: no root and no live cell reaches it. Its memory
            // is decommitted, so a stale access faults instead of reading old data.
            unsafe { self.mem.free_large(idx, committed) };
        }
    }
}

#[cfg(test)]
impl Heap {
    /// True if a cell starting at `addr` survived the last GC, without sweeping: on a page that
    /// waits for its lazy sweep, the mark bit decides.
    pub(crate) fn live_after_gc(&self, addr: usize) -> bool {
        use crate::memory::Chunk;
        let inner = self.inner.borrow();
        let Some(idx) = inner.mem.chunk_index(addr) else {
            return false;
        };
        match inner.mem.chunk(idx) {
            Chunk::Small => {
                let page = inner.mem.page(idx);
                if addr - page.addr() < FIRST_CELL_OFFSET || !page.vo(addr) {
                    return false;
                }
                !inner.sweep_queue.contains(&idx) || page.is_marked(addr)
            }
            Chunk::LargeHead { .. } => crate::trace::cell_start(&inner.mem, addr).is_some(),
            _ => false,
        }
    }

    /// Small pages still waiting for the lazy sweep.
    pub(crate) fn unswept_pages(&self) -> usize {
        self.inner.borrow().sweep_queue.len()
    }
}
