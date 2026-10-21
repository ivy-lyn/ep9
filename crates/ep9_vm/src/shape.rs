//! Shapes (H8, H17): hidden classes for JS objects. P1 built the shapes core; P2-OBJ put
//! every JS object on it (word0 shape ids in the shape space, prototype mode, the epoch).
//!
//! ```text
//!  object word0.shape_id ──(cage base | id)──> SHAPE in the shape region (first 256 MiB of the cage)
//!
//!  ShapeState root range [0] ─> PROTO_INFO(null)      PROTO_INFO(P), one per prototype P
//!                                 +24 SLOTS ──strong──> root shapes per (kind, capacity),
//!                                                       dictionary shapes per (kind, extensible)
//!  root ──+32 (weak)──> {a} ──+32──> {a,b} ──+32──> {a,b,c}       +16 parent links are strong
//!     \                   └──+40 TRANSITIONS (weak entries)──> {a,x}
//!      \  {a}, {a,b}, {a,b,c} share one DESCRIPTORS cell [a, b, c]: a shape sees its first
//!       \ `count` entries (the prefix rule). The branch {a,x} has its own copy [a, x].
//!
//!  prototype P ──word0──> unique shape U (PROTOTYPE | UNIQUE, no parent, no transitions)
//!                           +40 ──strong──> PROTO_INFO(P) ──+8──> P
//!  P gets a property: a new unique shape U' (same PROTO_INFO, descriptors by the prefix rule)
//! ```
//!
//! Every word of these cells after word0 is a Value: a cell, undefined, null or an int32.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | SHAPE | 128 | +8 proto, +16 parent, +24 DESCRIPTORS, +32 one transition (weak), +40 TRANSITIONS, or the PROTO_INFO of a prototype-mode shape, +48 counts, +56 flags | 64 |
//! | SLOTS | 71 | +8 capacity, +16 values | 16 + 8 cap |
//! | DESCRIPTORS | 72 | +8 capacity, +16 length, +24 DESC_INDEX or undefined, +32 entries: key, details | 32 + 16 cap |
//! | DESC_INDEX | 73 | +8 bucket count (a power of 2), +16 buckets: descriptor index or undefined | 16 + 8 buckets |
//! | TRANSITIONS | 74 | +8 capacity, +16 child shapes (weak) or undefined | 16 + 8 cap |
//! | PROTO_INFO | 75 | +8 prototype (or null), +16 shape count, +24 SLOTS of shapes, +32 reserved (M1 validity cell) | 40 |
//!
//! - **counts** (int32): property count in bits 0-11, in-object capacity in bits 12-19, the
//!   object kind id in bits 20-27.
//! - **flags** (int32): [`flag`]. A fast shape is always extensible; a non-extensible object
//!   is in dictionary mode (OM-10).
//! - **details** (int32, [`Details`]): attributes in bits 0-3 (writable, enumerable,
//!   configurable, [`ACCESSOR`]), representation in bits 4-6 and const in bit 7 (both
//!   reserved, 0), field index in bits 8-23.
//!
//! Rules:
//! - Only SHAPE lives in the shape space ([`heap_config`], [`register_kinds`]); its cells come
//!   from `Heap::alloc_shape` (W4-P2-SHAPE-ALLOC). An object's word0 id is installed with
//!   `ep9_heap::set_shape` ([`install`]) at allocation and on every change.
//! - An ep9_vm cage is aligned to [`CAGE_ALIGN`] and no larger, so [`shape_of`] finds the
//!   shape of any cell from its address alone: `(address & !(CAGE_ALIGN - 1)) | id`.
//! - Stored keys are atoms ([`key::intern_key`] in [`add`]). A lookup classifies its probe once
//!   ([`key::probe`]): an atom or a symbol compares by bits, any other string by content.
//! - Up to 8 keys a lookup scans; above 8 it uses the DESC_INDEX hash index of the
//!   descriptor array. The append path builds and extends the index, so a lookup never
//!   allocates (it is not a GC point).
//! - A hit at an index at or past the shape's count is a miss (the prefix rule).
//! - Only the last owner of a descriptor array appends in place (its count equals the array
//!   length). Growth copies the array and moves every ancestor that used it to the copy.
//!   Entries below the length never change. A unique shape may append too: every other user
//!   of the array sees only its own prefix.
//! - Transitions and the transition table's entries are weak; a dead child's slot reads
//!   undefined after the GC. Parent links, descriptors and PROTO_INFO lists are strong.
//! - No table is keyed by an address (H12): per-prototype root and dictionary shapes live in
//!   the prototype's PROTO_INFO, null-prototype ones in the ShapeState root range.
//! - Prototype mode (OM-8, OM-11): an object becomes a prototype the first time a shape names
//!   it as `[[Prototype]]` ([`proto_info_for`], on every path: new objects, [`with_proto`]
//!   for a unique shape too). It gets word0 flag IS_PROTOTYPE and a unique shape that holds
//!   its PROTO_INFO at +40. So every shape's proto is null or a prototype-mode object
//!   ([`new_shape`] asserts it in debug builds). Any structural change of a prototype-mode
//!   object bumps [`ShapeState::proto_epoch`] ([`note_change`]): a named property or an
//!   element that appears or goes away, new attributes, `[[SetPrototypeOf]]`,
//!   PreventExtensions, dictionary mode. A value store does not.
//! - The first array-index own property on a prototype-mode object (or an object with one
//!   becoming a prototype) sets the sticky [`proto_index_keys`] flag: until then `[[Set]]`
//!   adds array elements without walking the prototype chain.
//!
//! Hooks the rest of the VM already calls:
//!
//! | Hook | Called from | Job |
//! |---|---|---|
//! | [`heap_config`] | `Isolate::new`, before the heap exists | the shape space (H08-Q1) and the cage alignment |
//! | [`register_kinds`] | `object::register_kinds` | SHAPE (shape space), SLOTS, DESCRIPTORS, DESC_INDEX, TRANSITIONS, PROTO_INFO |
//! | [`init`] | `Runtime::new` | register the ShapeState root range and make the null PROTO_INFO |
//!
//! `Cx::proto_epoch` points at [`ShapeState::proto_epoch`], so a host call or a nested run
//! that changes a prototype is seen at once (never a stale copy).

use core::cell::Cell;
use core::ptr::NonNull;

use ep9_heap::{
    CellKind, CellPtr, CellSize, Heap, HeapConfig, KindId, RootStrength, Slot, Space, Tracer,
};
use ep9_value::Value;

use crate::frame::{NULL, UNDEFINED, int32_bits, int32_of};
use crate::key::{self, Probe};
use crate::kinds as kind;
use crate::object::{self, attr, cell, rd, rd_i32, wr};
use crate::runtime::{Root, Runtime, val};

/// SHAPE fields.
mod shf {
    pub const PROTO: usize = 8;
    pub const PARENT: usize = 16;
    pub const DESCRIPTORS: usize = 24;
    pub const TRANSITION: usize = 32;
    pub const TABLE: usize = 40;
    pub const COUNTS: usize = 48;
    pub const FLAGS: usize = 56;
    pub const SIZE: usize = 64;
}

/// DESCRIPTORS fields.
mod dsc {
    pub const CAPACITY: usize = 8;
    pub const LENGTH: usize = 16;
    pub const INDEX: usize = 24;
    pub const ENTRIES: usize = 32;
    pub const ENTRY: usize = 16;
}

/// DESC_INDEX fields.
mod dix {
    pub const BUCKETS: usize = 8;
    pub const DATA: usize = 16;
}

/// TRANSITIONS fields.
mod trf {
    pub const CAPACITY: usize = 8;
    pub const ENTRIES: usize = 16;
}

/// SLOTS fields (shared with the out-of-line property storage of objects, props.rs).
pub(crate) mod slf {
    pub const CAPACITY: usize = 8;
    pub const DATA: usize = 16;
}

/// PROTO_INFO fields.
mod pif {
    pub const PROTO: usize = 8;
    pub const COUNT: usize = 16;
    pub const SHAPES: usize = 24;
    pub const SIZE: usize = 40;
}

/// Shape flag bits (int32 at +56).
pub(crate) mod flag {
    /// The object may get new properties. Fast shapes always have it (OM-10).
    pub const EXTENSIBLE: i32 = 1;
    /// Properties live in the object's PROPS table; the shape has none and no transitions.
    pub const DICTIONARY: i32 = 2;
    /// The shape of a prototype object; +40 holds its PROTO_INFO.
    pub const PROTOTYPE: i32 = 4;
    /// Not in the transition tree and used by one object only (prototype mode): no parent,
    /// no transitions. A change makes a new unique shape.
    pub const UNIQUE: i32 = 8;
}

/// The cage alignment and largest cage of an ep9_vm isolate (W4-OBJ-CAGE): 64 GiB, the H7
/// default cage size. [`heap_config`] asks for it, so the cage base of any cell is its
/// address with the low 36 bits cleared, and [`shape_of`] needs no heap pointer.
pub(crate) const CAGE_ALIGN: usize = 64 << 30;
/// The shape region ep9_vm asks for (H08-Q1, I45).
pub(crate) const SHAPE_SPACE_BYTES: usize = 256 << 20;

/// The accessor attribute bit of a descriptor (H17, OM-9; P3-ACC stores the pair).
pub(crate) const ACCESSOR: i32 = 8;
/// The attribute bits a descriptor stores: writable, enumerable, configurable, accessor.
pub(crate) const ATTR_MASK: i32 = attr::WRITABLE | attr::ENUMERABLE | attr::CONFIGURABLE | ACCESSOR;
/// Up to this many keys a lookup scans the descriptors; above it uses the hash index.
pub(crate) const LINEAR_MAX: usize = 8;
/// Live transitions one shape may have; [`add`] reports [`Added::Dictionary`] past it.
pub(crate) const MAX_TRANSITIONS: usize = 64;
/// Properties a fast shape may have (V8 kMaxNumberOfDescriptors); more is dictionary mode.
pub(crate) const MAX_FAST_PROPERTIES: usize = 1020;

const MIN_DESCRIPTORS: usize = 4;
const MIN_TRANSITIONS: usize = 4;
const MIN_BUCKETS: usize = 16;
const MIN_SLOTS: usize = 4;

const COUNT_MASK: i32 = 0xFFF;
const CAPACITY_SHIFT: u32 = 12;
const KIND_SHIFT: u32 = 20;

/// Details bits 4-7: representation (3 bits) and const (1 bit), reserved and 0 in v0.
const RESERVED_DETAILS: i32 = 0xF0;
const FIELD_SHIFT: u32 = 8;

/// ShapeState root range slots.
mod root {
    /// The PROTO_INFO of the null prototype.
    pub const NULL_PROTO_INFO: usize = 0;
    /// The shape of a new function: %Function.prototype%, `length` and `name` (a cache;
    /// undefined until the first function).
    pub const FUNCTION_SHAPE: usize = 1;
    pub const COUNT: usize = 2;
}

/// Per-isolate shape state (a `Runtime` field).
#[derive(Debug)]
pub(crate) struct ShapeState {
    /// Bumped on every structural change of a prototype (OM-8); prototype IC handlers compare
    /// it.
    pub proto_epoch: Cell<u32>,
    /// Sticky, never cleared: set once a prototype-mode object has an array-index own
    /// property ([`proto_index_keys`]). While it is false no prototype chain has an index key,
    /// so `[[Set]]` may add a missing array element without the walk (V8's "no elements"
    /// protector).
    proto_index: Cell<bool>,
    /// A strong root range ([`init`]): see [`root`].
    roots: Box<[Slot]>,
}

impl Default for ShapeState {
    fn default() -> Self {
        ShapeState {
            proto_epoch: Cell::new(0),
            proto_index: Cell::new(false),
            roots: (0..root::COUNT)
                .map(|_| Slot::new(Value::UNDEFINED))
                .collect(),
        }
    }
}

/// The heap settings shapes need: a shape space of [`SHAPE_SPACE_BYTES`] (unless the
/// embedder asked for one), and a cage of at most [`CAGE_ALIGN`] bytes aligned to
/// [`CAGE_ALIGN`] (W4-OBJ-CAGE; the H7 default cage is exactly that size).
pub(crate) fn heap_config(mut cfg: HeapConfig) -> HeapConfig {
    if cfg.shape_space_bytes == 0 {
        cfg.shape_space_bytes = SHAPE_SPACE_BYTES;
    }
    cfg.cage.cage_size = cfg.cage.cage_size.min(CAGE_ALIGN);
    cfg.cage.align = cfg.cage.align.max(CAGE_ALIGN);
    cfg
}

// ---- kinds ----------------------------------------------------------------------------------

fn trace_shape(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: the heap passes a live SHAPE cell; +8 .. +64 are its 7 Value slots.
    let s = unsafe { c.slots(8, 7) };
    t.visit(&s[0]); // proto
    t.visit(&s[1]); // parent
    t.visit(&s[2]); // descriptors
    t.visit_weak(&s[3]); // the single transition
    t.visit(&s[4]); // the TRANSITIONS table (its entries are weak) or a PROTO_INFO
}

fn slots_size(c: CellPtr) -> usize {
    slf::DATA + 8 * rd_i32(c, slf::CAPACITY) as usize
}

fn trace_slots(c: CellPtr, t: &mut Tracer<'_>) {
    object::trace_from_8(c, t, slots_size(c));
}

fn descriptors_size(c: CellPtr) -> usize {
    dsc::ENTRIES + dsc::ENTRY * rd_i32(c, dsc::CAPACITY) as usize
}

fn trace_descriptors(c: CellPtr, t: &mut Tracer<'_>) {
    object::trace_from_8(c, t, descriptors_size(c));
}

fn index_size(c: CellPtr) -> usize {
    dix::DATA + 8 * rd_i32(c, dix::BUCKETS) as usize
}

/// DESC_INDEX holds only int32s and undefined.
fn trace_index(_: CellPtr, _: &mut Tracer<'_>) {}

fn transitions_size(c: CellPtr) -> usize {
    trf::ENTRIES + 8 * rd_i32(c, trf::CAPACITY) as usize
}

fn trace_transitions(c: CellPtr, t: &mut Tracer<'_>) {
    let n = rd_i32(c, trf::CAPACITY) as usize;
    // SAFETY: the heap passes a live TRANSITIONS cell; it has `n` Value slots from +16.
    for s in unsafe { c.slots(trf::ENTRIES, n) } {
        t.visit_weak(s);
    }
}

fn trace_proto_info(c: CellPtr, t: &mut Tracer<'_>) {
    object::trace_from_8(c, t, pif::SIZE);
}

/// Registers the shape cell kinds: SHAPE in the shape space (H08-Q1), the others in the
/// normal space.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind_in(
        kind::SHAPE,
        CellKind {
            name: "shape",
            size: CellSize::Fixed(shf::SIZE),
            trace: trace_shape,
        },
        Space::Shape,
    )
    .expect("register SHAPE in the shape space");
    let kinds: [(KindId, CellKind); 5] = [
        (
            kind::SLOTS,
            CellKind {
                name: "slots",
                size: CellSize::Dynamic(slots_size),
                trace: trace_slots,
            },
        ),
        (
            kind::DESCRIPTORS,
            CellKind {
                name: "descriptors",
                size: CellSize::Dynamic(descriptors_size),
                trace: trace_descriptors,
            },
        ),
        (
            kind::DESC_INDEX,
            CellKind {
                name: "descriptor-index",
                size: CellSize::Dynamic(index_size),
                trace: trace_index,
            },
        ),
        (
            kind::TRANSITIONS,
            CellKind {
                name: "transitions",
                size: CellSize::Dynamic(transitions_size),
                trace: trace_transitions,
            },
        ),
        (
            kind::PROTO_INFO,
            CellKind {
                name: "proto-info",
                size: CellSize::Fixed(pif::SIZE),
                trace: trace_proto_info,
            },
        ),
    ];
    for (id, k) in kinds {
        heap.register_kind(id, k).expect("register a VM cell kind");
    }
}

/// Runs once in `Runtime::new`, after the kinds are registered and before the realm exists:
/// registers the ShapeState root range and makes the PROTO_INFO of the null prototype.
pub(crate) fn init(rt: &Runtime) {
    // [`shape_of`] derives the cage base from a cell address; `heap_config` asked for this.
    assert!(
        rt.heap.cage_base().is_multiple_of(CAGE_ALIGN) && rt.heap.shape_space_bytes() != 0,
        "an ep9_vm heap needs a shape space and a cage aligned to CAGE_ALIGN (shape::heap_config)"
    );
    let roots = &rt.shapes.roots;
    // SAFETY: the slots live in a box that the Runtime owns. The heap is the Runtime's first
    // field, so it drops (with this range) before the box. They hold `undefined` now and
    // Value bits later (never 0).
    unsafe {
        rt.heap
            .add_root_range(NonNull::from(&roots[..]), RootStrength::Strong);
    }
    let info = new_proto_info(rt, NULL);
    // E3: a store into a root takes no barrier.
    roots[root::NULL_PROTO_INFO].init(val(info));
}

// ---- keys -----------------------------------------------------------------------------------
//
// The only calls into key.rs (H19 fills them in P1-STR).

#[inline]
fn classify(_rt: &Runtime, k: u64) -> Probe {
    key::probe(k)
}

#[inline]
fn same_key(stored: u64, k: u64, how: Probe) -> bool {
    match how {
        Probe::Bits => stored == k,
        Probe::Content => key::key_eq(stored, k),
    }
}

#[inline]
fn hash_of(k: u64) -> u32 {
    key::key_hash(k)
}

// ---- details --------------------------------------------------------------------------------

/// A descriptor's details word: attributes and field index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Details(i32);

impl Details {
    pub(crate) fn new(attrs: i32, field: u16) -> Details {
        debug_assert!(attrs & !ATTR_MASK == 0, "bad attributes {attrs:#x}");
        Details(attrs | (i32::from(field) << FIELD_SHIFT))
    }

    /// Writable, enumerable, configurable ([`attr`]) and [`ACCESSOR`].
    pub(crate) fn attrs(self) -> i32 {
        self.0 & ATTR_MASK
    }

    #[cfg_attr(not(test), allow(dead_code))] // P3-ACC reads it
    pub(crate) fn is_accessor(self) -> bool {
        self.0 & ACCESSOR != 0
    }

    /// The object slot of the property (v0: the descriptor index).
    pub(crate) fn field(self) -> u16 {
        (self.0 >> FIELD_SHIFT) as u16
    }

    fn bits(self) -> u64 {
        int32_bits(self.0)
    }

    fn from_bits(b: u64) -> Details {
        debug_assert!(val(b).is_int32());
        let d = int32_of(b);
        debug_assert!(
            d & RESERVED_DETAILS == 0,
            "reserved details bits set: {d:#x}"
        );
        Details(d)
    }
}

/// A lookup hit: the descriptor index and its details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub index: usize,
    pub details: Details,
}

/// What [`add`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Added {
    /// The child shape (an existing transition or a new one).
    Shape(u64),
    /// The object must switch to dictionary mode: the shape is a dictionary shape, the key
    /// is an array index, or a limit was reached (properties, transitions).
    Dictionary,
}

// ---- readers --------------------------------------------------------------------------------

/// A JS object kind id (16-63, kind table v1).
fn is_object_kind(k: KindId) -> bool {
    (16..=63).contains(&k.get())
}

#[inline]
pub(crate) fn is_shape(b: u64) -> bool {
    kind::cell_kind(b) == kind::SHAPE.get()
}

#[inline]
fn counts(s: CellPtr) -> i32 {
    rd_i32(s, shf::COUNTS)
}

#[inline]
fn count_of(s: CellPtr) -> usize {
    (counts(s) & COUNT_MASK) as usize
}

fn pack_counts(count: usize, capacity: u8, object_kind: u8) -> u64 {
    debug_assert!(count <= MAX_FAST_PROPERTIES);
    int32_bits(
        count as i32
            | (i32::from(capacity) << CAPACITY_SHIFT)
            | (i32::from(object_kind) << KIND_SHIFT),
    )
}

/// The number of properties of `shape` (0 for a dictionary shape).
pub(crate) fn count(shape: u64) -> usize {
    count_of(cell(shape))
}

/// The in-object slot capacity of objects with `shape`.
pub(crate) fn capacity(shape: u64) -> u8 {
    (counts(cell(shape)) >> CAPACITY_SHIFT) as u8
}

/// The kind id of objects with `shape`.
pub(crate) fn object_kind(shape: u64) -> u8 {
    (counts(cell(shape)) >> KIND_SHIFT) as u8
}

/// (in-object capacity, object kind id) of objects with `shape`, from one load: the field
/// address of a property needs both.
#[inline(always)]
pub(crate) fn field_layout(shape: u64) -> (usize, u8) {
    let c = counts(cell(shape));
    (
        usize::from((c >> CAPACITY_SHIFT) as u8),
        (c >> KIND_SHIFT) as u8,
    )
}

pub(crate) fn flags(shape: u64) -> i32 {
    rd_i32(cell(shape), shf::FLAGS)
}

pub(crate) fn is_dictionary(shape: u64) -> bool {
    flags(shape) & flag::DICTIONARY != 0
}

pub(crate) fn is_extensible(shape: u64) -> bool {
    flags(shape) & flag::EXTENSIBLE != 0
}

/// The `[[Prototype]]` of objects with `shape` (an object or null).
pub(crate) fn proto(shape: u64) -> u64 {
    rd(cell(shape), shf::PROTO)
}

/// The shape before the last property was added; undefined for a root or dictionary shape.
pub(crate) fn parent(shape: u64) -> u64 {
    rd(cell(shape), shf::PARENT)
}

/// The DESCRIPTORS cell of `shape`, or undefined if it has none.
pub(crate) fn descriptors(shape: u64) -> u64 {
    rd(cell(shape), shf::DESCRIPTORS)
}

#[inline]
fn key_off(i: usize) -> usize {
    dsc::ENTRIES + dsc::ENTRY * i
}

#[inline]
fn d_len(d: CellPtr) -> usize {
    rd_i32(d, dsc::LENGTH) as usize
}

#[inline]
fn d_cap(d: CellPtr) -> usize {
    rd_i32(d, dsc::CAPACITY) as usize
}

#[inline]
fn d_key(d: CellPtr, i: usize) -> u64 {
    rd(d, key_off(i))
}

#[inline]
fn d_details(d: CellPtr, i: usize) -> Details {
    Details::from_bits(rd(d, key_off(i) + 8))
}

/// The key of property `i` of `shape` (`i < count(shape)`), in property order.
pub(crate) fn key_at(shape: u64, i: usize) -> u64 {
    let s = cell(shape);
    assert!(i < count_of(s), "descriptor {i} past the shape's count");
    d_key(cell(rd(s, shf::DESCRIPTORS)), i)
}

/// The details of property `i` of `shape` (`i < count(shape)`).
pub(crate) fn details_at(shape: u64, i: usize) -> Details {
    let s = cell(shape);
    assert!(i < count_of(s), "descriptor {i} past the shape's count");
    d_details(cell(rd(s, shf::DESCRIPTORS)), i)
}

/// The live transitions out of `shape` (children that the last GC kept or that are new).
pub(crate) fn transition_count(shape: u64) -> usize {
    let s = cell(shape);
    let mut n = usize::from(rd(s, shf::TRANSITION) != UNDEFINED);
    let tab = rd(s, shf::TABLE);
    if tab != UNDEFINED && kind::cell_kind(tab) == kind::TRANSITIONS.get() {
        let t = cell(tab);
        n += (0..rd_i32(t, trf::CAPACITY) as usize)
            .filter(|&i| rd(t, trf::ENTRIES + 8 * i) != UNDEFINED)
            .count();
    }
    n
}

// ---- lookup ---------------------------------------------------------------------------------

/// The own property `key` of objects with `shape`: its descriptor index and details. Not a GC
/// point. A dictionary shape has no descriptors (the object's PROPS table holds them).
pub(crate) fn lookup(rt: &Runtime, shape: u64, key: u64) -> Option<Found> {
    debug_assert!(is_shape(shape));
    if count_of(cell(shape)) == 0 {
        return None;
    }
    lookup_with(shape, key, classify(rt, key))
}

#[cfg(test)]
thread_local! {
    /// Tests (W4-P1-GLOBAL-ONE-SCAN): a shape and how many times [`lookup_with`] scanned it.
    pub(crate) static SCANS: Cell<(u64, u32)> = const { Cell::new((0, 0)) };
}

/// [`lookup`] with a probe the caller classified once ([`key::probe`]) for a walk over
/// several shapes. Not a GC point.
pub(crate) fn lookup_with(shape: u64, key: u64, how: Probe) -> Option<Found> {
    debug_assert!(is_shape(shape));
    #[cfg(test)]
    SCANS.with(|c| {
        let (watched, n) = c.get();
        if watched == shape {
            c.set((watched, n + 1));
        }
    });
    let s = cell(shape);
    let n = count_of(s);
    if n == 0 {
        return None;
    }
    let d = cell(rd(s, shf::DESCRIPTORS));
    // The index exists only past LINEAR_MAX keys, so short shapes do not load it.
    let ix = if n <= LINEAR_MAX {
        UNDEFINED
    } else {
        rd(d, dsc::INDEX)
    };
    if ix == UNDEFINED {
        debug_assert!(n <= LINEAR_MAX, "{n} descriptors and no index");
        return (0..n)
            .find(|&i| same_key(d_key(d, i), key, how))
            .map(|i| Found {
                index: i,
                details: d_details(d, i),
            });
    }
    let ix = cell(ix);
    let mask = rd_i32(ix, dix::BUCKETS) as usize - 1;
    let mut b = hash_of(key) as usize & mask;
    loop {
        let e = rd(ix, dix::DATA + 8 * b);
        if e == UNDEFINED {
            return None;
        }
        // An entry past this shape's count belongs to a longer shape (the prefix rule): it is
        // not a key of `shape`, so the probe goes on. Keys are distinct in one array.
        let i = int32_of(e) as usize;
        if i < n && same_key(d_key(d, i), key, how) {
            return Some(Found {
                index: i,
                details: d_details(d, i),
            });
        }
        b = (b + 1) & mask;
    }
}

// ---- add and remove -------------------------------------------------------------------------

/// The shape of an object with `shape` after it gets the new own property `key` with
/// `attrs` ([`attr`] bits plus [`ACCESSOR`]): the existing transition or a new child. GC
/// point. `key` must not be a property of `shape` already.
pub(crate) fn add(rt: &Runtime, shape: u64, key: u64, attrs: i32) -> Added {
    debug_assert!(is_shape(shape));
    debug_assert!(attrs & !ATTR_MASK == 0, "bad attributes {attrs:#x}");
    let fl = flags(shape);
    if fl & flag::DICTIONARY != 0 {
        return Added::Dictionary;
    }
    let key = key::intern_key(rt, key);
    debug_assert!(
        lookup(rt, shape, key).is_none(),
        "add of a key the shape already has"
    );
    let how = classify(rt, key);
    if fl & flag::UNIQUE != 0 {
        return add_unique(rt, shape, key, attrs, how);
    }
    if let Some(child) = find_transition(shape, key, attrs, how) {
        return Added::Shape(child);
    }
    let n = count(shape);
    if n >= MAX_FAST_PROPERTIES
        || transition_count(shape) >= MAX_TRANSITIONS
        || key::is_index_key(key)
    {
        return Added::Dictionary;
    }
    // GC points below: `shape`, `key`, `d` and `child` live in Rust locals (CSS).
    let d = descriptors_for_child(rt, shape, key, Details::new(attrs, n as u16), how);
    let child = new_shape(
        rt,
        proto(shape),
        shape,
        d,
        pack_counts(n + 1, capacity(shape), object_kind(shape)),
        fl,
    );
    record_transition(rt, shape, child);
    Added::Shape(child)
}

/// The shape before the last property of `shape` was added (the parent). Not a GC point.
pub(crate) fn remove_last(shape: u64) -> u64 {
    debug_assert!(is_shape(shape));
    debug_assert!(
        flags(shape) & (flag::DICTIONARY | flag::UNIQUE) == 0,
        "remove_last on a dictionary or unique shape"
    );
    let p = parent(shape);
    assert!(p != UNDEFINED, "remove_last on a root shape");
    p
}

/// [`add`] on a unique (prototype-mode) shape: a new unique shape with one more property,
/// the same prototype, flags and PROTO_INFO. The descriptor array follows the prefix rule,
/// so it may be shared with tree shapes. GC point.
fn add_unique(rt: &Runtime, shape: u64, key: u64, attrs: i32, how: Probe) -> Added {
    let n = count(shape);
    if n >= MAX_FAST_PROPERTIES || key::is_index_key(key) {
        return Added::Dictionary;
    }
    let d = descriptors_for_child(rt, shape, key, Details::new(attrs, n as u16), how);
    let counts = pack_counts(n + 1, capacity(shape), object_kind(shape));
    Added::Shape(new_unique(
        rt,
        proto(shape),
        d,
        counts,
        flags(shape),
        unique_info(shape),
    ))
}

/// [`remove_last`] for a unique (prototype-mode) shape: a new unique shape without the last
/// property. GC point.
pub(crate) fn remove_last_unique(rt: &Runtime, shape: u64) -> u64 {
    debug_assert!(flags(shape) & (flag::DICTIONARY | flag::UNIQUE) == flag::UNIQUE);
    let n = count(shape);
    assert!(n > 0, "remove_last_unique on a shape without properties");
    let d = if n == 1 {
        UNDEFINED
    } else {
        descriptors(shape)
    };
    let counts = pack_counts(n - 1, capacity(shape), object_kind(shape));
    new_unique(
        rt,
        proto(shape),
        d,
        counts,
        flags(shape),
        unique_info(shape),
    )
}

/// The PROTO_INFO a unique shape carries.
fn unique_info(shape: u64) -> u64 {
    debug_assert!(flags(shape) & flag::UNIQUE != 0);
    rd(cell(shape), shf::TABLE)
}

/// A new unique shape (prototype mode) that holds `info` at +40. GC point.
fn new_unique(rt: &Runtime, proto: u64, d: u64, counts: u64, flags: i32, info: u64) -> u64 {
    debug_assert!(kind::cell_kind(info) == kind::PROTO_INFO.get());
    let s = new_shape(
        rt,
        proto,
        UNDEFINED,
        d,
        counts,
        flags | flag::PROTOTYPE | flag::UNIQUE,
    );
    // E2: the cell is new and nothing ran since its allocation.
    object::init(cell(s), shf::TABLE, info);
    s
}

/// The child of `shape` for the key and attributes, if the transition is live.
fn find_transition(shape: u64, key: u64, attrs: i32, how: Probe) -> Option<u64> {
    let s = cell(shape);
    let t = rd(s, shf::TRANSITION);
    if t != UNDEFINED && is_transition_to(t, key, attrs, how) {
        return Some(t);
    }
    let tab = rd(s, shf::TABLE);
    if tab == UNDEFINED {
        return None;
    }
    debug_assert!(kind::cell_kind(tab) == kind::TRANSITIONS.get());
    let tc = cell(tab);
    (0..rd_i32(tc, trf::CAPACITY) as usize)
        .map(|i| rd(tc, trf::ENTRIES + 8 * i))
        .find(|&c| c != UNDEFINED && is_transition_to(c, key, attrs, how))
}

/// True if `child` added `key` with `attrs` (its last descriptor).
fn is_transition_to(child: u64, key: u64, attrs: i32, how: Probe) -> bool {
    let c = cell(child);
    let last = count_of(c) - 1;
    let d = cell(rd(c, shf::DESCRIPTORS));
    d_details(d, last).attrs() == attrs && same_key(d_key(d, last), key, how)
}

/// The descriptor array of the child of `shape` that adds (`key`, `details`). GC point.
fn descriptors_for_child(rt: &Runtime, shape: u64, key: u64, details: Details, how: Probe) -> u64 {
    let n = count(shape);
    let old = descriptors(shape);
    if old == UNDEFINED {
        debug_assert!(n == 0);
        let d = new_descriptors(rt, MIN_DESCRIPTORS);
        return append(rt, d, key, details);
    }
    let od = cell(old);
    let len = d_len(od);
    debug_assert!(n <= len);
    if len == n {
        // `shape` is the last owner: append in place, or grow and move the whole chain.
        if n < d_cap(od) {
            return append(rt, old, key, details);
        }
        let nd = copy_descriptors(rt, od, n, 2 * n);
        let nd = append(rt, nd, key, details);
        let mut p = shape;
        while p != UNDEFINED && descriptors(p) == old {
            wr(&rt.heap, cell(p), shf::DESCRIPTORS, nd);
            p = parent(p);
        }
        return nd;
    }
    if d_details(od, n) == details && same_key(d_key(od, n), key, how) {
        // A child that died added this very entry: share the array again.
        return old;
    }
    // A branch: copy the prefix.
    let nd = copy_descriptors(rt, od, n, (n + 1).next_power_of_two().max(MIN_DESCRIPTORS));
    append(rt, nd, key, details)
}

fn new_descriptors(rt: &Runtime, cap: usize) -> u64 {
    let d = rt.alloc(kind::DESCRIPTORS, dsc::ENTRIES + dsc::ENTRY * cap);
    object::init(d, dsc::CAPACITY, int32_bits(cap as i32));
    object::init(d, dsc::LENGTH, int32_bits(0));
    d.to_value().to_bits()
}

/// A new array with the first `n` entries of `from` and room for `cap`. GC point.
fn copy_descriptors(rt: &Runtime, from: CellPtr, n: usize, cap: usize) -> u64 {
    debug_assert!(n < cap);
    let nd = new_descriptors(rt, cap);
    let d = cell(nd);
    for i in 0..n {
        object::init(d, key_off(i), rd(from, key_off(i)));
        object::init(d, key_off(i) + 8, rd(from, key_off(i) + 8));
    }
    object::init(d, dsc::LENGTH, int32_bits(n as i32));
    nd
}

/// Appends an entry to the array `d` (it has room) and keeps its index. GC point (the index
/// may be rebuilt). Returns `d`.
fn append(rt: &Runtime, d: u64, key: u64, details: Details) -> u64 {
    let dc = cell(d);
    let len = d_len(dc);
    debug_assert!(len < d_cap(dc));
    wr(&rt.heap, dc, key_off(len), key);
    wr(&rt.heap, dc, key_off(len) + 8, details.bits());
    wr(&rt.heap, dc, dsc::LENGTH, int32_bits(len as i32 + 1));
    let len = len + 1;
    if len > LINEAR_MAX {
        let ix = rd(dc, dsc::INDEX);
        if ix != UNDEFINED && 2 * len <= rd_i32(cell(ix), dix::BUCKETS) as usize {
            index_insert(cell(ix), hash_of(key), len - 1);
        } else {
            build_index(rt, d);
        }
    }
    d
}

/// Gives `d` a new hash index of all its entries. GC point.
fn build_index(rt: &Runtime, d: u64) {
    let len = d_len(cell(d));
    let buckets = (2 * len).next_power_of_two().max(MIN_BUCKETS);
    let ix = rt.alloc(kind::DESC_INDEX, dix::DATA + 8 * buckets);
    object::init(ix, dix::BUCKETS, int32_bits(buckets as i32));
    let dc = cell(d);
    for i in 0..len {
        index_insert(ix, hash_of(d_key(dc, i)), i);
    }
    wr(&rt.heap, dc, dsc::INDEX, ix.to_value().to_bits());
}

/// Stores descriptor index `i` in the first free bucket from `hash` (linear probing).
fn index_insert(ix: CellPtr, hash: u32, i: usize) {
    let mask = rd_i32(ix, dix::BUCKETS) as usize - 1;
    let mut b = hash as usize & mask;
    while rd(ix, dix::DATA + 8 * b) != UNDEFINED {
        b = (b + 1) & mask;
    }
    // E1: an int32 store needs no barrier.
    object::init(ix, dix::DATA + 8 * b, int32_bits(i as i32));
}

/// A new SHAPE cell in the shape space. GC point. v0: a full shape space is fatal, as a full
/// heap is (`Runtime::alloc`; H23 adds the OOM callback).
fn new_shape(rt: &Runtime, proto: u64, parent: u64, d: u64, counts: u64, flags: i32) -> u64 {
    // OM-8: a `[[Prototype]]` is in prototype mode ([`proto_info_for`]), or a change to it
    // would not bump the epoch.
    debug_assert!(
        proto == NULL || is_prototype_object(proto),
        "a shape whose proto {proto:#x} is not in prototype mode"
    );
    let s = match rt.heap.alloc_shape(kind::SHAPE, shf::SIZE) {
        Ok(c) => c,
        Err(e) => panic!("ep9: shape allocation failed: {e:?}"),
    };
    object::init(s, shf::PROTO, proto);
    object::init(s, shf::PARENT, parent);
    object::init(s, shf::DESCRIPTORS, d);
    object::init(s, shf::COUNTS, counts);
    object::init(s, shf::FLAGS, int32_bits(flags));
    s.to_value().to_bits()
}

/// Records `child` as a (weak) transition of `shape`. GC point.
fn record_transition(rt: &Runtime, shape: u64, child: u64) {
    let s = cell(shape);
    if rd(s, shf::TRANSITION) == UNDEFINED {
        wr(&rt.heap, s, shf::TRANSITION, child);
        return;
    }
    let tab = rd(s, shf::TABLE);
    if tab != UNDEFINED {
        let t = cell(tab);
        let cap = rd_i32(t, trf::CAPACITY) as usize;
        if let Some(i) = (0..cap).find(|&i| rd(t, trf::ENTRIES + 8 * i) == UNDEFINED) {
            wr(&rt.heap, t, trf::ENTRIES + 8 * i, child);
            return;
        }
    }
    let cap = if tab == UNDEFINED {
        MIN_TRANSITIONS
    } else {
        2 * rd_i32(cell(tab), trf::CAPACITY) as usize
    };
    let nt = rt.alloc(kind::TRANSITIONS, trf::ENTRIES + 8 * cap);
    object::init(nt, trf::CAPACITY, int32_bits(cap as i32));
    // The allocation may have collected: copy the entries that are still live.
    let mut n = 0;
    if tab != UNDEFINED {
        let t = cell(tab);
        for i in 0..rd_i32(t, trf::CAPACITY) as usize {
            let c = rd(t, trf::ENTRIES + 8 * i);
            if c != UNDEFINED {
                object::init(nt, trf::ENTRIES + 8 * n, c);
                n += 1;
            }
        }
    }
    object::init(nt, trf::ENTRIES + 8 * n, child);
    wr(&rt.heap, s, shf::TABLE, nt.to_value().to_bits());
}

// ---- root and dictionary shapes -------------------------------------------------------------

/// A new PROTO_INFO for the prototype `proto` (an object, or null for the one in the
/// ShapeState roots). GC point.
pub(crate) fn new_proto_info(rt: &Runtime, proto: u64) -> u64 {
    debug_assert!(proto == NULL || kind::is_object(proto));
    let c = rt.alloc(kind::PROTO_INFO, pif::SIZE);
    object::init(c, pif::PROTO, proto);
    object::init(c, pif::COUNT, int32_bits(0));
    c.to_value().to_bits()
}

/// The PROTO_INFO of the null prototype.
pub(crate) fn null_proto_info(rt: &Runtime) -> u64 {
    rt.shapes.roots[root::NULL_PROTO_INFO].bits()
}

/// The number of root and dictionary shapes `proto_info` holds.
#[cfg(test)]
pub(crate) fn proto_info_len(proto_info: u64) -> usize {
    rd_i32(cell(proto_info), pif::COUNT) as usize
}

/// The root shape for objects of `object_kind` with `capacity` in-object slots whose
/// prototype is the one of `proto_info`. Made once and kept by the PROTO_INFO. GC point.
pub(crate) fn root_shape_for(
    rt: &Runtime,
    proto_info: u64,
    object_kind: KindId,
    capacity: u8,
) -> u64 {
    debug_assert!(is_object_kind(object_kind));
    let want = pack_counts(0, capacity, object_kind.get());
    if let Some(s) = find_in_info(proto_info, |s| {
        flags(s) & flag::DICTIONARY == 0 && rd(cell(s), shf::COUNTS) == want
    }) {
        return s;
    }
    let p = rd(cell(proto_info), pif::PROTO);
    let s = new_shape(rt, p, UNDEFINED, UNDEFINED, want, flag::EXTENSIBLE);
    info_push(rt, proto_info, s);
    s
}

/// The shared dictionary shape for objects of `object_kind` whose prototype is the one of
/// `proto_info`. GC point.
pub(crate) fn dictionary_shape_for(
    rt: &Runtime,
    proto_info: u64,
    object_kind: KindId,
    extensible: bool,
) -> u64 {
    debug_assert!(is_object_kind(object_kind));
    let fl = flag::DICTIONARY | if extensible { flag::EXTENSIBLE } else { 0 };
    let want = pack_counts(0, 0, object_kind.get());
    if let Some(s) = find_in_info(proto_info, |s| {
        flags(s) == fl && rd(cell(s), shf::COUNTS) == want
    }) {
        return s;
    }
    let p = rd(cell(proto_info), pif::PROTO);
    let s = new_shape(rt, p, UNDEFINED, UNDEFINED, want, fl);
    info_push(rt, proto_info, s);
    s
}

fn find_in_info(proto_info: u64, pred: impl Fn(u64) -> bool) -> Option<u64> {
    let info = cell(proto_info);
    let n = rd_i32(info, pif::COUNT) as usize;
    if n == 0 {
        return None;
    }
    let list = cell(rd(info, pif::SHAPES));
    (0..n)
        .map(|i| rd(list, slf::DATA + 8 * i))
        .find(|&s| pred(s))
}

/// Appends `shape` to the list of `proto_info`. GC point.
fn info_push(rt: &Runtime, proto_info: u64, shape: u64) {
    let info = cell(proto_info);
    let n = rd_i32(info, pif::COUNT) as usize;
    let old = rd(info, pif::SHAPES);
    let cap = if old == UNDEFINED {
        0
    } else {
        rd_i32(cell(old), slf::CAPACITY) as usize
    };
    let list = if n < cap {
        cell(old)
    } else {
        let new_cap = (2 * cap).max(MIN_SLOTS);
        let l = rt.alloc(kind::SLOTS, slf::DATA + 8 * new_cap);
        object::init(l, slf::CAPACITY, int32_bits(new_cap as i32));
        for i in 0..n {
            object::init(l, slf::DATA + 8 * i, rd(cell(old), slf::DATA + 8 * i));
        }
        wr(&rt.heap, info, pif::SHAPES, l.to_value().to_bits());
        l
    };
    wr(&rt.heap, list, slf::DATA + 8 * n, shape);
    wr(&rt.heap, info, pif::COUNT, int32_bits(n as i32 + 1));
}

// ---- objects: word0, prototype mode, the epoch ----------------------------------------------

/// The shape of the JS object `o` (word0's ShapeId). Not a GC point; needs no heap pointer:
/// the cage is aligned to [`CAGE_ALIGN`] and the shape region starts at its base.
#[inline(always)]
pub(crate) fn shape_of(o: u64) -> u64 {
    // SAFETY: a cell value the VM holds is live (crate invariant, object.rs).
    let id = unsafe { cell(o).header() }.shape_id().get();
    debug_assert!(id != 0, "{o:#x} has no shape");
    let s = (o & !(CAGE_ALIGN as u64 - 1)) | u64::from(id);
    debug_assert!(is_shape(s), "word0 of {o:#x} does not name a shape");
    s
}

/// Installs `shape` in word0 of the JS object `o` with the shape barrier (barrier v2; OM-4).
/// Not a GC point.
#[inline]
pub(crate) fn install(rt: &Runtime, o: u64, shape: u64) {
    debug_assert!(is_shape(shape) && kind::is_object(o));
    debug_assert_eq!(
        object_kind(shape),
        kind::cell_kind(o),
        "shape of another kind"
    );
    let id = rt.heap.shape_id_of(cell(shape));
    // SAFETY: `o` is live (crate invariant).
    ep9_heap::set_shape(unsafe { cell(o).header() }, id, rt.heap.barrier());
}

/// True if `o` is in prototype mode (word0 flag IS_PROTOTYPE).
#[inline]
pub(crate) fn is_prototype_object(o: u64) -> bool {
    // SAFETY: `o` is live (crate invariant).
    unsafe { cell(o).header() }.flags() & kind::FLAG_IS_PROTOTYPE != 0
}

/// Records a structural change of `o` (add, delete, attribute change, `[[SetPrototypeOf]]`,
/// dictionary mode, PreventExtensions): bumps the epoch if `o` is a prototype (OM-8).
#[inline]
pub(crate) fn note_change(rt: &Runtime, o: u64) {
    if is_prototype_object(o) {
        let e = &rt.shapes.proto_epoch;
        e.set(e.get().wrapping_add(1));
    }
}

/// True if some prototype-mode object has, or once had, an array-index own property: an
/// element, a stored index key or a String wrapper index. False means no prototype chain
/// has an index key, so an inherited setter, getter or read-only property cannot decide a
/// `[[Set]]` of an index (§10.1.9.2). One load; not a GC point.
#[inline(always)]
pub(crate) fn proto_index_keys(rt: &Runtime) -> bool {
    rt.shapes.proto_index.get()
}

/// Records that the prototype-mode object `o` gets an array-index own property (sets the
/// flag of [`proto_index_keys`]). The callers check that `o` is a prototype and the key an
/// index: `props::own_add` and `object::array_set`. Not a GC point.
#[inline]
pub(crate) fn note_proto_index_key(rt: &Runtime, o: u64) {
    debug_assert!(is_prototype_object(o));
    rt.shapes.proto_index.set(true);
}

/// True if the object `o` may have an array-index own property: an array with a non-zero
/// `length` (holes too), a String wrapper of a non-empty string, or a stored index key. Not a
/// GC point. Out of line: [`proto_info_for`] runs it once per new prototype.
#[cold]
#[inline(never)]
fn may_have_index_keys(o: u64) -> bool {
    let wrapped = crate::wrapper::wrapper_value(o);
    (kind::is_array(o) && object::array_len(o) > 0)
        || wrapped.is_some_and(|s| kind::is_string(s) && object::str_len(s) > 0)
        || (0..object::own_count(o))
            .any(|i| key::array_index_of(object::own_entry(o, i).0).is_some())
}

/// The PROTO_INFO for the prototype `p` (an object, or null). The first call for an object
/// puts it in prototype mode: a new PROTO_INFO, word0 flag IS_PROTOTYPE and a unique shape
/// that holds the PROTO_INFO. GC point.
pub(crate) fn proto_info_for(rt: &Runtime, p: u64) -> u64 {
    if p == NULL {
        return null_proto_info(rt);
    }
    debug_assert!(kind::is_object(p));
    if is_prototype_object(p) {
        return unique_info(shape_of(p));
    }
    // GC points below: `p` and `info` live in Rust locals (CSS).
    let info = new_proto_info(rt, p);
    let old = shape_of(p);
    let fl = flags(old) & (flag::EXTENSIBLE | flag::DICTIONARY);
    let u = new_unique(
        rt,
        proto(old),
        descriptors(old),
        rd(cell(old), shf::COUNTS),
        fl,
        info,
    );
    // SAFETY: `p` is live (crate invariant).
    let h = unsafe { cell(p).header() };
    h.set_flags(h.flags() | kind::FLAG_IS_PROTOTYPE);
    install(rt, p, u);
    // An object that has index keys already becomes a prototype (`proto_index_keys`).
    if may_have_index_keys(p) {
        note_proto_index_key(rt, p);
    }
    info
}

/// The initial shape of a new object of `object_kind` with `capacity` in-object slots and
/// the prototype `proto` (an object or null). GC point.
pub(crate) fn initial_shape(rt: &Runtime, proto: u64, object_kind: KindId, capacity: u8) -> u64 {
    let info = proto_info_for(rt, proto);
    root_shape_for(rt, info, object_kind, capacity)
}

/// The shape of a new function before its own properties change: the root for
/// (%Function.prototype%, FUNCTION, 2 slots) plus `length` and `name`, both {w: false,
/// e: false, c: true}. Cached in the ShapeState roots. GC point.
pub(crate) fn function_shape(rt: &Runtime) -> u64 {
    let fp = rt.root(Root::FunctionPrototype);
    let cached = rt.shapes.roots[root::FUNCTION_SHAPE].bits();
    if cached != UNDEFINED && proto(cached) == fp {
        return cached;
    }
    let mut s = initial_shape(rt, fp, kind::FUNCTION, 2);
    for a in [crate::runtime::Atom::Length, crate::runtime::Atom::Name] {
        match add(rt, s, rt.atom(a), attr::CONFIGURABLE) {
            Added::Shape(c) => s = c,
            Added::Dictionary => unreachable!("a root shape takes two properties"),
        }
    }
    // E3: a store into a root takes no barrier.
    rt.shapes.roots[root::FUNCTION_SHAPE].init(val(s));
    s
}

/// The dictionary shape for an object whose shape is `shape`, with the same prototype and
/// kind: the shared one of its prototype's PROTO_INFO, or a new unique one in prototype mode
/// (it keeps the PROTO_INFO). GC point.
pub(crate) fn dictionary_shape_of(rt: &Runtime, shape: u64, extensible: bool) -> u64 {
    let k = KindId::new(object_kind(shape)).expect("an object kind");
    let ext = if extensible { flag::EXTENSIBLE } else { 0 };
    if flags(shape) & flag::UNIQUE != 0 {
        let counts = pack_counts(0, 0, k.get());
        return new_unique(
            rt,
            proto(shape),
            UNDEFINED,
            counts,
            flag::DICTIONARY | ext,
            unique_info(shape),
        );
    }
    let info = proto_info_for(rt, proto(shape));
    dictionary_shape_for(rt, info, k, extensible)
}

/// The shape for an object with `shape` (a unique one, or one without properties) after its
/// `[[Prototype]]` becomes `p`. `p` goes into prototype mode first on every path, so a later
/// change to it bumps the epoch (OM-8). A unique shape is copied with the new prototype; any
/// other shape becomes the matching root or dictionary shape of `p` (`capacity`: the
/// object's in-object slots). An object with properties in a shared fast shape goes to
/// dictionary mode first (OM-10), so it never reaches here. GC point.
pub(crate) fn with_proto(rt: &Runtime, shape: u64, p: u64, capacity: u8) -> u64 {
    let k = KindId::new(object_kind(shape)).expect("an object kind");
    let fl = flags(shape);
    // GC point: `shape` and `p` live in Rust locals (CSS).
    let info = proto_info_for(rt, p);
    if fl & flag::UNIQUE != 0 {
        let counts = rd(cell(shape), shf::COUNTS);
        let d = descriptors(shape);
        return new_unique(rt, p, d, counts, fl, unique_info(shape));
    }
    if fl & flag::DICTIONARY != 0 {
        dictionary_shape_for(rt, info, k, fl & flag::EXTENSIBLE != 0)
    } else {
        debug_assert!(count(shape) == 0, "a shared fast shape with properties");
        root_shape_for(rt, info, k, capacity)
    }
}

// ---- checks ---------------------------------------------------------------------------------

/// Checks the invariants of `shape` and its parent chain: counts, the prefix rule, distinct
/// keys, field indices and the hash index.
#[cfg(test)]
pub(crate) fn check(rt: &Runtime, shape: u64) -> Result<(), String> {
    if !is_shape(shape) {
        return Err(format!("{shape:#x} is not a shape"));
    }
    let n = count(shape);
    let unique = flags(shape) & flag::UNIQUE != 0;
    if unique
        && (flags(shape) & flag::PROTOTYPE == 0
            || parent(shape) != UNDEFINED
            || kind::cell_kind(unique_info(shape)) != kind::PROTO_INFO.get())
    {
        return Err("a unique shape without PROTOTYPE or PROTO_INFO, or with a parent".into());
    }
    if is_dictionary(shape) || n == 0 {
        return if n == 0 && parent(shape) == UNDEFINED && descriptors(shape) == UNDEFINED {
            Ok(())
        } else {
            Err("a root or dictionary shape with a parent or descriptors".into())
        };
    }
    let d = cell(descriptors(shape));
    let len = d_len(d);
    for i in 0..len {
        for j in 0..i {
            if key::key_eq(d_key(d, i), d_key(d, j)) {
                return Err(format!("descriptors {j} and {i} have one key"));
            }
        }
    }
    if unique {
        // No parent chain: check this shape's own prefix.
        if n > len {
            return Err(format!("count {n}, length {len}"));
        }
        for i in 0..n {
            if usize::from(details_at(shape, i).field()) != i
                || lookup(rt, shape, key_at(shape, i)).map(|f| f.index) != Some(i)
            {
                return Err(format!("the unique shape does not find descriptor {i}"));
            }
        }
        return Ok(());
    }
    let mut s = shape;
    while count(s) > 0 {
        let c = count(s);
        let ds = cell(descriptors(s));
        let (len, cap) = (d_len(ds), d_cap(ds));
        if !(c <= len && len <= cap) {
            return Err(format!("count {c}, length {len}, capacity {cap}"));
        }
        for i in 0..c {
            if key_at(s, i) != key_at(shape, i) || details_at(s, i) != details_at(shape, i) {
                return Err(format!("the shape of count {c} differs at descriptor {i}"));
            }
            if usize::from(details_at(s, i).field()) != i {
                return Err(format!(
                    "descriptor {i} has field {}",
                    details_at(s, i).field()
                ));
            }
        }
        if len > LINEAR_MAX && rd(ds, dsc::INDEX) == UNDEFINED {
            return Err(format!("{len} descriptors and no index"));
        }
        if c > LINEAR_MAX
            && let Some(i) =
                (0..c).find(|&i| lookup(rt, s, key_at(s, i)).map(|f| f.index) != Some(i))
        {
            return Err(format!(
                "the shape of count {c} does not find descriptor {i}"
            ));
        }
        let p = parent(s);
        if p == UNDEFINED || count(p) != c - 1 || proto(p) != proto(s) || flags(p) != flags(s) {
            return Err(format!("the parent of the shape of count {c} is wrong"));
        }
        s = p;
    }
    if parent(s) != UNDEFINED || descriptors(s) != UNDEFINED {
        return Err("the root has a parent or descriptors".into());
    }
    Ok(())
}
