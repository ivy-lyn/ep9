//! ArrayBuffer and SharedArrayBuffer data (ECMA-262 §25.1, §25.2; TypedArray TA-1 TA-VM,
//! design `docs/plan/notes/typedarray-design.md` §2.2, §2.3, §5): the slots of a buffer, the
//! BYTES cell, the off-heap stores and the isolate [`BufferTable`], and the byte access that
//! the views (`typed_array.rs`), the DataView and the Atomics share.
//!
//! ```text
//! ArrayBuffer / SAB (SLOT_OBJECT, ObjectClass 5 / 28)       Runtime.buffers: BufferTable (Rust memory)
//!   slot 0 store: BYTES cell | int32 entry | undefined     entries[i]: owner (weak), store, views (weak)
//!   slot 1 byteLength  2 maxByteLength (undefined: fixed)      store: Resizable(Reservation) | Shared(Arc)
//!   slot 3 detach key  4 flags (int32)                     one weak source (every GC kind):
//!   slot 5 its own entry index (int32)                       owner dead -> remove its external bytes,
//! DataView (SLOT_OBJECT, ObjectClass 29)                       free the entry (the store is dropped
//!   slot 4 its own entry index (int32; no store, no views)     after the GC); else drop dead views
//!
//!   BYTES (kind 85): +8 byte length, +16 bytes (zeroed)
//! ```
//!
//! | Buffer | Store | Data address | Freed by |
//! |---|---|---|---|
//! | fixed-length ArrayBuffer | a BYTES cell in slot 0 (on the heap; counts against `max_heap_bytes`) | the cell + 16 | the GC (a dead cell) |
//! | zero-length or detached ArrayBuffer | none (slot 0 undefined) | null | - |
//! | resizable ArrayBuffer | a [`Reservation`] of the maximum; the committed pages are the heap's external bytes | the reservation base (never moves) | the table's weak source |
//! | SharedArrayBuffer | an `Arc<`[`SharedBlock`]`>` (a reservation, the byte length as an atomic) | the block base | the last `Arc` |
//!
//! Rules:
//! - **GC (plan rule 8).** Rust memory holds a buffer and its views only weakly, in the
//!   table, which one weak source sweeps in every GC kind (minor too). The table is indexed by
//!   the int32 in slot 5 (a DataView: slot 4), never by an address (H12).
//! - **The brand (TA-1 fixer r3; Codex TA-1 r1 finding 1).** Every buffer and DataView that
//!   this file makes (an engine object) owns one table entry from birth: the maker inserts it
//!   right after the allocation (no GC point between them), and only `BufferTable::insert`
//!   sets an owner. [`buffer_class`] and [`is_data_view`] test that the entry that the table
//!   slot names has the object as its owner, so an object of these classes that the generic
//!   slot API makes (any slot count, any slot values) is not a buffer or a DataView here: a
//!   TypeError. The weak source frees a dead owner's entry in the GC that finds it dead, before
//!   the sweep can give its address again, so a new object never finds itself the owner of an
//!   old entry.
//! - **External bytes (needs N1).** A new off-heap store: the buffer is made and held in a Rust
//!   local first, then `Heap::add_external_bytes` (a GC point) runs with no borrow of the table,
//!   and only after `Ok` the store goes into the table; on `Err` the store is dropped. The weak
//!   source calls `Heap::remove_external_bytes` for a dead buffer's bytes (it takes no heap
//!   borrow). A dead store is dropped (unmapped) at the next table call, after the GC.
//! - **Detach (TA-D5, design §5.2).** Every live view of the buffer gets a null data word; a
//!   TYPED_FIXED view also gets length 0 and a fixed DataView byte length 0. A TYPED_VARIABLE
//!   view keeps its length field (its bound reads the buffer's byte length, 0 once detached).
//!   Then the buffer is flagged detached, its byte length is 0, and the store goes.
//! - **Resize.** A resizable store keeps its base, so the views keep their data word; the bytes
//!   from the byte length to the committed end are always 0 (a shrink zeroes them), so a later
//!   grow reads zeros.
//! - **Shared bytes (TA-D11).** Every access to SAB bytes is atomic: Relaxed for element and
//!   DataView access (a byte loop when unaligned), SeqCst for Atomics.
//! - **Caps (design §2.3, est.).** A `maxByteLength` above [`MAX_RESERVATION`] is a RangeError;
//!   a fixed buffer above `max_heap_bytes` or that the heap cannot allocate is a RangeError
//!   (CreateByteDataBlock).
//! - **Policy (M-R9).** Nothing here reads `ExecPolicy`, `ExecOptions` or the code set.

use core::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{
    AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering, Ordering::Relaxed,
};
use std::sync::{Arc, Mutex, PoisonError};

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};
use ep9_platform::va::{Reservation, allocation_granularity, page_size};

use crate::abi::ObjectClass;
use crate::external_ref::ExternalRef;
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::kinds;
use crate::layout;
use crate::object::{self, cell, init, rd};
use crate::runtime::{Runtime, val};

/// The internal slots of an ArrayBuffer or a SharedArrayBuffer (design §2.2).
pub(crate) mod slot {
    /// The store: a BYTES cell, the int32 entry index of an off-heap store, or undefined (no
    /// bytes: zero length or detached).
    pub const STORE: usize = 0;
    /// `[[ArrayBufferByteLength]]` (a number; 0 once detached). A SharedArrayBuffer's length
    /// is in its block (it can grow from another isolate); this slot keeps its first length.
    pub const BYTE_LENGTH: usize = 1;
    /// `[[ArrayBufferMaxByteLength]]`: a number, or undefined for a fixed-length buffer.
    pub const MAX_BYTE_LENGTH: usize = 2;
    /// `[[ArrayBufferDetachKey]]` (undefined by default).
    pub const DETACH_KEY: usize = 3;
    /// The [`super::flag`] bits (an int32).
    pub const FLAGS: usize = 4;
    /// The int32 index of the buffer's own [`super::BufferTable`] entry: the brand of
    /// [`super::buffer_class`].
    pub const TABLE: usize = 5;
    /// The slot count.
    pub const COUNT: usize = 6;
}

/// The bits of [`slot::FLAGS`].
pub(crate) mod flag {
    /// The buffer is detached (IsDetachedBuffer).
    pub const DETACHED: i32 = 1;
    /// A resizable ArrayBuffer or a growable SharedArrayBuffer (not IsFixedLengthArrayBuffer).
    pub const RESIZABLE: i32 = 2;
}

/// The internal slots of a DataView (design §2.2; ObjectClass 29).
pub(crate) mod dv_slot {
    /// `[[ViewedArrayBuffer]]`.
    pub const BUFFER: usize = 0;
    /// `[[ByteOffset]]` (a number).
    pub const BYTE_OFFSET: usize = 1;
    /// `[[ByteLength]]`: a number, or the int32 -1 (`layout::VIEW_LENGTH_AUTO`) for auto.
    pub const BYTE_LENGTH: usize = 2;
    /// The data address (an `ExternalRef<u8>` native word): the store base + the byte offset,
    /// null when detached (the H25 inventory).
    pub const DATA: usize = 3;
    /// The int32 index of the DataView's own [`super::BufferTable`] entry (no store, no
    /// views): the brand of [`super::is_data_view`].
    pub const TABLE: usize = 4;
    /// The slot count.
    pub const COUNT: usize = 5;
}

/// The largest `maxByteLength` of a resizable or growable buffer (design §2.3, est.: a T0
/// tuning fact, not a rule for other tiers): 2^32 bytes of address space are reserved at most.
pub(crate) const MAX_RESERVATION: u64 = 1 << 32;

/// Why a buffer operation failed: the caller throws the error of that kind with the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    Type(&'static str),
    Range(&'static str),
}

pub(crate) type BufResult<T> = Result<T, Fail>;

// ---- the BYTES cell (kind 85) --------------------------------------------------------------

/// The byte length of a BYTES cell (+8, a number Value).
fn bytes_len(c: CellPtr) -> usize {
    val(rd(c, layout::BYTES_LENGTH as usize))
        .as_number()
        .expect("a BYTES length") as usize
}

fn bytes_size(c: CellPtr) -> usize {
    layout::BYTES_DATA as usize + bytes_len(c).next_multiple_of(8)
}

/// The bytes are not Values: the GC visits nothing (as ELEMENTS_DOUBLE).
fn trace_bytes(_c: CellPtr, _t: &mut Tracer<'_>) {}

/// Registers the BYTES kind (85). `object::register_kinds` calls it.
pub(crate) fn register_kind(heap: &Heap) {
    heap.register_kind(
        kinds::BYTES,
        CellKind {
            name: "bytes",
            size: CellSize::Dynamic(bytes_size),
            trace: trace_bytes,
        },
    )
    .expect("register BYTES");
}

/// A new BYTES cell of `len` zero bytes, or `None` if the heap cannot give it. GC point.
fn new_bytes(rt: &Runtime, len: usize) -> Option<u64> {
    if len as u64 > rt.heap.max_heap_bytes() as u64 {
        return None;
    }
    let size = (layout::BYTES_DATA as usize).checked_add(len.checked_next_multiple_of(8)?)?;
    let c = rt.heap.alloc(kinds::BYTES, size).ok()?;
    init(
        c,
        layout::BYTES_LENGTH as usize,
        crate::conv::num(len as f64),
    );
    // SAFETY: `c` is the new live BYTES cell of `size` bytes; [16, size) holds no Value.
    unsafe {
        core::ptr::write_bytes(
            c.field::<u8>(layout::BYTES_DATA as usize),
            0,
            size - layout::BYTES_DATA as usize,
        );
    }
    Some(c.to_value().to_bits())
}

/// The address of byte 0 of the BYTES cell `b`.
fn bytes_data(b: u64) -> *mut u8 {
    debug_assert_eq!(kinds::cell_kind(b), kinds::BYTES.get());
    // SAFETY: `b` is a live BYTES cell; +16 is its first data byte.
    unsafe { cell(b).field::<u8>(layout::BYTES_DATA as usize) }
}

// ---- the off-heap stores and the BufferTable ----------------------------------------------

/// The bytes of a SharedArrayBuffer (§25.2): reserved to the maximum, committed up to the
/// byte length (page multiples), shared through an `Arc` by every isolate that holds the SAB
/// (TA-D11). The byte length is read and written with order seq-cst.
#[derive(Debug)]
pub(crate) struct SharedBlock {
    mem: Reservation,
    len: AtomicUsize,
    /// The maximum of a growable SAB, else `None`.
    max: Option<usize>,
    /// The committed bytes; the lock also orders grows.
    committed: Mutex<usize>,
    /// A number no other block of the process gets (the waiter-list key of TA-ATOMICS).
    serial: u64,
}

/// The next [`SharedBlock::serial`].
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

impl SharedBlock {
    /// The byte length (seq-cst).
    pub(crate) fn byte_length(&self) -> usize {
        self.len.load(Ordering::SeqCst)
    }

    /// The process-wide serial number of the block (never an address).
    #[expect(dead_code, reason = "first reader: TA-4 TA-ATOMICS (the waiter list)")]
    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    fn base(&self) -> *mut u8 {
        core::ptr::with_exposed_provenance_mut(self.mem.base())
    }
}

/// An off-heap store of the table.
#[derive(Debug)]
enum Store {
    /// A resizable ArrayBuffer: the reservation and its committed bytes.
    Resizable { mem: Reservation, committed: usize },
    /// A SharedArrayBuffer's block.
    Shared(Arc<SharedBlock>),
}

impl Store {
    fn base(&self) -> *mut u8 {
        match self {
            Store::Resizable { mem, .. } => core::ptr::with_exposed_provenance_mut(mem.base()),
            Store::Shared(b) => b.base(),
        }
    }
}

/// One table entry: a buffer (held weakly), its off-heap store and its views (held weakly).
#[derive(Debug)]
struct Entry {
    /// The buffer's Value bits; UNDEFINED for a free entry.
    owner: u64,
    store: Option<Store>,
    /// The external bytes this entry reported to the heap (`Heap::add_external_bytes`).
    external: usize,
    /// The live views (TypedArrays and DataViews) on the buffer, as of the last GC plus the
    /// ones made since. Not a cache: the list never refuses work (design §5.1).
    views: Vec<u64>,
}

#[derive(Debug, Default)]
struct Table {
    entries: Vec<Entry>,
    free: Vec<u32>,
    /// The stores of dead buffers: dropped (unmapped) at the next table call, after the GC.
    dead: Vec<Store>,
}

/// The isolate's buffer table (`Runtime::buffers`; design §5.1): Rust memory with one weak
/// source. No Value lives here strongly (plan rule 8).
pub(crate) struct BufferTable {
    inner: Rc<RefCell<Table>>,
    /// The isolate's heap, for the weak source's `remove_external_bytes`: set by the first
    /// table call that adds bytes (the `Runtime` is boxed, so the address stays).
    heap: Rc<Cell<*const Heap>>,
}

impl BufferTable {
    /// The empty table of a new isolate; registers its weak source with `heap`.
    pub(crate) fn new(heap: &Heap) -> BufferTable {
        let inner = Rc::new(RefCell::new(Table::default()));
        let hp: Rc<Cell<*const Heap>> = Rc::new(Cell::new(core::ptr::null()));
        let table = Rc::clone(&inner);
        let heap_ptr = Rc::clone(&hp);
        heap.add_weak_source(move |w| {
            let mut t = table
                .try_borrow_mut()
                .expect("the BufferTable is borrowed across a GC point");
            let t = &mut *t;
            for (i, e) in t.entries.iter_mut().enumerate() {
                if e.owner == UNDEFINED {
                    continue;
                }
                if w.is_live(val(e.owner)) {
                    e.views.retain(|&v| w.is_live(val(v)));
                    continue;
                }
                if e.external > 0 {
                    let h = heap_ptr.get();
                    assert!(!h.is_null(), "external bytes with no heap");
                    // SAFETY: the heap owns this weak source and runs it only while it is
                    // alive; `h` is the address of the `Runtime`'s heap (boxed, never moved).
                    // `remove_external_bytes` takes no heap borrow, so it may run in a GC.
                    unsafe { &*h }.remove_external_bytes(e.external);
                }
                if let Some(s) = e.store.take() {
                    t.dead.push(s);
                }
                *e = Entry {
                    owner: UNDEFINED,
                    store: None,
                    external: 0,
                    views: Vec::new(),
                };
                t.free.push(i as u32);
            }
        });
        BufferTable { inner, heap: hp }
    }

    /// Drops the stores of buffers that died (outside a GC). Not a GC point.
    fn drain_dead(&self) {
        let dead = core::mem::take(&mut self.inner.borrow_mut().dead);
        drop(dead);
    }

    /// A new entry for `owner` (no store, no views); its index. Not a GC point. The only
    /// writer of an owner: its callers are the makers of this file, right after they allocate
    /// `owner`.
    fn insert(&self, owner: u64, store: Option<Store>, external: usize) -> i32 {
        let mut t = self.inner.borrow_mut();
        let e = Entry {
            owner,
            store,
            external,
            views: Vec::new(),
        };
        let i = match t.free.pop() {
            Some(i) => {
                t.entries[i as usize] = e;
                i as usize
            }
            None => {
                t.entries.push(e);
                t.entries.len() - 1
            }
        };
        i32::try_from(i).expect("fewer than 2^31 buffers with a table entry")
    }

    /// Puts `store` and its external byte count into entry `i` (made with no store at its
    /// owner's birth). Not a GC point.
    fn set_store(&self, i: usize, store: Option<Store>, external: usize) {
        let mut t = self.inner.borrow_mut();
        let e = &mut t.entries[i];
        debug_assert!(e.store.is_none() && e.external == 0);
        e.store = store;
        e.external = external;
    }

    /// True if entry `i` exists and `o` is its owner (the brand). Not a GC point.
    pub(crate) fn owns(&self, i: usize, o: u64) -> bool {
        self.inner
            .borrow()
            .entries
            .get(i)
            .is_some_and(|e| e.owner == o)
    }

    /// The number of live entries (tests).
    #[cfg(test)]
    pub(crate) fn live_entries(&self) -> usize {
        self.inner
            .borrow()
            .entries
            .iter()
            .filter(|e| e.owner != UNDEFINED)
            .count()
    }

    /// The views the entry of `buffer` lists (tests).
    #[cfg(test)]
    pub(crate) fn views_of(&self, buffer: u64) -> Vec<u64> {
        match entry_index(buffer) {
            Some(i) => self.inner.borrow().entries[i].views.clone(),
            None => Vec::new(),
        }
    }

    /// The dead stores that wait for the next table call (tests).
    #[cfg(test)]
    pub(crate) fn dead_stores(&self) -> usize {
        self.inner.borrow().dead.len()
    }
}

/// Reports `n` more external bytes (needs N1): a GC point, called with the buffer in a Rust
/// local and no borrow of the table.
fn add_external(rt: &Runtime, n: usize) -> BufResult<()> {
    if n == 0 {
        return Ok(());
    }
    rt.buffers.heap.set(&rt.heap);
    rt.heap
        .add_external_bytes(n)
        .map_err(|_| Fail::Range("Array buffer allocation failed"))
}

// ---- buffers ----------------------------------------------------------------------------------

/// The ObjectClass of `b` if it is an ArrayBuffer or a SharedArrayBuffer made here: it owns
/// the table entry that its slot 5 names (the brand, module doc). `None` for any other value,
/// an object of these classes from the generic slot API included.
#[inline]
pub(crate) fn buffer_class(rt: &Runtime, b: u64) -> Option<ObjectClass> {
    let c = match object::slot_class(b)? {
        c if c == ObjectClass::ArrayBuffer.id() => ObjectClass::ArrayBuffer,
        c if c == ObjectClass::SharedArrayBuffer.id() => ObjectClass::SharedArrayBuffer,
        _ => return None,
    };
    owns_entry(rt, b, slot::COUNT, slot::TABLE).then_some(c)
}

/// True if `v` is a DataView made here ([`new_data_view`]): it owns the table entry that its
/// slot 4 names (the brand). A DataView reader (TA-2) trusts the slots of `v` only after this.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_data_view(rt: &Runtime, v: u64) -> bool {
    object::slot_class(v) == Some(ObjectClass::DataView.id())
        && owns_entry(rt, v, dv_slot::COUNT, dv_slot::TABLE)
}

/// True if `o` is a buffer or a DataView made here (an engine object; any value).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_engine_object(rt: &Runtime, o: u64) -> bool {
    buffer_class(rt, o).is_some() || is_data_view(rt, o)
}

/// True if the SLOT_OBJECT `o` has `count` slots and its slot `table` is the int32 index of
/// a table entry whose owner is `o` (the brand).
#[inline]
fn owns_entry(rt: &Runtime, o: u64, count: usize, table: usize) -> bool {
    object::slot_count(o) == count
        && val(object::slot_get(o, table))
            .as_int32()
            .is_some_and(|i| i >= 0 && rt.buffers.owns(i as usize, o))
}

/// True if the buffer `b` (its [`buffer_class`] is `Some`) is a SharedArrayBuffer.
#[inline]
pub(crate) fn is_shared(b: u64) -> bool {
    object::slot_class(b) == Some(ObjectClass::SharedArrayBuffer.id())
}

fn flags(b: u64) -> i32 {
    int32_of(object::slot_get(b, slot::FLAGS))
}

/// IsDetachedBuffer (§25.1.3.4) of the buffer `b`.
#[inline]
pub(crate) fn is_detached(b: u64) -> bool {
    flags(b) & flag::DETACHED != 0
}

/// True unless IsFixedLengthArrayBuffer (§25.1.3.7): a resizable ArrayBuffer or a growable
/// SharedArrayBuffer.
#[inline]
pub(crate) fn is_resizable(b: u64) -> bool {
    flags(b) & flag::RESIZABLE != 0
}

fn entry_index(b: u64) -> Option<usize> {
    let i = object::slot_get(b, slot::TABLE);
    (i != UNDEFINED).then(|| int32_of(i) as usize)
}

fn number_slot(b: u64, i: usize) -> u64 {
    val(object::slot_get(b, i))
        .as_number()
        .expect("a number slot") as u64
}

/// ArrayBufferByteLength (§25.1.3.2) with order seq-cst: 0 when detached.
pub(crate) fn byte_length(rt: &Runtime, b: u64) -> usize {
    if is_shared(b) {
        let i = entry_index(b).expect("a SharedArrayBuffer has an entry");
        return match &rt.buffers.inner.borrow().entries[i].store {
            Some(Store::Shared(block)) => block.byte_length(),
            _ => panic!("a SharedArrayBuffer without its block"),
        };
    }
    number_slot(b, slot::BYTE_LENGTH) as usize
}

/// `[[ArrayBufferMaxByteLength]]`, or `None` for a fixed-length buffer.
pub(crate) fn max_byte_length(b: u64) -> Option<u64> {
    let m = object::slot_get(b, slot::MAX_BYTE_LENGTH);
    (m != UNDEFINED).then(|| val(m).as_number().expect("a number") as u64)
}

/// The block of the SharedArrayBuffer `b`.
fn shared_block(rt: &Runtime, b: u64) -> Arc<SharedBlock> {
    let i = entry_index(b).expect("a SharedArrayBuffer has an entry");
    match &rt.buffers.inner.borrow().entries[i].store {
        Some(Store::Shared(block)) => Arc::clone(block),
        _ => panic!("a SharedArrayBuffer without its block"),
    }
}

/// The address of byte 0 of the store of `b`; null when it has none (zero length, detached).
pub(crate) fn data_base(rt: &Runtime, b: u64) -> *mut u8 {
    let s = object::slot_get(b, slot::STORE);
    if s == UNDEFINED {
        return core::ptr::null_mut();
    }
    if kinds::cell_kind(s) == kinds::BYTES.get() {
        return bytes_data(s);
    }
    let t = rt.buffers.inner.borrow();
    match &t.entries[int32_of(s) as usize].store {
        Some(st) => st.base(),
        None => core::ptr::null_mut(),
    }
}

/// The class's slots on a new SLOT_OBJECT, with its own table entry (no store yet), byte
/// length, maximum and flags. GC point.
fn new_buffer_object(
    rt: &Runtime,
    proto: u64,
    class: ObjectClass,
    len: u64,
    max: Option<u64>,
) -> u64 {
    let o = object::new_slot_object(rt, proto, class.id(), slot::COUNT);
    // The brand: `o` owns its entry from birth (no GC point since the allocation).
    let i = rt.buffers.insert(o, None, 0);
    object::slot_set(rt, o, slot::TABLE, int32_bits(i));
    let fl = if max.is_some() { flag::RESIZABLE } else { 0 };
    object::slot_set(rt, o, slot::BYTE_LENGTH, crate::conv::num(len as f64));
    let m = max.map_or(UNDEFINED, |m| crate::conv::num(m as f64));
    object::slot_set(rt, o, slot::MAX_BYTE_LENGTH, m);
    object::slot_set(rt, o, slot::FLAGS, int32_bits(fl));
    o
}

/// The reservation size for a maximum of `max` bytes: a multiple of the allocation
/// granularity, at least one.
fn reservation_size(max: u64) -> usize {
    let g = allocation_granularity();
    (max as usize).max(1).next_multiple_of(g)
}

/// AllocateArrayBuffer (§25.1.3.1) after OrdinaryCreateFromConstructor: a new ArrayBuffer
/// with prototype `proto` and `len` zero bytes; resizable up to `max` with `Some(max)`. GC
/// point.
pub(crate) fn allocate(rt: &Runtime, proto: u64, len: u64, max: Option<u64>) -> BufResult<u64> {
    rt.buffers.drain_dead();
    if let Some(m) = max {
        if len > m {
            return Err(Fail::Range("Invalid array buffer length"));
        }
        if m > MAX_RESERVATION {
            return Err(Fail::Range("Invalid array buffer max length"));
        }
    }
    if usize::try_from(len).is_err() {
        return Err(Fail::Range("Array buffer allocation failed"));
    }
    // GC points: `b` lives in a Rust local (CSS).
    let b = new_buffer_object(rt, proto, ObjectClass::ArrayBuffer, len, max);
    match max {
        None if len == 0 => {}
        None => {
            let bytes =
                new_bytes(rt, len as usize).ok_or(Fail::Range("Array buffer allocation failed"))?;
            object::slot_set(rt, b, slot::STORE, bytes);
        }
        Some(m) => {
            let mem = Reservation::reserve(reservation_size(m))
                .map_err(|_| Fail::Range("Array buffer allocation failed"))?;
            let committed = (len as usize).next_multiple_of(page_size());
            if committed > 0 {
                mem.commit(0, committed)
                    .map_err(|_| Fail::Range("Array buffer allocation failed"))?;
            }
            // Needs N1: the add is a GC point, with `b` in a Rust local and no table borrow; on
            // Err `mem` drops here.
            add_external(rt, committed)?;
            let i = entry_index(b).expect("an engine buffer has its entry");
            rt.buffers
                .set_store(i, Some(Store::Resizable { mem, committed }), committed);
            object::slot_set(rt, b, slot::STORE, int32_bits(i as i32));
        }
    }
    Ok(b)
}

/// AllocateSharedArrayBuffer (§25.2.2.1): a new SharedArrayBuffer with prototype `proto` and
/// `len` zero bytes, growable up to `max` with `Some(max)`. GC point.
pub(crate) fn allocate_shared(
    rt: &Runtime,
    proto: u64,
    len: u64,
    max: Option<u64>,
) -> BufResult<u64> {
    rt.buffers.drain_dead();
    let reserve = match max {
        Some(m) if len > m => return Err(Fail::Range("Invalid array buffer length")),
        Some(m) if m > MAX_RESERVATION => {
            return Err(Fail::Range("Invalid array buffer max length"));
        }
        Some(m) => m,
        None if len > MAX_RESERVATION => {
            return Err(Fail::Range("Array buffer allocation failed"));
        }
        None => len,
    };
    let b = new_buffer_object(rt, proto, ObjectClass::SharedArrayBuffer, len, max);
    let mem = Reservation::reserve(reservation_size(reserve))
        .map_err(|_| Fail::Range("Array buffer allocation failed"))?;
    let committed = (len as usize).next_multiple_of(page_size());
    if committed > 0 {
        mem.commit(0, committed)
            .map_err(|_| Fail::Range("Array buffer allocation failed"))?;
    }
    let block = Arc::new(SharedBlock {
        mem,
        len: AtomicUsize::new(len as usize),
        max: max.map(|m| m as usize),
        committed: Mutex::new(committed),
        serial: NEXT_SERIAL.fetch_add(1, Relaxed),
    });
    // Needs N1 (as `allocate`); TA-ORCH-4: this isolate counts the block once.
    add_external(rt, committed)?;
    let i = entry_index(b).expect("an engine buffer has its entry");
    rt.buffers
        .set_store(i, Some(Store::Shared(block)), committed);
    object::slot_set(rt, b, slot::STORE, int32_bits(i as i32));
    Ok(b)
}

/// Adds the view `v` (a TypedArray or a DataView) to the view list of the buffer `b` (its
/// entry exists from its birth). Not a GC point; no JS.
pub(crate) fn add_view(rt: &Runtime, b: u64, v: u64) {
    rt.buffers.drain_dead();
    let i = entry_index(b).expect("an engine buffer has its entry");
    rt.buffers.inner.borrow_mut().entries[i].views.push(v);
}

/// The data word of a view at `byte_offset` of a store whose byte 0 is at `base`.
fn data_at(base: *mut u8, byte_offset: usize) -> *const u8 {
    if base.is_null() {
        core::ptr::null()
    } else {
        base.wrapping_add(byte_offset).cast_const()
    }
}

/// The data word of a view at `byte_offset` of the buffer `b`.
pub(crate) fn view_data(rt: &Runtime, b: u64, byte_offset: usize) -> *const u8 {
    data_at(data_base(rt, b), byte_offset)
}

/// DetachArrayBuffer (§25.1.3.5) of the ArrayBuffer `b` with `key`: the steps of design §5.2.
/// A TypeError for a SharedArrayBuffer or a key that is not the buffer's (SameValue). Not a
/// GC point.
pub(crate) fn detach(rt: &Runtime, b: u64, key: u64) -> BufResult<()> {
    match buffer_class(rt, b) {
        Some(ObjectClass::ArrayBuffer) => {}
        Some(_) => return Err(Fail::Type("Cannot detach a SharedArrayBuffer")),
        None => return Err(Fail::Type("Receiver is not an ArrayBuffer")),
    }
    if !crate::property::same_value(object::slot_get(b, slot::DETACH_KEY), key) {
        return Err(Fail::Type("The detach key does not match"));
    }
    if is_detached(b) {
        return Ok(());
    }
    detach_unchecked(rt, b);
    Ok(())
}

/// The steps 2-4 of design §5.2 on a non-shared buffer `b` that is not detached: zero every
/// view, flag the buffer, drop its store. Not a GC point.
fn detach_unchecked(rt: &Runtime, b: u64) {
    rt.buffers.drain_dead();
    let i = entry_index(b);
    if let Some(i) = i {
        let views = core::mem::take(&mut rt.buffers.inner.borrow_mut().entries[i].views);
        for v in views {
            zero_view(rt, v);
        }
    }
    object::slot_set(rt, b, slot::FLAGS, int32_bits(flags(b) | flag::DETACHED));
    object::slot_set(rt, b, slot::BYTE_LENGTH, int32_bits(0));
    object::slot_set(rt, b, slot::STORE, UNDEFINED);
    if let Some(i) = i {
        let (store, external) = take_store(rt, i);
        if external > 0 {
            rt.heap.remove_external_bytes(external);
        }
        drop(store);
    }
}

/// Step 2 of design §5.2 for one view: a null data word; length 0 for a TYPED_FIXED view, byte
/// length 0 for a fixed DataView. Not a GC point.
fn zero_view(rt: &Runtime, v: u64) {
    if kinds::is_view(v) {
        crate::typed_array::detach_view(rt, v);
        return;
    }
    debug_assert_eq!(object::slot_class(v), Some(ObjectClass::DataView.id()));
    let null = ExternalRef::<u8>::new(core::ptr::null());
    object::slot_set(rt, v, dv_slot::DATA, ExternalRef::to_word(null));
    if object::slot_get(v, dv_slot::BYTE_LENGTH) != int32_bits(layout::VIEW_LENGTH_AUTO) {
        object::slot_set(rt, v, dv_slot::BYTE_LENGTH, int32_bits(0));
    }
}

/// ArrayBufferCopyAndDetach (§25.1.3.3) after its ToIndex step (`transfer`,
/// `transferToFixedLength`): a new ArrayBuffer with prototype `proto` and `new_len` bytes,
/// resizable with the old maximum if `preserve` and `b` is resizable. The store moves with no
/// copy when it can (a BYTES cell of the same length; a resizable reservation that stays
/// resizable); else the first min(old, new) bytes are copied. Then `b` is detached: only after
/// the new buffer has its store (the spec's order: AllocateArrayBuffer, step 7, before
/// DetachArrayBuffer), so an error leaves `b` as it was. GC point.
pub(crate) fn copy_and_detach(
    rt: &Runtime,
    b: u64,
    proto: u64,
    new_len: u64,
    preserve: bool,
) -> BufResult<u64> {
    match buffer_class(rt, b) {
        Some(ObjectClass::ArrayBuffer) => {}
        Some(_) => return Err(Fail::Type("Cannot transfer a SharedArrayBuffer")),
        None => return Err(Fail::Type("Receiver is not an ArrayBuffer")),
    }
    let new_max = if preserve && is_resizable(b) {
        max_byte_length(b)
    } else {
        None
    };
    if is_detached(b) {
        return Err(Fail::Type("Cannot transfer a detached ArrayBuffer"));
    }
    if object::slot_get(b, slot::DETACH_KEY) != UNDEFINED {
        return Err(Fail::Type(
            "Cannot transfer an ArrayBuffer with a detach key",
        ));
    }
    if let Some(m) = new_max
        && new_len > m
    {
        return Err(Fail::Range("Invalid array buffer length"));
    }
    let old_len = byte_length(rt, b) as u64;
    let store = object::slot_get(b, slot::STORE);
    // The move cases: the new buffer takes the store itself (design §5.2).
    if new_max.is_none() && !is_resizable(b) && new_len == old_len {
        let nb = new_buffer_object(rt, proto, ObjectClass::ArrayBuffer, new_len, None);
        // GC point above: `store` (a BYTES cell or undefined) is in a Rust local and `b` holds it.
        object::slot_set(rt, nb, slot::STORE, store);
        detach_unchecked(rt, b);
        return Ok(nb);
    }
    if new_max.is_some() {
        // A resizable store stays resizable: move the reservation, resize it, then detach `b`.
        let nb = new_buffer_object(rt, proto, ObjectClass::ArrayBuffer, old_len, new_max);
        let i = entry_index(b).expect("a resizable buffer has an entry");
        let j = entry_index(nb).expect("an engine buffer has its entry");
        let (moved, external) = take_store(rt, i);
        rt.buffers.set_store(j, moved, external);
        object::slot_set(rt, nb, slot::STORE, int32_bits(j as i32));
        // GC point (a grow adds external bytes): `b` and `nb` live in Rust locals, and no JS
        // runs before the detach, so the views of `b` see no state between the two.
        if let Err(e) = resize(rt, nb, new_len) {
            // The grow failed (its external bytes or its commit; `resize` took back what it
            // added): the store and its count go back to `b`, which stays as it was.
            let (store, external) = take_store(rt, j);
            let mut t = rt.buffers.inner.borrow_mut();
            t.entries[i].store = store;
            t.entries[i].external = external;
            return Err(e);
        }
        detach_unchecked(rt, b);
        return Ok(nb);
    }
    // Copy: allocate first (it can throw a RangeError and leaves `b` as it was).
    let nb = allocate(rt, proto, new_len, None)?;
    let n = old_len.min(new_len) as usize;
    if n > 0 {
        let from = data_base(rt, b);
        let to = data_base(rt, nb);
        // SAFETY: both stores are live (Rust locals hold `b` and `nb`), non-shared, and hold at
        // least `n` bytes; they are distinct allocations.
        unsafe { core::ptr::copy_nonoverlapping(from, to, n) };
    }
    detach_unchecked(rt, b);
    Ok(nb)
}

/// Takes the store and the external byte count out of table entry `i` (they move to another
/// entry, or the store drops). Not a GC point.
fn take_store(rt: &Runtime, i: usize) -> (Option<Store>, usize) {
    let mut t = rt.buffers.inner.borrow_mut();
    let e = &mut t.entries[i];
    (e.store.take(), core::mem::take(&mut e.external))
}

/// The resize of `ArrayBuffer.prototype.resize` (§25.1.6.6) after its ToIndex step. GC point
/// (the external bytes of a grow).
pub(crate) fn resize(rt: &Runtime, b: u64, new_len: u64) -> BufResult<()> {
    if buffer_class(rt, b) != Some(ObjectClass::ArrayBuffer) || !is_resizable(b) {
        return Err(Fail::Type("Receiver is not a resizable ArrayBuffer"));
    }
    if is_detached(b) {
        return Err(Fail::Type("Cannot resize a detached ArrayBuffer"));
    }
    let max = max_byte_length(b).expect("a resizable buffer has a maximum");
    if new_len > max {
        return Err(Fail::Range("Invalid array buffer length"));
    }
    rt.buffers.drain_dead();
    let i = entry_index(b).expect("a resizable buffer has an entry");
    let old_len = byte_length(rt, b);
    let new_len = new_len as usize;
    let want = new_len.next_multiple_of(page_size());
    let have = match &rt.buffers.inner.borrow().entries[i].store {
        Some(Store::Resizable { committed, .. }) => *committed,
        _ => panic!("a resizable buffer without its store"),
    };
    if want > have {
        // Needs N1: the add (a GC point) with no table borrow, then the commit.
        add_external(rt, want - have)?;
        let mut t = rt.buffers.inner.borrow_mut();
        let e = &mut t.entries[i];
        let Some(Store::Resizable { mem, committed }) = &mut e.store else {
            unreachable!("checked above")
        };
        if mem.commit(have, want - have).is_err() {
            drop(t);
            rt.heap.remove_external_bytes(want - have);
            return Err(Fail::Range("Array buffer allocation failed"));
        }
        *committed = want;
        e.external += want - have;
    } else if new_len < old_len {
        let mut t = rt.buffers.inner.borrow_mut();
        let e = &mut t.entries[i];
        let Some(Store::Resizable { mem, committed }) = &mut e.store else {
            unreachable!("checked above")
        };
        let base: *mut u8 = core::ptr::with_exposed_provenance_mut(mem.base());
        // The bytes [new_len, committed end) are 0 after a shrink (a later grow reads zeros):
        // the bytes past `old_len` are 0 already; zero [new_len, old_len) in the pages that
        // stay, and decommit the others.
        let zero_end = old_len.min(want);
        // SAFETY: [new_len, zero_end) lies in the committed pages of this reservation.
        unsafe { core::ptr::write_bytes(base.wrapping_add(new_len), 0, zero_end - new_len) };
        if want < have {
            // SAFETY: no view reads past the byte length: every access checks its bound against
            // the buffer's byte length first (TYPED_VARIABLE views, DataView, the buffer ops).
            let r = unsafe { mem.decommit(want, have - want) };
            if r.is_ok() {
                *committed = want;
                e.external -= have - want;
                drop(t);
                rt.heap.remove_external_bytes(have - want);
            } else if old_len > want {
                // The pages stay committed: zero them so the rule holds.
                // SAFETY: [want, old_len) is committed in this reservation.
                unsafe { core::ptr::write_bytes(base.wrapping_add(want), 0, old_len - want) };
            }
        }
    }
    object::slot_set(rt, b, slot::BYTE_LENGTH, crate::conv::num(new_len as f64));
    Ok(())
}

/// The grow of `SharedArrayBuffer.prototype.grow` (§25.2.5.3) after its ToIndex step. GC
/// point (the external bytes).
pub(crate) fn grow_shared(rt: &Runtime, b: u64, new_len: u64) -> BufResult<()> {
    if buffer_class(rt, b) != Some(ObjectClass::SharedArrayBuffer) || !is_resizable(b) {
        return Err(Fail::Type("Receiver is not a growable SharedArrayBuffer"));
    }
    let block = shared_block(rt, b);
    let max = block.max.expect("a growable block") as u64;
    if new_len > max {
        return Err(Fail::Range("Invalid array buffer length"));
    }
    let new_len = new_len as usize;
    if new_len < block.byte_length() {
        return Err(Fail::Range("Invalid array buffer length: it cannot shrink"));
    }
    let want = new_len.next_multiple_of(page_size());
    let have = *block
        .committed
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let extra = want.saturating_sub(have);
    // Needs N1: the add (a GC point) before the lock and with no table borrow.
    add_external(rt, extra)?;
    let mut committed = block
        .committed
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if new_len < block.byte_length() {
        drop(committed);
        rt.heap.remove_external_bytes(extra);
        return Err(Fail::Range("Invalid array buffer length: it cannot shrink"));
    }
    if want > *committed && block.mem.commit(*committed, want - *committed).is_err() {
        drop(committed);
        rt.heap.remove_external_bytes(extra);
        return Err(Fail::Range("Array buffer allocation failed"));
    }
    *committed = (*committed).max(want);
    block.len.store(new_len, Ordering::SeqCst);
    drop(committed);
    if extra > 0 {
        let i = entry_index(b).expect("a SAB has an entry");
        rt.buffers.inner.borrow_mut().entries[i].external += extra;
    }
    Ok(())
}

// ---- byte access ------------------------------------------------------------------------------

/// The size in bytes of the element type `t` (a Table 71 row).
#[inline(always)]
pub(crate) fn elem_size(t: u8) -> usize {
    usize::from(layout::VIEW_ELEMENT_SIZES[usize::from(t)])
}

/// True for the BigInt element types (rows 7 BigInt64 and 8 BigUint64).
#[inline(always)]
pub(crate) fn is_bigint_elem(t: u8) -> bool {
    t == 7 || t == 8
}

/// Reads `size` (1, 2, 4 or 8) bytes at `p` as a little-endian integer. Shared bytes: Relaxed
/// atomic loads of the element width when aligned, else a Relaxed byte loop.
///
/// # Safety
/// `[p, p + size)` is inside a live store.
#[inline]
pub(crate) unsafe fn read_raw(p: *const u8, size: usize, shared: bool) -> u64 {
    let p = p.cast_mut();
    if shared {
        if p.addr().is_multiple_of(size) {
            // SAFETY: aligned, inside a live store (caller); every access to these bytes is
            // atomic (TA-D11).
            return unsafe {
                match size {
                    1 => u64::from(AtomicU8::from_ptr(p).load(Relaxed)),
                    2 => u64::from(AtomicU16::from_ptr(p.cast()).load(Relaxed)),
                    4 => u64::from(AtomicU32::from_ptr(p.cast()).load(Relaxed)),
                    _ => AtomicU64::from_ptr(p.cast()).load(Relaxed),
                }
            };
        }
        let mut x = 0u64;
        for i in 0..size {
            // SAFETY: as above, one byte at a time.
            let byte = unsafe { AtomicU8::from_ptr(p.add(i)).load(Relaxed) };
            x |= u64::from(byte) << (8 * i);
        }
        return x;
    }
    // SAFETY: inside a live store (caller); no other thread writes non-shared bytes.
    unsafe {
        match size {
            1 => u64::from(p.read()),
            2 => u64::from(u16::from_le(p.cast::<u16>().read_unaligned())),
            4 => u64::from(u32::from_le(p.cast::<u32>().read_unaligned())),
            _ => u64::from_le(p.cast::<u64>().read_unaligned()),
        }
    }
}

/// Writes the low `size` bytes of `x` at `p`, little endian ([`read_raw`]'s rules).
///
/// # Safety
/// `[p, p + size)` is inside a live store.
#[inline]
pub(crate) unsafe fn write_raw(p: *mut u8, size: usize, x: u64, shared: bool) {
    if shared {
        if p.addr().is_multiple_of(size) {
            // SAFETY: as `read_raw`.
            unsafe {
                match size {
                    1 => AtomicU8::from_ptr(p).store(x as u8, Relaxed),
                    2 => AtomicU16::from_ptr(p.cast()).store(x as u16, Relaxed),
                    4 => AtomicU32::from_ptr(p.cast()).store(x as u32, Relaxed),
                    _ => AtomicU64::from_ptr(p.cast()).store(x, Relaxed),
                }
            }
            return;
        }
        for i in 0..size {
            // SAFETY: as above, one byte at a time.
            unsafe { AtomicU8::from_ptr(p.add(i)).store((x >> (8 * i)) as u8, Relaxed) };
        }
        return;
    }
    // SAFETY: as `read_raw`.
    unsafe {
        match size {
            1 => p.write(x as u8),
            2 => p.cast::<u16>().write_unaligned((x as u16).to_le()),
            4 => p.cast::<u32>().write_unaligned((x as u32).to_le()),
            _ => p.cast::<u64>().write_unaligned(x.to_le()),
        }
    }
}

/// The modulo-2^32 integer of a Number (the ToInt32 / ToUint32 core, §7.1.6-7): 0 for NaN and
/// ±∞, else the truncated value modulo 2^32.
fn to_uint32_bits(n: f64) -> u32 {
    if !n.is_finite() {
        return 0;
    }
    let t = n.trunc();
    if t.abs() < 9_007_199_254_740_992.0 {
        return t as i64 as u32;
    }
    // Exact: fmod of an integral double by a power of two.
    let m = t % 4_294_967_296.0;
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    m as u32
}

/// ToUint8Clamp (§7.1.12): NaN and below 0 give 0, above 255 gives 255, else round half to
/// even.
fn to_uint8_clamp(n: f64) -> u8 {
    if n.is_nan() || n <= 0.0 {
        return 0;
    }
    if n >= 255.0 {
        return 255;
    }
    n.round_ties_even() as u8
}

/// NumericToRawBytes (§25.1.3.17) of the Number `n` for the Number element type `t`, as the
/// little-endian integer of its bytes. One NaN pattern per type.
pub(crate) fn number_to_raw(t: u8, n: f64) -> u64 {
    match t {
        0 | 1 | 3 | 4 | 5 | 6 => u64::from(to_uint32_bits(n)),
        2 => u64::from(to_uint8_clamp(n)),
        9 => u64::from(ep9_math::f16::to_bits(n)),
        10 => {
            let f = if n.is_nan() { f32::NAN } else { n as f32 };
            u64::from(f.to_bits())
        }
        11 => {
            let d = if n.is_nan() { f64::NAN } else { n };
            d.to_bits()
        }
        _ => panic!("element type {t} holds no Number"),
    }
}

/// NumericToRawBytes of the BigInt `b` (Value bits) for BigInt64 or BigUint64: ℝ(b) modulo
/// 2^64 (ToBigInt64, ToBigUint64 share the bytes). Not a GC point.
pub(crate) fn bigint_to_raw(b: u64) -> u64 {
    let v = crate::bigint::view(&b);
    let low = v.digits.first().copied().unwrap_or(0);
    if v.neg { low.wrapping_neg() } else { low }
}

/// RawBytesToNumeric (§25.1.3.15) of the little-endian bytes `x` of element type `t`: a
/// Number, or a BigInt for rows 7 and 8 (a GC point then). NaN boxes as the one NaN.
pub(crate) fn raw_to_value(rt: &Runtime, t: u8, x: u64) -> u64 {
    let num = crate::conv::num;
    match t {
        0 => num(f64::from(x as u8 as i8)),
        1 | 2 => num(f64::from(x as u8)),
        3 => num(f64::from(x as u16 as i16)),
        4 => num(f64::from(x as u16)),
        5 => num(f64::from(x as u32 as i32)),
        6 => num(f64::from(x as u32)),
        7 => crate::bigint::from_i128(rt, i128::from(x as i64)),
        8 => crate::bigint::from_i128(rt, i128::from(x)),
        9 => num(ep9_math::f16::from_bits(x as u16)),
        10 => num(f64::from(f32::from_bits(x as u32))),
        11 => num(f64::from_bits(x)),
        _ => panic!("element type {t}"),
    }
}

/// The raw bytes of a converted value for element type `t`: a Number for rows 0-6 and 9-11, a
/// BigInt for rows 7 and 8; `None` for a value of the wrong type.
pub(crate) fn value_to_raw(t: u8, v: u64) -> Option<u64> {
    if is_bigint_elem(t) {
        kinds::is_bigint(v).then(|| bigint_to_raw(v))
    } else {
        val(v).as_number().map(|n| number_to_raw(t, n))
    }
}

/// The address of `[index, index + size)` of the buffer `b` after the checks of
/// GetValueFromBuffer and SetValueInBuffer: a buffer, not detached, inside its byte length.
fn checked_range(rt: &Runtime, b: u64, index: u64, size: usize) -> BufResult<*mut u8> {
    if buffer_class(rt, b).is_none() {
        return Err(Fail::Type(
            "Receiver is not an ArrayBuffer or a SharedArrayBuffer",
        ));
    }
    if is_detached(b) {
        return Err(Fail::Type(
            "Cannot perform this operation on a detached ArrayBuffer",
        ));
    }
    let len = byte_length(rt, b) as u64;
    if index.checked_add(size as u64).is_none_or(|end| end > len) {
        return Err(Fail::Range("Offset is outside the bounds of the buffer"));
    }
    Ok(data_base(rt, b).wrapping_add(index as usize))
}

/// GetValueFromBuffer (§25.1.3.16) with order unordered. GC point (a BigInt).
pub(crate) fn get_value(
    rt: &Runtime,
    b: u64,
    index: u64,
    t: u8,
    little_endian: bool,
) -> BufResult<u64> {
    let size = elem_size(t);
    let p = checked_range(rt, b, index, size)?;
    // SAFETY: `checked_range` checked the range against the live store.
    let mut x = unsafe { read_raw(p, size, is_shared(b)) };
    if !little_endian {
        x = x.swap_bytes() >> (64 - 8 * size);
    }
    Ok(raw_to_value(rt, t, x))
}

/// SetValueInBuffer (§25.1.3.18) with order unordered of the converted `v`. Not a GC point.
pub(crate) fn set_value(
    rt: &Runtime,
    b: u64,
    index: u64,
    t: u8,
    v: u64,
    little_endian: bool,
) -> BufResult<()> {
    let size = elem_size(t);
    let Some(mut x) = value_to_raw(t, v) else {
        return Err(WRONG_TYPE);
    };
    let p = checked_range(rt, b, index, size)?;
    if !little_endian {
        x = x.swap_bytes() >> (64 - 8 * size);
    }
    // SAFETY: `checked_range` checked the range against the live store.
    unsafe { write_raw(p, size, x, is_shared(b)) };
    Ok(())
}

/// CopyDataBlockBytes (§6.2.9.3) between the stores of two buffers (the same one too, with
/// overlap: as if through a temporary copy). Not a GC point.
pub(crate) fn copy_bytes(
    rt: &Runtime,
    to: u64,
    to_index: u64,
    from: u64,
    from_index: u64,
    count: u64,
) -> BufResult<()> {
    let dst = checked_range(rt, to, to_index, 0)?;
    let src = checked_range(rt, from, from_index, 0)?;
    let n = usize::try_from(count)
        .map_err(|_| Fail::Range("Offset is outside the bounds of the buffer"))?;
    checked_range(rt, to, to_index, n)?;
    checked_range(rt, from, from_index, n)?;
    if n == 0 {
        return Ok(());
    }
    if is_shared(to) || is_shared(from) {
        // Relaxed bytes (TA-D11), in the direction that keeps an overlap right.
        let forward = dst.addr() <= src.addr() || dst.addr() >= src.addr() + n;
        let step = |i: usize| {
            // SAFETY: both ranges were checked; each byte is read and written atomically when
            // its store is shared.
            unsafe {
                let x = read_raw(src.add(i), 1, true);
                write_raw(dst.add(i), 1, x, true);
            }
        };
        if forward {
            (0..n).for_each(step);
        } else {
            (0..n).rev().for_each(step);
        }
    } else {
        // SAFETY: both ranges were checked against live, non-shared stores.
        unsafe { core::ptr::copy(src, dst, n) };
    }
    Ok(())
}

// ---- Atomics: seq-cst access of an integer element (§25.4) ----------------------------------

/// The aligned address of the integer element at `index` of `b` for an Atomics access.
fn atomic_range(rt: &Runtime, b: u64, index: u64, t: u8) -> BufResult<*mut u8> {
    if is_bigint_elem(t) || (t <= 6 && t != 2) {
        let size = elem_size(t);
        if !index.is_multiple_of(size as u64) {
            return Err(Fail::Range("Invalid atomic access index"));
        }
        return checked_range(rt, b, index, size);
    }
    Err(Fail::Type("Atomics needs an integer element type"))
}

/// A seq-cst operation of the Atomics on one element ([`atomic_op`]).
#[derive(Clone, Copy, Debug)]
enum AtomicOp {
    /// Store the value.
    Store,
    /// A read-modify-write op of `Cx::atomic_read_modify_write` (0 add, 1 and, 2 exchange,
    /// 3 or, 4 sub, 5 xor).
    Rmw(u8),
    /// Compare with these bits; store the value only when equal.
    Cas(u64),
}

/// The seq-cst `op` with the value bits `x` on the aligned element at `p` of `size` bytes; the
/// result is the old bits (0 for a store).
///
/// # Safety
/// `p` is aligned to `size` and inside a live store.
unsafe fn atomic_op(p: *mut u8, size: usize, op: AtomicOp, x: u64) -> u64 {
    let s = Ordering::SeqCst;
    macro_rules! go {
        ($a:ty, $t:ty) => {{
            // SAFETY: aligned and inside a live store (caller).
            let a = unsafe { <$a>::from_ptr(p.cast()) };
            let x = x as $t;
            u64::from(match op {
                AtomicOp::Store => {
                    a.store(x, s);
                    0
                }
                AtomicOp::Rmw(0) => a.fetch_add(x, s),
                AtomicOp::Rmw(1) => a.fetch_and(x, s),
                AtomicOp::Rmw(2) => a.swap(x, s),
                AtomicOp::Rmw(3) => a.fetch_or(x, s),
                AtomicOp::Rmw(4) => a.fetch_sub(x, s),
                AtomicOp::Rmw(_) => a.fetch_xor(x, s),
                AtomicOp::Cas(e) => match a.compare_exchange(e as $t, x, s, s) {
                    Ok(v) | Err(v) => v,
                },
            })
        }};
    }
    match size {
        1 => go!(AtomicU8, u8),
        2 => go!(AtomicU16, u16),
        4 => go!(AtomicU32, u32),
        _ => go!(AtomicU64, u64),
    }
}

/// GetValueFromBuffer with order seq-cst (`Atomics.load`). GC point (a BigInt).
pub(crate) fn atomic_load(rt: &Runtime, b: u64, index: u64, t: u8) -> BufResult<u64> {
    let p = atomic_range(rt, b, index, t)?;
    let s = Ordering::SeqCst;
    // SAFETY: aligned to the element size and inside the live store (`atomic_range`).
    let x = unsafe {
        match elem_size(t) {
            1 => u64::from(AtomicU8::from_ptr(p).load(s)),
            2 => u64::from(AtomicU16::from_ptr(p.cast()).load(s)),
            4 => u64::from(AtomicU32::from_ptr(p.cast()).load(s)),
            _ => AtomicU64::from_ptr(p.cast()).load(s),
        }
    };
    Ok(raw_to_value(rt, t, x))
}

/// The error of a value that is not a Number (BigInt for rows 7 and 8).
const WRONG_TYPE: Fail = Fail::Type("The value has the wrong type for the element");

/// SetValueInBuffer with order seq-cst (`Atomics.store`). Not a GC point.
pub(crate) fn atomic_store(rt: &Runtime, b: u64, index: u64, t: u8, v: u64) -> BufResult<()> {
    let x = value_to_raw(t, v).ok_or(WRONG_TYPE)?;
    let p = atomic_range(rt, b, index, t)?;
    // SAFETY: aligned and checked.
    unsafe { atomic_op(p, elem_size(t), AtomicOp::Store, x) };
    Ok(())
}

/// GetModifySetValueInBuffer (§25.1.3.19), seq-cst, with the op of `Cx::atomic_read_modify_write`
/// (0 add, 1 and, 2 exchange, 3 or, 4 sub, 5 xor): the old element. GC point (a BigInt).
pub(crate) fn atomic_rmw(
    rt: &Runtime,
    b: u64,
    index: u64,
    t: u8,
    op: u8,
    v: u64,
) -> BufResult<u64> {
    let x = value_to_raw(t, v).ok_or(WRONG_TYPE)?;
    if op > 5 {
        return Err(Fail::Type("Unknown Atomics operation"));
    }
    let p = atomic_range(rt, b, index, t)?;
    // SAFETY: aligned and checked.
    let old = unsafe { atomic_op(p, elem_size(t), AtomicOp::Rmw(op), x) };
    Ok(raw_to_value(rt, t, old))
}

/// The compare-exchange of `Atomics.compareExchange` (§25.4.5), seq-cst: the old element; the
/// element changes only when its bytes equal those of `expected`. GC point (a BigInt).
pub(crate) fn atomic_compare_exchange(
    rt: &Runtime,
    b: u64,
    index: u64,
    t: u8,
    expected: u64,
    replacement: u64,
) -> BufResult<u64> {
    let e = value_to_raw(t, expected).ok_or(WRONG_TYPE)?;
    let r = value_to_raw(t, replacement).ok_or(WRONG_TYPE)?;
    let p = atomic_range(rt, b, index, t)?;
    // SAFETY: aligned and checked (`atomic_op` casts both to the element width).
    let old = unsafe { atomic_op(p, elem_size(t), AtomicOp::Cas(e), r) };
    Ok(raw_to_value(rt, t, old))
}

// ---- DataView ---------------------------------------------------------------------------------

/// A new DataView with prototype `proto` on the buffer `b` from `offset`: `Some(n)` bytes or
/// a length-tracking view (`None`). Checks that `b` is a buffer, not detached, and the range
/// is inside it. GC point.
pub(crate) fn new_data_view(
    rt: &Runtime,
    proto: u64,
    b: u64,
    offset: u64,
    len: Option<u64>,
) -> BufResult<u64> {
    if buffer_class(rt, b).is_none() {
        return Err(Fail::Type(
            "First argument to DataView constructor must be an ArrayBuffer",
        ));
    }
    if is_detached(b) {
        return Err(Fail::Type(
            "Cannot perform DataView on a detached ArrayBuffer",
        ));
    }
    let buf_len = byte_length(rt, b) as u64;
    if offset > buf_len || len.is_some_and(|n| offset.checked_add(n).is_none_or(|e| e > buf_len)) {
        return Err(Fail::Range(
            "Start offset is outside the bounds of the buffer",
        ));
    }
    // GC point: `b` lives in a Rust local (CSS).
    let v = object::new_slot_object(rt, proto, ObjectClass::DataView.id(), dv_slot::COUNT);
    // The brand: `v` owns its entry from birth (no GC point since the allocation).
    let j = rt.buffers.insert(v, None, 0);
    object::slot_set(rt, v, dv_slot::TABLE, int32_bits(j));
    object::slot_set(rt, v, dv_slot::BUFFER, b);
    object::slot_set(rt, v, dv_slot::BYTE_OFFSET, crate::conv::num(offset as f64));
    let bl = len.map_or(int32_bits(layout::VIEW_LENGTH_AUTO), |n| {
        crate::conv::num(n as f64)
    });
    object::slot_set(rt, v, dv_slot::BYTE_LENGTH, bl);
    let data = ExternalRef::new(view_data(rt, b, offset as usize));
    object::slot_set(rt, v, dv_slot::DATA, ExternalRef::to_word(data));
    add_view(rt, b, v);
    Ok(v)
}
