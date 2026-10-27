//! The megamorphic stub cache (owner M4-P3 IC2; OI-09, H16-Q3): a per-isolate table from
//! (operation, receiver shape, name) to a handler, probed by the miss path of a megamorphic
//! named-property site. V8's stub cache, one level.
//!
//! ```text
//!  megamorphic site (slot = hole, int32 MEGA)       Runtime::stub_cache (1024 entries, Rust memory)
//!   hit path: ic::load / ic::store: a miss            index = hash(op, ShapeId, name)
//!   miss path: slow::get_named / get_super / set_named
//!     ic::mega_load / ic::mega_store                  entry: key = op << 32 | ShapeId
//!       probe(op, ShapeId of o, name) ─────────────>         name (an atom's bits)
//!         Some(handler) -> the handler code of ic.rs         handler (a smi handler or an
//!         None          -> the generic op, then              IC_HANDLER cell of ic.rs)
//!     ic::record on a megamorphic entry ────insert─>  (the entry there is replaced)
//!
//!  every GC (a weak source of the heap) ──flush──> every entry empty
//! ```
//!
//! Rules:
//! - Wiring (M4-P3): the calls in `slow/property.rs` come from the integration fixer, which
//!   also sets `ic::MEGA_FILL`; until then the table is filled only in tests (and when a site
//!   goes megamorphic) and never probed outside them.
//! - The probe is on the miss path, not in `ic::load` / `ic::store`: there it would cost every
//!   monomorphic hit two more loads (the site's name operand and the runtime pointer; measured
//!   +1.4% instructions on t0-prop, mac a64). The miss path still saves the generic lookup.
//! - Handlers are the IC handlers of `ic.rs`, so a hit runs the same handler code as a
//!   monomorphic or polymorphic case: smi field handlers and `ARRAY_LENGTH` (loads and stores),
//!   `PROTO_LOAD` and `MISSING` (loads, through their validity cell), `ADD` (stores; a 0x24
//!   store checks its validity cell). `SLOW` cases and string receivers are never stored (a
//!   string's ShapeId is 0, and no entry has ShapeId 0).
//! - The operation is part of the key ([`Op`]): a field that a load reads is not always one
//!   that a store may write (read-only) or that CreateDataProperty keeps (attributes), and a
//!   0x24 add checks the chain while a 0x25 add does not.
//! - A case is the same for every site with that operation and name (its handler depends
//!   only on the receiver shape, the name and the prototype chain, which the validity cell
//!   guards), so one entry serves every megamorphic site.
//! - **Weak, flushed at every GC (plan rule 8, H16 K7, H16-Q3 default).** The table is Rust
//!   memory and the GC does not trace it: it keeps no shape, name or handler alive. A weak
//!   source of the heap ([`StubCache::new`]) empties it in every collection, before the
//!   sweep frees anything, so no entry outlives a GC and a ShapeId or an atom address that a
//!   dead cell leaves free can never match an old entry. The table holds no Value across a GC
//!   point: [`insert`] runs after the miss path's last allocation, and [`probe`] neither
//!   allocates nor calls.
//! - The flush costs one flag test when nothing was inserted since the last GC (the GC-stress
//!   mode collects at every allocation), else one store per entry.
//! - The table is not part of any export (facts, layout): compiled code reaches a megamorphic
//!   site through the generic path (H16 CacheIR `Generic`; MC-w14 IC2 line).

use core::cell::Cell;
use std::rc::Rc;

use ep9_heap::Heap;

use crate::runtime::Runtime;

/// The operations with their own entries (the high half of an entry's key; never 0, so an
/// empty entry matches nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Op {
    /// A named load (0x21, and 0x22 keyed on its lookup start).
    Load = 1,
    /// A named store (0x24 `SetNamedProperty`).
    Store = 2,
    /// CreateDataPropertyOrThrow (0x25 `DefineNamedOwnProperty`).
    Define = 3,
}

/// log2 of the number of entries.
const BITS: u32 = 10;
/// The number of entries (24 KiB per isolate).
pub(crate) const ENTRIES: usize = 1 << BITS;

/// One entry. An empty entry has key 0.
struct Entry {
    /// `op << 32 | ShapeId` of the receiver shape.
    key: Cell<u64>,
    /// The name (an atom's Value bits).
    name: Cell<u64>,
    /// The handler (`ic.rs`).
    handler: Cell<u64>,
}

impl Entry {
    fn empty() -> Entry {
        Entry {
            key: Cell::new(0),
            name: Cell::new(0),
            handler: Cell::new(0),
        }
    }
}

/// The isolate's stub cache (`Runtime::stub_cache`). The heap's weak source shares the table
/// and the flag (both `Rc`).
pub(crate) struct StubCache {
    /// A fixed-size array, so the masked index needs no bounds check on the hit path.
    entries: Rc<[Entry; ENTRIES]>,
    /// True if an entry was inserted since the last flush.
    used: Rc<Cell<bool>>,
}

impl StubCache {
    /// The empty cache of a new isolate; registers the flush with `heap` (a weak source, which
    /// runs in every collection). Not a GC point.
    pub(crate) fn new(heap: &Heap) -> StubCache {
        let all: Box<[Entry]> = (0..ENTRIES).map(|_| Entry::empty()).collect();
        let Ok(all): Result<Box<[Entry; ENTRIES]>, _> = all.try_into() else {
            unreachable!("ENTRIES entries")
        };
        let entries: Rc<[Entry; ENTRIES]> = Rc::from(all);
        let used = Rc::new(Cell::new(false));
        let (e, u) = (Rc::clone(&entries), Rc::clone(&used));
        heap.add_weak_source(move |_| flush(&e[..], &u));
        StubCache { entries, used }
    }

    /// The number of entries in use (tests).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.key.get() != 0).count()
    }
}

/// Empties every entry if one was inserted since the last flush.
fn flush(entries: &[Entry], used: &Cell<bool>) {
    if !used.replace(false) {
        return;
    }
    for e in entries {
        e.key.set(0);
    }
}

#[inline(always)]
fn entry_key(op: Op, id: u32) -> u64 {
    (u64::from(op as u8) << 32) | u64::from(id)
}

/// The entry index of (op, ShapeId, name). ShapeIds are 8-byte aligned offsets in the shape
/// region and atoms 8-byte aligned cell addresses, so both drop their low 3 bits first.
#[inline(always)]
fn index(op: Op, id: u32, name: u64) -> usize {
    let h = (id >> 3) ^ ((name as u32) >> 3).rotate_left(11) ^ (u32::from(op as u8) << 29);
    (h.wrapping_mul(0x9E37_79B1) >> (32 - BITS)) as usize
}

/// The handler of the entry for (`op`, ShapeId `id`, `name`), or `None`. Called only from
/// `ic::mega_load` and `ic::mega_store` (the miss path); never allocates, never calls. A ShapeId
/// of 0 (a string or a symbol) never matches.
#[allow(
    dead_code,
    reason = "called by slow::get_named / get_super / set_named once the integration fixer wires it (M4-P3 IC2 report)"
)]
#[inline(always)]
pub(crate) fn probe(rt: &Runtime, op: Op, id: u32, name: u64) -> Option<u64> {
    let e = &rt.stub_cache.entries[index(op, id, name)];
    (e.key.get() == entry_key(op, id) && e.name.get() == name).then(|| e.handler.get())
}

/// Puts `handler` in the entry for (`op`, ShapeId `id`, `name`), in place of what that entry
/// held. `id` is the ShapeId of a fast shape (not 0) and `handler` a handler that the hit path
/// of `op` runs (not `SLOW`). Called from `ic::record` on a megamorphic entry, after its last
/// allocation. Not a GC point.
pub(crate) fn insert(rt: &Runtime, op: Op, id: u32, name: u64, handler: u64) {
    debug_assert!(id != 0);
    let c = &rt.stub_cache;
    let e = &c.entries[index(op, id, name)];
    e.key.set(entry_key(op, id));
    e.name.set(name);
    e.handler.set(handler);
    c.used.set(true);
}
