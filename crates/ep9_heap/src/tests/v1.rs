//! V1, the remembered-set verifier (H11, 03-gc §7; plan v9 §6.2 GC4-V1), and the negative
//! controls of 03-gc §7 that V1 must catch.
//!
//! ```text
//! V1 (verify.rs check_old_cells), before the mark of every minor GC with verify_after_gc or
//! GC-stress; an error panics "heap verifier failed before a minor GC (V1)", nothing freed:
//!   old (marked) cell, not GREY       -> no young (unmarked) cell in a slot or in word0
//!   old large object                  -> a young cell only in a dirty card's slot (word0:
//!                                        card 0; slots outside visit_slots: the dirty byte)
//!   ages are colours                  -> young = WHITE; old = BLACK or GREY; large never GREY
//!   remembered lists and cards        -> each GREY cell listed once; a dirty card sets the
//!                                        dirty byte
//!
//! negative controls (a store site that breaks the barrier rule; StoreSite below):
//!   DropOneIn(n)       every nth barriered store is a plain store
//!   OldOnlyFilter      the filter reads the OLD value (H11-Q4: unsound for S1 and S2)
//!   ElideAfterEscape   E2 keeps a host "fresh" after it escaped
//! each one: V1 fails (deterministic scenario + the random mutator of stress.rs);
//! the same sequence with the control off: V1 passes
//! ```
//!
//! The other two controls of 03-gc §7 ("no root rescan", "no card rescan at termination")
//! break the S2 fixpoint, not the S1 logs: V2 and V3 (month 5) must catch those.
//!
//! **Elide after escape in S1.** A host that saw no GC since its allocation is young, so a
//! store into it needs no log in S1, escaped or not (a companion check below). The model
//! test's `escape` scenario loses an object because the concurrent marker reaches N through
//! B. In S1 the minor GC is that marker: it reaches N through B's log and makes N old, and
//! the store that the faulty region still elides is then an old-to-young store without the
//! barrier. So the S1 control is "elide after the escape, across the GC that followed it".

use std::panic::{AssertUnwindSafe, catch_unwind};

use ep9_value::{ShapeId, Value, Word0};

use super::*;
use crate::layout::CARD_SIZE;
use crate::{Space, gc_state, set_shape};

/// A negative control of 03-gc §7: how a store site breaks the barrier rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    /// The decided barrier (`Slot::set`). E2 elides a store only into a host allocated with
    /// no GC point since and not escaped.
    Off,
    /// Every nth barriered store is a plain store (`Slot::init`).
    DropOneIn(u64),
    /// The barrier fires when the slot's OLD value is a cell (and the host is BLACK), not the
    /// new one.
    OldOnlyFilter,
    /// E2 elides every store into a host allocated in the current region, also after the
    /// host escaped (and across the GC points after that).
    ElideAfterEscape,
}

/// A store site with a [`Control`]. The elision decision (E2) belongs to the caller, which
/// knows the region; this site does the barriered stores.
pub(super) struct StoreSite {
    pub(super) control: Control,
    stores: u64,
    /// Stores the control turned into plain stores.
    pub(super) dropped: u64,
}

impl StoreSite {
    pub(super) fn new(control: Control) -> StoreSite {
        StoreSite {
            control,
            stores: 0,
            dropped: 0,
        }
    }

    /// A barriered store of `v` into `slot` of the cell whose word0 is `host`.
    pub(super) fn store(&mut self, heap: &Heap, host: &Word0, slot: &Slot, v: Value) {
        let state = heap.barrier();
        match self.control {
            Control::DropOneIn(n) => {
                self.stores += 1;
                if self.stores.is_multiple_of(n) {
                    self.dropped += 1;
                    slot.init(v);
                } else {
                    slot.set(host, v, state);
                }
            }
            Control::OldOnlyFilter => {
                if crate::barrier::is_cell_bits(slot.bits()) && host.gc_state() <= state.threshold()
                {
                    crate::barrier::barrier_slow(host, slot, v);
                } else {
                    if crate::barrier::is_cell_bits(v.to_bits()) {
                        self.dropped += 1;
                    }
                    slot.init(v);
                }
            }
            Control::Off | Control::ElideAfterEscape => slot.set(host, v, state),
        }
    }

    /// [`StoreSite::store`] into slot `i` of the node `host`.
    pub(super) fn store_node(&mut self, heap: &Heap, host: CellPtr, i: usize, v: Value) {
        // SAFETY: `host` is a live node (caller).
        self.store(heap, unsafe { host.header() }, slot(host, i), v);
    }
}

/// A shape-space kind: 24 B, no references.
const SHAPE: KindId = KindId::new(40).unwrap();

fn no_trace(_: CellPtr, _: &mut Tracer<'_>) {}

/// A generational test heap. `checked`: `verify_after_gc`.
fn gen_heap(checked: bool, tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    test_heap(|c| {
        c.generational = true;
        c.verify_after_gc = checked;
        tweak(c);
    })
}

fn major(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(!heap.stats().last_minor);
}

fn minor(heap: &Heap) {
    heap.collect_minor_with_words(&[]);
    assert!(heap.stats().last_minor);
}

fn old(heap: &Heap, c: CellPtr) -> bool {
    crate::trace::is_marked(&heap.inner.borrow().mem, c.addr())
}

fn state(c: CellPtr) -> u8 {
    // SAFETY: callers pass live cells.
    unsafe { c.header() }.gc_state()
}

fn v1_runs(heap: &Heap) -> u64 {
    heap.inner.borrow().v1_runs
}

/// A minor GC that V1 must stop: returns its panic text, and checks that no GC ran.
fn minor_fails_v1(heap: &Heap) -> String {
    let collections = heap.stats().collections;
    let r = catch_unwind(AssertUnwindSafe(|| minor(heap)));
    let msg = r.expect_err("V1 must fail");
    let msg = msg
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| "a panic without a text".into());
    assert!(
        msg.contains("heap verifier failed before a minor GC (V1)"),
        "{msg}"
    );
    assert_eq!(heap.stats().collections, collections, "V1 stops the GC");
    msg
}

/// V1 runs before every minor GC with GC-stress (also without `verify_after_gc`) and with
/// `verify_after_gc`; never before a major GC, and not without either.
#[test]
fn v1_runs_before_every_minor_gc_under_gc_stress() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |c| {
            c.gc_stress = 1;
            c.stress_major_every = 8;
        });
        let root = new_node(&heap, 0, 1);
        let g = heap.new_global(root.to_value());
        for i in 1..=24 {
            let c = new_node(&heap, i, 1);
            store(
                &heap,
                CellPtr::from_value(heap.global(g)).unwrap(),
                0,
                c.to_value(),
            );
            heap.set_global(g, c.to_value());
        }
        let s = heap.stats();
        assert!(s.minor_collections >= 20, "{s:?}");
        assert!(
            s.collections > s.minor_collections,
            "a major one too: {s:?}"
        );
        assert_eq!(v1_runs(&heap), s.minor_collections, "checked {checked}");
        heap.drop_global(g);
    }
    let heap = gen_heap(false, |_| {});
    let g = heap.new_global(new_node(&heap, 1, 0).to_value());
    minor(&heap);
    minor(&heap);
    assert_eq!(v1_runs(&heap), 0, "no GC-stress, no verify: no V1");
    heap.set_verify_after_gc(true);
    minor(&heap);
    major(&heap);
    assert_eq!(v1_runs(&heap), 1, "one minor GC with the verify");
    heap.drop_global(g);
}

/// Ages are colours: V1 names a young cell that is not WHITE and an old large object that is
/// GREY (`Heap::verify` names only the old WHITE cell; tests/s1.rs). Each case is undone and
/// the next minor GC passes.
#[test]
fn v1_checks_each_age_against_its_colour() {
    let heap = gen_heap(true, |_| {});
    let root = new_node(&heap, 1, 2);
    let big = new_node(&heap, 2, 1100);
    let gs = [root, big].map(|c| heap.new_global(c.to_value()));
    major(&heap);
    let young = new_node(&heap, 3, 0);
    store(&heap, root, 0, young.to_value());
    assert!(!old(&heap, young) && state(young) == gc_state::WHITE);

    for (planted, text) in [
        (gc_state::BLACK, "not marked (young) but BLACK"),
        (gc_state::GREY, "not marked (young) but GREY"),
    ] {
        // SAFETY: `young` is live (reachable from `root`); restored below.
        unsafe { young.header() }.set_gc_state(planted);
        let msg = minor_fails_v1(&heap);
        assert!(msg.contains(text), "{msg}");
        // SAFETY: as above.
        unsafe { young.header() }.set_gc_state(gc_state::WHITE);
    }
    assert!(old(&heap, big) && state(big) == gc_state::BLACK);
    // SAFETY: `big` is rooted; restored below.
    unsafe { big.header() }.set_gc_state(gc_state::GREY);
    let msg = minor_fails_v1(&heap);
    assert!(msg.contains("a large object is never GREY"), "{msg}");
    // SAFETY: as above.
    unsafe { big.header() }.set_gc_state(gc_state::BLACK);

    let runs = v1_runs(&heap);
    minor(&heap);
    assert_eq!(v1_runs(&heap), runs + 1);
    assert!(old(&heap, young), "kept and promoted");
    for g in gs {
        heap.drop_global(g);
    }
}

/// An old large object: a young cell in a slot needs that slot's card dirty, and a young
/// word0 shape needs card 0 dirty (a shape install logs the cell start). A dirty card
/// elsewhere does not cover either.
#[test]
fn v1_reads_the_card_of_each_young_reference_of_a_large_object() {
    let heap = gen_heap(true, |c| c.shape_space_bytes = 1 << 20);
    heap.register_kind_in(
        SHAPE,
        CellKind {
            name: "test-v1-shape",
            size: CellSize::Fixed(24),
            trace: no_trace,
        },
        Space::Shape,
    )
    .unwrap();
    let big = new_node(&heap, 1, 1100);
    let g = heap.new_global(big.to_value());
    major(&heap);
    let card = |i: usize| (NODE_HEADER + 8 * i) / CARD_SIZE;
    assert!(card(1000) > 0 && card(500) > 0 && card(500) != card(1000));

    // A slot: a dirty card 15 does not cover a young cell in card 7.
    let mut site = StoreSite::new(Control::Off);
    let a = new_node(&heap, 2, 0);
    let b = new_node(&heap, 3, 0);
    site.store_node(&heap, big, 1000, a.to_value());
    slot(big, 500).init(b.to_value()); // no barrier
    let msg = minor_fails_v1(&heap);
    assert!(msg.contains("not in the S1 logs"), "{msg}");
    site.store_node(&heap, big, 500, b.to_value()); // card 7 dirty now
    minor(&heap);
    assert!(old(&heap, a) && old(&heap, b));

    // word0: card 15 dirty, card 0 clean, a young shape installed without the barrier.
    site.store_node(&heap, big, 1000, new_node(&heap, 4, 0).to_value());
    let shape = heap.alloc_shape(SHAPE, 24).unwrap();
    let id: ShapeId = heap.shape_id_of(shape);
    // SAFETY: `big` is rooted.
    unsafe { big.header() }.set_shape_id(id); // no barrier
    let msg = minor_fails_v1(&heap);
    assert!(msg.contains("not in the S1 logs"), "{msg}");
    // The shape barrier logs card 0, and the minor GC keeps the shape.
    // SAFETY: as above.
    set_shape(unsafe { big.header() }, id, heap.barrier());
    minor(&heap);
    assert!(heap.is_cell(shape.addr()) && old(&heap, shape));
    heap.verify().expect("verifier");
    heap.drop_global(g);
}

/// Control "drop 1 in N barriers" (N = 2): the second barriered store is a plain store into
/// an old host. V1 fails, small host and large host alike; with the control off the same
/// stores pass and every young cell lives.
#[test]
fn drop_one_in_n_barriers_fails_v1() {
    for control in [Control::Off, Control::DropOneIn(2)] {
        for large in [false, true] {
            let heap = gen_heap(true, |_| {});
            let n = if large { 1100 } else { 2 };
            let a = new_node(&heap, 1, n);
            let b = new_node(&heap, 2, n);
            let gs = [a, b].map(|c| heap.new_global(c.to_value()));
            major(&heap);
            let (y1, y2) = (new_node(&heap, 3, 0), new_node(&heap, 4, 0));
            let mut site = StoreSite::new(control);
            site.store_node(&heap, a, n - 1, y1.to_value());
            site.store_node(&heap, b, n - 1, y2.to_value());
            if control == Control::Off {
                minor(&heap);
                assert!(old(&heap, y1) && old(&heap, y2), "large {large}");
                heap.verify().expect("verifier");
            } else {
                assert_eq!(site.dropped, 1);
                let msg = minor_fails_v1(&heap);
                assert!(msg.contains("not in the S1 logs"), "large {large}: {msg}");
                assert!(heap.is_cell(y2.addr()), "nothing freed");
            }
            for g in gs {
                heap.drop_global(g);
            }
        }
    }
}

/// Control "old-only filter": the barrier fires only when the slot held a cell. A store of a
/// young cell over `undefined` is then not logged, and V1 fails; a store over a cell is
/// logged (the filter does run). With the control off both stores are logged.
#[test]
fn old_only_filter_fails_v1() {
    for control in [Control::Off, Control::OldOnlyFilter] {
        let heap = gen_heap(true, |_| {});
        let host = new_node(&heap, 1, 2);
        let other = new_node(&heap, 2, 2);
        let o = new_node(&heap, 3, 0);
        store(&heap, other, 0, o.to_value());
        let gs = [host, other].map(|c| heap.new_global(c.to_value()));
        major(&heap);
        let mut site = StoreSite::new(control);
        // `other` slot 0 holds the old cell `o`: both filters fire.
        let y1 = new_node(&heap, 4, 0);
        site.store_node(&heap, other, 0, y1.to_value());
        assert_eq!(state(other), gc_state::GREY, "{control:?}");
        // `host` slot 0 holds `undefined`: only the decided filter fires.
        let y2 = new_node(&heap, 5, 0);
        site.store_node(&heap, host, 0, y2.to_value());
        if control == Control::Off {
            assert_eq!(state(host), gc_state::GREY);
            minor(&heap);
            assert!(old(&heap, y1) && old(&heap, y2));
            heap.verify().expect("verifier");
        } else {
            assert_eq!(state(host), gc_state::BLACK, "not logged");
            let msg = minor_fails_v1(&heap);
            assert!(msg.contains("not in the S1 logs"), "{msg}");
        }
        for g in gs {
            heap.drop_global(g);
        }
    }
}

/// Control "elide after escape" (the model's `escape` scenario in S1; module doc): N escapes
/// into the old B, a minor GC reaches N through B's log and makes it old, then the faulty
/// region still elides a store of the young C into N. V1 fails. With the control off the
/// region ends N's freshness at the escape, so the store takes the barrier. Without a GC
/// between the escape and the store, N is still young and the elided store is safe in S1.
#[test]
fn elide_after_escape_fails_v1() {
    for (control, gc_between) in [
        (Control::Off, true),
        (Control::ElideAfterEscape, true),
        (Control::ElideAfterEscape, false),
    ] {
        let heap = gen_heap(true, |_| {});
        let b = new_node(&heap, 1, 1);
        let g = heap.new_global(b.to_value());
        major(&heap);
        let n = new_node(&heap, 2, 1); // allocated in the region: fresh
        let mut site = StoreSite::new(control);
        site.store_node(&heap, b, 0, n.to_value()); // N escapes into B (B logged)
        if gc_between {
            minor(&heap);
            assert!(old(&heap, n) && state(n) == gc_state::BLACK);
        }
        let c = new_node(&heap, 3, 0);
        let fresh = control == Control::ElideAfterEscape; // the region's E2 answer for N
        if fresh {
            slot(n, 0).init(c.to_value()); // elided
        } else {
            site.store_node(&heap, n, 0, c.to_value());
        }
        if control == Control::ElideAfterEscape && gc_between {
            let msg = minor_fails_v1(&heap);
            assert!(msg.contains("not in the S1 logs"), "{msg}");
        } else {
            minor(&heap);
            assert!(old(&heap, c), "{control:?}, GC between: {gc_between}");
            heap.verify().expect("verifier");
        }
        heap.drop_global(g);
    }
}

/// The random mutator of stress.rs under generational GC-stress (1-3) with each control: V1
/// fails (and nothing else fails first: the run's model would see a freed reachable cell);
/// with the control off the same seeds pass, E2 elisions included.
#[test]
#[cfg_attr(miri, ignore = "thousands of GCs; the scenarios above run under Miri")]
fn each_negative_control_makes_v1_fail_in_the_random_mutator() {
    use super::stress::run_controlled;
    for seed in 0..6u64 {
        let stress = 1 + (seed % 3) as u32;
        let seed = 0x0000_5EED_0000_00A1 + seed;
        let off = run_controlled(seed, 1500, stress, Control::Off, true)
            .unwrap_or_else(|e| panic!("seed {seed:#x} stress {stress}, control off: {e}"));
        assert!(
            off.minor > 0 && off.v1_runs == off.minor && off.elided > 0,
            "seed {seed:#x}: {off:?}"
        );
        for control in [
            Control::DropOneIn(16),
            Control::OldOnlyFilter,
            Control::ElideAfterEscape,
        ] {
            let msg = run_controlled(seed, 1500, stress, control, true)
                .err()
                .unwrap_or_else(|| panic!("seed {seed:#x} stress {stress}: {control:?} passed"));
            assert!(
                msg.contains("heap verifier failed before a minor GC (V1)")
                    && msg.contains("not in the S1 logs"),
                "seed {seed:#x} stress {stress} {control:?}: {msg}"
            );
        }
    }
}
