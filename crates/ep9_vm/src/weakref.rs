//! WeakRef and FinalizationRegistry (owner M4-P4 WEAKREF; GFF-WEAKREF, V9-D23): SLOT_OBJECT
//! classes (`ObjectClass::WeakRef` 11, `ObjectClass::FinalizationRegistry` 12) with one
//! internal WEAK_CELL kind (80) for the weak target, the `[[KeptAlive]]` list (§9.10,
//! AddToKeptObjects, ClearKeptObjects) and the state of the cleanup jobs. The `Cx` methods
//! over this are in `abi/weakref.rs`.
//!
//! ```text
//! WeakRef            SLOT_OBJECT, class 11, slot 0 ──> WEAK_CELL { TARGET (weak) }
//! FinalizationRegistry SLOT_OBJECT, class 12:
//!   slot 0 CLEANUP   [[CleanupCallback]]
//!   slot 1 CELLS     WEAK_CELL ──NEXT──> WEAK_CELL ──> ... (newest first; [[Cells]] whose
//!                    target may still be live)
//!   slot 2 CLEARED   WEAK_CELL ──NEXT──> ... ([[Cells]] whose target is empty, taken off
//!                    CELLS by the cleanup hook, waiting for their callback)
//! the isolate (Runtime::kept_objects, this file's KeptObjects; one root source):
//!   kept        a Set (SLOT_OBJECT + ORDERED_TABLE, strong keys) of [[KeptAlive]], or undefined
//!   registries  link WEAK_CELL { TARGET = a FinalizationRegistry (weak), NEXT } ──> ...
//!   pending     a Rust bool: a GC saw an empty target or a dead registry
//! ```
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | WEAK_CELL | 80 | +8 TARGET (weak: cleared to undefined when its cell dies), +16 HELD (strong; the held value, undefined for a WeakRef or a link), +24 TOKEN (weak; the unregister token, undefined = empty), +32 NEXT (strong; the next cell of the list, undefined = last) | 40 |
//!
//! One trace function for every use (V9-D23): TARGET and TOKEN are weak slots
//! (`Tracer::visit_weak`), HELD and NEXT strong. A target is an object or a symbol that
//! `Symbol.for` did not make (CanBeHeldWeakly, §9.13), so it is always a cell, and undefined
//! in TARGET can only mean "empty" (cleared by a GC).
//!
//! Rules:
//! - No Rust `Vec` holds a Value (plan rule 8): every list is a chain of WEAK_CELLs or a heap
//!   table, and the isolate's two list heads are root slots (`Cell<u64>`, E3: no barrier).
//! - A GC runs no JS and allocates nothing here. The weak source ([`Roots::scan`]) runs after
//!   marking: it reads the registries and their cells, clears a dead slot it meets
//!   (`WeakVisitor::visit`; the mark has cleared them already, so this only adds the
//!   verifier's checks of a checked mark), and sets `pending` when a registry died or a cell's
//!   target is empty. It stores no cell pointer. The cleanup hook
//!   (`Cx::cleanup_finalization_registries`, outside any GC) then moves the empty cells to
//!   CLEARED, drops the dead registries from the chain (barriered stores), and calls the
//!   callbacks: that is HostEnqueueFinalizationRegistryCleanupJob (§9.5.4) and the job.
//! - The generational GC (S1) needs no extra care: a WEAK_CELL is made with its TARGET and
//!   TOKEN (E2), and only the GC changes them (to undefined). So the first GC that the cell
//!   lives through traces it (it is young), and after that its TARGET and TOKEN are old or
//!   undefined; an old target never dies in a minor GC. A NEXT or slot store outside a GC is
//!   barriered (`object::wr`, `object::slot_set`).
//! - The weak source walks every registry and every cell at every GC (minor ones too):
//!   O(registries + cells). A per-GC worklist of the cells the mark met (V8's
//!   `weak_cells` worklist) needs a heap hook; that is a month-5 WEAKREF tail item.
//!
//! | Function | GC point | Job |
//! |---|---|---|
//! | [`KeptObjects::new`] | no | the isolate state: WEAK_CELL kind, root source, weak source |
//! | [`new_cell`] | yes | a WEAK_CELL |
//! | [`target`], [`held`], [`token`], [`next`], [`set_next`] | no | WEAK_CELL fields |
//! | [`KeptObjects::add`] | yes | AddToKeptObjects (§9.10.2) |
//! | [`KeptObjects::clear`] | no | ClearKeptObjects (§9.10.1) |
//! | [`KeptObjects::add_registry`] | yes | puts a new registry in the isolate's chain |
//! | [`KeptObjects::take_pending`] | no | the cleanup hook's "anything to do?" test |
//! | [`KeptObjects::registries`] | no | the first link of the chain |
//! | [`unlink_dead_registries`] | no | drops the links of dead registries |
//! | [`collect_cleared`] | no | moves the empty cells of a registry from CELLS to CLEARED |

use core::cell::Cell;
use std::rc::Rc;

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer, WeakVisitor};

use crate::abi::ObjectClass;
use crate::frame::{NULL, UNDEFINED};
use crate::kinds;
use crate::object::{self, cell, init, rd, wr};
use crate::ordered_table::{self as table, SET_WIDTH};
use crate::runtime::Runtime;

/// WEAK_CELL fields (byte offsets).
pub(crate) mod wc {
    /// The target (weak): an object or an unregistered symbol, or undefined once it died.
    pub const TARGET: usize = 8;
    /// The held value of a FinalizationRegistry cell (strong); undefined otherwise.
    pub const HELD: usize = 16;
    /// The unregister token (weak); undefined = empty.
    pub const TOKEN: usize = 24;
    /// The next cell of the list (strong); undefined = last.
    pub const NEXT: usize = 32;
    pub const SIZE: usize = 40;
}

/// The internal slots of a WeakRef object.
pub(crate) mod weak_ref {
    /// The WEAK_CELL that holds `[[WeakRefTarget]]`.
    pub const CELL: usize = 0;
    pub const COUNT: usize = 1;
}

/// The internal slots of a FinalizationRegistry object.
pub(crate) mod registry {
    /// `[[CleanupCallback]]` (callable).
    pub const CLEANUP: usize = 0;
    /// The first WEAK_CELL of the cells not yet seen empty (undefined: none).
    pub const CELLS: usize = 1;
    /// The first WEAK_CELL of the cells whose target is empty and whose callback has not run
    /// (undefined: none). Both lists together are `[[Cells]]`.
    pub const CLEARED: usize = 2;
    pub const COUNT: usize = 3;
}

#[cfg(test)]
thread_local! {
    /// Test-only negative control (`tests::weakref`): the trace function visits TARGET as a
    /// strong slot, so a target that only a WeakRef holds stays alive.
    pub(crate) static NEGATIVE_CONTROL_STRONG_TARGET: Cell<bool> = const { Cell::new(false) };
}

fn trace_cell(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: the heap passes a live WEAK_CELL; +8 .. +32 are Value slots (layout table).
    let (target, held, token, next) = unsafe {
        (
            c.slot(wc::TARGET),
            c.slot(wc::HELD),
            c.slot(wc::TOKEN),
            c.slot(wc::NEXT),
        )
    };
    #[cfg(test)]
    if NEGATIVE_CONTROL_STRONG_TARGET.with(Cell::get) {
        t.visit(target);
    } else {
        t.visit_weak(target);
    }
    #[cfg(not(test))]
    t.visit_weak(target);
    t.visit(held);
    t.visit_weak(token);
    t.visit(next);
}

/// A new WEAK_CELL with these fields (`token` undefined = empty; `next` undefined = last).
/// The stores are initializing (E2). GC point: the arguments live in Rust locals (CSS).
pub(crate) fn new_cell(rt: &Runtime, target: u64, held: u64, token: u64, next: u64) -> u64 {
    debug_assert!(kinds::is_object(target) || kinds::is_symbol(target));
    let c = rt.alloc(kinds::WEAK_CELL, wc::SIZE);
    init(c, wc::TARGET, target);
    init(c, wc::HELD, held);
    init(c, wc::TOKEN, token);
    init(c, wc::NEXT, next);
    c.to_value().to_bits()
}

/// True for a WEAK_CELL (the lists end at undefined; any other value also ends a walk, so
/// an embedder that wrote a registry's slots with `Cx::set_slot` cannot make a GC read a
/// cell of another kind as a WEAK_CELL).
pub(crate) fn is_weak_cell(c: u64) -> bool {
    kinds::cell_kind(c) == kinds::WEAK_CELL.get()
}

/// True for a FinalizationRegistry object with its [`registry::COUNT`] slots.
pub(crate) fn is_registry(o: u64) -> bool {
    object::slot_class(o) == Some(ObjectClass::FinalizationRegistry.id())
        && object::slot_count(o) == registry::COUNT
}

fn field(c: u64, off: usize) -> u64 {
    debug_assert_eq!(kinds::cell_kind(c), kinds::WEAK_CELL.get());
    rd(cell(c), off)
}

/// The target of the WEAK_CELL `c`, or undefined (empty).
pub(crate) fn target(c: u64) -> u64 {
    field(c, wc::TARGET)
}

/// The held value of the WEAK_CELL `c`.
pub(crate) fn held(c: u64) -> u64 {
    field(c, wc::HELD)
}

/// The unregister token of the WEAK_CELL `c`, or undefined (empty).
pub(crate) fn token(c: u64) -> u64 {
    field(c, wc::TOKEN)
}

/// The next cell after `c`, or undefined.
pub(crate) fn next(c: u64) -> u64 {
    field(c, wc::NEXT)
}

/// Sets the next cell after `c` (barriered). Not a GC point.
pub(crate) fn set_next(rt: &Runtime, c: u64, n: u64) {
    debug_assert_eq!(kinds::cell_kind(c), kinds::WEAK_CELL.get());
    wr(&rt.heap, cell(c), wc::NEXT, n);
}

/// The isolate's roots and the flag the weak source sets. Shared (`Rc`) by the root source,
/// the weak source and [`KeptObjects`].
struct Roots {
    /// `[[KeptAlive]]`: a Set collection object, or undefined (empty).
    kept: Cell<u64>,
    /// The first link of the registry chain, or undefined.
    registries: Cell<u64>,
    /// Set by the weak source when a registry died or a cell's target is empty; cleared by
    /// [`KeptObjects::take_pending`].
    pending: Cell<bool>,
}

impl Roots {
    fn trace(&self, t: &mut Tracer<'_>) {
        t.visit_bits(self.kept.get());
        t.visit_bits(self.registries.get());
    }

    /// The weak source: after marking, before the sweep. Reads only (no JS, no allocation, no
    /// store of a cell pointer); `w.visit` clears a slot whose cell is dead (the mark has done
    /// that for every traced cell already) and checks it in a checked mark.
    fn scan(&self, w: &mut WeakVisitor<'_>) {
        let mut link = self.registries.get();
        while link != UNDEFINED {
            let lc = cell(link);
            // SAFETY: a link is a live WEAK_CELL (the chain is strong from a root slot);
            // +8 is its weak TARGET slot.
            let fr_slot = unsafe { lc.slot(wc::TARGET) };
            w.visit(fr_slot);
            let fr = fr_slot.bits();
            if fr == UNDEFINED {
                self.pending.set(true);
            } else if is_registry(fr) {
                let mut c = object::slot_get(fr, registry::CELLS);
                while is_weak_cell(c) {
                    let cc = cell(c);
                    // SAFETY: `c` is a live WEAK_CELL (strong from the live registry).
                    let t = unsafe { cc.slot(wc::TARGET) };
                    w.visit(t);
                    if t.bits() == UNDEFINED {
                        self.pending.set(true);
                    }
                    // SAFETY: as above; +24 is the weak TOKEN slot.
                    w.visit(unsafe { cc.slot(wc::TOKEN) });
                    c = rd(cc, wc::NEXT);
                }
            }
            link = rd(lc, wc::NEXT);
        }
    }
}

/// The WeakRef state of the isolate (`Runtime::kept_objects`): the `[[KeptAlive]]` list, the
/// chain of FinalizationRegistry objects and the cleanup flag. `runtime.rs` has no owner
/// after M4-P0, so the field keeps its M4-P0 name and type, and everything is in this type.
pub(crate) struct KeptObjects {
    roots: Rc<Roots>,
}

impl KeptObjects {
    /// The empty state of a new isolate: registers the WEAK_CELL kind, the root source (the
    /// kept list and the registry chain, strong) and the weak source with `heap` (before the
    /// isolate's first allocation; `Runtime::new` calls it after `object::register_kinds`).
    pub(crate) fn new(heap: &Heap) -> KeptObjects {
        heap.register_kind(
            kinds::WEAK_CELL,
            CellKind {
                name: "weak-cell",
                size: CellSize::Fixed(wc::SIZE),
                trace: trace_cell,
            },
        )
        .expect("register WEAK_CELL");
        let roots = Rc::new(Roots {
            kept: Cell::new(UNDEFINED),
            registries: Cell::new(UNDEFINED),
            pending: Cell::new(false),
        });
        let r = Rc::clone(&roots);
        heap.add_root_source(move |t| r.trace(t));
        let r = Rc::clone(&roots);
        heap.add_weak_source(move |w| r.scan(w));
        KeptObjects { roots }
    }

    /// The kept list (a Set collection object), or undefined.
    pub(crate) fn get(&self) -> u64 {
        self.roots.kept.get()
    }

    /// Replaces the kept list (a root slot: no barrier, E3).
    pub(crate) fn set(&self, list: u64) {
        self.roots.kept.set(list);
    }

    /// AddToKeptObjects(value) (§9.10.2): `value` stays alive until the next
    /// [`KeptObjects::clear`]. A value already in the list is not added twice (a Set; the
    /// first add of an object gives it its identity hash, as a WeakMap key does). GC point:
    /// `value` lives in a Rust local (CSS).
    pub(crate) fn add(&self, rt: &Runtime, value: u64) {
        debug_assert!(kinds::is_object(value) || kinds::is_symbol(value));
        let mut set = self.get();
        if set == UNDEFINED {
            set = object::new_slot_object(rt, NULL, ObjectClass::Set.id(), 1);
            // GC point: `set` lives in a Rust local.
            let t = table::new_empty(rt, SET_WIDTH, false);
            object::slot_set(rt, set, table::TABLE_SLOT, t);
            self.set(set);
        }
        if table::set(rt, set, value, UNDEFINED).is_err() {
            // More than 2^24 distinct values kept in one job (`table::MAX_CAPACITY`).
            // AddToKeptObjects cannot throw, and dropping a value would let its target die
            // while the job still runs, so this fails as an out-of-memory does (H23).
            panic!("ep9: heap limit reached (more than 2^24 values kept alive in one job)");
        }
    }

    /// ClearKeptObjects (§9.10.1): empties `[[KeptAlive]]`. Not a GC point.
    pub(crate) fn clear(&self) {
        self.set(UNDEFINED);
    }

    /// The number of distinct values in `[[KeptAlive]]` (tests).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        let set = self.get();
        if set == UNDEFINED {
            0
        } else {
            table::live_count(table::table_of(set))
        }
    }

    /// Puts the new FinalizationRegistry `fr` first in the isolate's chain (a link WEAK_CELL
    /// whose TARGET holds it weakly). GC point: `fr` lives in a Rust local (CSS).
    pub(crate) fn add_registry(&self, rt: &Runtime, fr: u64) {
        debug_assert_eq!(
            object::slot_class(fr),
            Some(ObjectClass::FinalizationRegistry.id())
        );
        let link = new_cell(rt, fr, UNDEFINED, UNDEFINED, self.roots.registries.get());
        // A root slot (E3). The head read above stays the head: the allocation runs no JS.
        self.roots.registries.set(link);
    }

    /// The first link of the registry chain, or undefined.
    pub(crate) fn registries(&self) -> u64 {
        self.roots.registries.get()
    }

    /// True if a GC since the last call saw a dead registry or an empty target (and clears
    /// the flag). Not a GC point.
    pub(crate) fn take_pending(&self) -> bool {
        self.roots.pending.replace(false)
    }

    /// Sets the flag again (a cleanup hook that stopped at a throw leaves work behind).
    pub(crate) fn set_pending(&self) {
        self.roots.pending.set(true);
    }
}

/// Drops from the registry chain every link whose registry died (TARGET undefined).
/// Barriered stores. Not a GC point.
pub(crate) fn unlink_dead_registries(rt: &Runtime) {
    let k = &rt.kept_objects;
    let mut head = k.registries();
    while head != UNDEFINED && target(head) == UNDEFINED {
        head = next(head);
    }
    k.roots.registries.set(head);
    let mut prev = head;
    while prev != UNDEFINED {
        let mut n = next(prev);
        while n != UNDEFINED && target(n) == UNDEFINED {
            n = next(n);
        }
        if n != next(prev) {
            set_next(rt, prev, n);
        }
        prev = n;
    }
}

/// Moves every cell of the registry `fr` whose target is empty from CELLS to the end of
/// CLEARED, in CELLS order; returns how many. Barriered stores. Not a GC point.
pub(crate) fn collect_cleared(rt: &Runtime, fr: u64) -> usize {
    // Find the tail of CLEARED (usually empty).
    if !is_registry(fr) {
        return 0;
    }
    let mut tail = object::slot_get(fr, registry::CLEARED);
    if !is_weak_cell(tail) {
        tail = UNDEFINED;
    } else {
        while is_weak_cell(next(tail)) {
            tail = next(tail);
        }
    }
    let mut moved = 0;
    let mut prev = UNDEFINED;
    let mut c = object::slot_get(fr, registry::CELLS);
    while is_weak_cell(c) {
        let n = next(c);
        if target(c) == UNDEFINED {
            // Unlink from CELLS.
            if prev == UNDEFINED {
                object::slot_set(rt, fr, registry::CELLS, n);
            } else {
                set_next(rt, prev, n);
            }
            // Append to CLEARED.
            set_next(rt, c, UNDEFINED);
            if tail == UNDEFINED {
                object::slot_set(rt, fr, registry::CLEARED, c);
            } else {
                set_next(rt, tail, c);
            }
            tail = c;
            moved += 1;
        } else {
            prev = c;
        }
        c = n;
    }
    moved
}

/// Removes from the list in slot `list` of the registry `fr` every cell whose token is
/// `tok` (an object or an unregistered symbol: SameValue is identity); returns how many.
/// Barriered stores. Not a GC point.
pub(crate) fn remove_token(rt: &Runtime, fr: u64, list: usize, tok: u64) -> usize {
    debug_assert!(tok != UNDEFINED);
    let mut removed = 0;
    let mut prev = UNDEFINED;
    let mut c = object::slot_get(fr, list);
    while is_weak_cell(c) {
        let n = next(c);
        if token(c) == tok {
            if prev == UNDEFINED {
                object::slot_set(rt, fr, list, n);
            } else {
                set_next(rt, prev, n);
            }
            removed += 1;
        } else {
            prev = c;
        }
        c = n;
    }
    removed
}
