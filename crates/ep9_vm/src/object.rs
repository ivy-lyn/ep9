//! v0 object model (M0 week 3). Week 4 replaces the property dictionary with shapes (H8, H17)
//! and the strings with H19 strings. Every shortcut is marked `// v0:`.
//!
//! All cells are `ep9_heap` cells. Every word after word0 of an object, table or context is a
//! valid `Value` (raw pointers use the native-word encoding of `frame.rs`), so one trace
//! function visits them all. Kind ids are in `kinds.rs` (table v1). Strings live in
//! `string.rs`, the property dictionary in `props.rs` and the key helpers in `key.rs`; this
//! file re-exports their old names.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | OBJECT | 16 | +8 prototype, +16 property table or undefined, +24 flags (int32) | 32 |
//! | FUNCTION | 17 | object + 32 FunctionInfo (native word), +40 context, +48 `[[HomeObject]]` or undefined | 56 |
//! | ARRAY | 18 | object + 32 elements or undefined, +40 length (int32) | 48 |
//! | ERROR | 19 | object + 32 unit index (int32 or undefined), +40 source offset (int32) | 48 |
//! | ITERATOR | 20 | object + 32 iterated array or string (undefined when done), +40 next index (int32) | 48 |
//! | PRIMITIVE_WRAPPER | 21 | object + 1 fixed field: the primitive | 40 |
//! | BOUND_FUNCTION | 22 | object + 3 fixed fields: target, bound `this`, bound arguments | 56 |
//! | ELEMENTS | 65 | +8 capacity (int32), +16 values (hole = empty) | 16 + 8 cap |
//! | CONTEXT | 66 | +8 previous, +16 scope id (int32), +24 length (int32), +32 slots | 32 + 8 len |
//!
//! Fixed fields are the words after the object header (from +32). The fixed-field API
//! ([`new_object_with_fields`], [`fixed_get`], [`fixed_set`]) reads and writes them by
//! index, so PRIMITIVE_WRAPPER and BOUND_FUNCTION code never hard-codes an offset (H17 moves
//! the header; these signatures stay).

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, KindId, Slot, Tracer};

use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of, native_of, native_ptr};
use crate::kinds as kind;
use crate::runtime::{Root, Runtime, val};
use crate::unit::FunctionInfo;

pub(crate) use crate::key::key_eq;
pub(crate) use crate::kinds::{
    cell_kind, is_array, is_context, is_error, is_function, is_iterator, is_object, is_string,
};
pub(crate) use crate::props::{own_count, own_define, own_delete, own_entry, own_get, own_replace};
pub(crate) use crate::string::{concat, new_string, new_string_utf16, str_len, str_view};

/// Object fields shared by every JS object kind.
pub(crate) mod obj {
    pub const PROTO: usize = 8;
    pub const PROPS: usize = 16;
    pub const FLAGS: usize = 24;
    pub const SIZE: usize = 32;
    /// Flag bits.
    pub const EXTENSIBLE: i32 = 1;
    /// A function with `[[Construct]]`.
    pub const CONSTRUCTOR: i32 = 2;
}

pub(crate) mod fun {
    pub const INFO: usize = 32;
    pub const CONTEXT: usize = 40;
    /// `[[HomeObject]]` (§10.2): set for methods; undefined otherwise.
    pub const HOME: usize = 48;
    pub const SIZE: usize = 56;
}

pub(crate) mod iter {
    pub const ITERATED: usize = 32;
    pub const NEXT: usize = 40;
    pub const SIZE: usize = 48;
}

pub(crate) mod arr {
    pub const ELEMENTS: usize = 32;
    pub const LENGTH: usize = 40;
    pub const SIZE: usize = 48;
    /// v0: dense arrays only, up to this length. Week 4 adds elements kinds (H18).
    pub const MAX_LENGTH: u32 = 1 << 26;
}

pub(crate) mod err {
    pub const UNIT: usize = 32;
    pub const OFFSET: usize = 40;
    pub const SIZE: usize = 48;
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

/// Property attribute bits (int32 in the table).
pub(crate) mod attr {
    pub const WRITABLE: i32 = 1;
    pub const ENUMERABLE: i32 = 2;
    pub const CONFIGURABLE: i32 = 4;
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

fn trace_object(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, obj::SIZE);
}

fn trace_object48(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, 48);
}

fn trace_function(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, fun::SIZE);
}

fn trace_wrapper(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, obj::SIZE + 8 * fixed::WRAPPER);
}

fn trace_bound(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, obj::SIZE + 8 * fixed::BOUND);
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

/// Registers every VM cell kind with a new heap: the kinds of this file, then each module
/// that owns a layout (strings, the property dictionary, symbols, shapes, feedback,
/// accessors, ICs).
pub(crate) fn register_kinds(heap: &Heap) {
    let kinds: [(KindId, CellKind); 9] = [
        (
            kind::OBJECT,
            CellKind {
                name: "object",
                size: CellSize::Fixed(obj::SIZE),
                trace: trace_object,
            },
        ),
        (
            kind::FUNCTION,
            CellKind {
                name: "function",
                size: CellSize::Fixed(fun::SIZE),
                trace: trace_function,
            },
        ),
        (
            kind::ARRAY,
            CellKind {
                name: "array",
                size: CellSize::Fixed(arr::SIZE),
                trace: trace_object48,
            },
        ),
        (
            kind::ERROR,
            CellKind {
                name: "error",
                size: CellSize::Fixed(err::SIZE),
                trace: trace_object48,
            },
        ),
        (
            kind::ITERATOR,
            CellKind {
                name: "iterator",
                size: CellSize::Fixed(iter::SIZE),
                trace: trace_object48,
            },
        ),
        (
            kind::PRIMITIVE_WRAPPER,
            CellKind {
                name: "primitive-wrapper",
                size: CellSize::Fixed(obj::SIZE + 8 * fixed::WRAPPER),
                trace: trace_wrapper,
            },
        ),
        (
            kind::BOUND_FUNCTION,
            CellKind {
                name: "bound-function",
                size: CellSize::Fixed(obj::SIZE + 8 * fixed::BOUND),
                trace: trace_bound,
            },
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

/// A new object of `kind` (any JS object kind) with `size` bytes.
pub(crate) fn new_object_of(rt: &Runtime, k: KindId, size: usize, proto: u64) -> CellPtr {
    let c = rt.alloc(k, size);
    init(c, obj::PROTO, proto);
    init(c, obj::FLAGS, int32_bits(obj::EXTENSIBLE));
    c
}

/// `OrdinaryObjectCreate(proto)`.
pub(crate) fn new_object(rt: &Runtime, proto: u64) -> u64 {
    new_object_of(rt, kind::OBJECT, obj::SIZE, proto)
        .to_value()
        .to_bits()
}

pub(crate) fn new_plain_object(rt: &Runtime) -> u64 {
    new_object(rt, rt.root(Root::ObjectPrototype))
}

/// A new extensible object of kind `k` with prototype `proto` and `n` fixed fields, all
/// undefined. `n` must be the kind's fixed-field count (its registered size). GC point.
#[allow(dead_code)] // wk4-stub
pub(crate) fn new_object_with_fields(rt: &Runtime, k: KindId, proto: u64, n: usize) -> u64 {
    new_object_of(rt, k, obj::SIZE + 8 * n, proto)
        .to_value()
        .to_bits()
}

/// Fixed field `i` of the object `o`.
#[allow(dead_code)] // wk4-stub
pub(crate) fn fixed_get(o: u64, i: usize) -> u64 {
    rd(cell(o), obj::SIZE + 8 * i)
}

/// Stores fixed field `i` of the object `o` (barriered).
#[allow(dead_code)] // wk4-stub
pub(crate) fn fixed_set(rt: &Runtime, o: u64, i: usize, v: u64) {
    wr(&rt.heap, cell(o), obj::SIZE + 8 * i, v);
}

pub(crate) fn proto_of(o: u64) -> u64 {
    rd(cell(o), obj::PROTO)
}

/// Sets the prototype of `o` to `p` (an object or null), unless that makes a cycle
/// (§10.1.2 OrdinarySetPrototypeOf step 8). Returns false for a cycle.
pub(crate) fn set_proto(rt: &Runtime, o: u64, p: u64) -> bool {
    let mut q = p;
    while q != NULL {
        if q == o {
            return false;
        }
        q = proto_of(q);
    }
    wr(&rt.heap, cell(o), obj::PROTO, p);
    true
}

pub(crate) fn flags_of(o: u64) -> i32 {
    rd_i32(cell(o), obj::FLAGS)
}

// ---- arrays --------------------------------------------------------------------------------

pub(crate) fn new_array(rt: &Runtime, capacity: usize) -> u64 {
    let a = new_object_of(rt, kind::ARRAY, arr::SIZE, rt.root(Root::ArrayPrototype));
    init(a, arr::LENGTH, int32_bits(0));
    if capacity > 0 {
        let e = new_elements(rt, capacity);
        wr(&rt.heap, a, arr::ELEMENTS, e.to_value().to_bits());
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
    let e = rd(cell(a), arr::ELEMENTS);
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

/// Stores element `i`, growing the array. False if `i` is past the v0 dense limit.
pub(crate) fn array_set(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if i >= arr::MAX_LENGTH {
        return false;
    }
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
        wr(&rt.heap, cell(a), arr::ELEMENTS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, e, elems::DATA + 8 * i as usize, v);
    if i >= array_len(a) {
        wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(i as i32 + 1));
    }
    true
}

/// Sets `length` (§10.4.2.4 ArraySetLength, v0: dense, no non-configurable elements). False
/// if `len` is past the v0 dense limit.
pub(crate) fn array_set_length(rt: &Runtime, a: u64, len: u32) -> bool {
    if len > arr::MAX_LENGTH {
        return false;
    }
    let old = array_len(a);
    if len < old
        && let Some(e) = elements_of(a)
    {
        let cap = rd_i32(e, elems::CAPACITY) as u32;
        for j in len..old.min(cap) {
            wr(&rt.heap, e, elems::DATA + 8 * j as usize, HOLE);
        }
    }
    wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(len as i32));
    true
}

// ---- functions, contexts, errors -----------------------------------------------------------

pub(crate) fn new_function(rt: &Runtime, info: *const FunctionInfo, context: u64) -> u64 {
    // SAFETY: function infos outlive the isolate's cells (units and host infos are never
    // freed while the isolate lives).
    let fi = unsafe { &*info };
    let f = new_object_of(
        rt,
        kind::FUNCTION,
        fun::SIZE,
        rt.root(Root::FunctionPrototype),
    );
    let mut flags = obj::EXTENSIBLE;
    if fi.is_constructor() {
        flags |= obj::CONSTRUCTOR;
    }
    init(f, obj::FLAGS, int32_bits(flags));
    init(f, fun::INFO, native_of(info));
    init(f, fun::CONTEXT, context);
    f.to_value().to_bits()
}

/// The FunctionInfo of a function cell.
pub(crate) fn function_info<'a>(f: u64) -> &'a FunctionInfo {
    // SAFETY: a FUNCTION cell's slot +32 holds the native word of a FunctionInfo that lives as
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

/// A new v0 iterator over the array or string `iterated` (CreateArrayIterator, §23.1.5.1, kind
/// "value"; CreateStringIterator, §22.1.5.1). `proto` is its prototype.
pub(crate) fn new_iterator(rt: &Runtime, proto: u64, iterated: u64) -> u64 {
    let c = new_object_of(rt, kind::ITERATOR, iter::SIZE, proto);
    init(c, iter::ITERATED, iterated);
    init(c, iter::NEXT, int32_bits(0));
    c.to_value().to_bits()
}

/// (iterated object or undefined, next index) of an ITERATOR cell.
pub(crate) fn iterator_state(it: u64) -> (u64, u32) {
    let c = cell(it);
    (rd(c, iter::ITERATED), rd_i32(c, iter::NEXT) as u32)
}

/// Updates an ITERATOR cell; `iterated = undefined` marks it done.
pub(crate) fn set_iterator_state(rt: &Runtime, it: u64, iterated: u64, next: u32) {
    let c = cell(it);
    wr(&rt.heap, c, iter::ITERATED, iterated);
    wr(&rt.heap, c, iter::NEXT, int32_bits(next as i32));
}

/// IsConstructor (§7.2.4): a FUNCTION with `[[Construct]]`, or a bound function whose target
/// is a constructor (`bound::is_constructor`).
pub(crate) fn is_constructor(f: u64) -> bool {
    let k = cell_kind(f);
    if k == kind::FUNCTION.get() {
        flags_of(f) & obj::CONSTRUCTOR != 0
    } else {
        k == kind::BOUND_FUNCTION.get() && crate::bound::is_constructor(f)
    }
}

pub(crate) fn new_context(rt: &Runtime, previous: u64, scope: i32, len: usize) -> u64 {
    let c = rt.alloc(kind::CONTEXT, ctxf::SLOTS + 8 * len);
    init(c, ctxf::PREVIOUS, previous);
    init(c, ctxf::SCOPE, int32_bits(scope));
    init(c, ctxf::LENGTH, int32_bits(len as i32));
    c.to_value().to_bits()
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
    new_object_of(rt, kind::ERROR, err::SIZE, proto)
        .to_value()
        .to_bits()
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
