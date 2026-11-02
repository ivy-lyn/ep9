//! S1 step 2 (plan v9 §5.1, GFF-S1A part 2; P2-ORCH-11): the minor GC, the major GC, the
//! policy, generational GC-stress and the verifier's generational invariant.
//!
//! ```text
//! generational heap, after a GC: survivors are marked (old) and BLACK; new cells are young
//!   minor GC: marks from the roots, stops at old cells, traces the S1 logs (GREY small hosts,
//!             dirty cards of old large objects), runs the ephemeron fixpoint and the weak
//!             parts; frees only unmarked (young) cells; empties the logs
//!   major GC: clears every mark, marks everything, empties the logs
//!   verify (minor): before the mark, no old cell outside the logs holds a young cell
//! ```
//!
//! Most cases run twice: a plain mark and a checked one (`verify_after_gc`).

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use ep9_value::{ShapeId, Value};

use super::*;
use crate::layout::{CARD_SIZE, PAGE_SIZE};
use crate::{Space, gc_state, set_shape};

/// An ephemeron box: word0, a key slot at +8, a value slot at +16 (one WeakMap entry).
const EPHBOX: KindId = KindId::new(30).unwrap();
/// 24 B: word0, a strong slot at +8, a raw word; its trace function panics on demand.
const PANICKY: KindId = KindId::new(31).unwrap();
/// A shape-space kind: 24 B, no references.
const SHAPE: KindId = KindId::new(32).unwrap();

fn ephbox_trace(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: a live EPHBOX has Value slots at +8 and +16.
    unsafe { t.visit_ephemeron(c.slot(8), c.slot(16)) };
}

thread_local! {
    static PANIC_NOW: Cell<bool> = const { Cell::new(false) };
}

fn panicky_trace(c: CellPtr, t: &mut Tracer<'_>) {
    if PANIC_NOW.get() {
        std::panic::resume_unwind(Box::new("test callback panic"));
    }
    // SAFETY: a live PANICKY cell has a Value slot at +8.
    t.visit(unsafe { c.slot(8) });
}

fn no_trace(_: CellPtr, _: &mut Tracer<'_>) {}

/// A generational test heap with the kinds of this file.
fn gen_heap(checked: bool, tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    let heap = test_heap(|c| {
        c.generational = true;
        c.verify_after_gc = checked;
        tweak(c);
    });
    heap.register_kind(
        EPHBOX,
        CellKind {
            name: "test-ephbox",
            size: CellSize::Fixed(24),
            trace: ephbox_trace,
        },
    )
    .unwrap();
    heap.register_kind(
        PANICKY,
        CellKind {
            name: "test-minor-panicky",
            size: CellSize::Fixed(24),
            trace: panicky_trace,
        },
    )
    .unwrap();
    heap
}

/// A major GC with precise roots only.
fn major(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(!heap.stats().last_minor);
}

/// A minor GC with precise roots only.
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

/// Stores `v` into the slot at `off` of `host` with the barrier.
fn store_at(heap: &Heap, host: CellPtr, off: usize, v: Value) {
    // SAFETY: `host` is live and `off` is one of its Value slots (callers).
    unsafe { host.slot(off) }.set(unsafe { host.header() }, v, heap.barrier());
}

/// Stores `v` into slot `i` of the node `host` with NO barrier: the store a missing barrier
/// makes (negative controls only).
fn store_without_barrier(host: CellPtr, i: usize, v: Value) {
    slot(host, i).init(v);
}

#[test]
fn a_minor_gc_frees_dead_young_cells_and_keeps_old_ones() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |_| {});
        let old_root = new_node(&heap, 1, 1);
        let old_dead = new_node(&heap, 2, 0);
        let g = heap.new_global(old_root.to_value());
        let g_dead = heap.new_global(old_dead.to_value());
        major(&heap);
        assert!(old(&heap, old_root) && old(&heap, old_dead));
        heap.drop_global(g_dead);

        let young_dead = new_node(&heap, 3, 0).addr();
        let young_kept = new_node(&heap, 4, 0);
        let g_young = heap.new_global(young_kept.to_value());
        minor(&heap);
        assert!(
            !heap.is_cell(young_dead),
            "a young cell nothing reaches dies"
        );
        assert!(heap.is_cell(young_kept.addr()) && old(&heap, young_kept));
        assert_eq!(state(young_kept), gc_state::BLACK, "a survivor is BLACK");
        assert!(
            heap.is_cell(old_dead.addr()),
            "an old cell lives until the next major GC"
        );
        assert_eq!(heap.stats().minor_collections, 1);
        heap.verify().expect("verifier");

        major(&heap);
        assert!(!heap.is_cell(old_dead.addr()), "the major GC frees it");
        assert!(heap.is_cell(young_kept.addr()));
        assert_eq!(heap.stats().minor_collections, 1);
        assert_eq!(heap.stats().collections, 3);
        heap.verify().expect("verifier");
        heap.drop_global(g);
        heap.drop_global(g_young);
    }
}

/// An old cell reached only through an old cell stops the minor mark: it is not traced, and
/// its young children (stored with the barrier, so the parent is logged) still live.
#[test]
fn a_remembered_host_keeps_its_young_cell() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |_| {});
        let host = new_node(&heap, 1, 2);
        let g = heap.new_global(host.to_value());
        major(&heap);
        let young = new_node(&heap, 2, 1);
        let grandchild = new_node(&heap, 3, 0);
        store(&heap, young, 0, grandchild.to_value()); // young host: no log
        store(&heap, host, 1, young.to_value()); // old host: logged
        assert_eq!(state(host), gc_state::GREY);
        minor(&heap);
        assert!(heap.is_cell(young.addr()) && heap.is_cell(grandchild.addr()));
        assert!(old(&heap, young) && old(&heap, grandchild), "promoted");
        assert_eq!(state(host), gc_state::BLACK, "traced again, BLACK");
        assert_eq!(heap.stats().last_marked_cells, 3, "host + 2 young cells");
        heap.verify()
            .expect("the logs are empty, the invariant holds");

        // The promoted cell is old: a store into it logs it.
        let newer = new_node(&heap, 4, 0);
        store(&heap, young, 0, newer.to_value());
        assert_eq!(state(young), gc_state::GREY);
        minor(&heap);
        assert!(heap.is_cell(newer.addr()) && heap.is_cell(grandchild.addr()));
        heap.verify().expect("verifier");
        heap.drop_global(g);
    }
}

/// An old large object: the minor GC traces only the slots of its dirty cards. A young cell
/// stored with the barrier lives; one planted in a clean card with no barrier is not traced
/// and dies (only safe because the test clears the slot before anything reads it).
#[test]
fn a_minor_gc_traces_only_the_dirty_cards() {
    let heap = gen_heap(false, |_| {});
    let big = new_node(&heap, 1, 1100);
    assert_eq!(heap.stats().large_cells, 1);
    let g = heap.new_global(big.to_value());
    major(&heap);
    let logged = new_node(&heap, 2, 0);
    let planted = new_node(&heap, 3, 0);
    store(&heap, big, 1000, logged.to_value()); // card 15, dirty
    let planted_slot = 10; // card 0
    assert_ne!(
        (NODE_HEADER + 8 * planted_slot) / CARD_SIZE,
        (NODE_HEADER + 8 * 1000) / CARD_SIZE
    );
    store_without_barrier(big, planted_slot, planted.to_value());
    let planted_addr = planted.addr();
    minor(&heap);
    assert!(heap.is_cell(logged.addr()), "the dirty card was traced");
    assert!(!heap.is_cell(planted_addr), "the clean card was not");
    store_without_barrier(big, planted_slot, Value::UNDEFINED);
    assert_eq!(
        heap.stats().last_marked_cells,
        2,
        "the large object + the logged cell"
    );
    heap.verify().expect("verifier");
    // A store into card 0 now dirties it: the next minor GC keeps its young cell.
    let young = new_node(&heap, 4, 0);
    store(&heap, big, planted_slot, young.to_value());
    minor(&heap);
    assert!(heap.is_cell(young.addr()));
    heap.verify().expect("verifier");
    heap.drop_global(g);
}

/// The generational invariant (verify.rs): an old host outside the S1 logs that holds a young
/// cell is a store without the barrier. `Heap::verify` reports it, and so does V1 before the
/// mark of a minor GC, which then panics before it frees the young cell.
#[test]
fn the_verifier_reports_a_store_without_the_barrier() {
    let heap = gen_heap(false, |_| {});
    let host = new_node(&heap, 1, 1);
    let big = new_node(&heap, 2, 1100);
    let gs = [host, big].map(|c| heap.new_global(c.to_value()));
    major(&heap);
    heap.verify().expect("clean");
    let young = new_node(&heap, 3, 0);
    let gy = heap.new_global(young.to_value());

    store_without_barrier(host, 0, young.to_value());
    let e = heap.verify().expect_err("a small old host");
    assert!(e.to_string().contains("not in the S1 logs"), "{e}");
    store_without_barrier(host, 0, Value::UNDEFINED);
    heap.verify().expect("restored");

    // A large host: a clean card is checked, a dirty one may hold young cells.
    store_without_barrier(big, 10, young.to_value());
    let e = heap
        .verify()
        .expect_err("an old large object, no dirty card");
    assert!(e.to_string().contains("not in the S1 logs"), "{e}");
    store(&heap, big, 1000, young.to_value()); // card 15 dirty
    let e = heap.verify().expect_err("card 0 is still clean");
    assert!(e.to_string().contains("not in the S1 logs"), "{e}");
    store(&heap, big, 11, young.to_value()); // card 0 dirty
    heap.verify().expect("both cards are dirty");

    // The word0 shape is an edge too (no shape space here: checked in the shape case).
    // V1 finds the same break before the minor GC's mark, and nothing is freed.
    heap.set_verify_after_gc(true);
    store_without_barrier(host, 0, young.to_value());
    heap.drop_global(gy);
    let collections = heap.stats().collections;
    let r = catch_unwind(AssertUnwindSafe(|| minor(&heap)));
    let msg = r.expect_err("the verify must fail");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        msg.contains("heap verifier failed before a minor GC (V1)")
            && msg.contains("not in the S1 logs"),
        "{msg}"
    );
    assert_eq!(heap.stats().collections, collections, "no GC ran");
    assert!(heap.is_cell(young.addr()), "the young cell was not freed");
    for g in gs {
        heap.drop_global(g);
    }
}

/// Weak slots, weak handles and weak sources in a minor GC: a young cell that died is
/// cleared; an old cell is not (it is live until a major GC finds it unreachable).
#[test]
fn a_minor_gc_clears_weak_references_to_dead_young_cells() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |_| {});
        let wb = heap.alloc(WEAKBOX, 24).unwrap();
        let g = heap.new_global(wb.to_value());
        let old_target = new_node(&heap, 1, 0);
        let g_old = heap.new_global(old_target.to_value());
        major(&heap);
        heap.drop_global(g_old);

        let calls = Rc::new(Cell::new((0u32, 0u32)));
        let young = new_node(&heap, 2, 0);
        let (young_v, old_v) = (young.to_value(), old_target.to_value());
        let seen = Rc::clone(&calls);
        heap.add_weak_source(move |w| {
            let (n, dead) = seen.get();
            let dead = dead + u32::from(!w.is_live(young_v)) + u32::from(!w.is_live(old_v));
            seen.set((n + 1, dead));
        });
        let wh_young = heap.new_weak(young.to_value());
        let wh_old = heap.new_weak(old_target.to_value());
        store_at(&heap, wb, 8, young.to_value()); // the weak slot; logs the box
        assert_eq!(state(wb), gc_state::GREY);
        minor(&heap);
        assert_eq!(
            calls.get(),
            (1, 1),
            "the weak source ran; only the young cell is dead"
        );
        // SAFETY: `wb` is rooted; +8 is its weak slot.
        let weak_slot = unsafe { wb.slot(8) }.bits();
        assert_eq!(weak_slot, Value::UNDEFINED.to_bits(), "cleared");
        assert!(heap.weak(wh_young).is_none());
        assert_eq!(heap.weak(wh_old).map(Value::to_bits), Some(old_v.to_bits()));
        assert!(heap.is_cell(old_target.addr()));

        major(&heap);
        assert!(heap.weak(wh_old).is_none(), "the major GC clears it");
        heap.drop_global(g);
    }
}

/// Ephemerons in a minor GC (P2-ORCH-11): the fixpoint covers young keys; an old key counts
/// as live; a chain key1 -> value1 -> key2 resolves.
#[test]
fn a_minor_gc_runs_the_ephemeron_fixpoint_on_young_keys() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |_| {});
        let boxes: Vec<CellPtr> = (0..4).map(|_| heap.alloc(EPHBOX, 24).unwrap()).collect();
        let gs: Vec<_> = boxes
            .iter()
            .map(|b| heap.new_global(b.to_value()))
            .collect();
        let old_key = new_node(&heap, 1, 0);
        let g_old_key = heap.new_global(old_key.to_value());
        major(&heap);
        heap.drop_global(g_old_key); // old and unreachable: still counts as live

        // 0: a rooted young key keeps its young value (v0b, made below).
        let k0 = new_node(&heap, 10, 0);
        let g_k0 = heap.new_global(k0.to_value());
        // 1: an unreachable young key: key and value die.
        let k1 = new_node(&heap, 12, 0);
        let v1 = new_node(&heap, 13, 0);
        // 2: an old key keeps its young value.
        let v2 = new_node(&heap, 14, 0);
        // 3: a chain: box 3's key is reached only through v0 (box 0's value).
        let k3 = new_node(&heap, 15, 0);
        let v3 = new_node(&heap, 16, 0);
        let v0b = new_node(&heap, 17, 1);
        store(&heap, v0b, 0, k3.to_value());
        for (b, k, v) in [
            (boxes[0], k0.to_value(), v0b.to_value()),
            (boxes[1], k1.to_value(), v1.to_value()),
            (boxes[2], old_key.to_value(), v2.to_value()),
            (boxes[3], k3.to_value(), v3.to_value()),
        ] {
            store_at(&heap, b, 8, k);
            store_at(&heap, b, 16, v);
        }
        let (k1a, v1a) = (k1.addr(), v1.addr());
        minor(&heap);
        // SAFETY: every box is rooted; +8 and +16 are its slots.
        let entry = |b: CellPtr| unsafe { (b.slot(8).bits(), b.slot(16).bits()) };
        let undef = Value::UNDEFINED.to_bits();
        assert_eq!(entry(boxes[0]).1, v0b.to_value().to_bits());
        assert_eq!(entry(boxes[1]), (undef, undef));
        assert!(!heap.is_cell(k1a) && !heap.is_cell(v1a));
        assert_eq!(entry(boxes[2]).1, v2.to_value().to_bits());
        assert!(heap.is_cell(v2.addr()));
        assert_eq!(entry(boxes[3]).1, v3.to_value().to_bits());
        assert!(heap.is_cell(k3.addr()) && heap.is_cell(v3.addr()));
        heap.verify().expect("verifier");

        // The old key dies in a major GC, and its value with it.
        major(&heap);
        assert_eq!(entry(boxes[2]), (undef, undef));
        heap.drop_global(g_k0);
        for g in gs {
            heap.drop_global(g);
        }
    }
}

/// Generational GC-stress (V9-D27): a GC before every allocation, every 64th one major.
#[test]
fn gc_stress_runs_a_major_gc_every_64th_collection() {
    let heap = gen_heap(true, |c| c.gc_stress = 1);
    let root = new_node(&heap, 0, 1);
    let g = heap.new_global(root.to_value());
    let before = heap.stats();
    for i in 1..=130 {
        let c = new_node(&heap, i, 1);
        store(
            &heap,
            CellPtr::from_value(heap.global(g)).unwrap(),
            0,
            c.to_value(),
        );
        heap.set_global(g, c.to_value());
    }
    let after = heap.stats();
    let gcs = after.collections - before.collections;
    let minors = after.minor_collections - before.minor_collections;
    assert_eq!(gcs, 130);
    // Stress GCs 1..=131 counted from the heap's start (the root's allocation was the 1st).
    assert_eq!(gcs - minors, 2, "the 64th and the 128th are major");
    heap.drop_global(g);

    let every = gen_heap(false, |c| {
        c.gc_stress = 1;
        c.stress_major_every = 1;
    });
    for i in 0..10 {
        new_node(&every, i, 0);
    }
    assert_eq!(every.stats().minor_collections, 0, "1: every one is major");
    let s0 = test_heap(|c| c.gc_stress = 1);
    for i in 0..10 {
        new_node(&s0, i, 0);
    }
    assert_eq!(s0.stats().minor_collections, 0, "S0: every GC is full");
}

/// The budget policy: budget GCs are minor while the old bytes stay below twice the live
/// bytes of the last major GC; then one is major.
#[test]
#[cfg_attr(
    miri,
    ignore = "20,000 allocations; the stress model covers the policy under Miri"
)]
fn the_budget_policy_runs_minor_gcs_then_a_major_one() {
    let heap = gen_heap(false, |c| c.initial_budget = PAGE_SIZE);
    // A rooted list that grows: its cells become old, so the old bytes grow.
    let head = new_node(&heap, 0, 1);
    let g = heap.new_global(head.to_value());
    let mut tail = head;
    let mut kinds = Vec::new();
    let mut seen = 0;
    for i in 1..20_000u32 {
        let c = new_node(&heap, i, 30); // 256 B
        if i % 2 == 0 {
            store(&heap, tail, 0, c.to_value());
            tail = c;
        }
        let st = heap.stats();
        if st.collections != seen {
            seen = st.collections;
            kinds.push(st.last_minor);
        }
    }
    let minors = kinds.iter().filter(|&&m| m).count();
    let majors = kinds.len() - minors;
    assert!(
        minors >= 4 && majors >= 1,
        "{minors} minor, {majors} major: {kinds:?}"
    );
    assert!(kinds[0], "the first budget GC is minor");
    heap.verify().expect("verifier");
    // The list is whole: node 0, then every even id.
    let mut c = head;
    let mut want = 0;
    loop {
        assert_eq!(node_id(c), want);
        let next = slot(c, 0).get();
        if next.to_bits() == Value::UNDEFINED.to_bits() {
            break;
        }
        c = CellPtr::from_value(next).unwrap();
        want += 2;
    }
    assert_eq!(want, 19_998);
    heap.drop_global(g);
}

/// A huge `initial_budget` (an embedder that never wants a budget GC) builds a heap: the
/// initial major-GC limit saturates as `after_gc` does (`2 * usize::MAX` overflowed in
/// `Heap::new`, a panic with overflow checks). S0 and S1; both GC kinds then still run.
/// Under Miri 50 cells instead of 2000 (time).
#[test]
fn a_huge_initial_budget_saturates_the_major_limit() {
    for generational in [false, true] {
        let heap = test_heap(|c| {
            c.initial_budget = usize::MAX;
            c.generational = generational;
        });
        let head = new_node(&heap, 1, 1);
        let g = heap.new_global(head.to_value());
        let cells: u32 = if cfg!(miri) { 50 } else { 2_000 };
        for i in 2..cells {
            let c = new_node(&heap, i, 30);
            if i == 2 {
                store(&heap, head, 0, c.to_value());
            }
        }
        assert_eq!(heap.stats().collections, 0, "no budget GC ({generational})");
        heap.collect_minor_with_words(&[]);
        assert_eq!(heap.stats().last_minor, generational);
        heap.collect_with_words(&[]);
        assert!(!heap.stats().last_minor);
        assert_eq!(
            node_id(CellPtr::from_value(slot(head, 0).get()).unwrap()),
            2
        );
        heap.verify().expect("verifier");
        heap.drop_global(g);
    }
}

/// `collect_minor` on an S0 heap is a full collection.
#[test]
fn collect_minor_on_an_s0_heap_is_a_full_gc() {
    let heap = test_heap(|_| {});
    let dead = new_node(&heap, 1, 0).addr();
    heap.collect_with_words(&[]);
    clear_stack();
    let young = new_node(&heap, 2, 0).addr();
    heap.collect_minor_with_words(&[]);
    assert!(!heap.stats().last_minor);
    assert!(!heap.is_cell(dead) && !heap.is_cell(young));
    assert_eq!(heap.stats().minor_collections, 0);
}

/// A conservative root keeps a young cell in a minor GC (the scan marks it).
#[test]
#[cfg_attr(miri, ignore = "no conservative scan under Miri")]
fn a_conservative_root_keeps_a_young_cell() {
    let heap = gen_heap(true, |_| {});
    major(&heap);
    let young = new_node(&heap, 1, 0);
    let dead = new_node(&heap, 2, 0).addr();
    heap.collect_minor_with_words(&[young.addr() + 8]);
    assert!(heap.stats().last_minor);
    assert!(heap.is_cell(young.addr()) && !heap.is_cell(dead));
    assert!(old(&heap, young));
}

/// A young shape installed into an old host takes the barrier, so the minor GC keeps it
/// (word0 is an edge).
#[test]
fn a_minor_gc_keeps_a_young_shape_of_an_old_host() {
    for checked in [false, true] {
        let heap = gen_heap(checked, |c| c.shape_space_bytes = 1 << 20);
        heap.register_kind_in(
            SHAPE,
            CellKind {
                name: "test-minor-shape",
                size: CellSize::Fixed(24),
                trace: no_trace,
            },
            Space::Shape,
        )
        .unwrap();
        let host = new_node(&heap, 1, 0);
        let g = heap.new_global(host.to_value());
        major(&heap);
        let shape = heap.alloc_shape(SHAPE, 24).unwrap();
        let id: ShapeId = heap.shape_id_of(shape);
        // SAFETY: `host` is rooted.
        set_shape(unsafe { host.header() }, id, heap.barrier());
        assert_eq!(state(host), gc_state::GREY);
        minor(&heap);
        assert!(heap.is_cell(shape.addr()) && old(&heap, shape));
        heap.verify().expect("verifier");
        heap.drop_global(g);
    }
}

/// A trace function panics during a minor GC: every cell counts as old, the logs stay and
/// their hosts are GREY again, the verifier is clean, and the next minor GC works.
#[test]
fn a_caught_panic_in_a_minor_gc_keeps_every_cell_and_log() {
    let heap = gen_heap(false, |_| {});
    let host = new_node(&heap, 1, 1);
    let p = heap.alloc(PANICKY, 24).unwrap();
    let gs = [host, p].map(|c| heap.new_global(c.to_value()));
    major(&heap);
    let young = new_node(&heap, 2, 0);
    let young_dead = new_node(&heap, 3, 0);
    store(&heap, host, 0, young.to_value());
    // The panicky cell is old: a store into it logs it, so the minor GC traces it.
    store_at(&heap, p, 8, young.to_value());
    assert_eq!(state(p), gc_state::GREY);

    PANIC_NOW.set(true);
    let r = catch_unwind(AssertUnwindSafe(|| minor(&heap)));
    PANIC_NOW.set(false);
    assert!(r.is_err(), "the trace function did not panic");
    assert_eq!(state(host), gc_state::GREY, "GREY again");
    assert_eq!(state(p), gc_state::GREY, "GREY again");
    assert!(old(&heap, young) && old(&heap, young_dead), "all old");
    assert_eq!(state(young), gc_state::BLACK);
    heap.verify().expect("clean after the caught panic");

    heap.set_verify_after_gc(true);
    minor(&heap);
    assert!(heap.is_cell(young.addr()), "young (now old) cell kept");
    assert!(
        heap.is_cell(young_dead.addr()),
        "old until the next major GC"
    );
    major(&heap);
    assert!(!heap.is_cell(young_dead.addr()));
    for g in gs {
        heap.drop_global(g);
    }
}

/// The verify of a minor GC counts the old cells: kept = old + pushed. A young cell whose
/// mark bit is set before the GC counts as old but is WHITE: the verifier names it.
#[test]
fn the_minor_verify_rejects_a_marked_white_cell() {
    let heap = gen_heap(true, |_| {});
    let root = new_node(&heap, 1, 1);
    let g = heap.new_global(root.to_value());
    major(&heap);
    let young = new_node(&heap, 2, 0);
    store(&heap, root, 0, young.to_value());
    {
        let inner = heap.inner.borrow();
        let page = inner.mem.page(inner.mem.chunk_index(young.addr()).unwrap());
        assert!(page.mark(young.addr()), "planted mark");
    }
    let r = catch_unwind(AssertUnwindSafe(|| minor(&heap)));
    let msg = r.expect_err("the verify must fail");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("marked (old) but WHITE"), "{msg}");
    heap.drop_global(g);
}

/// S1 is the default since M5-P2 (plan v10 V10-D17): `HeapConfig::default()` builds a
/// generational heap, so a heap made from it (the CLI, REPRL, test262 and every isolate that
/// does not set the flag) runs minor GCs, and its barrier threshold is GEN. `test_heap` stays
/// S0 for the S0 tests (`tests/mod.rs`); this test takes the flag from the default.
#[test]
fn s1_is_the_default_since_m5_p2() {
    assert!(
        HeapConfig::default().generational,
        "V10-D17: S1 is the default"
    );
    let heap = test_heap(|c| c.generational = HeapConfig::default().generational);
    assert_eq!(heap.barrier().threshold(), crate::barrier::threshold::GEN);
    let root = new_node(&heap, 1, 1);
    let g = heap.new_global(root.to_value());
    heap.collect_minor_with_words(&[]);
    let st = heap.stats();
    assert!(st.last_minor, "a minor GC, not a full one");
    assert_eq!(st.minor_collections, 1);
    assert!(
        heap.is_cell(root.addr()),
        "the rooted cell survives and is old"
    );
    heap.drop_global(g);
}

/// A chain of `n` rooted nodes (ids `first..first + n`): node i holds node i+1 in slot 0.
/// Every 7th node is medium (40 slots, 336 B: above a line, so the overflow page when the main
/// TLAB has no room), the others small (30 slots, 256 B: one line each). Returns the head; `g`
/// roots it.
fn rooted_chain(heap: &Heap, first: u32, n: u32) -> (CellPtr, crate::GlobalHandle) {
    let head = new_node(heap, first, 30);
    let g = heap.new_global(head.to_value());
    let mut prev = head;
    for id in first + 1..first + n {
        let c = new_node(heap, id, if id % 7 == 0 { 40 } else { 30 });
        store(heap, prev, 0, c.to_value());
        prev = c;
    }
    (head, g)
}

/// Walks the chain from `head` and checks every id: a cell that an allocation overwrote
/// breaks the chain or its ids.
fn check_chain(head: CellPtr, first: u32, n: u32) {
    let mut c = head;
    for id in first..first + n {
        assert_eq!(node_id(c), id, "chain cell {id} overwritten");
        if id + 1 < first + n {
            c = CellPtr::from_value(slot(c, 0).get()).expect("chain link");
        }
    }
}

/// H09-Q6, M5-P2 GC6: an S1 sweep does not walk a page with no dead cell; it keeps the page's
/// line marks, which cover every live cell (the allocator marks each run it hands out, and
/// the overflow page whole, and gives back the unused tails). So the later allocation into free lines never lands on a live
/// cell: small cells, medium cells, after a minor and after a major GC. Not under Miri
/// (5,000 nodes); the other S1 tests run the skip there.
#[test]
#[cfg_attr(miri, ignore)]
fn an_s1_sweep_skips_a_page_with_no_dead_cell_and_reuses_only_free_lines() {
    use crate::heap::sweep_counter::skipped;
    let heap = gen_heap(false, |_| {});
    let (old, g_old) = rooted_chain(&heap, 1, 4000);
    major(&heap);
    heap.finish_sweeping();
    let pages = heap.stats().small_pages as u64;
    assert!(pages >= 4, "{pages} pages");
    // No allocation since the last GC: no page holds a dead cell, none is walked.
    let s0 = skipped();
    minor(&heap);
    heap.finish_sweeping();
    assert_eq!(skipped() - s0, pages, "every page skipped after a minor GC");
    let s1 = skipped();
    major(&heap);
    heap.finish_sweeping();
    assert_eq!(
        skipped() - s1,
        pages,
        "every page skipped after a major GC (S1)"
    );
    // Young cells: garbage and a rooted chain, in the free lines of old pages and in new
    // pages. The pages that got young garbage are walked (their dead cells are freed); the
    // full old pages are not.
    let mut garbage = Vec::new();
    for i in 0..3000u32 {
        garbage.push(new_node(&heap, 100_000 + i, if i % 7 == 0 { 40 } else { 30 }).addr());
    }
    let (young, g_young) = rooted_chain(&heap, 10_000, 1000);
    let s2 = skipped();
    minor(&heap);
    heap.finish_sweeping();
    let walked_and_skipped = heap.stats().small_pages as u64;
    let skipped_now = skipped() - s2;
    assert!(skipped_now >= 1, "the full old pages are skipped");
    assert!(
        skipped_now < walked_and_skipped,
        "the pages with garbage are walked"
    );
    assert!(
        garbage.iter().all(|&a| !heap.is_cell(a)),
        "dead young cells freed"
    );
    // Fill the free lines (old pages' holes, the freed garbage lines) with a third chain,
    // then check that no live cell was overwritten.
    let (third, g_third) = rooted_chain(&heap, 20_000, 4000);
    check_chain(old, 1, 4000);
    check_chain(young, 10_000, 1000);
    check_chain(third, 20_000, 4000);
    heap.verify().expect("verifier");
    minor(&heap);
    major(&heap);
    check_chain(old, 1, 4000);
    check_chain(young, 10_000, 1000);
    check_chain(third, 20_000, 4000);
    heap.verify().expect("verifier");
    for g in [g_old, g_young, g_third] {
        heap.drop_global(g);
    }
}

thread_local! {
    static SIZE_PANIC_NOW: Cell<bool> = const { Cell::new(false) };
}

/// A 16 B leaf whose size function panics on demand.
const PANICKY_SIZE: KindId = KindId::new(33).unwrap();

fn panicky_size(_: CellPtr) -> usize {
    if SIZE_PANIC_NOW.get() {
        std::panic::resume_unwind(Box::new("test size panic"));
    }
    16
}

/// A size function panics while an S1 sweep walks a page: the walk had cleared the page's
/// line marks and set only those of the cells before the panic, so the unwind marks every
/// line used. After it the page holds no dead cell (the unwind set every mark bit to its VO
/// bit), so the next S1 sweep keeps those line marks; without the fill, an allocation would
/// land on the live cells after the panic point. Every cell is old (BLACK) when the walk
/// panics, as after a major GC; the page unwind does not make a WHITE cell BLACK.
#[test]
#[cfg_attr(miri, ignore)]
fn a_size_panic_in_an_s1_sweep_leaves_every_line_used() {
    let heap = gen_heap(false, |_| {});
    heap.register_kind(
        PANICKY_SIZE,
        CellKind {
            name: "test-minor-panicky-size",
            size: CellSize::Dynamic(panicky_size),
            trace: no_trace,
        },
    )
    .unwrap();
    let host = new_node(&heap, 1, 3);
    let g = heap.new_global(host.to_value());
    let dead = new_node(&heap, 2, 0);
    store(&heap, host, 2, dead.to_value());
    let odd = heap.alloc(PANICKY_SIZE, 16).unwrap();
    store(&heap, host, 1, odd.to_value());
    let (chain, g_chain) = rooted_chain(&heap, 10, 600);
    store(&heap, host, 0, chain.to_value());
    let page_of = |c: CellPtr| heap.inner.borrow().mem.chunk_index(c.addr());
    assert_eq!(
        page_of(chain),
        page_of(host),
        "the chain starts on the host's page"
    );
    assert!(odd.addr() < chain.addr());
    major(&heap);
    heap.finish_sweeping();
    // `dead` is old and now unreachable: the next sweep walks the host's page and calls the
    // size function of `odd`, which panics.
    store(&heap, host, 2, Value::UNDEFINED);
    major(&heap);
    SIZE_PANIC_NOW.set(true);
    let r = catch_unwind(AssertUnwindSafe(|| heap.finish_sweeping()));
    SIZE_PANIC_NOW.set(false);
    assert!(r.is_err(), "the size function did not panic");
    assert!(
        heap.is_cell(dead.addr()),
        "the unwind keeps the page's cells"
    );
    minor(&heap);
    heap.finish_sweeping();
    let (more, g_more) = rooted_chain(&heap, 5_000, 2000);
    check_chain(chain, 10, 600);
    check_chain(more, 5_000, 2000);
    assert_eq!(
        CellPtr::from_value(slot(host, 1).get()).unwrap().addr(),
        odd.addr()
    );
    heap.verify().expect("verifier");
    for g in [g, g_chain, g_more] {
        heap.drop_global(g);
    }
}

/// A shape-space kind of 8 KiB with no references (the shape TLAB's too-short tail).
const BIG_MINOR_SHAPE: KindId = KindId::new(34).unwrap();

/// The free lines of the page that holds `addr`.
fn free_lines(heap: &Heap, addr: usize) -> usize {
    use crate::layout::{FIRST_LINE, LINES_PER_PAGE};
    let inner = heap.inner.borrow();
    let page = inner.mem.page(inner.mem.chunk_index(addr).unwrap());
    (FIRST_LINE..LINES_PER_PAGE)
        .filter(|&l| !page.line_used(l))
        .count()
}

/// H09-Q6, M5-P2 GC6: a GC gives back the unused tail of each retired run (the main TLAB, the
/// overflow page, the shape TLAB) and of a shape run too short for its cell, so a page that
/// the S1 sweep then skips (no dead cell) still has those lines free. Without it, the run's
/// lines stay marked used until a cell of the page dies: a page with one live cell stays
/// full.
#[test]
fn an_s1_gc_gives_back_the_unused_tail_of_each_run() {
    use crate::heap::sweep_counter::skipped;
    use crate::layout::{LINE_SIZE, MAX_SMALL_SIZE, PAGE_CELL_BYTES};
    let heap = gen_heap(false, |c| c.shape_space_bytes = 1 << 20);
    heap.register_kind_in(
        BIG_MINOR_SHAPE,
        CellKind {
            name: "test-minor-big-shape",
            size: CellSize::Fixed(MAX_SMALL_SIZE),
            trace: no_trace,
        },
        Space::Shape,
    )
    .unwrap();
    // One medium cell (no main TLAB yet: the overflow page) and one small cell (the main
    // TLAB: a fresh page, one run), both rooted.
    let medium = new_node(&heap, 2, 40);
    let small = new_node(&heap, 1, 2);
    let gs = [small, medium].map(|c| heap.new_global(c.to_value()));
    // Shape space: a page of big shapes, then one more, which leaves the first page's tail
    // (shorter than 8 KiB but longer than a line) and takes a second page.
    let roots = heap.scratch_roots();
    let per_page = PAGE_CELL_BYTES / MAX_SMALL_SIZE;
    let shapes: Vec<CellPtr> = (0..=per_page)
        .map(|_| heap.alloc_shape(BIG_MINOR_SHAPE, MAX_SMALL_SIZE).unwrap())
        .collect();
    for s in &shapes {
        roots.push(s.to_value());
    }
    let page_of = |a: usize| heap.inner.borrow().mem.chunk_index(a).unwrap();
    assert_ne!(page_of(small.addr()), page_of(medium.addr()));
    assert_ne!(page_of(shapes[0].addr()), page_of(shapes[per_page].addr()));
    let tail_lines = (PAGE_CELL_BYTES - per_page * MAX_SMALL_SIZE) / LINE_SIZE;
    assert!(
        tail_lines >= 1,
        "the first shape page has a tail of whole lines"
    );
    // The tail of the first shape page went back when the shape TLAB moved on.
    assert_eq!(free_lines(&heap, shapes[0].addr()), tail_lines);
    let s0 = skipped();
    minor(&heap);
    heap.finish_sweeping();
    let pages = heap.stats().small_pages as u64;
    assert_eq!(pages, 4, "two ordinary pages and two shape pages");
    assert_eq!(
        skipped() - s0,
        pages,
        "no page holds a dead cell: none is walked"
    );
    let used = |cells: usize| cells.div_ceil(LINE_SIZE);
    let all = PAGE_CELL_BYTES / LINE_SIZE;
    assert_eq!(
        free_lines(&heap, small.addr()),
        all - used(NODE_HEADER + 8 * 2)
    );
    assert_eq!(
        free_lines(&heap, medium.addr()),
        all - used(NODE_HEADER + 8 * 40)
    );
    assert_eq!(free_lines(&heap, shapes[0].addr()), tail_lines);
    assert_eq!(
        free_lines(&heap, shapes[per_page].addr()),
        all - used(MAX_SMALL_SIZE)
    );
    // The next small cell reuses the first page.
    let next = new_node(&heap, 3, 2);
    assert_eq!(page_of(next.addr()), page_of(small.addr()));
    heap.verify().expect("verifier");
    drop(roots);
    for g in gs {
        heap.drop_global(g);
    }
}

/// The overflow page moves on when a medium cell does not fit its tail: the old page's tail
/// lines are free again, before and after an S1 sweep that skips the page. Only medium cells
/// are allocated, so no main TLAB takes them first.
#[test]
fn an_s1_overflow_page_gives_back_its_tail_when_it_moves_on() {
    use crate::heap::sweep_counter::skipped;
    use crate::layout::{LINE_SIZE, PAGE_CELL_BYTES};
    let heap = gen_heap(false, |_| {});
    let slots = 1000; // 8,016 B: medium (above a line, at most MAX_SMALL_SIZE)
    let size = NODE_HEADER + 8 * slots;
    let per_page = PAGE_CELL_BYTES / size;
    let roots = heap.scratch_roots();
    let cells: Vec<CellPtr> = (0..=per_page as u32)
        .map(|i| new_node(&heap, i, slots))
        .collect();
    for c in &cells {
        roots.push(c.to_value());
    }
    let page_of = |a: usize| heap.inner.borrow().mem.chunk_index(a).unwrap();
    let (first, last) = (cells[0].addr(), cells[per_page].addr());
    assert_eq!(page_of(cells[per_page - 1].addr()), page_of(first));
    assert_ne!(
        page_of(last),
        page_of(first),
        "the last cell took a new page"
    );
    let tail = PAGE_CELL_BYTES / LINE_SIZE - (per_page * size).div_ceil(LINE_SIZE);
    assert!(tail >= 1, "the first page has a tail of whole lines");
    assert_eq!(free_lines(&heap, first), tail);
    let s0 = skipped();
    minor(&heap);
    heap.finish_sweeping();
    assert_eq!(skipped() - s0, 2, "both pages skipped (no dead cell)");
    assert_eq!(free_lines(&heap, first), tail);
    heap.verify().expect("verifier");
    drop(roots);
}
