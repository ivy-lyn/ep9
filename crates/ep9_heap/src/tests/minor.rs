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
/// cell is a store without the barrier. `Heap::verify` reports it, and so does the verify of
/// a minor GC, before its mark (which would free the young cell).
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
    // The verify of a minor GC finds the same break before its mark frees the young cell.
    heap.set_verify_after_gc(true);
    store_without_barrier(host, 0, young.to_value());
    heap.drop_global(gy);
    let r = catch_unwind(AssertUnwindSafe(|| minor(&heap)));
    let msg = r.expect_err("the verify must fail");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        msg.contains("heap verifier failed after GC") && msg.contains("not in the S1 logs"),
        "{msg}"
    );
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
