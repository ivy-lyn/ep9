#![allow(dead_code)] // m3-stub
//! Ordered hash tables (owner M3-P1 B-COLLECTIONS): the storage of Map, Set, WeakMap and
//! WeakSet (§24.1-§24.4), the ORDERED_TABLE cell (kind 77, OBJECT_LAYOUT 3). Its kind and its
//! trace function live here (V8-A9), so B-COLLECTIONS owns both.
//!
//! | Offset | Field |
//! |---|---|
//! | +8 | capacity: the entry count the cell has room for (int32) |
//! | +16 | live count (int32) |
//! | +24 | deleted count (int32) |
//! | +32 | bucket count, a power of 2 (int32) |
//! | +40 | entry width: 3 for a Map (key, value, chain), 2 for a Set (key, chain) (int32) |
//! | +48 | flags (int32): bit 0 [`FLAG_WEAK_KEYS`], set when the table is made, never changed |
//! | +56 | the bucket heads (bucket count Values: an int32 entry index, or -1 for none) |
//! | then | the entries (capacity x width Values) |
//!
//! Size = 56 + 8 x (bucket count + capacity x width), read from the cell. Every word after
//! word0 is a Value (H17), so the M3-P0 trace function visits all of them: a strong trace, also
//! for a weak table. B-COLLECTIONS writes the weak trace (ephemeron keys) and fills H12-Q1 (the
//! identity hash, `symbol::next_identity_hash`).
//!
//! No Rust `Vec`/`Box` holds Values across a GC point here (plan §1 rule 8): the table is a
//! heap cell.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::frame::{UNDEFINED, int32_bits};
use crate::kinds as kind;
use crate::object::{cell, init, rd_i32, trace_from_8};
use crate::runtime::Runtime;

/// Field offsets of an ORDERED_TABLE cell.
pub(crate) mod otf {
    pub const CAPACITY: usize = 8;
    pub const LIVE: usize = 16;
    pub const DELETED: usize = 24;
    pub const BUCKETS: usize = 32;
    pub const WIDTH: usize = 40;
    pub const FLAGS: usize = 48;
    /// The first bucket head; the entries follow the bucket heads.
    pub const DATA: usize = 56;
}

/// Flag bit 0: the keys are weak (WeakMap, WeakSet). Set when the table is made.
pub(crate) const FLAG_WEAK_KEYS: i32 = 1;

/// The entry width of a Map table (key, value, chain).
pub(crate) const MAP_WIDTH: usize = 3;
/// The entry width of a Set table (key, chain).
pub(crate) const SET_WIDTH: usize = 2;

/// The size of an ORDERED_TABLE cell (read from the cell).
fn table_size(c: CellPtr) -> usize {
    let buckets = rd_i32(c, otf::BUCKETS) as usize;
    let capacity = rd_i32(c, otf::CAPACITY) as usize;
    let width = rd_i32(c, otf::WIDTH) as usize;
    otf::DATA + 8 * (buckets + capacity * width)
}

/// M3-P0: a strong trace of every Value slot, weak tables included (B-COLLECTIONS makes the
/// keys of a weak table weak).
fn trace_table(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, table_size(c));
}

/// Registers the ORDERED_TABLE cell kind (`object::register_kinds` calls it).
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::ORDERED_TABLE,
        CellKind {
            name: "ordered-table",
            size: CellSize::Dynamic(table_size),
            trace: trace_table,
        },
    )
    .expect("register a VM cell kind");
}

/// A new empty table with room for `capacity` entries of `width` Values in `buckets` buckets
/// (a power of 2), weak keys if `weak`: counts 0, every bucket head -1, every entry slot
/// undefined. GC point.
pub(crate) fn new_table(
    rt: &Runtime,
    capacity: usize,
    buckets: usize,
    width: usize,
    weak: bool,
) -> u64 {
    assert!(buckets.is_power_of_two(), "bucket count {buckets}");
    assert!(
        width == MAP_WIDTH || width == SET_WIDTH,
        "entry width {width}"
    );
    let size = otf::DATA + 8 * (buckets + capacity * width);
    let c = rt.alloc(kind::ORDERED_TABLE, size);
    // The size function reads these three fields: set them before the next GC point.
    init(c, otf::CAPACITY, int32_bits(capacity as i32));
    init(c, otf::BUCKETS, int32_bits(buckets as i32));
    init(c, otf::WIDTH, int32_bits(width as i32));
    init(c, otf::LIVE, int32_bits(0));
    init(c, otf::DELETED, int32_bits(0));
    init(
        c,
        otf::FLAGS,
        int32_bits(if weak { FLAG_WEAK_KEYS } else { 0 }),
    );
    for i in 0..buckets {
        init(c, otf::DATA + 8 * i, int32_bits(-1));
    }
    for i in 0..capacity * width {
        init(c, otf::DATA + 8 * (buckets + i), UNDEFINED);
    }
    c.to_value().to_bits()
}

/// True if the table `t` has weak keys (flag bit 0).
pub(crate) fn is_weak(t: u64) -> bool {
    debug_assert!(kind::cell_kind(t) == kind::ORDERED_TABLE.get());
    rd_i32(cell(t), otf::FLAGS) & FLAG_WEAK_KEYS != 0
}

/// The byte size of the table `t` (layout tests).
pub(crate) fn size_of_table(t: u64) -> usize {
    table_size(cell(t))
}
