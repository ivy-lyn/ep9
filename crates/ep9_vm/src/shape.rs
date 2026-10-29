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
//! | TRANSITIONS | 74 | +8 capacity, +16 child shapes (weak) or undefined; the same weak array is the users list of a VALIDITY_CELL (`validity.rs`) | 16 + 8 cap |
//! | PROTO_INFO | 75 | +8 prototype (or null), +16 shape count, +24 SLOTS of shapes, +32 the prototype's current VALIDITY_CELL (79, `validity.rs`; undefined until an IC case needs it; the null prototype's is made in [`init`]; traced strongly) | 40 |
//!
//! - **counts** (int32): property count in bits 0-11, in-object capacity in bits 12-19, the
//!   object kind id in bits 20-27, and (OBJECT_LAYOUT 5) the elements kind in bits 28-31
//!   ([`ELEMENTS_KIND_SHIFT`]; the values are in `elements.rs`; M5-P1 ELEM1). Every shape
//!   made from another one keeps its elements kind (a child, a unique copy, the dictionary
//!   shape, a new prototype's root); only [`with_elements_kind`] changes it, and the roots and
//!   dictionary shapes of a PROTO_INFO are kept per elements kind.
//! - **flags** (int32): [`flag`]. A fast shape is always extensible; a non-extensible object
//!   is in dictionary mode (OM-10). OBJECT_LAYOUT 5: bits 4-7 ([`flag::SLACK_SHIFT`]) are
//!   reserved and unused since M5-P2: SLACK reads the count from the bytecode (`slack.rs`).
//! - **details** (int32, [`Details`]): attributes in bits 0-3 (writable, enumerable,
//!   configurable, [`ACCESSOR`]), representation in bits 4-6 and const in bit 7 (OBJECT_LAYOUT
//!   5 names the values, [`rep`] and [`DETAILS_CONST`]; M5-P2 CONSTREP writes them, the rules
//!   are in `field_rep.rs`), field index in bits 8-23.
//! - **Field families** (M5-P2 CONSTREP): the shape that added descriptor `i` is the field's
//!   owner ([`field_owner`]). Its descendants in the transition tree share that entry, or hold
//!   a copy of it (a branch). A change of the representation or const bits of a field writes
//!   every copy in place ([`set_field_bits_in_family`]); the bits only ever widen.
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
//!   object ([`note_change`]: a named property or an element that appears or goes away, new
//!   attributes, `[[SetPrototypeOf]]`, PreventExtensions, dictionary mode) invalidates its
//!   validity cell and the cells of every prototype below it (`validity.rs`, H16 K5), and
//!   bumps [`ShapeState::proto_epoch`] (kept for the debug cross-check of the IC hit paths,
//!   plan v9 §3.3 and V9-A25). A value store does neither.
//! - The first array-index own property on a prototype-mode object (or an object with one, or
//!   a routed ARGUMENTS or MODULE_NAMESPACE object, becoming a prototype) sets the sticky
//!   [`proto_index_keys`] flag: until then `[[Set]]` adds array elements without walking the
//!   prototype chain.
//!
//! Hooks the rest of the VM already calls:
//!
//! | Hook | Called from | Job |
//! |---|---|---|
//! | [`heap_config`] | `Isolate::new`, before the heap exists | the shape space (H08-Q1) and the cage alignment |
//! | [`register_kinds`] | `object::register_kinds` | SHAPE (shape space), SLOTS, DESCRIPTORS, DESC_INDEX, TRANSITIONS, PROTO_INFO |
//! | [`init`] | `Runtime::new` | register the ShapeState root range and make the null PROTO_INFO and its validity cell |
//!
//! Since M4-P1 no IC hit path compares [`ShapeState::proto_epoch`] (the validity cells do);
//! only the debug cross-check of `ic.rs` reads it, from the runtime (M5-P0 removed
//! `Cx::proto_epoch`).

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
    /// The prototype's current VALIDITY_CELL (`validity.rs`) or undefined.
    pub const VALIDITY: usize = 32;
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
    /// OBJECT_LAYOUT 5: bits 4-7 of a shape's flags, declared for a slack counter (H17 slack
    /// row). Reserved and unused since M5-P2: SLACK reads the count from the bytecode
    /// (`slack.rs`), and `shape::add` copies flags to children, so a root counter would reach
    /// them. Always 0.
    #[allow(dead_code)] // reserved
    pub const SLACK_SHIFT: u32 = 4;
    /// OBJECT_LAYOUT 5: the reserved bits 4-7.
    #[allow(dead_code)] // reserved
    pub const SLACK_MASK: i32 = 0xF << SLACK_SHIFT;
}

/// OBJECT_LAYOUT 5: the elements kind of a shape's objects in `counts` bits 28-31 (H18-Q3;
/// the values are `elements.rs` constants, written by M5-P1 ELEM1).
pub(crate) const ELEMENTS_KIND_SHIFT: u32 = 28;
/// OBJECT_LAYOUT 5: the elements-kind bits of `counts` (bit 31 is the int32 sign bit).
pub(crate) const ELEMENTS_KIND_MASK: i32 = (0xF_u32 << ELEMENTS_KIND_SHIFT) as i32;

/// OBJECT_LAYOUT 5: the representation values of details bits 4-6 (H17 rep/const row, C-02;
/// M5-P2 CONSTREP writes them, `field_rep.rs`). 5-7 are reserved. One-way: NONE -> INT32 ->
/// DOUBLE -> ANY, NONE -> HEAP -> ANY.
pub(crate) mod rep {
    /// The bits of a details word that hold the representation.
    pub const SHIFT: u32 = 4;
    pub const MASK: i32 = 0x7 << SHIFT;
    /// No value stored yet. A field never keeps it after its first store.
    pub const NONE: i32 = 0;
    /// Every value is an int32.
    pub const INT32: i32 = 1;
    /// Every value is a number (an int32 or a double; a Value holds a double unboxed, H6).
    pub const DOUBLE: i32 = 2;
    /// No value is a number: a cell, or an immediate such as undefined, null or a boolean.
    pub const HEAP: i32 = 3;
    /// Any value.
    pub const ANY: i32 = 4;
}

/// OBJECT_LAYOUT 5: details bit 7, the field is const: no object of the field's family stored
/// a second value into it since it was added (H17 rep/const row; M5-P2 CONSTREP).
pub(crate) const DETAILS_CONST: i32 = 1 << 7;

/// The representation and const bits of a details word (bits 4-7).
pub(crate) const FIELD_BITS: i32 = rep::MASK | DETAILS_CONST;

/// The field bits of a field that no writer checks: any value, not const. A shape made by
/// [`add`] (no value known) gets them.
pub(crate) const ANY_FIELD: i32 = rep::ANY << rep::SHIFT;

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

/// Details bits 4-7: representation (3 bits, [`rep`]) and const (1 bit, [`DETAILS_CONST`]),
/// written by M5-P2 CONSTREP (`field_rep.rs`).
const _: () = assert!(FIELD_BITS == 0xF0);
const FIELD_SHIFT: u32 = 8;

/// ShapeState root range slots.
mod root {
    /// The PROTO_INFO of the null prototype.
    pub const NULL_PROTO_INFO: usize = 0;
    /// The shape of a new function: %Function.prototype%, `length` and `name` (a cache;
    /// undefined until the first function).
    pub const FUNCTION_SHAPE: usize = 1;
    /// The shape of a new generator function: [`FUNCTION_SHAPE`] with
    /// %GeneratorFunction.prototype% as the prototype (a cache; undefined until the first
    /// generator function).
    pub const GENERATOR_FUNCTION_SHAPE: usize = 2;
    /// The shape of a new async function: [`FUNCTION_SHAPE`] with %AsyncFunction.prototype%
    /// as the prototype (a cache; undefined until the first async function).
    pub const ASYNC_FUNCTION_SHAPE: usize = 3;
    /// The shape of a new async generator function: [`FUNCTION_SHAPE`] with
    /// %AsyncGeneratorFunction.prototype% as the prototype (a cache; undefined until the
    /// first async generator function).
    pub const ASYNC_GENERATOR_FUNCTION_SHAPE: usize = 4;
    pub const COUNT: usize = 5;
    /// The first function shape cache slot: the slots from here to [`COUNT`] hold a shape
    /// over a prototype of the current realm (`clear_realm_caches`).
    pub const FIRST_REALM_CACHE: usize = FUNCTION_SHAPE;
}

/// The last value of [`ShapeState::proto_epoch`]: it stays there (it never wraps;
/// W4-P3FIX-EPOCH-SAT). Since M4-P1 no IC case compares the epoch (the validity cells of
/// `validity.rs` guard them, H16 K5), so a saturated epoch changes no IC result; the debug
/// cross-check of `ic.rs` then checks every hit.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) const EPOCH_SATURATED: u32 = u32::MAX;

/// Per-isolate shape state (a `Runtime` field).
#[derive(Debug)]
pub(crate) struct ShapeState {
    /// Bumped on every structural change of a prototype (OM-8), up to [`EPOCH_SATURATED`].
    /// No IC hit path compares it since M4-P1 (validity cells); the debug cross-check of the
    /// hit paths reads it (plan v9 V9-A25, R1), and tests do.
    pub proto_epoch: Cell<u32>,
    /// Sticky, never cleared: set once a prototype-mode object has an array-index own
    /// property, or a routed kind becomes a prototype ([`proto_index_keys`]). While it is
    /// false no prototype chain has an index key,
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
    crate::validity::register_kind(heap);
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
    // GC point: `info` is in the root range now.
    let c = crate::validity::new_root_cell(rt);
    set_validity_cell(rt, info, c);
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

    /// The representation ([`rep`]).
    pub(crate) fn rep(self) -> i32 {
        (self.0 & rep::MASK) >> rep::SHIFT
    }

    /// True if the field is const ([`DETAILS_CONST`]).
    pub(crate) fn is_const(self) -> bool {
        self.0 & DETAILS_CONST != 0
    }

    /// The representation and const bits ([`FIELD_BITS`]), in place.
    pub(crate) fn field_bits(self) -> i32 {
        self.0 & FIELD_BITS
    }

    /// These details with the field bits `b` ([`FIELD_BITS`], in place).
    pub(crate) fn with_field_bits(self, b: i32) -> Details {
        debug_assert!(b & !FIELD_BITS == 0 && (b & rep::MASK) >> rep::SHIFT <= rep::ANY);
        Details((self.0 & !FIELD_BITS) | b)
    }

    /// The same property and field: attributes and field index (not the field bits).
    fn same_slot(self, other: Details) -> bool {
        (self.0 ^ other.0) & !FIELD_BITS == 0
    }

    fn bits(self) -> u64 {
        int32_bits(self.0)
    }

    fn from_bits(b: u64) -> Details {
        debug_assert!(val(b).is_int32());
        let d = int32_of(b);
        debug_assert!(
            (d & rep::MASK) >> rep::SHIFT <= rep::ANY,
            "a reserved representation in {d:#x}"
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

/// [`pack_counts`] with the elements-kind bits `ek_bits` (counts bits 28-31, as
/// [`ek_bits_of`] gives them).
fn pack_counts_ek(count: usize, capacity: u8, object_kind: u8, ek_bits: i32) -> u64 {
    debug_assert!(ek_bits & !ELEMENTS_KIND_MASK == 0);
    int32_bits(int32_of(pack_counts(count, capacity, object_kind)) | ek_bits)
}

/// The elements-kind bits (counts bits 28-31, in place) of `shape`.
#[inline(always)]
fn ek_bits_of(shape: u64) -> i32 {
    counts(cell(shape)) & ELEMENTS_KIND_MASK
}

/// The elements-kind bits of kind `ek` (0-15), in place.
#[inline(always)]
const fn ek_bits(ek: u8) -> i32 {
    ((ek as u32) << ELEMENTS_KIND_SHIFT) as i32
}

/// The elements kind (`elements::kind`, 0-15) of the objects with `shape` (H18-Q3). Not a GC
/// point.
#[inline(always)]
pub(crate) fn elements_kind(shape: u64) -> u8 {
    debug_assert!(is_shape(shape));
    // No checked `cell`: its panic path would give the T0 keyed handlers a frame (M5-P1).
    let p = core::ptr::with_exposed_provenance_mut::<ep9_value::Word0>(shape as usize);
    // SAFETY: a shape value is the address of a live shape cell (never 0), whose provenance
    // `Value::from_cell_ptr` exposed (`shape_of` rebuilds it from the object's address).
    let s = unsafe { CellPtr::from_raw(NonNull::new_unchecked(p)) };
    ((counts(s) as u32) >> ELEMENTS_KIND_SHIFT) as u8
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

// ---- field families (M5-P2 CONSTREP) ----------------------------------------------------------

/// The field bits ([`FIELD_BITS`], in place) of descriptor `i` of the fast shape `shape`. Not
/// a GC point. Always inlined, with no checked `cell` (its panic path would give a T0 store
/// handler a frame, as in [`elements_kind`]).
#[inline(always)]
pub(crate) fn field_bits_at(shape: u64, i: usize) -> i32 {
    debug_assert!(is_shape(shape) && i < count(shape));
    let s = raw_cell(shape);
    let d = raw_cell(rd(s, shf::DESCRIPTORS));
    int32_of(rd(d, key_off(i) + 8)) & FIELD_BITS
}

/// The representation ([`rep`]) of the last descriptor of the fast shape `shape` (the field
/// an add-property transition to `shape` writes). Not a GC point; always inlined, no checked
/// `cell` (the `ADD` hit of `ic::store`).
#[inline(always)]
pub(crate) fn last_field_rep(shape: u64) -> i32 {
    debug_assert!(is_shape(shape) && count(shape) > 0);
    let s = raw_cell(shape);
    let n = (int32_of(rd(s, shf::COUNTS)) & COUNT_MASK) as usize;
    let d = raw_cell(rd(s, shf::DESCRIPTORS));
    (int32_of(rd(d, key_off(n - 1) + 8)) & rep::MASK) >> rep::SHIFT
}

/// The cell of a shape or DESCRIPTORS value, built as [`elements_kind`] does (no checked
/// `cell`).
#[inline(always)]
fn raw_cell(b: u64) -> CellPtr {
    debug_assert!(kind::is_cell_bits(b) && b != 0);
    let p = core::ptr::with_exposed_provenance_mut::<ep9_value::Word0>(b as usize);
    // SAFETY: `b` is the address of a live cell (a shape, or the DESCRIPTORS cell a live shape
    // holds), whose provenance `Value::from_cell_ptr` exposed.
    unsafe { CellPtr::from_raw(NonNull::new_unchecked(p)) }
}

/// The owner of descriptor `i` of the fast shape `shape` (`i < count(shape)`): the shape that
/// added it. A tree shape walks its parents back to it; a unique (prototype-mode) shape has
/// no parent and is its own owner. Not a GC point.
pub(crate) fn field_owner(shape: u64, i: usize) -> u64 {
    debug_assert!(i < count(shape) && !is_dictionary(shape));
    let mut o = shape;
    loop {
        let p = parent(o);
        if p == UNDEFINED || count(p) <= i {
            return o;
        }
        o = p;
    }
}

/// Writes descriptor `i` of the field family of `owner` ([`field_owner`]): `f` maps the
/// details of each DESCRIPTORS cell that `owner` or a live descendant uses to the new ones;
/// `f` only widens the field bits, and a join, so a cell that several shapes share (the
/// prefix rule) is read again but written once. A cell that another family shares (a unique
/// shape) sees the wider bits too, which is safe. Returns true if any cell changed. Not a GC
/// point: the walk's stack of shapes is Rust memory only until it returns.
pub(crate) fn set_field_bits_in_family(
    owner: u64,
    i: usize,
    f: impl Fn(Details) -> Details,
) -> bool {
    let mut changed = false;
    let mut todo = vec![owner];
    while let Some(sh) = todo.pop() {
        let c = cell(sh);
        let d = rd(c, shf::DESCRIPTORS);
        if d != UNDEFINED && count_of(c) > i {
            let dc = cell(d);
            let old = d_details(dc, i);
            let new = f(old);
            debug_assert!(old.same_slot(new));
            if new != old {
                // E1: an int32 store needs no barrier.
                object::init(dc, key_off(i) + 8, new.bits());
                changed = true;
            }
        }
        if flags(sh) & (flag::UNIQUE | flag::DICTIONARY) != 0 {
            continue;
        }
        let t = rd(c, shf::TRANSITION);
        if t != UNDEFINED {
            todo.push(t);
        }
        let tab = rd(c, shf::TABLE);
        if tab != UNDEFINED && kind::cell_kind(tab) == kind::TRANSITIONS.get() {
            let tc = cell(tab);
            for j in 0..rd_i32(tc, trf::CAPACITY) as usize {
                let ch = rd(tc, trf::ENTRIES + 8 * j);
                if ch != UNDEFINED {
                    todo.push(ch);
                }
            }
        }
    }
    changed
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
/// point. `key` must not be a property of `shape` already. A new field gets [`ANY_FIELD`]
/// (no value is known); a writer that knows the first value uses [`add_with`].
pub(crate) fn add(rt: &Runtime, shape: u64, key: u64, attrs: i32) -> Added {
    add_with(rt, shape, key, attrs, ANY_FIELD)
}

/// [`add`], where a new descriptor gets the field bits `bits` ([`FIELD_BITS`]: the
/// representation of the first value and the const bit, `field_rep::initial`). An existing
/// transition keeps its bits: the caller joins its value into them
/// (`field_rep::on_init_store`). GC point.
pub(crate) fn add_with(rt: &Runtime, shape: u64, key: u64, attrs: i32, bits: i32) -> Added {
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
        return add_unique(rt, shape, key, attrs, bits, how);
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
    let details = Details::new(attrs, n as u16).with_field_bits(bits);
    let d = descriptors_for_child(rt, shape, key, details, how);
    let child = new_shape(
        rt,
        proto(shape),
        shape,
        d,
        pack_counts_ek(
            n + 1,
            capacity(shape),
            object_kind(shape),
            ek_bits_of(shape),
        ),
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
fn add_unique(rt: &Runtime, shape: u64, key: u64, attrs: i32, bits: i32, how: Probe) -> Added {
    let n = count(shape);
    if n >= MAX_FAST_PROPERTIES || key::is_index_key(key) {
        return Added::Dictionary;
    }
    let details = Details::new(attrs, n as u16).with_field_bits(bits);
    let d = descriptors_for_child(rt, shape, key, details, how);
    let counts = pack_counts_ek(
        n + 1,
        capacity(shape),
        object_kind(shape),
        ek_bits_of(shape),
    );
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
    let counts = pack_counts_ek(
        n - 1,
        capacity(shape),
        object_kind(shape),
        ek_bits_of(shape),
    );
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
    let shared = d_details(od, n);
    if shared.same_slot(details) && same_key(d_key(od, n), key, how) {
        // A child that died added this very entry: share the array again. The entry keeps
        // the field bits of its old users, widened by the new child's (one-way; a live
        // unique shape that shares the array may still use the entry).
        let b = crate::field_rep::join_bits(shared.field_bits(), details.field_bits());
        if b != shared.field_bits() {
            // E1: an int32 store needs no barrier.
            object::init(od, key_off(n) + 8, shared.with_field_bits(b).bits());
            crate::field_rep::note_widened(rt, n);
        }
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

/// The PROTO_INFO of `p`, a prototype-mode object or null (every shape's prototype is one,
/// [`new_shape`]). Unlike [`proto_info_for`] it makes nothing. Not a GC point.
#[inline]
pub(crate) fn proto_info_of(rt: &Runtime, p: u64) -> u64 {
    if p == NULL {
        return null_proto_info(rt);
    }
    debug_assert!(is_prototype_object(p), "{p:#x} is not in prototype mode");
    unique_info(shape_of(p))
}

/// The VALIDITY_CELL in the PROTO_INFO `info` (+32), or undefined. Not a GC point.
#[inline]
pub(crate) fn validity_cell_of(info: u64) -> u64 {
    rd(cell(info), pif::VALIDITY)
}

/// Makes `c` the current VALIDITY_CELL of the PROTO_INFO `info` (`validity::cell_for`). Not a
/// GC point.
pub(crate) fn set_validity_cell(rt: &Runtime, info: u64, c: u64) {
    debug_assert!(kind::cell_kind(c) == kind::VALIDITY_CELL.get());
    wr(&rt.heap, cell(info), pif::VALIDITY, c);
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
    root_shape_for_kind(rt, proto_info, object_kind, capacity, 0)
}

/// [`root_shape_for`] with the elements kind `ek` (`elements::kind`; 0, PACKED_INT32, for a
/// new object). Each (object kind, capacity, elements kind) has its own root. GC point.
fn root_shape_for_kind(
    rt: &Runtime,
    proto_info: u64,
    object_kind: KindId,
    capacity: u8,
    ek: u8,
) -> u64 {
    debug_assert!(is_object_kind(object_kind));
    let want = pack_counts_ek(0, capacity, object_kind.get(), ek_bits(ek));
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
/// `proto_info`, elements kind 0. GC point.
#[cfg_attr(not(test), allow(dead_code))] // tests/shapes.rs; the VM asks the elements kind's
pub(crate) fn dictionary_shape_for(
    rt: &Runtime,
    proto_info: u64,
    object_kind: KindId,
    extensible: bool,
) -> u64 {
    dictionary_shape_for_kind(rt, proto_info, object_kind, extensible, 0)
}

/// [`dictionary_shape_for`] with the elements kind `ek` (one shared dictionary shape per
/// (object kind, extensible, elements kind)). GC point.
fn dictionary_shape_for_kind(
    rt: &Runtime,
    proto_info: u64,
    object_kind: KindId,
    extensible: bool,
    ek: u8,
) -> u64 {
    debug_assert!(is_object_kind(object_kind));
    let fl = flag::DICTIONARY | if extensible { flag::EXTENSIBLE } else { 0 };
    let want = pack_counts_ek(0, 0, object_kind.get(), ek_bits(ek));
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
/// dictionary mode, PreventExtensions). If `o` is a prototype (OM-8): invalidates its
/// validity cell and the cells of every prototype below it (`validity::invalidate`, H16 K5),
/// and bumps the epoch, which saturates at [`EPOCH_SATURATED`] (W4-P3FIX-EPOCH-SAT; only the
/// debug cross-check of the IC hit paths reads it). A prototype always has a unique shape
/// with its PROTO_INFO (prototype mode keeps it on every change). Not a GC point.
#[inline]
pub(crate) fn note_change(rt: &Runtime, o: u64) {
    if is_prototype_object(o) {
        let e = &rt.shapes.proto_epoch;
        e.set(e.get().saturating_add(1));
        let s = shape_of(o);
        debug_assert!(
            flags(s) & flag::UNIQUE != 0,
            "a prototype without a unique shape"
        );
        crate::validity::invalidate(rt, validity_cell_of(unique_info(s)));
        crate::fuse::on_proto_change(rt, o);
    }
}

/// True if some prototype-mode object has, or once had, an array-index own property: an
/// element, a stored index key or a String wrapper index; or is a routed kind (ARGUMENTS,
/// MODULE_NAMESPACE), whose hooks may answer for an index. False means no prototype chain
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
/// `length` (holes too), another object with an element (M5-P1 ELEM1), a String wrapper of a non-empty string, a stored index key, or a
/// routed kind (ARGUMENTS, MODULE_NAMESPACE: its hooks answer for index keys that are not in
/// its storage). Not a GC point. Out of line: [`proto_info_for`] runs it once per new
/// prototype.
#[cold]
#[inline(never)]
fn may_have_index_keys(o: u64) -> bool {
    let wrapped = crate::wrapper::wrapper_value(o);
    object::may_have_elements(o)
        || wrapped.is_some_and(|s| kind::is_string(s) && object::str_len(s) > 0)
        || (0..object::own_count(o))
            .any(|i| key::array_index_of(object::own_entry(o, i).0).is_some())
        || crate::exotic::is_routed(kind::cell_kind(o))
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
    cached_function_shape(rt, rt.root(Root::FunctionPrototype), root::FUNCTION_SHAPE)
}

/// The shape of a new generator function (§15.5.3, §15.5.4: OrdinaryFunctionCreate with
/// %GeneratorFunction.prototype%): [`function_shape`] with that prototype. Cached in the
/// ShapeState roots. Before the intrinsic is installed it is the [`function_shape`]. GC
/// point.
pub(crate) fn generator_function_shape(rt: &Runtime) -> u64 {
    use crate::builtins::{IntrinsicId, intrinsic};
    let gfp = intrinsic(rt, IntrinsicId::GeneratorFunctionPrototype);
    if !kind::is_object(gfp) {
        return function_shape(rt);
    }
    cached_function_shape(rt, gfp, root::GENERATOR_FUNCTION_SHAPE)
}

/// Empties the function shape caches ([`function_shape`], [`generator_function_shape`],
/// [`async_function_shape`], [`async_generator_function_shape`]): the realm switch
/// (`realm::switch`) calls it, so a cache holds only a shape of the current realm.
/// A cache that kept the shape of the last realm that made a function would keep that realm
/// live (shape -> PROTO_INFO -> its `%Function.prototype%` -> its REALM record) after nothing
/// else holds it. A hit checks the prototype, so this is not needed for correct prototypes.
/// Not a GC point.
pub(crate) fn clear_realm_caches(rt: &Runtime) {
    for slot in &rt.shapes.roots[root::FIRST_REALM_CACHE..root::COUNT] {
        // E3: a store into a root takes no barrier.
        slot.init(Value::UNDEFINED);
    }
}

/// The shape of a new async function (§15.8.3: OrdinaryFunctionCreate with
/// %AsyncFunction.prototype%): [`function_shape`] with that prototype. Cached in the
/// ShapeState roots. Before the intrinsic is installed it is the [`function_shape`]. GC
/// point.
pub(crate) fn async_function_shape(rt: &Runtime) -> u64 {
    use crate::builtins::{IntrinsicId, intrinsic};
    let afp = intrinsic(rt, IntrinsicId::AsyncFunctionPrototype);
    if !kind::is_object(afp) {
        return function_shape(rt);
    }
    cached_function_shape(rt, afp, root::ASYNC_FUNCTION_SHAPE)
}

/// The shape of a new async generator function (§15.6.3: OrdinaryFunctionCreate with
/// %AsyncGeneratorFunction.prototype%): [`function_shape`] with that prototype. Cached in the
/// ShapeState roots. Before the intrinsic is installed it is the [`function_shape`]. GC
/// point.
pub(crate) fn async_generator_function_shape(rt: &Runtime) -> u64 {
    use crate::builtins::{IntrinsicId, intrinsic};
    let agfp = intrinsic(rt, IntrinsicId::AsyncGeneratorFunctionPrototype);
    if !kind::is_object(agfp) {
        return function_shape(rt);
    }
    cached_function_shape(rt, agfp, root::ASYNC_GENERATOR_FUNCTION_SHAPE)
}

/// The function shape over the prototype `fp` (FUNCTION, 2 slots, `length` and `name`),
/// cached in root slot `slot`. GC point.
fn cached_function_shape(rt: &Runtime, fp: u64, slot: usize) -> u64 {
    let cached = rt.shapes.roots[slot].bits();
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
    rt.shapes.roots[slot].init(val(s));
    s
}

/// The dictionary shape for an object whose shape is `shape`, with the same prototype and
/// kind: the shared one of its prototype's PROTO_INFO, or a new unique one in prototype mode
/// (it keeps the PROTO_INFO). GC point.
pub(crate) fn dictionary_shape_of(rt: &Runtime, shape: u64, extensible: bool) -> u64 {
    let k = KindId::new(object_kind(shape)).expect("an object kind");
    let ext = if extensible { flag::EXTENSIBLE } else { 0 };
    let ek = elements_kind(shape);
    if flags(shape) & flag::UNIQUE != 0 {
        let counts = pack_counts_ek(0, 0, k.get(), ek_bits(ek));
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
    dictionary_shape_for_kind(rt, info, k, extensible, ek)
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
    let ek = elements_kind(shape);
    // GC point: `shape` and `p` live in Rust locals (CSS).
    let info = proto_info_for(rt, p);
    if fl & flag::UNIQUE != 0 {
        let counts = rd(cell(shape), shf::COUNTS);
        let d = descriptors(shape);
        return new_unique(rt, p, d, counts, fl, unique_info(shape));
    }
    if fl & flag::DICTIONARY != 0 {
        dictionary_shape_for_kind(rt, info, k, fl & flag::EXTENSIBLE != 0, ek)
    } else {
        debug_assert!(count(shape) == 0, "a shared fast shape with properties");
        root_shape_for_kind(rt, info, k, capacity, ek)
    }
}

/// The shape for an object with `shape` after its elements kind becomes `ek` (H18, M5-P1
/// ELEM1; the store side of a one-way transition, `elements::transition`): the same
/// prototype, object kind, properties and flags. A unique shape is copied with the new
/// kind; a dictionary shape becomes the shared one of that kind; a tree shape is found again
/// from the root of that kind by its keys in order (the transitions of that tree). `None` if
/// that walk meets a transition limit (`add` answers `Added::Dictionary`): the caller moves
/// the object to dictionary mode and asks again. GC point.
pub(crate) fn with_elements_kind(rt: &Runtime, shape: u64, ek: u8) -> Option<u64> {
    debug_assert!(ek <= crate::elements::kind::MAX);
    if elements_kind(shape) == ek {
        return Some(shape);
    }
    let fl = flags(shape);
    if fl & flag::UNIQUE != 0 {
        let c = (counts(cell(shape)) & !ELEMENTS_KIND_MASK) | ek_bits(ek);
        let d = descriptors(shape);
        return Some(new_unique(
            rt,
            proto(shape),
            d,
            int32_bits(c),
            fl,
            unique_info(shape),
        ));
    }
    let k = KindId::new(object_kind(shape)).expect("an object kind");
    // GC points below: `shape` lives in a Rust local (CSS); its prototype is in prototype
    // mode already, so this makes nothing.
    let info = proto_info_for(rt, proto(shape));
    if fl & flag::DICTIONARY != 0 {
        return Some(dictionary_shape_for_kind(
            rt,
            info,
            k,
            fl & flag::EXTENSIBLE != 0,
            ek,
        ));
    }
    let mut s = root_shape_for_kind(rt, info, k, capacity(shape), ek);
    for i in 0..count(shape) {
        let (key, d) = (key_at(shape, i), details_at(shape, i));
        // A new field gets the old field's bits (M5-P2 CONSTREP).
        match add_with(rt, s, key, d.attrs(), d.field_bits()) {
            Added::Shape(c) => s = c,
            Added::Dictionary => return None,
        }
    }
    debug_assert_eq!(flags(s), fl, "a tree shape with other flags");
    // The object keeps its values: the fields of a tree shape found again widen to hold them
    // (M5-P2 CONSTREP; not a GC point).
    crate::field_rep::join_fields(rt, shape, s);
    Some(s)
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
