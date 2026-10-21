//! The property dictionary of an object (week 3; shapes take over the fast case, H17, and
//! this table stays as dictionary-mode storage).
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | PROPS | 64 | +8 capacity, +16 count (int32s), +24 entries: key, value, attributes | 24 + 24 cap |
//!
//! The object holds the table in its `obj::PROPS` slot (undefined while it has none). Keys
//! keep insertion order.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::frame::{UNDEFINED, int32_bits};
use crate::key::key_eq;
use crate::kinds as kind;
use crate::object::{cell, init, obj, rd, rd_i32, trace_from_8, wr};
use crate::runtime::Runtime;

mod propf {
    pub const CAPACITY: usize = 8;
    pub const COUNT: usize = 16;
    pub const ENTRIES: usize = 24;
    pub const ENTRY: usize = 24;
}

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

fn props_cell(o: u64) -> Option<CellPtr> {
    let p = rd(cell(o), obj::PROPS);
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

/// Index of `key` in the own property table of `o`.
pub(crate) fn find(o: u64, key: u64) -> Option<(CellPtr, usize)> {
    let p = props_cell(o)?;
    // v0: linear search with content compare. Shapes replace it (week 4).
    (0..props_count(p))
        .find(|&i| key_eq(rd(p, entry(i)), key))
        .map(|i| (p, i))
}

/// An own data property: (value, attributes).
pub(crate) fn own_get(o: u64, key: u64) -> Option<(u64, i32)> {
    let (p, i) = find(o, key)?;
    Some((rd(p, entry(i) + 8), rd_i32(p, entry(i) + 16)))
}

/// Writes an existing own property's value. False if `o` has no such property.
pub(crate) fn own_replace(rt: &Runtime, o: u64, key: u64, v: u64) -> bool {
    match find(o, key) {
        Some((p, i)) => {
            wr(&rt.heap, p, entry(i) + 8, v);
            true
        }
        None => false,
    }
}

/// Defines or overwrites an own data property (insertion order kept). GC point.
pub(crate) fn own_define(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    if let Some((p, i)) = find(o, key) {
        wr(&rt.heap, p, entry(i) + 8, v);
        wr(&rt.heap, p, entry(i) + 16, int32_bits(attrs));
        return;
    }
    let old = props_cell(o);
    let (count, cap) = old.map_or((0, 0), |p| (props_count(p), props_capacity(p)));
    let p = if count < cap {
        old.expect("a table with room")
    } else {
        // GC point: `o`, `key`, `v` and `old` live in Rust locals (conservative scan, H10).
        let n = new_props(rt, (cap * 2).max(4));
        if let Some(old) = old {
            for j in 0..count * 3 {
                init(n, propf::ENTRIES + 8 * j, rd(old, propf::ENTRIES + 8 * j));
            }
            init(n, propf::COUNT, int32_bits(count as i32));
        }
        wr(&rt.heap, cell(o), obj::PROPS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, p, entry(count), key);
    wr(&rt.heap, p, entry(count) + 8, v);
    wr(&rt.heap, p, entry(count) + 16, int32_bits(attrs));
    wr(&rt.heap, p, propf::COUNT, int32_bits(count as i32 + 1));
}

/// Removes an own property. Returns its attributes, or `None` if it was absent.
pub(crate) fn own_delete(rt: &Runtime, o: u64, key: u64) -> Option<i32> {
    let (p, i) = find(o, key)?;
    let attrs = rd_i32(p, entry(i) + 16);
    let count = props_count(p);
    // v0: shift the tail down (keeps insertion order).
    for j in i * 3..(count - 1) * 3 {
        wr(
            &rt.heap,
            p,
            propf::ENTRIES + 8 * j,
            rd(p, propf::ENTRIES + 8 * (j + 3)),
        );
    }
    for k in 0..3 {
        wr(&rt.heap, p, entry(count - 1) + 8 * k, UNDEFINED);
    }
    wr(&rt.heap, p, propf::COUNT, int32_bits(count as i32 - 1));
    Some(attrs)
}

/// Number of own named properties.
pub(crate) fn own_count(o: u64) -> usize {
    props_cell(o).map_or(0, props_count)
}

/// Own named property `i` in insertion order: (key, value, attributes).
pub(crate) fn own_entry(o: u64, i: usize) -> (u64, u64, i32) {
    let p = props_cell(o).expect("own_entry on an object without properties");
    (
        rd(p, entry(i)),
        rd(p, entry(i) + 8),
        rd_i32(p, entry(i) + 16),
    )
}
