//! Own named-property storage of a JS object (H17 v0): raw reads and stores that ignore
//! attributes. The internal methods (`property.rs`) check attributes and extensibility on
//! top of these.
//!
//! ```text
//!  fast mode:        shape descriptors (key, attrs, field i)    field i < capacity: in-object slot i
//!                                                               field i >= capacity: SLOTS[i - capacity] (+8)
//!  dictionary mode:  +8 PROPS table: key, value, attrs, in insertion order (shape flag DICTIONARY)
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | PROPS | 64 | +8 capacity, +16 count (int32s), +24 entries: key, value, attributes | 24 + 24 cap |
//! | SLOTS | 71 | +8 capacity, +16 values (`shape.rs` owns the kind) | 16 + 8 cap |
//!
//! Fast shapes only grow by appending a property and shrink by removing the last one (back
//! to the parent shape). Everything else moves the object to dictionary mode (OM-10,
//! [`to_dictionary`]): a delete that is not the last property, an attribute change (data and
//! accessor included), an array-index key on a non-array, `[[SetPrototypeOf]]` with own
//! properties (`object::set_proto`), PreventExtensions, and the shape limits (1020
//! properties, 64 transitions). Stored keys are atoms or symbols (`key::intern_key`); a
//! lookup classifies its probe once (`key::probe`).

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::exotic;
use crate::frame::{UNDEFINED, int32_bits};
use crate::key::{self, Probe};
use crate::kinds as kind;
use crate::object::{self, cell, init, obj, rd, rd_i32, trace_from_8, wr};
use crate::runtime::{Atom, Runtime};
use crate::shape::{self, Added, slf};

mod propf {
    pub const CAPACITY: usize = 8;
    pub const COUNT: usize = 16;
    pub const ENTRIES: usize = 24;
    pub const ENTRY: usize = 24;
}

const MIN_TABLE: usize = 4;
const MIN_SLOTS: usize = 4;

fn props_size(c: CellPtr) -> usize {
    propf::ENTRIES + propf::ENTRY * rd_i32(c, propf::CAPACITY) as usize
}

fn trace_props(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, props_size(c));
}

/// Registers the PROPS kind.
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::PROPS,
        CellKind {
            name: "property-table",
            size: CellSize::Dynamic(props_size),
            trace: trace_props,
        },
    )
    .expect("register a VM cell kind");
}

// ---- the dictionary table ---------------------------------------------------------------------

/// The PROPS table of `o` (dictionary mode), if it has one.
fn table_of(o: u64) -> Option<CellPtr> {
    let p = rd(cell(o), obj::SLOTS);
    (p != UNDEFINED).then(|| cell(p))
}

fn props_count(p: CellPtr) -> usize {
    rd_i32(p, propf::COUNT) as usize
}

fn props_capacity(p: CellPtr) -> usize {
    rd_i32(p, propf::CAPACITY) as usize
}

#[inline]
fn entry(i: usize) -> usize {
    propf::ENTRIES + propf::ENTRY * i
}

pub(crate) fn new_props(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kind::PROPS, propf::ENTRIES + propf::ENTRY * cap);
    init(c, propf::CAPACITY, int32_bits(cap as i32));
    init(c, propf::COUNT, int32_bits(0));
    c
}

/// Index of `key` in the PROPS table of `o` (dictionary mode).
fn dict_find(o: u64, key: u64, how: Probe) -> Option<(CellPtr, usize)> {
    let p = table_of(o)?;
    let n = props_count(p);
    let i = match how {
        Probe::Bits => (0..n).find(|&i| rd(p, entry(i)) == key),
        Probe::Content => (0..n).find(|&i| key::key_eq(rd(p, entry(i)), key)),
    }?;
    Some((p, i))
}

/// Appends an entry to the PROPS table of `o` (dictionary mode; `key` is not in it). GC
/// point.
fn dict_add(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    let old = table_of(o);
    let (count, cap) = old.map_or((0, 0), |p| (props_count(p), props_capacity(p)));
    let p = if count < cap {
        old.expect("a table with room")
    } else {
        // GC point: `o`, `key`, `v` and `old` live in Rust locals (conservative scan, H10).
        let n = new_props(rt, (cap * 2).max(MIN_TABLE));
        if let Some(old) = old {
            for j in 0..count * 3 {
                init(n, propf::ENTRIES + 8 * j, rd(old, propf::ENTRIES + 8 * j));
            }
            init(n, propf::COUNT, int32_bits(count as i32));
        }
        wr(&rt.heap, cell(o), obj::SLOTS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, p, entry(count), key);
    wr(&rt.heap, p, entry(count) + 8, v);
    wr(&rt.heap, p, entry(count) + 16, int32_bits(attrs));
    wr(&rt.heap, p, propf::COUNT, int32_bits(count as i32 + 1));
}

/// Removes entry `i` of the table `p` (the tail shifts down: insertion order stays).
fn dict_remove(rt: &Runtime, p: CellPtr, i: usize) {
    let count = props_count(p);
    for j in i * 3..(count - 1) * 3 {
        wr(
            &rt.heap,
            p,
            propf::ENTRIES + 8 * j,
            rd(p, propf::ENTRIES + 8 * (j + 3)),
        );
    }
    for k in 0..3 {
        // E1: undefined is not a cell.
        init(p, entry(count - 1) + 8 * k, UNDEFINED);
    }
    init(p, propf::COUNT, int32_bits(count as i32 - 1));
}

// ---- fast-mode fields ---------------------------------------------------------------------------

/// Where field `f` of `o` (fast shape `s`) lives: (cell, byte offset).
#[inline(always)]
fn field_ref(o: u64, s: u64, f: usize) -> (CellPtr, usize) {
    let c = cell(o);
    let (cap, k) = shape::field_layout(s);
    if f < cap {
        let fixed = object::fixed_count(k);
        (c, obj::FIXED + 8 * (fixed + f))
    } else {
        let sl = cell(rd(c, obj::SLOTS));
        debug_assert!(f - cap < rd_i32(sl, slf::CAPACITY) as usize);
        (sl, slf::DATA + 8 * (f - cap))
    }
}

#[inline]
fn field_get(o: u64, s: u64, f: usize) -> u64 {
    let (c, off) = field_ref(o, s, f);
    rd(c, off)
}

#[inline]
fn field_set(rt: &Runtime, o: u64, s: u64, f: usize, v: u64) {
    let (c, off) = field_ref(o, s, f);
    wr(&rt.heap, c, off, v);
}

/// Makes room for field `f` of `o` under the fast shape `s` (a new SLOTS cell if the
/// out-of-line store is too small). GC point.
fn reserve_field(rt: &Runtime, o: u64, s: u64, f: usize) {
    let cap = usize::from(shape::capacity(s));
    if f < cap {
        return;
    }
    let need = f - cap + 1;
    let old = rd(cell(o), obj::SLOTS);
    let have = if old == UNDEFINED {
        0
    } else {
        rd_i32(cell(old), slf::CAPACITY) as usize
    };
    if need <= have {
        return;
    }
    let n = need.max(2 * have).max(MIN_SLOTS);
    // GC point: `o` and `old` live in Rust locals.
    let c = rt.alloc(kind::SLOTS, slf::DATA + 8 * n);
    init(c, slf::CAPACITY, int32_bits(n as i32));
    for i in 0..have {
        init(c, slf::DATA + 8 * i, rd(cell(old), slf::DATA + 8 * i));
    }
    wr(&rt.heap, cell(o), obj::SLOTS, c.to_value().to_bits());
}

/// Stores undefined in field `f` (a removed property), so the GC does not keep its value.
fn field_clear(o: u64, s: u64, f: usize) {
    let (c, off) = field_ref(o, s, f);
    // E1: undefined is not a cell.
    init(c, off, UNDEFINED);
}

// ---- lookups ------------------------------------------------------------------------------------

/// Where an own property was found.
#[derive(Clone, Copy, Debug)]
enum Hit {
    /// Fast mode: the shape and the descriptor (field) index.
    Fast(u64, usize),
    /// Dictionary mode: the table and the entry index.
    Dict(CellPtr, usize),
}

/// The own property `key` of `o`, with its probe classified once: where it is, its value and
/// its attributes. Not a GC point.
#[inline]
fn find_own(o: u64, key: u64, how: Probe) -> Option<(Hit, u64, i32)> {
    find_own_in(o, shape::shape_of(o), key, how)
}

/// [`find_own`] with the shape `s` of `o` already loaded.
#[inline(always)]
fn find_own_in(o: u64, s: u64, key: u64, how: Probe) -> Option<(Hit, u64, i32)> {
    if shape::is_dictionary(s) {
        let (p, i) = dict_find(o, key, how)?;
        return Some((
            Hit::Dict(p, i),
            rd(p, entry(i) + 8),
            rd_i32(p, entry(i) + 16),
        ));
    }
    let f = shape::lookup_with(s, key, how)?;
    let v = field_get(o, s, f.index);
    Some((Hit::Fast(s, f.index), v, f.details.attrs()))
}

/// An own property of `o` with a probe the caller classified: (value, attributes). Not a GC
/// point.
#[inline]
pub(crate) fn own_get_with(o: u64, key: u64, how: Probe) -> Option<(u64, i32)> {
    find_own(o, key, how).map(|(_, v, a)| (v, a))
}

/// One step of a `[[Get]]` walk: the value of the own property `key` of `o`, or else the
/// `[[Prototype]]` of `o` (an object or null), from one shape load. Not a GC point.
#[inline]
pub(crate) fn own_value_or_proto(o: u64, key: u64, how: Probe) -> Result<u64, u64> {
    let s = shape::shape_of(o);
    match find_own_in(o, s, key, how) {
        Some((_, v, _)) => Ok(v),
        None => Err(shape::proto(s)),
    }
}

/// An own named property: (value, attributes). Not a GC point.
pub(crate) fn own_get(o: u64, key: u64) -> Option<(u64, i32)> {
    own_get_with(o, key, key::probe(key))
}

/// Writes an existing own property's value (attributes are not checked). False if `o` has
/// no such property. Not a GC point.
pub(crate) fn own_replace(rt: &Runtime, o: u64, key: u64, v: u64) -> bool {
    own_replace_with(rt, o, key, key::probe(key), v)
}

/// `o.key = v` on an own data property in one lookup: `None` if `o` has no own `key`,
/// `Some(false)` if it is not writable (nothing stored), `Some(true)` once stored. Not a GC
/// point.
#[inline]
pub(crate) fn own_store_if_writable(
    rt: &Runtime,
    o: u64,
    key: u64,
    how: Probe,
    v: u64,
) -> Option<bool> {
    let (hit, _, attrs) = find_own(o, key, how)?;
    if attrs & object::attr::WRITABLE == 0 {
        return Some(false);
    }
    match hit {
        Hit::Fast(s, f) => field_set(rt, o, s, f, v),
        Hit::Dict(p, i) => wr(&rt.heap, p, entry(i) + 8, v),
    }
    Some(true)
}

/// [`own_replace`] with a classified probe.
pub(crate) fn own_replace_with(rt: &Runtime, o: u64, key: u64, how: Probe, v: u64) -> bool {
    match find_own(o, key, how) {
        Some((Hit::Fast(s, f), _, _)) => {
            field_set(rt, o, s, f, v);
            true
        }
        Some((Hit::Dict(p, i), _, _)) => {
            wr(&rt.heap, p, entry(i) + 8, v);
            true
        }
        None => false,
    }
}

// ---- defines and deletes ------------------------------------------------------------------------

/// Defines or overwrites an own property with `attrs` (`attr` bits; ACCESSOR for an accessor
/// pair). A raw store: it checks neither `[[Extensible]]` nor the current attributes. The key
/// is stored as an atom. A new key appends (insertion order); new attributes on an existing
/// key move a fast object to dictionary mode (OM-10). GC point.
pub(crate) fn own_define(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    debug_assert!(object::is_object(o));
    let key = key::intern_key(rt, key);
    let how = key::probe(key);
    match find_own(o, key, how) {
        Some((hit, _, old)) if old == attrs => match hit {
            Hit::Fast(s, f) => field_set(rt, o, s, f, v),
            Hit::Dict(p, i) => wr(&rt.heap, p, entry(i) + 8, v),
        },
        Some((hit, _, _)) => {
            if let Hit::Fast(..) = hit {
                to_dictionary(rt, o);
            }
            let (p, i) = dict_find(o, key, how).expect("the key after dictionary mode");
            wr(&rt.heap, p, entry(i) + 8, v);
            init(p, entry(i) + 16, int32_bits(attrs));
            shape::note_change(rt, o);
        }
        None => own_add(rt, o, key, v, attrs),
    }
}

/// Adds the new own property `key` (an atom or a symbol that `o` does not have): a raw
/// store, like [`own_define`] without the lookup. GC point.
pub(crate) fn own_add(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    debug_assert!(
        !crate::kinds::is_string(key) || crate::string::is_atom(key),
        "a stored key is an atom"
    );
    debug_assert!(own_get(o, key).is_none(), "own_add of a key the object has");
    if kind::is_function(o) && key != rt.atom(Atom::Prototype) {
        // A lazy `prototype` is older than `key`: make it now so it keeps its place in
        // `[[OwnPropertyKeys]]` (`exotic::lazy_prototype_key`). GC point: `o`, `key` and `v`
        // live in Rust locals.
        if exotic::lazy_prototype_key(rt, o).is_some() {
            exotic::function_prototype(rt, o);
        }
    }
    let s = shape::shape_of(o);
    if !shape::is_dictionary(s) {
        // GC points: `o`, `key`, `v` and `child` live in Rust locals (CSS); `child` is also
        // a (weak) transition of `s`, or a unique shape nothing else holds yet.
        match shape::add(rt, s, key, attrs) {
            Added::Shape(child) => {
                let f = shape::count(child) - 1;
                reserve_field(rt, o, child, f);
                field_set(rt, o, child, f, v);
                shape::install(rt, o, child);
                shape::note_change(rt, o);
                return;
            }
            Added::Dictionary => to_dictionary(rt, o),
        }
    }
    dict_add(rt, o, key, v, attrs);
    shape::note_change(rt, o);
}

/// Removes an own property. Returns its attributes, or `None` if it was absent. Deleting the
/// last-added property of a fast object goes back to the parent shape; any other delete
/// moves it to dictionary mode first (OM-10). GC point.
pub(crate) fn own_delete(rt: &Runtime, o: u64, key: u64) -> Option<i32> {
    let how = key::probe(key);
    let (hit, _, attrs) = find_own(o, key, how)?;
    match hit {
        Hit::Fast(s, f) if f + 1 == shape::count(s) => {
            let unique = shape::flags(s) & shape::flag::UNIQUE != 0;
            // GC point (a unique shape is new): `o` and `s` live in Rust locals.
            let parent = if unique {
                shape::remove_last_unique(rt, s)
            } else {
                shape::remove_last(s)
            };
            field_clear(o, s, f);
            shape::install(rt, o, parent);
        }
        Hit::Fast(..) => {
            to_dictionary(rt, o);
            let (p, i) = dict_find(o, key, how).expect("the key after dictionary mode");
            dict_remove(rt, p, i);
        }
        Hit::Dict(p, i) => dict_remove(rt, p, i),
    }
    shape::note_change(rt, o);
    Some(attrs)
}

/// Moves `o` to dictionary mode (OM-10): its properties go into a PROPS table in property
/// order, and it gets the dictionary shape of its prototype (shared), or a new unique one in
/// prototype mode. No change for an object that is in dictionary mode already. GC point.
pub(crate) fn to_dictionary(rt: &Runtime, o: u64) {
    let s = shape::shape_of(o);
    if shape::is_dictionary(s) {
        return;
    }
    // GC points first: the shape, then the table (filled with initializing stores, E2).
    let ds = shape::dictionary_shape_of(rt, s, true);
    let n = shape::count(s);
    let p = new_props(rt, (n + 1).next_power_of_two().max(MIN_TABLE));
    for i in 0..n {
        let d = shape::details_at(s, i);
        init(p, entry(i), shape::key_at(s, i));
        init(p, entry(i) + 8, field_get(o, s, usize::from(d.field())));
        init(p, entry(i) + 16, int32_bits(d.attrs()));
    }
    init(p, propf::COUNT, int32_bits(n as i32));
    // The in-object slots are unused in dictionary mode (E1: undefined is not a cell).
    let c = cell(o);
    let first = obj::FIXED + 8 * object::fixed_count(object::cell_kind(o));
    for i in 0..object::inobject_count(c) {
        init(c, first + 8 * i, UNDEFINED);
    }
    wr(&rt.heap, c, obj::SLOTS, p.to_value().to_bits());
    shape::install(rt, o, ds);
    shape::note_change(rt, o);
}

/// Makes `o` non-extensible: dictionary mode (OM-10) with a non-extensible dictionary shape.
/// GC point.
pub(crate) fn prevent_extensions(rt: &Runtime, o: u64) {
    to_dictionary(rt, o);
    // GC point: `o` lives in a Rust local.
    let ds = shape::dictionary_shape_of(rt, shape::shape_of(o), false);
    shape::install(rt, o, ds);
    shape::note_change(rt, o);
}

// ---- enumeration --------------------------------------------------------------------------------

/// Number of own named properties (strings and symbols).
pub(crate) fn own_count(o: u64) -> usize {
    let s = shape::shape_of(o);
    if shape::is_dictionary(s) {
        table_of(o).map_or(0, props_count)
    } else {
        shape::count(s)
    }
}

/// Own named property `i` in insertion order: (key, value, attributes).
pub(crate) fn own_entry(o: u64, i: usize) -> (u64, u64, i32) {
    let s = shape::shape_of(o);
    if shape::is_dictionary(s) {
        let p = table_of(o).expect("own_entry on an object without properties");
        return (
            rd(p, entry(i)),
            rd(p, entry(i) + 8),
            rd_i32(p, entry(i) + 16),
        );
    }
    let d = shape::details_at(s, i);
    (
        shape::key_at(s, i),
        field_get(o, s, usize::from(d.field())),
        d.attrs(),
    )
}
