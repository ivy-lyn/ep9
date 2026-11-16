//! Own named-property storage of a JS object (H17 v0): raw reads and stores that ignore
//! attributes. The internal methods (`property.rs`) check attributes and extensibility on
//! top of these. An accessor property (`attr::ACCESSOR`) stores its ACCESSOR_PAIR
//! (`accessor.rs`) as the value: a raw read returns the pair, never a JS value.
//!
//! ```text
//!  fast mode:        shape descriptors (key, attrs, field i)    field i < capacity: in-object slot i
//!                                                               field i >= capacity: SLOTS[i - capacity] (+8)
//!  dictionary mode:  +8 PROPS table (PROPS_INDEXED above 8): key, value, attrs, in insertion order (shape flag DICTIONARY)
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | PROPS | 64 | +8 capacity, +16 count (int32s), +24 entries: key, value, attributes | 24 + 24 cap |
//! | SLOTS | 71 | +8 capacity, +16 values (`shape.rs` owns the kind) | 16 + 8 cap |
//! | PROPS_INDEXED | 84 | +8 capacity, +16 count (int32s), +24 index, +32 entries as PROPS (Dictionary v1, `props_index.rs`) | 32 + 24 cap |
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
use crate::props_index;
use crate::runtime::{Atom, Runtime};
use crate::shape::{self, Added, LINEAR_MAX, slf};

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

/// Registers the PROPS kind, and PROPS_INDEXED (OBJECT_LAYOUT 6, `props_index.rs`).
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
    crate::props_index::register_kinds(heap);
}

// ---- the dictionary table ---------------------------------------------------------------------

/// The table of `o` (dictionary mode: a PROPS or PROPS_INDEXED cell), if it has one. Both
/// kinds keep capacity at +8 and the live count at +16.
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

/// Byte offset of entry `i` of a PROPS table.
#[inline]
fn entry(i: usize) -> usize {
    propf::ENTRIES + propf::ENTRY * i
}

/// Byte offset of entry `i` of the table `p` (PROPS or PROPS_INDEXED).
#[inline]
fn entry_in(p: CellPtr, i: usize) -> usize {
    if props_index::is_indexed(p) {
        props_index::entry(i)
    } else {
        entry(i)
    }
}

pub(crate) fn new_props(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kind::PROPS, propf::ENTRIES + propf::ENTRY * cap);
    init(c, propf::CAPACITY, int32_bits(cap as i32));
    init(c, propf::COUNT, int32_bits(0));
    c
}

/// Where `key` is in the table of `o` (dictionary mode): the table and the byte offset of the
/// entry. A PROPS_INDEXED table answers from its index (`props_index::find`); a PROPS table
/// is scanned. Not a GC point.
fn dict_find(o: u64, key: u64, how: Probe) -> Option<(CellPtr, usize)> {
    let p = table_of(o)?;
    if let Some(hit) = props_index::find(p, key, how) {
        return hit.map(|i| (p, props_index::entry(i)));
    }
    let n = props_count(p);
    let i = match how {
        Probe::Bits => (0..n).find(|&i| rd(p, entry(i)) == key),
        Probe::Content => (0..n).find(|&i| key::key_eq(rd(p, entry(i)), key)),
    }?;
    Some((p, entry(i)))
}

/// Appends an entry to the table of `o` (dictionary mode; `key` is not in it). The entry
/// after the 8th moves a PROPS table to PROPS_INDEXED (Dictionary v1; never back). GC point.
fn dict_add(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    let old = table_of(o);
    if let Some(p) = old.filter(|&p| props_index::is_indexed(p)) {
        props_index::add(rt, o, p, key, v, attrs);
        return;
    }
    let (count, cap) = old.map_or((0, 0), |p| (props_count(p), props_capacity(p)));
    if count + 1 > LINEAR_MAX {
        let p = to_indexed(rt, o, old.expect("a table with 8 entries"));
        props_index::add(rt, o, p, key, v, attrs);
        return;
    }
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

/// Moves the full PROPS table `old` of `o` (8 entries) to a new PROPS_INDEXED table and
/// returns it. GC point: `o` and `old` live in Rust locals.
fn to_indexed(rt: &Runtime, o: u64, old: CellPtr) -> CellPtr {
    let count = props_count(old);
    let n = props_index::alloc(rt, props_index::capacity_for(count));
    for i in 0..count {
        for k in 0..3 {
            // E2: `n` is new and no GC point ran since its allocation.
            init(n, props_index::entry(i) + 8 * k, rd(old, entry(i) + 8 * k));
        }
    }
    props_index::seal(n, count);
    wr(&rt.heap, cell(o), obj::SLOTS, n.to_value().to_bits());
    n
}

/// Removes the entry at byte offset `off` of the table `p`: a PROPS table shifts its tail
/// down (insertion order stays); a PROPS_INDEXED table leaves a hole (`props_index::remove`).
fn dict_remove(rt: &Runtime, p: CellPtr, off: usize) {
    if props_index::is_indexed(p) {
        props_index::remove(rt, p, (off - props_index::entry(0)) / propf::ENTRY);
        return;
    }
    let i = (off - entry(0)) / propf::ENTRY;
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

/// A second store of `v` into field `f` of `o` (fast shape `s`): widens the field's
/// representation and clears its const bit first (`field_rep::before_field_store`, M5-P2
/// CONSTREP). Not a GC point.
#[inline(always)]
fn field_set(rt: &Runtime, o: u64, s: u64, f: usize, v: u64) {
    crate::field_rep::before_field_store(rt, s, f, v);
    let (c, off) = field_ref(o, s, f);
    wr(&rt.heap, c, off, v);
}

/// The first store of `v` into the new field `f` of `o`, under the shape `s` that the add
/// gave (`own_add`): the const bit stays; a transition that other objects took widens to `v`
/// (`field_rep::on_init_store`, M5-P2 CONSTREP). Not a GC point.
#[inline]
fn field_init(rt: &Runtime, o: u64, s: u64, f: usize, v: u64) {
    crate::field_rep::on_init_store(rt, s, f, v);
    let (c, off) = field_ref(o, s, f);
    wr(&rt.heap, c, off, v);
}

/// Makes room for field `f` of `o` under the fast shape `s` (a new SLOTS cell if the
/// out-of-line store is too small; also the add with growth of `ic::grow_add`). GC point.
///
/// The front only: an in-object field (`f` below the capacity of `s`) returns with no call.
/// It has two callers (`own_add` and `ic::grow_add`), so it is `#[inline(always)]`: as an
/// out-of-line function, each add paid a call and 6 register saves on this early return
/// (M8-P0 bench mode: t0-gen +1.9% Ir). The rest is [`reserve_field_slots`].
#[inline(always)]
pub(crate) fn reserve_field(rt: &Runtime, o: u64, s: u64, f: usize) {
    let cap = usize::from(shape::capacity(s));
    if f < cap {
        return;
    }
    reserve_field_slots(rt, o, f - cap + 1);
}

/// The out-of-line part of [`reserve_field`]: makes the SLOTS cell of `o` hold at least `need`
/// fields (a new cell if it is too small). GC point.
#[cold]
#[inline(never)]
fn reserve_field_slots(rt: &Runtime, o: u64, need: usize) {
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
    /// Dictionary mode: the table and the byte offset of the entry.
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
        let (p, e) = dict_find(o, key, how)?;
        return Some((Hit::Dict(p, e), rd(p, e + 8), rd_i32(p, e + 16)));
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

/// One step of a `[[Get]]` walk: (value, attributes) of the own property `key` of `o`, or
/// else the `[[Prototype]]` of `o` (an object or null), from one shape load. For an accessor
/// (`attr::ACCESSOR`) the value is its ACCESSOR_PAIR (`accessor.rs`). Not a GC point. Always
/// inlined: the `[[Get]]` walk is hot until the ICs.
#[inline(always)]
pub(crate) fn own_value_or_proto(o: u64, key: u64, how: Probe) -> Result<(u64, i32), u64> {
    let s = shape::shape_of(o);
    match find_own_in(o, s, key, how) {
        Some((_, v, a)) => Ok((v, a)),
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
/// `Some(false)` if it is not writable (nothing stored; an accessor has no `WRITABLE` bit, so
/// it gives `Some(false)` too), `Some(true)` once stored. Not a GC point. Always inlined: the
/// `[[Set]]` walk is hot until the ICs.
#[inline(always)]
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
        Hit::Dict(p, e) => wr(&rt.heap, p, e + 8, v),
    }
    Some(true)
}

/// [`own_replace`] with a classified probe. The property must be a data property (an
/// accessor's pair is replaced through [`own_define`]).
pub(crate) fn own_replace_with(rt: &Runtime, o: u64, key: u64, how: Probe, v: u64) -> bool {
    let found = find_own(o, key, how);
    debug_assert!(
        found.is_none_or(|(_, _, a)| a & object::attr::ACCESSOR == 0),
        "own_replace of an accessor"
    );
    match found {
        Some((Hit::Fast(s, f), _, _)) => {
            field_set(rt, o, s, f, v);
            true
        }
        Some((Hit::Dict(p, e), _, _)) => {
            wr(&rt.heap, p, e + 8, v);
            true
        }
        None => false,
    }
}

// ---- defines and deletes ------------------------------------------------------------------------

/// Defines or overwrites an own property with `attrs` (`attr` bits; ACCESSOR for an accessor
/// pair). A raw store: it checks neither `[[Extensible]]` nor the current attributes. The key
/// is stored as an atom. A new key appends (insertion order); new attributes on an existing
/// key move a fast object to dictionary mode (OM-10). A new pair in place of an accessor's
/// old one is a structural change (`accessor.rs`): it bumps the epoch of a prototype. GC
/// point.
pub(crate) fn own_define(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    debug_assert!(object::is_object(o));
    let key = key::intern_key(rt, key);
    let how = key::probe(key);
    match find_own(o, key, how) {
        Some((hit, old_v, old)) if old == attrs => {
            match hit {
                Hit::Fast(s, f) => field_set(rt, o, s, f, v),
                Hit::Dict(p, e) => wr(&rt.heap, p, e + 8, v),
            }
            if attrs & object::attr::ACCESSOR != 0 && old_v != v {
                shape::note_change(rt, o);
            }
        }
        Some((hit, _, _)) => {
            if let Hit::Fast(..) = hit {
                to_dictionary(rt, o);
            }
            let (p, e) = dict_find(o, key, how).expect("the key after dictionary mode");
            wr(&rt.heap, p, e + 8, v);
            init(p, e + 16, int32_bits(attrs));
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
    if shape::is_prototype_object(o) && key::array_index_of(key).is_some() {
        shape::note_proto_index_key(rt, o);
    }
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
        match shape::add_with(rt, s, key, attrs, crate::field_rep::initial(v)) {
            Added::Shape(child) => {
                let f = shape::count(child) - 1;
                reserve_field(rt, o, child, f);
                field_init(rt, o, child, f, v);
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
            let (p, e) = dict_find(o, key, how).expect("the key after dictionary mode");
            dict_remove(rt, p, e);
        }
        Hit::Dict(p, e) => dict_remove(rt, p, e),
    }
    shape::note_change(rt, o);
    Some(attrs)
}

/// Moves `o` to dictionary mode (OM-10): its properties go into a table in property order (a
/// PROPS table, or PROPS_INDEXED above 8 properties), and it gets the dictionary shape of its
/// prototype (shared), or a new unique one in prototype mode. No change for an object that is
/// in dictionary mode already. GC point.
pub(crate) fn to_dictionary(rt: &Runtime, o: u64) {
    let s = shape::shape_of(o);
    if shape::is_dictionary(s) {
        return;
    }
    // GC points first: the shape, then the table (filled with initializing stores, E2).
    let ds = shape::dictionary_shape_of(rt, s, true);
    let n = shape::count(s);
    let indexed = n > LINEAR_MAX;
    let p = if indexed {
        props_index::alloc(rt, props_index::capacity_for(n))
    } else {
        new_props(rt, (n + 1).next_power_of_two().max(MIN_TABLE))
    };
    for i in 0..n {
        let d = shape::details_at(s, i);
        let e = entry_in(p, i);
        init(p, e, shape::key_at(s, i));
        init(p, e + 8, field_get(o, s, usize::from(d.field())));
        init(p, e + 16, int32_bits(d.attrs()));
    }
    if indexed {
        props_index::seal(p, n);
    } else {
        init(p, propf::COUNT, int32_bits(n as i32));
    }
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

/// Own named property `i` in insertion order: (key, value, attributes). A PROPS_INDEXED
/// table with holes is compacted first (`props_index::settle`; no allocation), so `i` counts
/// live entries only. Not a GC point.
pub(crate) fn own_entry(o: u64, i: usize) -> (u64, u64, i32) {
    let s = shape::shape_of(o);
    if shape::is_dictionary(s) {
        let p = table_of(o).expect("own_entry on an object without properties");
        props_index::settle(p);
        let e = entry_in(p, i);
        return (rd(p, e), rd(p, e + 8), rd_i32(p, e + 16));
    }
    let d = shape::details_at(s, i);
    (
        shape::key_at(s, i),
        field_get(o, s, usize::from(d.field())),
        d.attrs(),
    )
}
