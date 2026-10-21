//! JS objects on shapes: the H17 v0 layout (M0 week 4), plus contexts and the ELEMENTS
//! store. Shapes are in `shape.rs`, own-property storage (fast slots and the dictionary table)
//! in `props.rs`, internal methods in `property.rs`, exotic own properties in `exotic.rs`.
//!
//! ```text
//!  JS object (kinds 16-63)                                   word0 flags
//!  +0  word0: shape_id | gc_state | kind | flags              bits 0-7  in-object slot count
//!  +8  SLOTS (out-of-line fields) | PROPS (dictionary) | undefined   bit 8  IS_PROTOTYPE
//!  +16 ELEMENTS (arrays) | undefined                          bit 9  reserved (hash present)
//!  +24 the kind's fixed fields, then the in-object slots      bits 10-15 reserved
//!  size = 24 + 8 * (fixed fields + in-object slots)   (read from the cell: kind + flags)
//! ```
//!
//! `[[Prototype]]` and `[[Extensible]]` live in the shape. A function's `[[Construct]]` bit
//! comes from its FunctionInfo (OM-11).
//!
//! | Kind | Id | Fixed fields (from +24) | In-object slots |
//! |---|---|---|---|
//! | OBJECT | 16 | none | 4 for `{}`; n for a literal with n named keys; 8 for other prototypes (`new F`) |
//! | FUNCTION | 17 | +24 FunctionInfo (native word), +32 context, +40 `[[HomeObject]]` or undefined | 2 (`length`, `name`) |
//! | ARRAY | 18 | +24 length (int32); elements at +16 | 0 |
//! | ERROR | 19 | +24 unit index (int32 or undefined), +32 source offset (int32) | 2 |
//! | ITERATOR | 20 | +24 iterated value: an object (undefined when done, null while an array iterator's `next` runs user code) or a string ("" when done), +32 next index (int32), +40 kind (int32, [`iter_kind`]: values, keys, entries) | 0 |
//! | PRIMITIVE_WRAPPER | 21 | +24 the primitive | 0 |
//! | BOUND_FUNCTION | 22 | +24 target, +32 bound `this`, +40 bound arguments | 2 |
//! | GENERATOR | 23 | +24 state, +32 mode (int32s), +40 function, +48 receiver, +56 context, +64 registers (an ELEMENTS cell), +72 resume mode (int32), +80 async slot ([`genf`]) | 0 |
//! | SLOT_OBJECT | 24 | +24 ObjectClass id (int32), +32 internal slots: an ELEMENTS cell of n Values, or undefined when n = 0 ([`slot_obj`]) | 0 |
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | ELEMENTS | 65 | +8 capacity (int32), +16 values (hole = empty) | 16 + 8 cap |
//! | CONTEXT | 66 | +8 previous, +16 scope (native word: a [`ContextScope`] record), +24 length (int32), +32 slots | 32 + 8 len |
//!
//! Every word after word0 is a valid `Value` (raw pointers use the native-word encoding of
//! `frame.rs`), so one trace function per size visits them all; no trace function visits
//! the word0 shape (the heap marks it, H08). The fixed-field API ([`new_object_with_fields`],
//! [`fixed_get`], [`fixed_set`]) reads and writes fixed fields by index, so
//! PRIMITIVE_WRAPPER and BOUND_FUNCTION code never hard-codes an offset.
//!
//! OBJECT_LAYOUT 2 (`versions.rs`, M2-P0): the GENERATOR and SLOT_OBJECT kinds, the ITERATOR
//! kind field (+40; the cell grows from 40 to 48 B) and the CONTEXT scope word, which is now a
//! native word that points at the VM-side [`ContextScope`] of the context's unit scope (C5),
//! so a context names its slots: [`context_scope`] gives the unit and the scope descriptor.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, KindId, Slot, Tracer};

use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of, native_of, native_ptr};
use crate::kinds as kind;
use crate::runtime::{Atom, Root, Runtime, val};
use crate::shape;
use crate::unit::{ContextScope, FunctionInfo, LoadedUnit};

pub(crate) use crate::key::key_eq;
pub(crate) use crate::kinds::{
    cell_kind, is_array, is_context, is_error, is_function, is_iterator, is_object, is_string,
};
pub(crate) use crate::props::{own_count, own_define, own_delete, own_entry, own_get, own_replace};
pub(crate) use crate::string::{concat, new_string, new_string_utf16, str_len, str_view};

/// Object fields shared by every JS object kind (H17 v0).
pub(crate) mod obj {
    /// SLOTS (fast mode, out-of-line fields), PROPS (dictionary mode) or undefined.
    pub const SLOTS: usize = 8;
    /// ELEMENTS or undefined (v0: arrays only).
    pub const ELEMENTS: usize = 16;
    /// The first fixed field; the in-object slots follow the fixed fields.
    pub const FIXED: usize = 24;
    /// Most in-object slots (word0 flag bits 0-7).
    pub const MAX_INOBJECT: usize = 255;
    /// Flag bits of [`super::flags_of`] (the week-3 FLAGS word, now computed).
    pub const EXTENSIBLE: i32 = 1;
    /// A function with `[[Construct]]`.
    pub const CONSTRUCTOR: i32 = 2;
}

pub(crate) mod fun {
    pub const INFO: usize = 24;
    pub const CONTEXT: usize = 32;
    /// `[[HomeObject]]` (§10.2): set for methods; undefined otherwise.
    pub const HOME: usize = 40;
    pub const FIXED: usize = 3;
    /// In-object slots: `length` and `name`.
    pub const CAPACITY: u8 = 2;
}

pub(crate) mod iter {
    pub const ITERATED: usize = 24;
    pub const NEXT: usize = 32;
    /// The iteration kind (int32, [`super::iter_kind`]).
    pub const KIND: usize = 40;
    pub const FIXED: usize = 3;
}

/// The kind field of an ITERATOR cell (CreateArrayIterator's `kind`, §23.1.5.1). String
/// iterators use [`VALUES`](iter_kind::VALUES).
pub(crate) mod iter_kind {
    pub const VALUES: i32 = 0;
    #[allow(dead_code)] // caller: P4-B-ARRAY (`Array.prototype.keys`)
    pub const KEYS: i32 = 1;
    #[allow(dead_code)] // caller: P4-B-ARRAY (`Array.prototype.entries`)
    pub const ENTRIES: i32 = 2;
}

/// GENERATOR fixed fields (OBJECT_LAYOUT 2). Every field holds a Value; state, mode and
/// resume mode are int32s. One kind serves generators, async functions and async generators
/// (the mode). Accessors: [`new_generator`], [`gen_get`], [`gen_set`], [`gen_regs`].
pub(crate) mod genf {
    /// The suspend point id, or a closed/executing marker (int32).
    pub const STATE: usize = 24;
    /// [`MODE_SYNC`], [`MODE_ASYNC_FUNCTION`] or [`MODE_ASYNC_GENERATOR`] (int32).
    pub const MODE: usize = 32;
    /// The generator function.
    pub const FUNCTION: usize = 40;
    /// The receiver (`this`) of the body.
    pub const RECEIVER: usize = 48;
    /// The context of the suspended frame.
    pub const CONTEXT: usize = 56;
    /// The saved parameters and registers: an ELEMENTS cell (never a Rust buffer, plan rule 8).
    pub const REGS: usize = 64;
    /// How the frame resumes: next, return or throw (int32).
    pub const RESUME_MODE: usize = 72;
    /// Async functions and async generators: the promise or the request queue.
    #[allow(dead_code)] // month 3 (async)
    pub const ASYNC: usize = 80;
    pub const FIXED: usize = 8;
    pub const MODE_SYNC: i32 = 0;
    #[allow(dead_code)] // month 3 (async)
    pub const MODE_ASYNC_FUNCTION: i32 = 1;
    #[allow(dead_code)] // month 3 (async)
    pub const MODE_ASYNC_GENERATOR: i32 = 2;
    /// [`STATE`] of a completed generator (M2-P1-GEN-NUMBERS; suspend ids are >= 0).
    pub const STATE_CLOSED: i32 = -1;
    /// [`STATE`] while the body runs.
    pub const STATE_EXECUTING: i32 = -2;
    /// [`RESUME_MODE`] values (M2-P1-GEN-NUMBERS, V8's `JSGeneratorObject::ResumeMode`).
    pub const RESUME_NEXT: i32 = 0;
    pub const RESUME_RETURN: i32 = 1;
    pub const RESUME_THROW: i32 = 2;
    /// [`REGS`] index of the value that `next`/`return`/`throw` passed (`ResumeGenerator`
    /// puts it in the accumulator); the saved parameters, then the registers, follow it.
    pub const REGS_INPUT: usize = 0;
}

/// SLOT_OBJECT fixed fields (OBJECT_LAYOUT 2): the ObjectClass id and a separate cell for the
/// internal slots. The internal slots are never in-object slots: property code owns those
/// (`props::field_ref` stores named fields there, `props::to_dictionary` clears them), and it
/// never touches fixed fields. P3-ABI-SLOTS adds the accessors.
#[allow(dead_code)] // caller: P3-ABI-SLOTS (abi/slots.rs)
pub(crate) mod slot_obj {
    /// The ObjectClass id (int32).
    pub const CLASS: usize = 24;
    /// The internal slots: an ELEMENTS cell of n Values, or undefined when the class has none.
    pub const SLOTS: usize = 32;
    pub const FIXED: usize = 2;
}

pub(crate) mod arr {
    pub const LENGTH: usize = 24;
    pub const FIXED: usize = 1;
    /// v0: dense arrays only, up to this length. Elements kinds come with H18 (M1a).
    pub const MAX_LENGTH: u32 = 1 << 26;
}

pub(crate) mod err {
    pub const UNIT: usize = 24;
    pub const OFFSET: usize = 32;
    pub const FIXED: usize = 2;
}

/// Fixed-field counts of the kinds that only use the fixed-field API.
pub(crate) mod fixed {
    /// PRIMITIVE_WRAPPER: the primitive value.
    pub const WRAPPER: usize = 1;
    /// BOUND_FUNCTION: target, bound `this`, bound arguments.
    pub const BOUND: usize = 3;
}

mod elems {
    pub const CAPACITY: usize = 8;
    pub const DATA: usize = 16;
}

mod ctxf {
    pub const PREVIOUS: usize = 8;
    pub const SCOPE: usize = 16;
    pub const LENGTH: usize = 24;
    pub const SLOTS: usize = 32;
}

/// Property attribute bits (int32 in the dictionary table; bits 0-3 of shape details).
pub(crate) mod attr {
    pub const WRITABLE: i32 = 1;
    pub const ENUMERABLE: i32 = 2;
    pub const CONFIGURABLE: i32 = 4;
    /// The slot holds an ACCESSOR_PAIR (H17, OM-9; P3-ACC stores the pairs). Equals
    /// `shape::ACCESSOR`.
    pub const ACCESSOR: i32 = 8;
    /// Ordinary assignment: all three.
    pub const DEFAULT: i32 = WRITABLE | ENUMERABLE | CONFIGURABLE;
    /// Built-in methods and prototype links: writable, configurable, not enumerable.
    pub const HIDDEN: i32 = WRITABLE | CONFIGURABLE;
}

// ---- raw cell access ----------------------------------------------------------------------
//
// Crate invariant (H10): a cell that VM code holds (in a Rust local, on the VM stack or in a
// rooted slot) is live, and the heap never moves cells (S0-S2). The helpers below read and
// write the Value slots of such cells at offsets that the kind's layout table defines.

#[inline(always)]
pub(crate) fn cell(b: u64) -> CellPtr {
    debug_assert!(kind::is_cell_bits(b) && b != 0);
    CellPtr::from_value(val(b)).expect("a cell value")
}

#[inline(always)]
fn slot_of<'a>(c: CellPtr, off: usize) -> &'a Slot {
    // SAFETY: `c` is live (crate invariant) and every caller passes an offset of a Value slot
    // inside the cell's layout (checked against the cell's own capacity where it varies).
    unsafe { c.slot(off) }
}

#[inline(always)]
pub(crate) fn rd(c: CellPtr, off: usize) -> u64 {
    slot_of(c, off).bits()
}

/// Initializing store into a cell allocated since the last GC point (E2: no barrier).
#[inline(always)]
pub(crate) fn init(c: CellPtr, off: usize, v: u64) {
    slot_of(c, off).init(val(v));
}

/// Barriered store.
#[inline(always)]
pub(crate) fn wr(heap: &Heap, c: CellPtr, off: usize, v: u64) {
    // SAFETY: `c` is live (crate invariant).
    let h = unsafe { c.header() };
    slot_of(c, off).set(h, val(v), heap.barrier());
}

#[inline(always)]
pub(crate) fn rd_i32(c: CellPtr, off: usize) -> i32 {
    let b = rd(c, off);
    debug_assert!(val(b).is_int32());
    int32_of(b)
}

// ---- kind registration ---------------------------------------------------------------------

/// Visits every word in [8, size) of `c` as a Value slot.
pub(crate) fn trace_from_8(c: CellPtr, t: &mut Tracer<'_>, size: usize) {
    // SAFETY: the heap calls trace functions with live cells of the registered kind; every
    // word in [8, size) of these kinds is a Value slot (layout tables).
    t.visit_slots(unsafe { c.slots(8, (size - 8) / 8) });
}

/// The in-object slot count of a JS object cell (word0 flag bits 0-7).
#[inline(always)]
pub(crate) fn inobject_count(c: CellPtr) -> usize {
    // SAFETY: `c` is live (crate invariant; the heap passes live cells to size functions).
    usize::from(unsafe { c.header() }.flags() & kind::FLAG_INOBJECT_MASK)
}

/// The size of a JS object cell with `F` fixed fields.
fn object_size<const F: usize>(c: CellPtr) -> usize {
    obj::FIXED + 8 * (F + inobject_count(c))
}

fn trace_object<const F: usize>(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, object_size::<F>(c));
}

/// Fixed-field counts by kind id (0 for OBJECT and for ids that are not JS object kinds).
const FIXED_COUNTS: [u8; 256] = {
    let mut t = [0u8; 256];
    t[kind::FUNCTION.get() as usize] = fun::FIXED as u8;
    t[kind::ARRAY.get() as usize] = arr::FIXED as u8;
    t[kind::ERROR.get() as usize] = err::FIXED as u8;
    t[kind::ITERATOR.get() as usize] = iter::FIXED as u8;
    t[kind::PRIMITIVE_WRAPPER.get() as usize] = fixed::WRAPPER as u8;
    t[kind::BOUND_FUNCTION.get() as usize] = fixed::BOUND as u8;
    t[kind::GENERATOR.get() as usize] = genf::FIXED as u8;
    t[kind::SLOT_OBJECT.get() as usize] = slot_obj::FIXED as u8;
    t
};

/// The fixed-field count of the JS object kind `k` (16-63): one table load.
#[inline(always)]
pub(crate) fn fixed_count(k: u8) -> usize {
    debug_assert!(
        [
            kind::OBJECT,
            kind::FUNCTION,
            kind::ARRAY,
            kind::ERROR,
            kind::ITERATOR,
            kind::PRIMITIVE_WRAPPER,
            kind::BOUND_FUNCTION,
            kind::GENERATOR,
            kind::SLOT_OBJECT
        ]
        .iter()
        .any(|x| x.get() == k),
        "kind {k} is not a JS object kind"
    );
    usize::from(FIXED_COUNTS[usize::from(k)])
}

/// The in-object slot count new objects of kind `k` get (the OBJECT count is chosen per
/// prototype in [`new_object`]).
fn default_capacity(k: KindId) -> u8 {
    if k == kind::FUNCTION || k == kind::ERROR || k == kind::BOUND_FUNCTION {
        2
    } else if k == kind::OBJECT {
        4
    } else {
        0
    }
}

fn elements_size(c: CellPtr) -> usize {
    elems::DATA + 8 * rd_i32(c, elems::CAPACITY) as usize
}

fn trace_elements(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, elements_size(c));
}

fn context_size(c: CellPtr) -> usize {
    ctxf::SLOTS + 8 * rd_i32(c, ctxf::LENGTH) as usize
}

fn trace_context(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, context_size(c));
}

fn object_kind(
    name: &'static str,
    size: fn(CellPtr) -> usize,
    trace: fn(CellPtr, &mut Tracer<'_>),
) -> CellKind {
    CellKind {
        name,
        size: CellSize::Dynamic(size),
        trace,
    }
}

/// Registers every VM cell kind with a new heap: the kinds of this file, then each module
/// that owns a layout (strings, the property dictionary, symbols, shapes, feedback,
/// accessors, ICs).
pub(crate) fn register_kinds(heap: &Heap) {
    let kinds: [(KindId, CellKind); 11] = [
        (
            kind::OBJECT,
            object_kind("object", object_size::<0>, trace_object::<0>),
        ),
        (
            kind::FUNCTION,
            object_kind(
                "function",
                object_size::<{ fun::FIXED }>,
                trace_object::<{ fun::FIXED }>,
            ),
        ),
        (
            kind::ARRAY,
            object_kind(
                "array",
                object_size::<{ arr::FIXED }>,
                trace_object::<{ arr::FIXED }>,
            ),
        ),
        (
            kind::ERROR,
            object_kind(
                "error",
                object_size::<{ err::FIXED }>,
                trace_object::<{ err::FIXED }>,
            ),
        ),
        (
            kind::ITERATOR,
            object_kind(
                "iterator",
                object_size::<{ iter::FIXED }>,
                trace_object::<{ iter::FIXED }>,
            ),
        ),
        (
            kind::PRIMITIVE_WRAPPER,
            object_kind(
                "primitive-wrapper",
                object_size::<{ fixed::WRAPPER }>,
                trace_object::<{ fixed::WRAPPER }>,
            ),
        ),
        (
            kind::BOUND_FUNCTION,
            object_kind(
                "bound-function",
                object_size::<{ fixed::BOUND }>,
                trace_object::<{ fixed::BOUND }>,
            ),
        ),
        (
            kind::GENERATOR,
            object_kind(
                "generator",
                object_size::<{ genf::FIXED }>,
                trace_object::<{ genf::FIXED }>,
            ),
        ),
        (
            kind::SLOT_OBJECT,
            object_kind(
                "slot-object",
                object_size::<{ slot_obj::FIXED }>,
                trace_object::<{ slot_obj::FIXED }>,
            ),
        ),
        (
            kind::ELEMENTS,
            CellKind {
                name: "elements",
                size: CellSize::Dynamic(elements_size),
                trace: trace_elements,
            },
        ),
        (
            kind::CONTEXT,
            CellKind {
                name: "context",
                size: CellSize::Dynamic(context_size),
                trace: trace_context,
            },
        ),
    ];
    crate::string::register_kinds(heap);
    for (id, k) in kinds {
        heap.register_kind(id, k).expect("register a VM cell kind");
    }
    crate::props::register_kinds(heap);
    crate::symbol::register_kinds(heap);
    crate::shape::register_kinds(heap);
    crate::feedback::register_kinds(heap);
    crate::accessor::register_kinds(heap);
    crate::ic::register_kinds(heap);
}

// ---- objects --------------------------------------------------------------------------------

/// A new JS object of kind `k` with prototype `proto`, `capacity` in-object slots and no
/// properties; its fixed fields and slots hold undefined. GC point.
pub(crate) fn new_js_object(rt: &Runtime, k: KindId, proto: u64, capacity: u8) -> CellPtr {
    // GC points: the shape first (it may make `proto` a prototype), then the cell. Both
    // live in Rust locals (CSS).
    let s = shape::initial_shape(rt, proto, k, capacity);
    alloc_with_shape(rt, k, s, capacity)
}

/// Allocates a JS object of kind `k` with `capacity` in-object slots and installs `shape`
/// (a live shape of that kind) before the next GC point. GC point (the allocation).
fn alloc_with_shape(rt: &Runtime, k: KindId, s: u64, capacity: u8) -> CellPtr {
    let size = obj::FIXED + 8 * (fixed_count(k.get()) + usize::from(capacity));
    let c = rt.alloc(k, size);
    // The size function reads the in-object count: set it before the next GC point.
    // SAFETY: `c` is the new live cell.
    unsafe { c.header() }.set_flags(u16::from(capacity));
    shape::install(rt, c.to_value().to_bits(), s);
    c
}

/// `OrdinaryObjectCreate(proto)` (`proto` an object or null). In-object slots: 4 when the
/// prototype is %Object.prototype% (`{}`), 8 otherwise (`new F`, which reaches here
/// through OrdinaryCreateFromConstructor, and Object.create). GC point.
pub(crate) fn new_object(rt: &Runtime, proto: u64) -> u64 {
    let cap = if proto == rt.root(Root::ObjectPrototype) {
        4
    } else {
        8
    };
    new_object_with_capacity(rt, proto, cap)
}

/// An ordinary object with `capacity` in-object slots (literals: one per named key, at most
/// [`obj::MAX_INOBJECT`]). GC point.
pub(crate) fn new_object_with_capacity(rt: &Runtime, proto: u64, capacity: usize) -> u64 {
    let cap = capacity.min(obj::MAX_INOBJECT) as u8;
    new_js_object(rt, kind::OBJECT, proto, cap)
        .to_value()
        .to_bits()
}

pub(crate) fn new_plain_object(rt: &Runtime) -> u64 {
    new_object(rt, rt.root(Root::ObjectPrototype))
}

/// A new extensible object of kind `k` with prototype `proto` and `n` fixed fields, all
/// undefined. `n` must be the kind's fixed-field count. GC point.
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn new_object_with_fields(rt: &Runtime, k: KindId, proto: u64, n: usize) -> u64 {
    assert_eq!(
        n,
        fixed_count(k.get()),
        "kind {} has another fixed-field count",
        k.get()
    );
    new_js_object(rt, k, proto, default_capacity(k))
        .to_value()
        .to_bits()
}

/// Fixed field `i` of the object `o`.
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn fixed_get(o: u64, i: usize) -> u64 {
    debug_assert!(i < fixed_count(cell_kind(o)));
    rd(cell(o), obj::FIXED + 8 * i)
}

/// Stores fixed field `i` of the object `o` (barriered).
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn fixed_set(rt: &Runtime, o: u64, i: usize, v: u64) {
    debug_assert!(i < fixed_count(cell_kind(o)));
    wr(&rt.heap, cell(o), obj::FIXED + 8 * i, v);
}

/// `[[Prototype]]` of the object `o` (an object or null): its shape's.
#[inline]
pub(crate) fn proto_of(o: u64) -> u64 {
    shape::proto(shape::shape_of(o))
}

/// Sets the prototype of `o` to `p` (an object or null), unless that makes a cycle
/// (§10.1.2 OrdinarySetPrototypeOf step 8). Returns false for a cycle. A raw store: it
/// does not check `[[Extensible]]` (`property::set_prototype_of` does). GC point.
pub(crate) fn set_proto(rt: &Runtime, o: u64, p: u64) -> bool {
    let mut q = p;
    while q != NULL {
        if q == o {
            return false;
        }
        q = proto_of(q);
    }
    if proto_of(o) == p {
        return true;
    }
    let s = shape::shape_of(o);
    if !shape::is_dictionary(s) && shape::flags(s) & shape::flag::UNIQUE == 0 && shape::count(s) > 0
    {
        // OM-10: an object with properties in a shared fast shape goes to dictionary mode.
        crate::props::to_dictionary(rt, o);
    }
    // GC point below: `o` and `p` live in Rust locals.
    let cap = inobject_count(cell(o)) as u8;
    let ns = shape::with_proto(rt, shape::shape_of(o), p, cap);
    shape::install(rt, o, ns);
    shape::note_change(rt, o);
    true
}

/// The week-3 FLAGS word, computed: [`obj::EXTENSIBLE`] from the shape, [`obj::CONSTRUCTOR`]
/// for a FUNCTION with `[[Construct]]`.
#[allow(dead_code)] // the P0 compatibility API (no caller left)
pub(crate) fn flags_of(o: u64) -> i32 {
    let mut f = 0;
    if shape::is_extensible(shape::shape_of(o)) {
        f |= obj::EXTENSIBLE;
    }
    if is_constructor(o) {
        f |= obj::CONSTRUCTOR;
    }
    f
}

// ---- arrays --------------------------------------------------------------------------------

pub(crate) fn new_array(rt: &Runtime, capacity: usize) -> u64 {
    let a = new_js_object(rt, kind::ARRAY, rt.root(Root::ArrayPrototype), 0);
    init(a, arr::LENGTH, int32_bits(0));
    if capacity > 0 {
        let e = new_elements(rt, capacity);
        wr(&rt.heap, a, obj::ELEMENTS, e.to_value().to_bits());
    }
    a.to_value().to_bits()
}

fn new_elements(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * cap);
    init(c, elems::CAPACITY, int32_bits(cap as i32));
    for i in 0..cap {
        init(c, elems::DATA + 8 * i, HOLE);
    }
    c
}

pub(crate) fn array_len(a: u64) -> u32 {
    rd_i32(cell(a), arr::LENGTH) as u32
}

fn elements_of(a: u64) -> Option<CellPtr> {
    let e = rd(cell(a), obj::ELEMENTS);
    (e != UNDEFINED).then(|| cell(e))
}

/// Element `i`: `Some(value)` if `i < length` and not a hole.
pub(crate) fn array_get(a: u64, i: u32) -> Option<u64> {
    if i >= array_len(a) {
        return None;
    }
    let e = elements_of(a)?;
    if i as usize >= rd_i32(e, elems::CAPACITY) as usize {
        return None;
    }
    let v = rd(e, elems::DATA + 8 * i as usize);
    (v != HOLE).then_some(v)
}

/// Stores element `i`, growing the array; [`HOLE`] removes it. False if `i` is past the v0
/// dense limit. A raw store: callers check `[[Extensible]]` for a new element, and a
/// `[[Set]]` caller the prototype chain (`shape::proto_index_keys`). Every element write goes
/// through here and [`array_set_length`]: an element that appears or goes away is a
/// structural change of a prototype (OM-8, `shape::note_change`); a value store is not. An
/// element that appears on a prototype also sets `shape::proto_index_keys`.
pub(crate) fn array_set(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if i >= arr::MAX_LENGTH {
        return false;
    }
    // Only a prototype pays for more than the flag test.
    let structural = shape::is_prototype_object(a) && array_get(a, i).is_some() != (v != HOLE);
    let cap = elements_of(a).map_or(0, |e| rd_i32(e, elems::CAPACITY) as usize);
    let e = if (i as usize) < cap {
        elements_of(a).expect("elements")
    } else {
        // GC point: `a` and `v` live in Rust locals.
        let n = new_elements(rt, (i as usize + 1).max(cap * 2).max(4));
        if let Some(old) = elements_of(a) {
            for j in 0..cap {
                init(n, elems::DATA + 8 * j, rd(old, elems::DATA + 8 * j));
            }
        }
        wr(&rt.heap, cell(a), obj::ELEMENTS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, e, elems::DATA + 8 * i as usize, v);
    if i >= array_len(a) {
        wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(i as i32 + 1));
    }
    if structural {
        if v != HOLE {
            shape::note_proto_index_key(rt, a);
        }
        shape::note_change(rt, a);
    }
    true
}

/// Sets `length` (§10.4.2.4 ArraySetLength, v0: dense, no non-configurable elements). False
/// if `len` is past the v0 dense limit. A cut that removes an element is a structural change
/// of a prototype (OM-8); growth and a cut over holes only change the value of `length`.
pub(crate) fn array_set_length(rt: &Runtime, a: u64, len: u32) -> bool {
    if len > arr::MAX_LENGTH {
        return false;
    }
    let old = array_len(a);
    if len < old
        && let Some(e) = elements_of(a)
    {
        let end = old.min(rd_i32(e, elems::CAPACITY) as u32);
        let at = |j: u32| elems::DATA + 8 * j as usize;
        // Only a prototype pays for more than the flag test.
        let structural = shape::is_prototype_object(a) && (len..end).any(|j| rd(e, at(j)) != HOLE);
        for j in len..end {
            wr(&rt.heap, e, at(j), HOLE);
        }
        if structural {
            shape::note_change(rt, a);
        }
    }
    wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(len as i32));
    true
}

// ---- functions, contexts, errors -----------------------------------------------------------

/// A new function object for `info` with `context`. It gets the own data properties
/// `length` and `name` from the FunctionInfo, {w: false, e: false, c: true} (§10.2.9
/// SetFunctionName, §10.2.10 SetFunctionLength), in its 2 in-object slots. `prototype`
/// stays lazy (`exotic.rs`). Its `[[Prototype]]` is %Function.prototype%, or
/// %GeneratorFunction.prototype% for a generator function (§15.5.3). GC point.
pub(crate) fn new_function(rt: &Runtime, info: *const FunctionInfo, context: u64) -> u64 {
    // SAFETY: function infos outlive the isolate's cells (units and host infos are never
    // freed while the isolate lives).
    let fi = unsafe { &*info };
    // GC points first (the name atom of a host function, the shape); the values live in
    // Rust locals (CSS).
    let name = function_name(rt, fi);
    let s = if fi.is_generator() {
        shape::generator_function_shape(rt)
    } else {
        shape::function_shape(rt)
    };
    let f = alloc_with_shape(rt, kind::FUNCTION, s, fun::CAPACITY);
    init(f, fun::INFO, native_of(info));
    init(f, fun::CONTEXT, context);
    let slots = obj::FIXED + 8 * fun::FIXED;
    init(f, slots, crate::conv::num(f64::from(fi.length)));
    init(f, slots + 8, name);
    f.to_value().to_bits()
}

/// The `name` value of a new function for `fi`: the unit's atom for a bytecode function (no
/// allocation), the interned text for a host function. GC point.
fn function_name(rt: &Runtime, fi: &FunctionInfo) -> u64 {
    if let Some(u) = fi.loaded_unit() {
        return match u.unit.functions[fi.function_id as usize].name {
            Some(id) => u.string(id.index()),
            None => rt.atom(Atom::Empty),
        };
    }
    if fi.name.is_empty() {
        rt.atom(Atom::Empty)
    } else {
        crate::atoms::intern_str(rt, &fi.name)
    }
}

/// The FunctionInfo of a function cell.
pub(crate) fn function_info<'a>(f: u64) -> &'a FunctionInfo {
    // SAFETY: a FUNCTION cell's slot +24 holds the native word of a FunctionInfo that lives as
    // long as the isolate.
    unsafe { &*native_ptr::<FunctionInfo>(rd(cell(f), fun::INFO)) }
}

pub(crate) fn function_context(f: u64) -> u64 {
    rd(cell(f), fun::CONTEXT)
}

/// `[[HomeObject]]` of a function cell (undefined if it has none).
pub(crate) fn home_object(f: u64) -> u64 {
    rd(cell(f), fun::HOME)
}

pub(crate) fn set_home_object(rt: &Runtime, f: u64, home: u64) {
    wr(&rt.heap, cell(f), fun::HOME, home);
}

/// A new ITERATOR cell with prototype `proto` over `iterated`: an object for an array iterator
/// (CreateArrayIterator, §23.1.5.1, kind "value"; the object is read as an array-like), or a
/// string for a string iterator (CreateStringIterator, §22.1.5.1). The other states are listed
/// at [`iterator_state`].
pub(crate) fn new_iterator(rt: &Runtime, proto: u64, iterated: u64) -> u64 {
    new_iterator_of_kind(rt, proto, iterated, iter_kind::VALUES)
}

/// [`new_iterator`] with the iteration kind `k` ([`iter_kind`]). GC point.
pub(crate) fn new_iterator_of_kind(rt: &Runtime, proto: u64, iterated: u64, k: i32) -> u64 {
    let c = new_js_object(rt, kind::ITERATOR, proto, 0);
    init(c, iter::ITERATED, iterated);
    init(c, iter::NEXT, int32_bits(0));
    init(c, iter::KIND, int32_bits(k));
    c.to_value().to_bits()
}

/// The iteration kind of an ITERATOR cell ([`iter_kind`]).
#[allow(dead_code)] // caller: P4-B-ARRAY (the array iterator `next` for keys and entries)
pub(crate) fn iterator_kind(it: u64) -> i32 {
    rd_i32(cell(it), iter::KIND)
}

/// (iterated value, next index) of an ITERATOR cell. Array iterator: an object (suspended),
/// null (executing: `next` runs user code, W4-P3FIX-ITER-EXEC-NULL) or undefined (done).
/// String iterator: a string, or "" (done).
pub(crate) fn iterator_state(it: u64) -> (u64, u32) {
    let c = cell(it);
    (rd(c, iter::ITERATED), rd_i32(c, iter::NEXT) as u32)
}

/// Updates an ITERATOR cell; `iterated` = undefined (array iterator) or "" (string iterator)
/// marks it done, and null marks an array iterator executing.
pub(crate) fn set_iterator_state(rt: &Runtime, it: u64, iterated: u64, next: u32) {
    let c = cell(it);
    wr(&rt.heap, c, iter::ITERATED, iterated);
    wr(&rt.heap, c, iter::NEXT, int32_bits(next as i32));
}

// ---- generators (GENERATOR cells, [`genf`]) ---------------------------------------------------

/// A new GENERATOR object with prototype `proto` for the generator function `function`, in
/// state executing with resume mode next: `receiver` is the body's `this`, `context` its
/// context, and REGS a new ELEMENTS cell of `regs` Values (holes; the input slot undefined).
/// GC point.
pub(crate) fn new_generator(
    rt: &Runtime,
    proto: u64,
    function: u64,
    receiver: u64,
    context: u64,
    mode: i32,
    regs: usize,
) -> u64 {
    // GC points first (the ELEMENTS cell, then the object); the values live in Rust locals
    // (CSS) until the object holds them.
    let r = new_elements(rt, regs.max(1));
    init(r, elems::DATA + 8 * genf::REGS_INPUT, UNDEFINED);
    let r = r.to_value().to_bits();
    let g = new_js_object(rt, kind::GENERATOR, proto, 0);
    init(g, genf::STATE, int32_bits(genf::STATE_EXECUTING));
    init(g, genf::MODE, int32_bits(mode));
    init(g, genf::FUNCTION, function);
    init(g, genf::RECEIVER, receiver);
    init(g, genf::CONTEXT, context);
    init(g, genf::REGS, r);
    init(g, genf::RESUME_MODE, int32_bits(genf::RESUME_NEXT));
    init(g, genf::ASYNC, UNDEFINED);
    g.to_value().to_bits()
}

/// GENERATOR field `off` ([`genf`]) of `g`.
#[inline]
pub(crate) fn gen_get(g: u64, off: usize) -> u64 {
    debug_assert!(cell_kind(g) == kind::GENERATOR.get());
    debug_assert!(
        (genf::STATE..genf::STATE + 8 * genf::FIXED).contains(&off) && off.is_multiple_of(8)
    );
    rd(cell(g), off)
}

/// An int32 GENERATOR field (state, mode, resume mode) of `g`.
#[inline]
pub(crate) fn gen_get_i32(g: u64, off: usize) -> i32 {
    int32_of(gen_get(g, off))
}

/// Stores GENERATOR field `off` of `g` (barriered).
#[inline]
pub(crate) fn gen_set(rt: &Runtime, g: u64, off: usize, v: u64) {
    debug_assert!(cell_kind(g) == kind::GENERATOR.get());
    debug_assert!(
        (genf::STATE..genf::STATE + 8 * genf::FIXED).contains(&off) && off.is_multiple_of(8)
    );
    wr(&rt.heap, cell(g), off, v);
}

/// The saved values of a generator (its REGS cell): `[input, params.., registers..]`
/// ([`genf::REGS_INPUT`]). A view of a live cell: hold it only while the generator is held,
/// and do not keep it across a GC point after [`gen_release`].
#[derive(Clone, Copy)]
pub(crate) struct GenRegs {
    cell: CellPtr,
    len: usize,
}

impl GenRegs {
    /// The number of saved slots.
    #[inline]
    pub fn len(self) -> usize {
        self.len
    }

    /// Slot `i` (`i < len`).
    #[inline]
    pub fn get(self, i: usize) -> u64 {
        assert!(i < self.len, "generator slot {i} out of range");
        rd(self.cell, elems::DATA + 8 * i)
    }

    /// Stores slot `i` (`i < len`, barriered).
    #[inline]
    pub fn set(self, rt: &Runtime, i: usize, v: u64) {
        assert!(i < self.len, "generator slot {i} out of range");
        wr(&rt.heap, self.cell, elems::DATA + 8 * i, v);
    }
}

/// The saved values of `g`, or `None` once they were released ([`gen_release`]).
pub(crate) fn gen_regs(g: u64) -> Option<GenRegs> {
    let r = gen_get(g, genf::REGS);
    if r == UNDEFINED {
        return None;
    }
    debug_assert!(cell_kind(r) == kind::ELEMENTS.get());
    let c = cell(r);
    Some(GenRegs {
        cell: c,
        len: rd_i32(c, elems::CAPACITY) as usize,
    })
}

/// Drops the saved values of the completed generator `g` (REGS = undefined), so it keeps
/// nothing alive that only its frame used.
pub(crate) fn gen_release(rt: &Runtime, g: u64) {
    gen_set(rt, g, genf::REGS, UNDEFINED);
}

/// IsConstructor (§7.2.4): a FUNCTION whose FunctionInfo has `[[Construct]]`, or a bound
/// function whose target is a constructor (`bound::is_constructor`).
pub(crate) fn is_constructor(f: u64) -> bool {
    let k = cell_kind(f);
    if k == kind::FUNCTION.get() {
        function_info(f).is_constructor()
    } else {
        k == kind::BOUND_FUNCTION.get() && crate::bound::is_constructor(f)
    }
}

/// A new context with `len` slots (undefined) for the unit scope `scope`, whose previous
/// context is `previous`. `scope` is an entry of its unit's [`LoadedUnit::context_scopes`],
/// which lives as long as the isolate. GC point.
pub(crate) fn new_context(
    rt: &Runtime,
    previous: u64,
    scope: *const ContextScope,
    len: usize,
) -> u64 {
    let c = rt.alloc(kind::CONTEXT, ctxf::SLOTS + 8 * len);
    init(c, ctxf::PREVIOUS, previous);
    init(c, ctxf::SCOPE, native_of(scope));
    init(c, ctxf::LENGTH, int32_bits(len as i32));
    c.to_value().to_bits()
}

/// The unit and the scope descriptor of the context `c` (C5): a slot name resolves with
/// `unit.string(scope.slots[i].name.index())`.
pub(crate) fn context_scope<'a>(c: u64) -> (&'a LoadedUnit, &'a ep9_bytecode::Scope) {
    // SAFETY: the SCOPE word of a CONTEXT cell is the native word of a `ContextScope` in a
    // boxed LoadedUnit (`new_context`); units are never freed while the isolate lives
    // (`Runtime::units` only grows), and the record's pointers point into that unit.
    let cs = unsafe { &*native_ptr::<ContextScope>(rd(cell(c), ctxf::SCOPE)) };
    // SAFETY: as above; `scope` points into the unit's verified scope table, which never
    // changes after verification.
    unsafe { (&*cs.unit, &*cs.scope) }
}

pub(crate) fn context_previous(c: u64) -> u64 {
    rd(cell(c), ctxf::PREVIOUS)
}

/// Slot `i`, or `None` if out of range (H02-Q2: the VM bounds-checks context slots).
pub(crate) fn context_get(c: u64, i: u32) -> Option<u64> {
    let cc = cell(c);
    ((i as usize) < rd_i32(cc, ctxf::LENGTH) as usize).then(|| rd(cc, ctxf::SLOTS + 8 * i as usize))
}

pub(crate) fn context_set(rt: &Runtime, c: u64, i: u32, v: u64) -> bool {
    let cc = cell(c);
    if (i as usize) < rd_i32(cc, ctxf::LENGTH) as usize {
        wr(&rt.heap, cc, ctxf::SLOTS + 8 * i as usize, v);
        true
    } else {
        false
    }
}

pub(crate) fn new_error_object(rt: &Runtime, proto: u64) -> u64 {
    let c = new_js_object(rt, kind::ERROR, proto, default_capacity(kind::ERROR));
    c.to_value().to_bits()
}

/// The (unit, source offset) recorded when an Error was created.
pub(crate) fn error_site(e: u64) -> Option<(u32, u32)> {
    let c = cell(e);
    let u = rd(c, err::UNIT);
    (u != UNDEFINED).then(|| (int32_of(u) as u32, int32_of(rd(c, err::OFFSET)) as u32))
}

pub(crate) fn set_error_site(rt: &Runtime, e: u64, unit: u32, offset: u32) {
    wr(&rt.heap, cell(e), err::UNIT, int32_bits(unit as i32));
    wr(&rt.heap, cell(e), err::OFFSET, int32_bits(offset as i32));
}
