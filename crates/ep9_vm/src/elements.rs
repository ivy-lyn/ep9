//! Elements kinds (owner M5-P1 ELEM1, then M5-P2 ELEM2; H18, V10-D7, V10-D8, MC-w15-21): the
//! kind of an object's elements store, its storage form, the one-way transitions and the
//! keyed kinds-set bits of IC_LAYOUT 3. M5-P0 fixed the values (OBJECT_LAYOUT 5, IC_LAYOUT 3;
//! V10-A21: `layout.rs`, `kinds.rs` and `versions.rs` are frozen after M5-P0); M5-P1 ELEM1
//! builds the lattice below; DICTIONARY (an ELEMENTS_DICT cell) is M5-P2 ELEM2's, so no object
//! has kind 6 yet.
//!
//! ```text
//!  value  kind              storage form (one storage form counts as one IC case)
//!  0      PACKED_INT32      ┐
//!  1      HOLEY_INT32       │ Values in an ELEMENTS cell (kind 65)
//!  4      PACKED_ELEMENTS   │
//!  5      HOLEY_ELEMENTS    ┘
//!  2      PACKED_DOUBLE     ┐ raw f64 in an ELEMENTS_DOUBLE cell (kind 81; hole = the NaN
//!  3      HOLEY_DOUBLE      ┘ 0xFFF7_FFFF_FFF7_FFFF, `object::delems`)
//!  6      DICTIONARY          an ELEMENTS_DICT cell (kind 82; M5-P2 ELEM2)
//!  7-15   reserved, append-only (NONEXTENSIBLE, SEALED, FROZEN if built later)
//!  bit 0 = holey (for 0-5)
//!
//!  The lattice (a store moves right or down, never back; `join`):
//!   PACKED_INT32 ──> PACKED_DOUBLE ──> PACKED_ELEMENTS
//!        │                │                  │
//!        v                v                  v
//!   HOLEY_INT32  ──> HOLEY_DOUBLE  ──> HOLEY_ELEMENTS ──> DICTIONARY (ELEM2)
//! ```
//!
//! | Where | What |
//! |---|---|
//! | shape `counts` bits 28-31 (`shape::elements_kind`) | the kind of the shape's objects (H18-Q3); every JS object kind has one; a new object starts at PACKED_INT32 (0) with no elements cell |
//! | the +16 field of a JS object (`object::obj::ELEMENTS`) | undefined (no element yet), an ELEMENTS cell (Values: kinds 0, 1, 4, 5) or an ELEMENTS_DOUBLE cell (kinds 2, 3) |
//! | keyed slot value half, bits 10-16 (IC_LAYOUT 3, [`KINDS_SET_SHIFT`]) | the set of kinds seen at the site, one bit per value 0-6: every miss of an index key (`profile::record_keyed_cold`), a fast hit in the sampled window (`profile::keyed_hit_kind`; a monomorphic StoreKeyed hit needs none: its shape holds the kind); bits 0-9 keep their meaning (`profile::keyed`) |
//! | an ALLOC_SITE handler +16 (IC_LAYOUT 3, `ic::hk::ALLOC_SITE`) | the kind an array-literal site makes (MODES-D19, V10-D6; M5-P2 ELEM2) |
//!
//! What makes each kind (the store side, `object::array_set`, `object::element_set`):
//!
//! | Store | Kind after it |
//! |---|---|
//! | an int32 Value | the kind as it was (INT32 holds int32 Values only) |
//! | another number (a double Value: -0, NaN, a fraction, a large integer) | at least DOUBLE |
//! | any other Value | at least ELEMENTS |
//! | a hole (`delete a[i]`), an index past `length` + 0, a `length` that grows | the holey form |
//! | any element of an object that is not an Array | the holey form (no `length` says where the elements end) |
//!
//! Rules (H18; `tests::elements`):
//! - Transitions are one-way and happen on the store side only ([`transition`]): a load
//!   never changes a kind (MC-w15-21; V8 removed its load-side transitions). A shape change
//!   is the only change a kind transition makes to the object besides its elements cell.
//! - Every DOUBLE store makes a NaN canonical (`object::double_bits`: any NaN is written as
//!   0x7FF8_0000_0000_0000), so no store writes the hole pattern (V10-D7). A DOUBLE load
//!   boxes with `Value::number` (an integral value comes back as an int32, as it was stored).
//! - The storage form follows the kind: a transition between forms converts the elements
//!   cell in place of the old one (INT32 -> DOUBLE: each int32 becomes its f64; DOUBLE ->
//!   ELEMENTS: each f64 becomes its number Value; the hole stays the hole). No Value is
//!   boxed in the heap: a double Value is a NaN-box (H6).
//! - The kinds set of a keyed slot records the kind of the receiver before the access; the
//!   storage form, not the kind, decides how T0 reads an element (`object::element_at`
//!   dispatches on the cell kind), so kinds with one storage form are one case for it.
//! - The object model reads no execution policy (MODES-D19, plan v10 rule 40): `ics_only`
//!   keeps every kind, transition and elements cell; it drops only the `profile.rs` record
//!   hooks, so the kinds set of a keyed slot stays empty there.
//!
//! Index keys on objects that are not Arrays (H18-Q3, M5-P1): every JS object kind but Array
//! (its own paths), PRIMITIVE_WRAPPER (String wrappers keep their exotic indices) and the
//! routed kinds (ARGUMENTS keeps its own +16 elements; module namespaces and proxies have none)
//! keeps a new {w, e, c} data property with an array-index key in its +16 elements store
//! ([`plain_elements_kind`]), so such a key no longer moves a fast object to dictionary mode
//! (OM-10). An index key that the elements store cannot hold stays a stored property, as in
//! v0: an accessor, other attributes (`Object.freeze` moves each element there), an index
//! [`PLAIN_GAP`] or more past the capacity, or one at or past `object::arr::MAX_LENGTH`.
//! One index key lives in one place: the paths of `property.rs` read the elements first,
//! then the property storage, and add to the elements only when the storage has no such
//! key. An int32 key (`o[i] = v`, `property::set_index`, `define_index`) makes the elements
//! cell; a string index key (`{0: v}`, `Object.defineProperty(o, "0", d)`, a `[[Set]]` walk)
//! goes to the elements only when the object has a cell already, else it takes the key
//! path (OM-10, as in v0): the index test of a string key is a call, and every named define
//! would pay it (M5-P1 BENCH-SET, ci-v0). M5-P2 ELEM2's DICTIONARY kind takes the stored
//! index keys over.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::frame::{HOLE, int32_bits};
use crate::kinds as kinds_;
use crate::object::{self, delems, elems, obj};
use crate::runtime::Runtime;
use crate::shape;

/// The elements-kind values (OBJECT_LAYOUT 5; V10-A21). Append-only.
pub(crate) mod kind {
    pub const PACKED_INT32: u8 = 0;
    pub const HOLEY_INT32: u8 = 1;
    pub const PACKED_DOUBLE: u8 = 2;
    pub const HOLEY_DOUBLE: u8 = 3;
    pub const PACKED_ELEMENTS: u8 = 4;
    pub const HOLEY_ELEMENTS: u8 = 5;
    pub const DICTIONARY: u8 = 6;
    /// The values in use (0-6); 7-15 are reserved.
    pub const COUNT: u8 = 7;
    /// The largest value the 4 shape bits hold.
    pub const MAX: u8 = 15;
    /// Bit 0 of a value 0-5: the kind allows holes.
    pub const HOLEY_BIT: u8 = 1;
}

/// The storage form of an elements kind (MC-w15-21: one storage form is one IC case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Storage {
    /// Values in an ELEMENTS cell: PACKED/HOLEY INT32 and ELEMENTS.
    Values,
    /// Raw f64 in an ELEMENTS_DOUBLE cell: PACKED/HOLEY DOUBLE.
    Doubles,
    /// An ELEMENTS_DICT cell.
    Dictionary,
}

/// The storage form of kind `k` (0-6), or `None` for a reserved value.
pub(crate) const fn storage(k: u8) -> Option<Storage> {
    match k {
        kind::PACKED_INT32 | kind::HOLEY_INT32 | kind::PACKED_ELEMENTS | kind::HOLEY_ELEMENTS => {
            Some(Storage::Values)
        }
        kind::PACKED_DOUBLE | kind::HOLEY_DOUBLE => Some(Storage::Doubles),
        kind::DICTIONARY => Some(Storage::Dictionary),
        _ => None,
    }
}

/// The FactSnapshot elements kind (`ep9_facts::ElementsKind`, schema 1) of the VM kind `k`, or
/// `None` for a reserved value (7-15). The two numberings differ at 1-4 (VM: OBJECT_LAYOUT 5,
/// V10-A21; facts: holey kinds after the packed ones), so every reader converts through this
/// one map, never a cast (MODES-CHECK-w18 MC-w18-4; the month-6 schema bump either renumbers
/// `ElementsKind` to the VM order or keeps the map).
#[cfg_attr(not(test), allow(dead_code))] // first reader: M5-P2 ELEM2 (ALLOC_SITE facts)
pub(crate) const fn to_fact(k: u8) -> Option<ep9_facts::ElementsKind> {
    use ep9_facts::ElementsKind as F;
    match k {
        kind::PACKED_INT32 => Some(F::PackedInt32),
        kind::HOLEY_INT32 => Some(F::HoleyInt32),
        kind::PACKED_DOUBLE => Some(F::PackedDouble),
        kind::HOLEY_DOUBLE => Some(F::HoleyDouble),
        kind::PACKED_ELEMENTS => Some(F::PackedElements),
        kind::HOLEY_ELEMENTS => Some(F::HoleyElements),
        kind::DICTIONARY => Some(F::Dictionary),
        _ => None,
    }
}

/// The VM kind of the FactSnapshot elements kind `f`: the inverse of [`to_fact`].
#[cfg_attr(not(test), allow(dead_code))] // first reader: M5-P2 ELEM2
pub(crate) const fn from_fact(f: ep9_facts::ElementsKind) -> u8 {
    use ep9_facts::ElementsKind as F;
    match f {
        F::PackedInt32 => kind::PACKED_INT32,
        F::HoleyInt32 => kind::HOLEY_INT32,
        F::PackedDouble => kind::PACKED_DOUBLE,
        F::HoleyDouble => kind::HOLEY_DOUBLE,
        F::PackedElements => kind::PACKED_ELEMENTS,
        F::HoleyElements => kind::HOLEY_ELEMENTS,
        F::Dictionary => kind::DICTIONARY,
    }
}

/// True for the holey kinds (1, 3, 5).
#[cfg_attr(not(test), allow(dead_code))] // tests (layout5, elements)
pub(crate) const fn is_holey(k: u8) -> bool {
    k < kind::DICTIONARY && k & kind::HOLEY_BIT != 0
}

/// The holey form of kind `k` (0-5); DICTIONARY stays.
#[inline(always)]
pub(crate) const fn holey(k: u8) -> u8 {
    if k < kind::DICTIONARY {
        k | kind::HOLEY_BIT
    } else {
        k
    }
}

/// The least kind at or above both `a` and `b` in the lattice (0-6): the bases INT32 (0) <
/// DOUBLE (2) < ELEMENTS (4) < DICTIONARY (6) in value order, holey if either is.
#[inline(always)]
pub(crate) const fn join(a: u8, b: u8) -> u8 {
    if a >= kind::DICTIONARY || b >= kind::DICTIONARY {
        return kind::DICTIONARY;
    }
    let base = {
        let (x, y) = (a & !kind::HOLEY_BIT, b & !kind::HOLEY_BIT);
        if x > y { x } else { y }
    };
    base | ((a | b) & kind::HOLEY_BIT)
}

/// True if `to` is `from` or more general (a transition `from` -> `to` is allowed).
#[inline(always)]
pub(crate) const fn allows(from: u8, to: u8) -> bool {
    join(from, to) == to
}

/// The packed kind a store of the Value `v` (not the hole) needs at least: PACKED_INT32 for
/// an int32, PACKED_DOUBLE for another number, PACKED_ELEMENTS for anything else.
#[inline(always)]
pub(crate) fn value_kind(v: u64) -> u8 {
    let x = crate::runtime::val(v);
    if x.is_int32() {
        kind::PACKED_INT32
    } else if x.is_double() {
        kind::PACKED_DOUBLE
    } else {
        kind::PACKED_ELEMENTS
    }
}

/// IC_LAYOUT 3: the first bit of the kinds set in a keyed slot's value half (bits 10-16, one
/// bit per kind value 0-6; bits 0-9 keep their IC_LAYOUT 2 meaning, `profile::keyed`). M5-P1
/// ELEM1 writes it (`profile::record_keyed_cold`, `profile::keyed_hit_kind`).
pub(crate) const KINDS_SET_SHIFT: u32 = 10;

/// The kinds-set bit of kind `k` (0-6).
pub(crate) const fn kinds_set_bit(k: u8) -> u32 {
    1 << (KINDS_SET_SHIFT + k as u32)
}

/// The kinds-set bits of a keyed slot's value half (bits 10-16).
#[cfg_attr(not(test), allow(dead_code))] // tests (layout5, elements)
pub(crate) const KINDS_SET_MASK: u32 = ((1 << kind::COUNT) - 1) << KINDS_SET_SHIFT;

/// The elements kind of the JS object `o` (its shape's, H18-Q3). Not a GC point.
#[inline(always)]
pub(crate) fn kind_of(o: u64) -> u8 {
    shape::elements_kind(shape::shape_of(o))
}

/// The elements kind of the array `a` ([`kind_of`]). A value that is not a JS object gets
/// HOLEY_ELEMENTS, the most general dense kind (a reader assumes nothing of it).
#[inline(always)]
#[cfg_attr(not(test), allow(dead_code))] // tests (layout5, elements)
pub(crate) fn kind_of_array(a: u64) -> u8 {
    if kinds_::is_object(a) {
        kind_of(a)
    } else {
        kind::HOLEY_ELEMENTS
    }
}

/// The JS object kinds besides ARRAY that keep array-index data properties in their +16
/// elements store (H18-Q3; see the module docs): OBJECT, FUNCTION, ERROR, ITERATOR,
/// BOUND_FUNCTION, GENERATOR and SLOT_OBJECT. Not PRIMITIVE_WRAPPER and not the routed kinds
/// (ARGUMENTS, MODULE_NAMESPACE, PROXY, PROXY_CALLABLE). One compare per range.
#[inline(always)]
pub(crate) fn plain_elements_kind(k: u8) -> bool {
    const _: () = assert!(
        kinds_::OBJECT.get() == 16
            && kinds_::FUNCTION.get() == 17
            && kinds_::ARRAY.get() == 18
            && kinds_::ERROR.get() == 19
            && kinds_::ITERATOR.get() == 20
            && kinds_::PRIMITIVE_WRAPPER.get() == 21
            && kinds_::BOUND_FUNCTION.get() == 22
            && kinds_::GENERATOR.get() == 23
            && kinds_::SLOT_OBJECT.get() == 24
    );
    matches!(k, 16 | 17 | 19 | 20 | 22..=24)
}

/// The largest distance past the capacity at which a new element of an object that is not
/// an Array still goes to its elements store (H18-Q2's gap, est.); a farther index stays a
/// stored property, so `o[1e6] = x` allocates no dense range. Arrays keep the v0 rule (dense up
/// to `object::arr::MAX_LENGTH`) until DICTIONARY (M5-P2 ELEM2).
pub(crate) const PLAIN_GAP: u32 = 1024;

/// Moves the object `o` to elements kind `to` (a store needs it; `to` is more general than
/// its kind, [`allows`]): converts its elements cell when the storage form changes, then
/// installs the shape of kind `to` (`shape::with_elements_kind`). One-way; only stores call
/// it. GC point: `o` lives in a Rust local of the caller.
#[cold]
#[inline(never)]
pub(crate) fn transition(rt: &Runtime, o: u64, to: u8) {
    let from = kind_of(o);
    debug_assert!(
        from != to && allows(from, to),
        "elements kind {from} -> {to}"
    );
    debug_assert!(
        to < kind::DICTIONARY,
        "DICTIONARY elements are M5-P2 ELEM2's"
    );
    // GC point (the new cell): filled with raw or initializing stores before the next one.
    let e = object::rd(object::cell(o), obj::ELEMENTS);
    let converted = if e != crate::frame::UNDEFINED && storage(from) != storage(to) {
        Some(match storage(to) {
            Some(Storage::Doubles) => to_doubles(rt, object::cell(e)),
            _ => to_values(rt, object::cell(e)),
        })
    } else {
        None
    };
    // GC point: `o` and the new cell live in Rust locals (CSS).
    let ns = match shape::with_elements_kind(rt, shape::shape_of(o), to) {
        Some(s) => s,
        None => {
            // A transition limit on the way: dictionary mode, then its shape of kind `to`.
            crate::props::to_dictionary(rt, o);
            shape::with_elements_kind(rt, shape::shape_of(o), to)
                .expect("a dictionary shape takes any elements kind")
        }
    };
    if let Some(c) = converted {
        object::wr(
            &rt.heap,
            object::cell(o),
            obj::ELEMENTS,
            c.to_value().to_bits(),
        );
    }
    shape::install(rt, o, ns);
}

/// A new ELEMENTS_DOUBLE cell with the elements of the ELEMENTS cell `e` (int32 Values and
/// holes only: an INT32 kind). GC point (the allocation).
fn to_doubles(rt: &Runtime, e: CellPtr) -> CellPtr {
    let cap = object::rd_i32(e, elems::CAPACITY) as usize;
    let d = new_double_cell(rt, cap);
    for i in 0..cap {
        let v = object::rd(e, elems::DATA + 8 * i);
        let bits = if v == HOLE {
            delems::HOLE
        } else {
            debug_assert!(
                crate::runtime::val(v).is_int32(),
                "an INT32 kind holds {v:#x}"
            );
            object::double_bits(v)
        };
        object::raw_set(d, delems::DATA + 8 * i, bits);
    }
    d
}

/// A new ELEMENTS cell with the elements of the ELEMENTS_DOUBLE cell `d`, each as its number
/// Value (the hole stays the hole). GC point (the allocation); no Value is boxed.
fn to_values(rt: &Runtime, d: CellPtr) -> CellPtr {
    let cap = object::rd_i32(d, delems::CAPACITY) as usize;
    let c = rt.alloc(kinds_::ELEMENTS, elems::DATA + 8 * cap);
    object::init(c, elems::CAPACITY, int32_bits(cap as i32));
    for i in 0..cap {
        let bits = object::raw_get(d, delems::DATA + 8 * i);
        let v = if bits == delems::HOLE {
            HOLE
        } else {
            object::double_value(bits)
        };
        object::init(c, elems::DATA + 8 * i, v);
    }
    c
}

/// A new ELEMENTS_DOUBLE cell of capacity `cap`, every element a hole. GC point.
pub(crate) fn new_double_cell(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kinds_::ELEMENTS_DOUBLE, delems::DATA + 8 * cap);
    object::init(c, delems::CAPACITY, int32_bits(cap as i32));
    for i in 0..cap {
        object::raw_set(c, delems::DATA + 8 * i, delems::HOLE);
    }
    c
}

fn double_cell_size(c: CellPtr) -> usize {
    delems::DATA + 8 * object::rd_i32(c, delems::CAPACITY) as usize
}

/// The capacity word is an int32 Value and the data words are raw f64 bits (V9-D39, H21 P1):
/// the GC visits none of them.
fn trace_double_cell(_c: CellPtr, _t: &mut Tracer<'_>) {}

/// Registers ELEMENTS_DOUBLE (kind 81, OBJECT_LAYOUT 5). `object::register_kinds` calls it.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kinds_::ELEMENTS_DOUBLE,
        CellKind {
            name: "elements-double",
            size: CellSize::Dynamic(double_cell_size),
            trace: trace_double_cell,
        },
    )
    .expect("register ELEMENTS_DOUBLE");
}

/// Checks the elements invariants of the JS object `o` (tests): the cell's storage form is
/// its kind's, a packed array has no hole below `length`, an INT32 kind holds int32 Values
/// only, no DOUBLE word is a NaN other than the canonical one or the hole, and an object that
/// is not an Array with elements has a holey kind.
#[cfg(test)]
pub(crate) fn check(o: u64) -> Result<(), String> {
    let k = kind_of(o);
    let e = object::rd(object::cell(o), obj::ELEMENTS);
    if k >= kind::DICTIONARY {
        return Err(format!("kind {k} before M5-P2"));
    }
    let array = kinds_::is_array(o);
    if !array && kinds_::cell_kind(o) == kinds_::ARGUMENTS.get() {
        return Ok(());
    }
    if e == crate::frame::UNDEFINED {
        return if array || k == kind::PACKED_INT32 || is_holey(k) {
            Ok(())
        } else {
            Err(format!("kind {k} on an object with no elements"))
        };
    }
    if !array && !is_holey(k) {
        return Err(format!(
            "a packed kind {k} on an object that is not an Array"
        ));
    }
    let c = object::cell(e);
    let ck = kinds_::cell_kind(e);
    let want = match storage(k) {
        Some(Storage::Doubles) => kinds_::ELEMENTS_DOUBLE.get(),
        _ => kinds_::ELEMENTS.get(),
    };
    if ck != want {
        return Err(format!("kind {k} with a cell of kind {ck}"));
    }
    let cap = crate::frame::int32_of(object::rd(c, elems::CAPACITY)) as u32;
    let len = if array { object::array_len(o) } else { cap };
    for i in 0..cap {
        let present = object::element_get(o, i).is_some();
        if !is_holey(k) && i < len && !present {
            return Err(format!("a hole at {i} in packed kind {k}"));
        }
        if array && i >= len && present {
            return Err(format!("an element at {i} past length {len}"));
        }
        if ck == kinds_::ELEMENTS_DOUBLE.get() {
            let b = object::raw_get(c, delems::DATA + 8 * i as usize);
            if f64::from_bits(b).is_nan() && b != delems::HOLE && b != delems::CANONICAL_NAN {
                return Err(format!("the NaN {b:#x} at {i}"));
            }
        } else if k <= kind::HOLEY_INT32
            && let Some(v) = object::element_get(o, i)
            && !crate::runtime::val(v).is_int32()
        {
            return Err(format!("{v:#x} at {i} in an INT32 kind"));
        }
    }
    Ok(())
}

const _: () = {
    assert!(kind::COUNT as u32 + KINDS_SET_SHIFT <= 31);
    assert!(kind::DICTIONARY < kind::COUNT && kind::COUNT <= kind::MAX);
    assert!(join(kind::PACKED_INT32, kind::PACKED_DOUBLE) == kind::PACKED_DOUBLE);
    assert!(join(kind::HOLEY_INT32, kind::PACKED_DOUBLE) == kind::HOLEY_DOUBLE);
    assert!(join(kind::PACKED_ELEMENTS, kind::HOLEY_DOUBLE) == kind::HOLEY_ELEMENTS);
    assert!(holey(kind::PACKED_ELEMENTS) == kind::HOLEY_ELEMENTS);
};
