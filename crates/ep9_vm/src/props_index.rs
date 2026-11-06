//! Dictionary v1 (owner M6-P1 DICT1; the H17 "Dictionary v1" part, plan v11 V11-D18): the
//! hash index of a dictionary-mode object with more than 8 own properties, so a lookup no
//! longer scans every entry (`props::dict_find`).
//!
//! ```text
//!  PROPS 64 (Frozen layout core, H17:176; unchanged)    up to 8 entries, a linear scan
//!  PROPS_INDEXED 84 (OBJECT_LAYOUT 6, VM6 step 4)        above 8 entries:
//!    +8 capacity, +16 count (the live entries), +24 index (an ELEMENTS cell, or undefined
//!    below 9 entries), +32 entries (key, value, attrs) as PROPS 64
//!  index (ELEMENTS 65): +8 capacity = 1 + B, +16 used (int32: entries [0, used) are live
//!    or deleted), +24 + 8b bucket b (int32 entry number, or undefined = empty), B = 2 x the
//!    table capacity (a power of 2), linear probing from key_hash(key) & (B - 1)
//!  hook: props::dict_find -> find (before the linear scan of a PROPS 64 table)
//! ```
//!
//! | Step | PROPS_INDEXED with an index (9 or more live entries) | without one (8 or fewer) |
//! |---|---|---|
//! | find | probe the buckets; a bucket whose entry is deleted (key undefined) does not match, so the probe goes on | scan [0, count) |
//! | add | append at `used`, insert in the index; at `used == capacity`: compact in place if at most half the entries are live, else a new table of twice the capacity | append at `count`; the 9th entry builds the index |
//! | delete | the entry's three words become undefined (a hole; no tail shift); its bucket stays and matches nothing. At 8 live entries: compact, drop the index | tail shift (at most 8 entries) |
//! | enumerate (`props::own_entry`) | compact first if there are holes, so entry `i` is the `i`-th live entry in insertion order, as in PROPS | as PROPS |
//!
//! An object moves from PROPS to PROPS_INDEXED above 8 entries (`props::dict_add`,
//! `props::to_dictionary`) and never moves back: below 9 live entries its table keeps kind 84
//! with no index. The index holds int32s only, so a store into it needs no barrier (E1); it is
//! never shared and never escapes this module. A compaction moves values inside the table cell
//! with no allocation and no `&Heap` (`own_entry` has none): each moved cell value goes through
//! the barrier slow path ([`ep9_heap::barrier_slow`]), which logs the host as any store into
//! it would (a LOS host's card of the new slot), so the move is a barriered store.
//!
//! The hash is `key::key_hash` (a string's table hash or a symbol's identity hash): seeded per
//! process, it orders buckets only, never the property order (the entries keep insertion order).

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};

use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::key::{self, Probe};
use crate::kinds as kind;
use crate::object::{cell, elems, init, rd, rd_i32, trace_from_8, wr};
use crate::runtime::{Runtime, val};
use crate::shape::LINEAR_MAX;

/// The PROPS_INDEXED 84 fields (OBJECT_LAYOUT 6; exported as `layout::PROPS_INDEXED_*`).
pub(crate) mod pif {
    /// The entry capacity (an int32 Value).
    pub const CAPACITY: usize = 8;
    /// The entries in use (an int32 Value): the live entries, not the deleted ones.
    pub const COUNT: usize = 16;
    /// The hash index: an ELEMENTS 65 cell (`ixf`), or undefined at 8 or fewer live entries.
    pub const INDEX: usize = 24;
    /// The first entry: key, value, attributes (int32), as in PROPS 64.
    pub const ENTRIES: usize = 32;
    /// Bytes per entry.
    pub const ENTRY: usize = 24;
}

/// The index cell fields (an ELEMENTS cell, kind 65: `+8` capacity is its word count).
mod ixf {
    /// The next entry number to append (an int32): entries `[0, used)` are live or deleted.
    pub const USED: usize = super::elems::DATA;
    /// Bucket 0; bucket `b` is at `BUCKETS + 8 * b`.
    pub const BUCKETS: usize = super::elems::DATA + 8;
}

/// The smallest PROPS_INDEXED capacity (a power of 2 above [`LINEAR_MAX`] + 1).
pub(crate) const MIN_CAPACITY: usize = 16;

/// The size of a PROPS_INDEXED cell: the header words and `capacity` entries.
fn props_indexed_size(c: CellPtr) -> usize {
    pif::ENTRIES + pif::ENTRY * rd_i32(c, pif::CAPACITY) as usize
}

/// Every word after word0 is a Value (int32s, the index cell or undefined, the entries).
fn trace_props_indexed(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, props_indexed_size(c));
}

/// Registers the PROPS_INDEXED kind (OBJECT_LAYOUT 6; `props::register_kinds` calls it).
pub(crate) fn register_kinds(heap: &Heap) {
    heap.register_kind(
        kind::PROPS_INDEXED,
        CellKind {
            name: "property-table-indexed",
            size: CellSize::Dynamic(props_indexed_size),
            trace: trace_props_indexed,
        },
    )
    .expect("register a VM cell kind");
}

// ---- cells ----------------------------------------------------------------------------------

/// Whether the dictionary table `p` (PROPS or PROPS_INDEXED) is a PROPS_INDEXED cell.
#[inline(always)]
pub(crate) fn is_indexed(p: CellPtr) -> bool {
    // SAFETY: `p` is a live table cell (crate invariant).
    unsafe { p.header() }.kind() == kind::PROPS_INDEXED.get()
}

/// Byte offset of entry `i` of a PROPS_INDEXED cell.
#[inline(always)]
pub(crate) const fn entry(i: usize) -> usize {
    pif::ENTRIES + pif::ENTRY * i
}

/// The capacity of a new PROPS_INDEXED table for `n` live entries and one more.
pub(crate) fn capacity_for(n: usize) -> usize {
    (n + 1).next_power_of_two().max(MIN_CAPACITY)
}

#[inline]
fn count(p: CellPtr) -> usize {
    rd_i32(p, pif::COUNT) as usize
}

#[inline]
fn capacity(p: CellPtr) -> usize {
    rd_i32(p, pif::CAPACITY) as usize
}

/// The index cell of `p`, if it has one.
#[inline]
fn index_of(p: CellPtr) -> Option<CellPtr> {
    let ix = rd(p, pif::INDEX);
    (ix != UNDEFINED).then(|| cell(ix))
}

/// The number of buckets of the index `ix` (a power of 2).
#[inline]
fn buckets(ix: CellPtr) -> usize {
    rd_i32(ix, elems::CAPACITY) as usize - 1
}

/// The next entry number to append: `used` of the index, or `count` with no index (then
/// there is no hole).
#[inline]
fn used(p: CellPtr) -> usize {
    index_of(p).map_or_else(|| count(p), |ix| rd_i32(ix, ixf::USED) as usize)
}

/// A new index for a table of capacity `cap`: `2 * cap` empty buckets (the allocation fills
/// every word with undefined), `used` 0. GC point.
fn new_index(rt: &Runtime, cap: usize) -> CellPtr {
    let b = (2 * cap).next_power_of_two();
    let ix = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * (1 + b));
    init(ix, elems::CAPACITY, int32_bits(1 + b as i32));
    init(ix, ixf::USED, int32_bits(0));
    ix
}

/// A new, empty PROPS_INDEXED table of capacity `cap` (a power of 2, at least
/// [`MIN_CAPACITY`]) with an empty index. The caller fills entries `[0, n)` with initializing
/// stores (E2: no GC point after this call) and then calls [`seal`]. GC point.
pub(crate) fn alloc(rt: &Runtime, cap: usize) -> CellPtr {
    debug_assert!(cap.is_power_of_two() && cap >= MIN_CAPACITY);
    // The index first: the table's initializing stores must follow its own allocation with no
    // GC point between. `ix` lives in a Rust local over the second allocation (CSS, H10).
    let ix = new_index(rt, cap);
    let p = rt.alloc(kind::PROPS_INDEXED, entry(cap));
    init(p, pif::CAPACITY, int32_bits(cap as i32));
    init(p, pif::COUNT, int32_bits(0));
    init(p, pif::INDEX, ix.to_value().to_bits());
    p
}

/// Ends the fill of a table from [`alloc`]: entries `[0, n)` are live (no hole). Builds the
/// index. `n` is above [`LINEAR_MAX`], or exactly [`LINEAR_MAX`] when the caller adds the 9th
/// entry next with no GC point between (`props::dict_add`). Not a GC point.
pub(crate) fn seal(p: CellPtr, n: usize) {
    debug_assert!(n >= LINEAR_MAX && n <= capacity(p));
    // E1: int32 stores.
    init(p, pif::COUNT, int32_bits(n as i32));
    reindex(p);
}

// ---- the index --------------------------------------------------------------------------------

#[inline]
fn same_key(stored: u64, k: u64, how: Probe) -> bool {
    match how {
        Probe::Bits => stored == k,
        // A deleted entry's key is undefined, which equals no key.
        Probe::Content => stored != UNDEFINED && key::key_eq(stored, k),
    }
}

/// Stores entry number `i` in the first empty bucket from `hash` (linear probing). The index
/// has twice as many buckets as the table has entries, so one is empty. Not a GC point.
fn index_insert(ix: CellPtr, hash: u32, i: usize) {
    let mask = buckets(ix) - 1;
    let mut b = hash as usize & mask;
    while rd(ix, ixf::BUCKETS + 8 * b) != UNDEFINED {
        b = (b + 1) & mask;
    }
    // E1: an int32 store needs no barrier.
    init(ix, ixf::BUCKETS + 8 * b, int32_bits(i as i32));
}

/// Rebuilds the index of `p` from its entries `[0, count)`, which hold no hole. Not a GC point.
fn reindex(p: CellPtr) {
    let ix = index_of(p).expect("an index to rebuild");
    for b in 0..buckets(ix) {
        // E1: undefined is not a cell.
        init(ix, ixf::BUCKETS + 8 * b, UNDEFINED);
    }
    let n = count(p);
    for i in 0..n {
        index_insert(ix, key::key_hash(rd(p, entry(i))), i);
    }
    init(ix, ixf::USED, int32_bits(n as i32));
}

/// The hook of a dictionary lookup (`props::dict_find`), before the linear scan of the table
/// `p`: `Some(Some(i))` if `p` is a PROPS_INDEXED table with `key` at entry `i`,
/// `Some(None)` if `p` is one without `key`, `None` if `p` is a PROPS table (the caller
/// scans). Not a GC point.
#[inline(always)]
pub(crate) fn find(p: CellPtr, key: u64, how: Probe) -> Option<Option<usize>> {
    is_indexed(p).then(|| find_indexed(p, key, how))
}

/// [`find`] in a PROPS_INDEXED table.
fn find_indexed(p: CellPtr, key: u64, how: Probe) -> Option<usize> {
    let Some(ix) = index_of(p) else {
        // 8 or fewer live entries, no hole.
        let n = count(p);
        return (0..n).find(|&i| same_key(rd(p, entry(i)), key, how));
    };
    let mask = buckets(ix) - 1;
    let mut b = key::key_hash(key) as usize & mask;
    loop {
        let e = rd(ix, ixf::BUCKETS + 8 * b);
        if e == UNDEFINED {
            return None;
        }
        let i = int32_of(e) as usize;
        if same_key(rd(p, entry(i)), key, how) {
            return Some(i);
        }
        b = (b + 1) & mask;
    }
}

// ---- add, delete, compaction ----------------------------------------------------------------

/// Writes entry `i` of `p`: key, value, attributes. Not a GC point.
fn put(rt: &Runtime, p: CellPtr, i: usize, key: u64, v: u64, attrs: i32) {
    wr(&rt.heap, p, entry(i), key);
    wr(&rt.heap, p, entry(i) + 8, v);
    // E1: an int32 store needs no barrier.
    init(p, entry(i) + 16, int32_bits(attrs));
}

/// Clears entry `i` of `p` (a hole, or past the last entry), so the GC keeps no key or value
/// of it. Not a GC point.
fn clear(p: CellPtr, i: usize) {
    for k in 0..3 {
        // E1: undefined is not a cell.
        init(p, entry(i) + 8 * k, UNDEFINED);
    }
}

/// Stores `v`, a word read from another slot of the same cell `p`, at `off`: a barriered
/// store without a `&Heap` (callers with no runtime: `props::own_entry`). A cell value goes
/// through the barrier slow path, which stores and logs the host (an old small host greys, a
/// LOS host dirties the card of `off`); any other value needs no barrier (E1). Not a GC point.
fn move_store(p: CellPtr, off: usize, v: u64) {
    if kind::is_cell_bits(v) {
        // SAFETY: `p` is a live cell (crate invariant) and `off` is the offset of a Value slot
        // of its entries (inside its capacity).
        unsafe { ep9_heap::barrier_slow(p.header(), p.slot(off), val(v)) };
    } else {
        init(p, off, v);
    }
}

/// Moves the live entries of `p` down over its holes, in order, and clears the rest. With 9
/// or more live entries it rebuilds the index; with fewer it drops it (the form below 9
/// entries). No allocation: not a GC point.
fn compact(p: CellPtr) {
    let used = used(p);
    let mut n = 0;
    for j in 0..used {
        let k = rd(p, entry(j));
        if k == UNDEFINED {
            continue;
        }
        if j != n {
            move_store(p, entry(n), k);
            move_store(p, entry(n) + 8, rd(p, entry(j) + 8));
            // E1: an int32 store needs no barrier.
            init(p, entry(n) + 16, rd(p, entry(j) + 16));
            clear(p, j);
        }
        n += 1;
    }
    debug_assert_eq!(n, count(p), "the live entries of a PROPS_INDEXED table");
    if n > LINEAR_MAX {
        reindex(p);
    } else {
        // E1: undefined is not a cell. The index cell becomes garbage.
        init(p, pif::INDEX, UNDEFINED);
    }
}

/// Before an enumeration reads entry `i` of the table `p` (`props::own_entry`): compacts a
/// PROPS_INDEXED table that has holes, so entry `i` is its `i`-th live entry in insertion
/// order. No allocation: not a GC point.
#[inline]
pub(crate) fn settle(p: CellPtr) {
    if is_indexed(p) && used(p) != count(p) {
        compact(p);
    }
}

/// Appends `key` (not in the table) with `v` and `attrs` to `p`, the PROPS_INDEXED table of
/// `o`. GC point: a new index, or a new table of twice the capacity.
pub(crate) fn add(rt: &Runtime, o: u64, p: CellPtr, key: u64, v: u64, attrs: i32) {
    debug_assert!(is_indexed(p));
    let mut p = p;
    let n = count(p);
    if index_of(p).is_none() && n + 1 > LINEAR_MAX {
        // The 9th entry. GC point: `o`, `p`, `key` and `v` live in Rust locals (CSS, H10).
        let ix = new_index(rt, capacity(p));
        wr(&rt.heap, p, pif::INDEX, ix.to_value().to_bits());
        reindex(p);
    }
    let cap = capacity(p);
    if used(p) == cap {
        if 2 * n <= cap {
            compact(p);
        } else {
            p = grow(rt, o, p, 2 * cap);
        }
    }
    let i = used(p);
    debug_assert!(i < capacity(p));
    put(rt, p, i, key, v, attrs);
    init(p, pif::COUNT, int32_bits(n as i32 + 1));
    if let Some(ix) = index_of(p) {
        index_insert(ix, key::key_hash(key), i);
        init(ix, ixf::USED, int32_bits(i as i32 + 1));
    }
}

/// Copies the live entries of `p` (the table of `o`) into a new table of capacity `cap`,
/// installs it in `o` and returns it. GC point: `o` and `p` live in Rust locals.
fn grow(rt: &Runtime, o: u64, p: CellPtr, cap: usize) -> CellPtr {
    let n = alloc(rt, cap);
    let mut m = 0;
    for j in 0..used(p) {
        let k = rd(p, entry(j));
        if k == UNDEFINED {
            continue;
        }
        // E2: `n` is new and no GC point ran since its allocation.
        init(n, entry(m), k);
        init(n, entry(m) + 8, rd(p, entry(j) + 8));
        init(n, entry(m) + 16, rd(p, entry(j) + 16));
        m += 1;
    }
    seal(n, m);
    wr(
        &rt.heap,
        cell(o),
        crate::object::obj::SLOTS,
        n.to_value().to_bits(),
    );
    n
}

/// Removes entry `i` of the PROPS_INDEXED table `p`. With an index the entry becomes a hole
/// (no tail shift); at 8 live entries the table is compacted and drops its index. Without an
/// index (8 or fewer entries) the tail shifts down, as in PROPS. Not a GC point.
pub(crate) fn remove(rt: &Runtime, p: CellPtr, i: usize) {
    debug_assert!(is_indexed(p));
    let n = count(p);
    if index_of(p).is_none() {
        for j in i..n - 1 {
            wr(&rt.heap, p, entry(j), rd(p, entry(j + 1)));
            wr(&rt.heap, p, entry(j) + 8, rd(p, entry(j + 1) + 8));
            // E1: an int32 store needs no barrier.
            init(p, entry(j) + 16, rd(p, entry(j + 1) + 16));
        }
        clear(p, n - 1);
        init(p, pif::COUNT, int32_bits(n as i32 - 1));
        return;
    }
    clear(p, i);
    init(p, pif::COUNT, int32_bits(n as i32 - 1));
    if n - 1 <= LINEAR_MAX {
        compact(p);
    }
}

// ---- test views -----------------------------------------------------------------------------

/// The state of a PROPS_INDEXED table for the tests: (capacity, live count, used, bucket
/// count or 0 without an index).
#[cfg(test)]
pub(crate) fn state(p: CellPtr) -> (usize, usize, usize, usize) {
    (
        capacity(p),
        count(p),
        used(p),
        index_of(p).map_or(0, buckets),
    )
}
