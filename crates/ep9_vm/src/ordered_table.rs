//! Ordered hash tables (owner M3-P1 B-COLLECTIONS): the storage of Map, Set, WeakMap and
//! WeakSet (§24.1-§24.4), the ORDERED_TABLE cell (kind 77, OBJECT_LAYOUT 3). Its kind and its
//! trace function live here (V8-A9).
//!
//! | Offset | Field |
//! |---|---|
//! | +8 | capacity: the entry count the cell has room for (int32) |
//! | +16 | live count (int32) |
//! | +24 | deleted count (int32): entries used but deleted; `live + deleted` is the next entry |
//! | +32 | bucket count, a power of 2 (int32) |
//! | +40 | entry width: 3 for a Map (key, value, chain), 2 for a Set (key, chain) (int32) |
//! | +48 | flags (int32): bit 0 [`FLAG_WEAK_KEYS`] (set when the table is made, never changed); bit 1 [`FLAG_OBSOLETE`] and bit 2 [`FLAG_CLEARED`] (set once, when a rehash or `clear` replaces the table) |
//! | +56 | the bucket heads (bucket count Values: an int32 entry index, or -1 for none). In an obsolete table, head 0 holds the table that replaced it |
//! | then | the entries (capacity x width Values) |
//!
//! Size = 56 + 8 x (bucket count + capacity x width), read from the cell.
//!
//! ```text
//! a close table (V8 OrderedHashTable): entries in insertion order, hash chains by index
//!   set(k, v)   normalize k (-0 -> +0, one Value per number), hash, find in the chain of
//!               bucket hash & (buckets - 1); new: append at entry live + deleted, link it
//!               first in its chain; full: rehash into a new table first
//!   delete(k)   key := HOLE, value := undefined; the chain link stays (lookups pass it)
//!   rehash      a new table with the live entries in order; the old one gets FLAG_OBSOLETE
//!               and head 0 = the new table (a forward), so a live cursor follows it
//!   clear       a new empty table; the old one gets FLAG_OBSOLETE | FLAG_CLEARED
//!   cursor      (table, index): skips HOLE entries; at an obsolete table it moves to the
//!               forward, at index = the live entries before it (0 after a clear)
//! ```
//!
//! Keys and hashes (H12): a number is stored in its one canonical Value (`Value::number`, -0
//! as +0, one NaN), so SameValueZero is a bits compare for numbers; a string compares and
//! hashes by content (the keyed string hash); a symbol uses its identity hash (+16); an object
//! uses its identity hash (H12-Q1, [`identity_hash`]): a 30-bit value from
//! `symbol::next_identity_hash`, stored on first use as an own property under the
//! isolate's VM-internal private name (`symbol::identity_hash_key`, one for all realms;
//! attributes {}; `[[OwnPropertyKeys]]` and for-in never list a private key),
//! with word0 flag bit 9 (`FLAG_HASH_PRESENT`) set, so a lookup of an object that was never a
//! key reads no property. No hash input is an address (H12-Q2).
//!
//! Weak keys (WeakMap, WeakSet): the trace function visits each key slot of a weak table with
//! `Tracer::visit_weak`, so the GC clears a dead key to undefined at the end of the
//! collection. The value of an entry is strong while its key is there. The value of an entry
//! whose key is already gone (undefined) is weak too, so the next GC frees it; the entry
//! itself goes at the next rehash. Known gap (plan-v8 §3 B-COLLECTIONS task 3): no ephemeron
//! fixpoint, so a value that refers to its own key keeps that key alive. The fixpoint comes
//! with WeakRef and FinalizationRegistry in M1 (`docs/plan/03-gc.md`).
//!
//! No Rust `Vec`/`Box` holds Values across a GC point here (plan §1 rule 8): the table is a
//! heap cell.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};
use ep9_value::Value;

use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::kinds::{self as kind, is_object, is_string, is_symbol};
use crate::object::{self, cell, init, rd, rd_i32, trace_from_8, wr};
use crate::props;
use crate::runtime::{Runtime, val};
use crate::string;
use crate::symbol;

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
/// Flag bit 1: a rehash or `clear` replaced this table; bucket head 0 holds the new table.
pub(crate) const FLAG_OBSOLETE: i32 = 2;
/// Flag bit 2 (with [`FLAG_OBSOLETE`]): `clear` replaced it, so a cursor restarts at 0.
pub(crate) const FLAG_CLEARED: i32 = 4;

/// The entry width of a Map table (key, value, chain).
pub(crate) const MAP_WIDTH: usize = 3;
/// The entry width of a Set table (key, chain).
pub(crate) const SET_WIDTH: usize = 2;

/// The bucket count of a new table (its capacity is twice the bucket count).
const MIN_BUCKETS: usize = 2;
/// The most entries one table holds (V8's limit for a Map or Set: 2^24). Past it, an insert
/// fails (a RangeError in the caller).
pub(crate) const MAX_CAPACITY: usize = 1 << 24;

/// The size of an ORDERED_TABLE cell (read from the cell).
fn table_size(c: CellPtr) -> usize {
    let buckets = rd_i32(c, otf::BUCKETS) as usize;
    let capacity = rd_i32(c, otf::CAPACITY) as usize;
    let width = rd_i32(c, otf::WIDTH) as usize;
    otf::DATA + 8 * (buckets + capacity * width)
}

/// The trace function. A strong table (Map, Set) visits every Value slot. A weak table
/// (WeakMap, WeakSet) visits the counts, the flags and the bucket heads strongly (int32s, or
/// the forward of an obsolete table); each WeakSet key slot with `visit_weak` (cleared to
/// undefined at GC end if its cell dies); each WeakMap entry whose key is there as an
/// ephemeron (`visit_ephemeron`: the key is weak, and the value is traced only once the GC
/// has marked the key by some other path, so a value that reaches its own key does not keep
/// it alive); the value of an entry whose key is already gone (undefined: the key died in an
/// earlier GC; HOLE: deleted) with `visit_weak` (nothing may read it again, and a dangling
/// slot is never left behind); the chain slots hold int32s only.
fn trace_table(c: CellPtr, t: &mut Tracer<'_>) {
    let size = table_size(c);
    if rd_i32(c, otf::FLAGS) & FLAG_WEAK_KEYS == 0 {
        trace_from_8(c, t, size);
        return;
    }
    let buckets = rd_i32(c, otf::BUCKETS) as usize;
    let capacity = rd_i32(c, otf::CAPACITY) as usize;
    let width = rd_i32(c, otf::WIDTH) as usize;
    let head_words = (otf::DATA - 8) / 8 + buckets;
    // SAFETY: the heap calls trace functions with live cells of the registered kind; the words
    // in [8, DATA + 8 x buckets) and every entry word below `table_size` are Value slots.
    let slots = unsafe { c.slots(8, (size - 8) / 8) };
    t.visit_slots(&slots[..head_words]);
    for i in 0..capacity {
        let e = &slots[head_words + i * width..head_words + (i + 1) * width];
        if width != MAP_WIDTH {
            t.visit_weak(&e[0]);
            continue;
        }
        let key = e[0].bits();
        if key == UNDEFINED || key == HOLE {
            t.visit_weak(&e[1]);
        } else {
            t.visit_ephemeron(&e[0], &e[1]);
        }
    }
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

/// A new empty table for a Map (`width` [`MAP_WIDTH`]) or a Set ([`SET_WIDTH`]). GC point.
pub(crate) fn new_empty(rt: &Runtime, width: usize, weak: bool) -> u64 {
    new_table(rt, 2 * MIN_BUCKETS, MIN_BUCKETS, width, weak)
}

/// True if the table `t` has weak keys (flag bit 0).
pub(crate) fn is_weak(t: u64) -> bool {
    debug_assert!(kind::cell_kind(t) == kind::ORDERED_TABLE.get());
    rd_i32(cell(t), otf::FLAGS) & FLAG_WEAK_KEYS != 0
}

/// The byte size of the table `t` (layout tests).
#[cfg(test)]
pub(crate) fn size_of_table(t: u64) -> usize {
    table_size(cell(t))
}

// ---- fields --------------------------------------------------------------------------------

fn field(c: CellPtr, off: usize) -> usize {
    rd_i32(c, off) as usize
}

fn set_field(c: CellPtr, off: usize, n: usize) {
    // E1: an int32 is not a cell, so no barrier.
    init(c, off, int32_bits(n as i32));
}

/// The live entry count of the table `t` (cleared weak keys still count until a rehash).
pub(crate) fn live_count(t: u64) -> usize {
    field(cell(t), otf::LIVE)
}

/// The entries in use: live and deleted; the next new entry goes here.
fn used(c: CellPtr) -> usize {
    field(c, otf::LIVE) + field(c, otf::DELETED)
}

/// The byte offset of entry `i` of the table `c`.
#[inline]
fn entry(c: CellPtr, i: usize) -> usize {
    let buckets = field(c, otf::BUCKETS);
    let width = field(c, otf::WIDTH);
    otf::DATA + 8 * (buckets + i * width)
}

/// The byte offset of the chain link of the entry at `e`.
#[inline]
fn chain(c: CellPtr, e: usize) -> usize {
    e + 8 * (field(c, otf::WIDTH) - 1)
}

/// The byte offset of the head of the bucket of `hash`.
#[inline]
fn bucket(c: CellPtr, hash: u32) -> usize {
    let buckets = field(c, otf::BUCKETS);
    otf::DATA + 8 * (hash as usize & (buckets - 1))
}

/// The key of entry `i` (HOLE: deleted; undefined in a weak table: its key died).
pub(crate) fn key_at(t: u64, i: usize) -> u64 {
    let c = cell(t);
    rd(c, entry(c, i))
}

/// The value of entry `i` (a Map entry; for a Set entry, its key).
pub(crate) fn value_at(t: u64, i: usize) -> u64 {
    let c = cell(t);
    let e = entry(c, i);
    if field(c, otf::WIDTH) == MAP_WIDTH {
        rd(c, e + 8)
    } else {
        rd(c, e)
    }
}

// ---- keys and hashes -----------------------------------------------------------------------

/// The stored form of the key `k`: a number in its one canonical Value (`Value::number`: an
/// int32 when it is one, -0 as +0, one NaN), anything else as it is. Two keys are
/// SameValueZero exactly when their normal forms are [`same_key`].
pub(crate) fn normalize(k: u64) -> u64 {
    match val(k).as_double() {
        Some(d) => Value::number(if d == 0.0 { 0.0 } else { d }).to_bits(),
        None => k,
    }
}

/// SameValueZero of two normalized keys: the same bits, or two strings with the same code
/// units.
#[inline]
fn same_key(a: u64, b: u64) -> bool {
    a == b
        || (is_string(a)
            && is_string(b)
            && !(string::is_atom(a) && string::is_atom(b))
            && string::equals(a, b))
}

/// A 32-bit mix of 64 bits (the splitmix64 finalizer), for keys that are not cells.
fn mix(x: u64) -> u32 {
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z as u32
}

/// The private name under which an object keeps its identity hash (H12-Q1): one per isolate
/// (`symbol::identity_hash_key`, made by `symbol::install`), so an object has the same hash
/// in every realm. Not a GC point.
fn hash_key(rt: &Runtime) -> u64 {
    let k = symbol::identity_hash_key(rt);
    debug_assert!(is_symbol(k), "symbol::install made the identity-hash key");
    k
}

/// True if the object `o` has an identity hash (word0 flag bit 9). Not a GC point.
fn has_hash(o: u64) -> bool {
    // SAFETY: `o` is a live object cell (crate invariant).
    unsafe { cell(o).header() }.flags() & kind::FLAG_HASH_PRESENT != 0
}

/// The identity hash of the object `o` (H12-Q1): the stored one, or, with `create`, a new
/// 30-bit value from the isolate's PRNG (`symbol::next_identity_hash`), stored as an own
/// property under the private name [`hash_key`] with attributes {} (a raw store that ignores
/// `[[Extensible]]`, so a frozen object can be a key), with word0 flag bit 9 set. `None` when
/// `o` has none and `create` is false. GC point only when it creates one.
pub(crate) fn identity_hash(rt: &Runtime, o: u64, create: bool) -> Option<u32> {
    debug_assert!(is_object(o));
    if has_hash(o)
        && let Some((h, _)) = props::own_get(o, hash_key(rt))
    {
        return val(h).as_int32().map(|h| h as u32);
    }
    if !create {
        return None;
    }
    // GC points: `o` lives in a Rust local (CSS).
    let key = hash_key(rt);
    let h = symbol::next_identity_hash(rt);
    props::own_add(rt, o, key, int32_bits(h as i32), 0);
    // SAFETY: `o` is a live object cell (crate invariant).
    let header = unsafe { cell(o).header() };
    header.set_flags(header.flags() | kind::FLAG_HASH_PRESENT);
    Some(h)
}

/// The hash of the normalized key `k`; an object without an identity hash gets one only if
/// `create` (else `None`: it is not a key of any table). GC point only for a new identity
/// hash.
fn hash_of(rt: &Runtime, k: u64, create: bool) -> Option<u32> {
    if is_string(k) {
        Some(string::hash(k))
    } else if is_symbol(k) {
        Some(symbol::hash(k))
    } else if is_object(k) {
        identity_hash(rt, k, create)
    } else {
        Some(mix(k))
    }
}

/// The hash of a key already in a table (its identity hash exists). Not a GC point.
fn stored_hash(rt: &Runtime, k: u64) -> u32 {
    hash_of(rt, k, false).expect("a stored key has a hash")
}

// ---- lookups -------------------------------------------------------------------------------

/// The entry of the normalized key `k` with hash `h` in the table cell `c`, if any.
fn find(c: CellPtr, k: u64, h: u32) -> Option<usize> {
    let mut i = rd_i32(c, bucket(c, h));
    while i >= 0 {
        let e = entry(c, i as usize);
        if same_key(rd(c, e), k) {
            return Some(i as usize);
        }
        i = rd_i32(c, chain(c, e));
    }
    None
}

/// The entry of the key `k` in the table `t`, if any. Not a GC point (a lookup makes no
/// identity hash). A weak table holds only objects and symbols: any other key is not there
/// (and undefined must not find an entry whose key the GC cleared).
pub(crate) fn lookup(rt: &Runtime, t: u64, k: u64) -> Option<usize> {
    if is_weak(t) && !is_object(k) && !is_symbol(k) {
        return None;
    }
    let k = normalize(k);
    let h = hash_of(rt, k, false)?;
    find(cell(t), k, h)
}

// ---- writes --------------------------------------------------------------------------------

/// The table is full and holds [`MAX_CAPACITY`] entries already.
#[derive(Debug)]
pub(crate) struct TableFull;

/// The internal slot of a Map, Set, WeakMap or WeakSet object that holds its table.
pub(crate) const TABLE_SLOT: usize = 0;

/// The table of the collection object `o` (internal slot [`TABLE_SLOT`]).
pub(crate) fn table_of(o: u64) -> u64 {
    object::slot_get(o, TABLE_SLOT)
}

/// Sets the value of the key `k` (normalized here) to `v` (ignored for a Set), or appends a
/// new entry, in the table of the collection object `o`: Map.prototype.set (§24.1.3.9),
/// Set.prototype.add (§24.2.4.1) and the weak ones. A full table is rehashed first. GC point
/// (a new identity hash, a rehash).
pub(crate) fn set(rt: &Runtime, o: u64, k: u64, v: u64) -> Result<(), TableFull> {
    let k = normalize(k);
    // GC point: `o`, `k` and `v` live in Rust locals (CSS).
    let hash = hash_of(rt, k, true).expect("a new key gets a hash");
    let mut c = cell(table_of(o));
    if let Some(i) = find(c, k, hash) {
        if field(c, otf::WIDTH) == MAP_WIDTH {
            wr(&rt.heap, c, entry(c, i) + 8, v);
        }
        return Ok(());
    }
    if used(c) == field(c, otf::CAPACITY) {
        // GC point: `o`, `k` and `v` live in Rust locals.
        c = cell(rehash(rt, o)?);
    }
    append(rt, c, k, v, hash);
    Ok(())
}

/// Appends the entry (k, v) with hash `hash` at the next free index of `c` (not full) and
/// links it first in its chain. Not a GC point.
fn append(rt: &Runtime, c: CellPtr, k: u64, v: u64, hash: u32) {
    let i = used(c);
    debug_assert!(i < field(c, otf::CAPACITY));
    let e = entry(c, i);
    let b = bucket(c, hash);
    wr(&rt.heap, c, e, k);
    if field(c, otf::WIDTH) == MAP_WIDTH {
        wr(&rt.heap, c, e + 8, v);
    }
    init(c, chain(c, e), rd(c, b));
    set_field(c, b, i);
    set_field(c, otf::LIVE, field(c, otf::LIVE) + 1);
}

/// Deletes the key `k` from the table of the collection object `o`: true if it was there. The entry keeps its place and its chain link;
/// its key becomes HOLE and its value undefined. Not a GC point.
pub(crate) fn delete(rt: &Runtime, o: u64, k: u64) -> bool {
    let t = table_of(o);
    let Some(i) = lookup(rt, t, k) else {
        return false;
    };
    let c = cell(t);
    let e = entry(c, i);
    // E1: HOLE and undefined are not cells.
    init(c, e, HOLE);
    if field(c, otf::WIDTH) == MAP_WIDTH {
        init(c, e + 8, UNDEFINED);
    }
    set_field(c, otf::LIVE, field(c, otf::LIVE) - 1);
    set_field(c, otf::DELETED, field(c, otf::DELETED) + 1);
    true
}

/// Empties the collection object `o`: it gets a new empty table, and the old one forwards to it
/// with [`FLAG_CLEARED`], so a live cursor goes on at entry 0 of the new one. GC point.
pub(crate) fn clear(rt: &Runtime, o: u64) {
    let old = table_of(o);
    if used(cell(old)) == 0 {
        return;
    }
    let width = field(cell(old), otf::WIDTH);
    // GC point: `old` lives in a Rust local.
    let t = new_empty(rt, width, is_weak(old));
    forward(rt, old, t, FLAG_CLEARED);
    object::slot_set(rt, o, TABLE_SLOT, t);
}

/// Replaces the table of the collection object `o` (full) by a new one with the live
/// entries in order; deleted entries and dead weak keys are dropped. The new capacity is the
/// old one if half of it or more was deleted, else twice it; then it halves while a quarter
/// of it still holds every live entry and one more, so at least half of it is free after the
/// rehash (amortized O(1) inserts). The old table forwards to the new one. Returns the new
/// table. GC point.
fn rehash(rt: &Runtime, o: u64) -> Result<u64, TableFull> {
    let old = table_of(o);
    let oc = cell(old);
    let weak = rd_i32(oc, otf::FLAGS) & FLAG_WEAK_KEYS != 0;
    let width = field(oc, otf::WIDTH);
    let n = used(oc);
    let live = (0..n)
        .filter(|&i| survives(rd(oc, entry(oc, i)), weak))
        .count();
    let needed = live + 1;
    if needed > MAX_CAPACITY {
        return Err(TableFull);
    }
    let cap = field(oc, otf::CAPACITY);
    let mut capacity = if field(oc, otf::DELETED) >= cap / 2 {
        cap
    } else {
        2 * cap
    };
    while capacity > 2 * MIN_BUCKETS && capacity / 4 >= needed {
        capacity /= 2;
    }
    let capacity = capacity.clamp(2 * MIN_BUCKETS, MAX_CAPACITY);
    // GC point: `o` and `old` live in Rust locals. A weak key that dies in it is cleared, and
    // the copy below skips it.
    let t = new_table(rt, capacity, capacity / 2, width, weak);
    let c = cell(t);
    for i in 0..n {
        let e = entry(oc, i);
        let k = rd(oc, e);
        if !survives(k, weak) {
            continue;
        }
        let v = if width == MAP_WIDTH {
            rd(oc, e + 8)
        } else {
            UNDEFINED
        };
        // A stored key has its hash: no GC point since `t` was made.
        append(rt, c, k, v, stored_hash(rt, k));
    }
    forward(rt, old, t, 0);
    object::slot_set(rt, o, TABLE_SLOT, t);
    Ok(t)
}

/// True if the key `k` of a table entry is live: not deleted (HOLE), and in a weak table
/// not cleared by the GC (undefined; a weak key is never undefined otherwise).
fn survives(k: u64, weak: bool) -> bool {
    k != HOLE && !(weak && k == UNDEFINED)
}

/// Marks `old` obsolete (with `extra` flags) and makes bucket head 0 point to `new`.
fn forward(rt: &Runtime, old: u64, new: u64, extra: i32) {
    let c = cell(old);
    let flags = rd_i32(c, otf::FLAGS) | FLAG_OBSOLETE | extra;
    init(c, otf::FLAGS, int32_bits(flags));
    wr(&rt.heap, c, otf::DATA, new);
}

// ---- cursors -------------------------------------------------------------------------------

/// Moves the cursor (`t`, `i`) to the next live entry and returns it, past it; `None` at the
/// end (the cursor then stays at the end, so an entry added later is still seen). At an
/// obsolete table the cursor first moves to the table that replaced it: to entry 0 after a
/// `clear`, else to the index of the first live entry at or after `i` (the count of live
/// entries before `i`: a rehash keeps their order). Not a GC point.
pub(crate) fn cursor_next(t: &mut u64, i: &mut usize) -> Option<usize> {
    loop {
        let c = cell(*t);
        let flags = rd_i32(c, otf::FLAGS);
        if flags & FLAG_OBSOLETE != 0 {
            *i = if flags & FLAG_CLEARED != 0 {
                0
            } else {
                let weak = flags & FLAG_WEAK_KEYS != 0;
                (0..*i)
                    .filter(|&j| survives(rd(c, entry(c, j)), weak))
                    .count()
            };
            *t = rd(c, otf::DATA);
            continue;
        }
        let n = used(c);
        while *i < n {
            let j = *i;
            *i += 1;
            if rd(c, entry(c, j)) != HOLE {
                return Some(j);
            }
        }
        return None;
    }
}
