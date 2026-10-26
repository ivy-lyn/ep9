//! S1 step 1 behind `HeapConfig::generational` (plan v9 §4.2 task 2; V9-D21): the barrier
//! logs, sticky marks, and the verifier's S1 rules.
//!
//! ```text
//! generational heap: threshold GEN; after a GC the survivors are BLACK and marked (old)
//!   store young -> old small host:  slow path; host GREY; its page's remembered list += host
//!   store again into the GREY host:  fast path (GREY is above GEN): logged once per cycle
//!   store into an old LOS host:      slow path every time; card (slot - cell) / 512 dirty
//!   next GC: clears the marks, marks (GREY -> BLACK), empties the lists and cards, keeps
//!            the marks of the cells it keeps
//! ```

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use ep9_value::{ShapeId, Value, Word0};

use super::*;
use crate::barrier::probe::ep9_probe_barrier;
use crate::barrier::slow_counter;
use crate::layout::{CARD_SIZE, Large, PAGE_MASK};
use crate::{BarrierState, gc_state, set_shape, threshold};

fn gen_heap(tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    test_heap(|c| {
        c.generational = true;
        tweak(c);
    })
}

/// A full collection with precise roots only (the conservative scan reads no words).
fn precise_gc(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(heap.last_conservative_roots().is_empty());
}

/// Every remembered host of every small page, as addresses.
fn remembered(heap: &Heap) -> Vec<usize> {
    let inner = heap.inner.borrow();
    let mem = &inner.mem;
    let mut out = Vec::new();
    for idx in mem.small_pages() {
        let page = mem.page(idx);
        // SAFETY: no store runs while the test reads the list.
        let offs = unsafe { page.header().remembered.offsets() };
        out.extend(offs.iter().map(|&o| page.addr() + o as usize));
    }
    out
}

/// The dirty byte and the dirty card indices of the large object whose cell is `cell`.
fn cards(heap: &Heap, cell: CellPtr) -> (u8, Vec<usize>) {
    let _inner = heap.inner.borrow();
    let base = cell.addr() & PAGE_MASK;
    // SAFETY: `cell` is a live LOS cell, so its first chunk holds its header; the pointer has
    // the reservation's provenance.
    let large =
        unsafe { Large::from_base(core::ptr::NonNull::new(heap_ptr(heap, base).cast()).unwrap()) };
    let h = large.header();
    let dirty = crate::remembered::cards(h).map_or(Vec::new(), |c| {
        (0..c.len())
            .filter(|&i| c[i].load(core::sync::atomic::Ordering::Relaxed) != 0)
            .collect()
    });
    (h.dirty.load(core::sync::atomic::Ordering::Relaxed), dirty)
}

fn marked(heap: &Heap, c: CellPtr) -> bool {
    crate::trace::is_marked(&heap.inner.borrow().mem, c.addr())
}

fn state(c: CellPtr) -> u8 {
    // SAFETY: callers pass live cells.
    unsafe { c.header() }.gc_state()
}

#[test]
fn the_flag_sets_the_gen_threshold() {
    assert_eq!(test_heap(|_| {}).barrier().threshold(), threshold::OFF);
    assert_eq!(gen_heap(|_| {}).barrier().threshold(), threshold::GEN);
}

#[test]
fn a_store_into_an_old_host_logs_it_once() {
    for checked in [false, true] {
        let heap = gen_heap(|c| c.verify_after_gc = checked);
        let host = new_node(&heap, 1, 2);
        let g = heap.new_global(host.to_value());
        let before = slow_counter::get();
        store(&heap, host, 0, new_node(&heap, 2, 0).to_value());
        assert_eq!(
            slow_counter::get(),
            before,
            "a young (WHITE) host does not fire"
        );
        precise_gc(&heap);
        assert_eq!(state(host), gc_state::BLACK);
        assert!(remembered(&heap).is_empty());

        let young = new_node(&heap, 3, 0);
        let before = slow_counter::get();
        store(&heap, host, 1, young.to_value());
        assert_eq!(slow_counter::get(), before + 1, "an old (BLACK) host fires");
        assert_eq!(state(host), gc_state::GREY);
        assert_eq!(remembered(&heap), vec![host.addr()]);
        store(&heap, host, 0, young.to_value());
        store(&heap, host, 0, Value::int32(4));
        assert_eq!(
            slow_counter::get(),
            before + 1,
            "a GREY host does not fire again"
        );
        assert_eq!(remembered(&heap).len(), 1, "logged once");
        heap.verify().expect("every GREY host is listed once");

        let young_addr = young.addr();
        precise_gc(&heap);
        assert_eq!(state(host), gc_state::BLACK, "the mark sets it BLACK again");
        assert!(remembered(&heap).is_empty(), "the GC empties the lists");
        assert!(heap.is_cell(young_addr));
        assert_eq!(slot(host, 1).get().to_bits(), young.to_value().to_bits());
        heap.verify().expect("verifier");
        heap.drop_global(g);
    }
}

#[test]
fn the_shape_install_logs_the_host() {
    let heap = gen_heap(|_| {});
    let host = new_node(&heap, 1, 0);
    let g = heap.new_global(host.to_value());
    precise_gc(&heap);
    // SAFETY: `host` is rooted. Without a shape space the ShapeId is no reference.
    set_shape(unsafe { host.header() }, ShapeId::new(0), heap.barrier());
    assert_eq!(state(host), gc_state::GREY);
    assert_eq!(remembered(&heap), vec![host.addr()]);
    heap.verify().expect("verifier");
    precise_gc(&heap);
    heap.drop_global(g);
}

#[test]
fn a_large_host_dirties_the_card_of_the_slot() {
    for checked in [false, true] {
        let heap = gen_heap(|c| c.verify_after_gc = checked);
        let host = new_node(&heap, 1, 1100);
        assert_eq!(heap.stats().large_cells, 1);
        let g = heap.new_global(host.to_value());
        precise_gc(&heap);
        assert_eq!(cards(&heap, host), (0, vec![]));

        let young = new_node(&heap, 2, 0);
        let before = slow_counter::get();
        store(&heap, host, 1000, young.to_value());
        let card = (NODE_HEADER + 8 * 1000) / CARD_SIZE;
        assert_eq!(card, 15);
        assert_eq!(cards(&heap, host), (1, vec![card]));
        assert_eq!(state(host), gc_state::BLACK, "a large host stays BLACK");
        store(&heap, host, 0, young.to_value());
        assert_eq!(slow_counter::get(), before + 2, "so every store fires");
        assert_eq!(cards(&heap, host), (1, vec![0, card]));
        assert!(
            remembered(&heap).is_empty(),
            "no list entry for a large host"
        );
        heap.verify().expect("verifier");

        precise_gc(&heap);
        assert_eq!(cards(&heap, host), (0, vec![]), "the GC cleans the cards");
        assert!(heap.is_cell(young.addr()));
        heap.drop_global(g);
    }
}

#[test]
fn sticky_marks_stay_on_old_cells_until_the_next_gc() {
    let heap = gen_heap(|c| c.verify_after_gc = true);
    let old = new_node(&heap, 1, 1);
    let big = new_node(&heap, 2, 1100);
    let g_old = heap.new_global(old.to_value());
    let g_big = heap.new_global(big.to_value());
    let dead = new_node(&heap, 3, 0).addr();
    precise_gc(&heap);
    heap.finish_sweeping();
    assert!(
        marked(&heap, old) && marked(&heap, big),
        "the sweep keeps their marks"
    );
    assert!(!heap.is_cell(dead));
    let young = new_node(&heap, 4, 0);
    assert!(!marked(&heap, young), "a new cell is unmarked (young)");
    heap.verify().expect("marks only on cell starts");

    // The next GC clears the old marks first: an old cell that lost its last root dies.
    let (old_addr, big_addr) = (old.addr(), big.addr());
    heap.drop_global(g_old);
    heap.drop_global(g_big);
    precise_gc(&heap);
    assert!(!heap.is_cell(old_addr) && !heap.is_cell(big_addr));
    heap.verify().expect("verifier");
}

#[test]
fn an_s0_heap_never_logs() {
    let heap = test_heap(|_| {});
    let host = new_node(&heap, 1, 1);
    let g = heap.new_global(host.to_value());
    precise_gc(&heap);
    heap.barrier().set_threshold(threshold::GEN);
    let before = slow_counter::get();
    store(&heap, host, 0, new_node(&heap, 2, 0).to_value());
    heap.barrier().set_threshold(threshold::OFF);
    assert_eq!(slow_counter::get(), before + 1);
    assert_eq!(state(host), gc_state::BLACK, "not logged: the heap is S0");
    assert!(remembered(&heap).is_empty());
    heap.verify().expect("verifier");
    heap.drop_global(g);
}

/// While a generational heap lives on this thread, a host outside it (a word on the stack)
/// only gets the store: the slow path never reads a page header outside a heap.
#[test]
fn a_host_outside_every_heap_only_stores() {
    let heap = gen_heap(|_| {});
    let target = new_node(&heap, 1, 0);
    let host = Word0::new(ShapeId::new(0), gc_state::BLACK, 0, 0);
    let slot = Slot::new(Value::UNDEFINED);
    let before = slow_counter::get();
    ep9_probe_barrier(&host, &slot, target.to_value(), heap.barrier());
    assert_eq!(slow_counter::get(), before + 1);
    assert_eq!(slot.get().to_bits(), target.to_value().to_bits());
    assert_eq!(host.gc_state(), gc_state::BLACK);
    let st = BarrierState::new(threshold::MARKING);
    ep9_probe_barrier(&host, &slot, target.to_value(), &st);
    assert!(remembered(&heap).is_empty());
}

#[test]
fn a_list_grows_past_its_first_capacity() {
    let heap = gen_heap(|c| c.verify_after_gc = true);
    let n = 100;
    let root = new_node(&heap, 0, n);
    let g = heap.new_global(root.to_value());
    let hosts: Vec<CellPtr> = (0..n)
        .map(|i| {
            let h = new_node(&heap, 1 + i as u32, 1);
            store(&heap, root, i, h.to_value());
            h
        })
        .collect();
    precise_gc(&heap);
    let young = new_node(&heap, 999, 0);
    for &h in &hosts {
        store(&heap, h, 0, young.to_value());
    }
    let mut got = remembered(&heap);
    got.sort_unstable();
    let mut want: Vec<usize> = hosts.iter().map(|h| h.addr()).collect();
    want.sort_unstable();
    assert_eq!(got, want);
    heap.verify().expect("verifier");
    precise_gc(&heap);
    assert!(remembered(&heap).is_empty());
    assert!(heap.is_cell(young.addr()));
    heap.drop_global(g);
}

/// GC-stress with the flag and the verify after every GC: a list grows one young node at a
/// time from an old tail, so every link is an old-to-young store that logs.
#[test]
fn gc_stress_with_the_flag_keeps_every_link() {
    let n = if cfg!(miri) { 20 } else { 300 };
    let heap = gen_heap(|c| {
        c.gc_stress = 1;
        c.verify_after_gc = true;
    });
    let head = new_node(&heap, 0, 1);
    let g = heap.new_global(head.to_value());
    let tail = heap.new_global(head.to_value());
    for i in 1..=n {
        let next = new_node(&heap, i, 1);
        let t = CellPtr::from_value(heap.global(tail)).unwrap();
        assert_eq!(state(t), gc_state::BLACK, "the tail survived a GC");
        store(&heap, t, 0, next.to_value());
        assert_eq!(state(t), gc_state::GREY);
        heap.set_global(tail, next.to_value());
    }
    heap.collect();
    let mut c = head;
    for i in 0..=n {
        assert_eq!(node_id(c), i);
        if i < n {
            c = CellPtr::from_value(slot(c, 0).get()).unwrap();
        }
    }
    heap.verify().expect("verifier");
    heap.drop_global(tail);
    heap.drop_global(g);
}

/// Negative controls of the verifier's S1 rules: a GREY host that no list names, and a listed
/// host that is not GREY.
#[test]
fn the_verifier_checks_the_remembered_lists() {
    let heap = gen_heap(|_| {});
    let (a, b) = (new_node(&heap, 1, 1), new_node(&heap, 2, 1));
    let ga = heap.new_global(a.to_value());
    let gb = heap.new_global(b.to_value());
    precise_gc(&heap);
    store(&heap, a, 0, b.to_value());
    heap.verify().expect("clean");
    // SAFETY: `b` is rooted; the test restores its state.
    unsafe { b.header() }.set_gc_state(gc_state::GREY);
    let e = heap.verify().expect_err("an unlisted GREY host");
    assert!(e.to_string().contains("1 remembered"), "{e}");
    // SAFETY: as above.
    unsafe { b.header() }.set_gc_state(gc_state::BLACK);
    // SAFETY: `a` is rooted; the test restores its state.
    unsafe { a.header() }.set_gc_state(gc_state::BLACK);
    let e = heap.verify().expect_err("a listed host that is not GREY");
    assert!(e.to_string().contains("is not a GREY cell start"), "{e}");
    // SAFETY: as above.
    unsafe { a.header() }.set_gc_state(gc_state::GREY);
    heap.verify().expect("restored");
    precise_gc(&heap);
    heap.drop_global(ga);
    heap.drop_global(gb);
}

thread_local! {
    static PANIC_NOW: Cell<bool> = const { Cell::new(false) };
}

/// 24 B: word0, a strong slot at +8, a raw word; its trace function panics on demand.
const PANICKY: KindId = KindId::new(16).unwrap();

fn panicky_trace(c: CellPtr, t: &mut Tracer<'_>) {
    if PANIC_NOW.get() {
        std::panic::resume_unwind(Box::new("test callback panic"));
    }
    // SAFETY: a live PANICKY cell has a Value slot at +8.
    t.visit(unsafe { c.slot(8) });
}

/// A trace function panics during a GC of a generational heap: every cell counts as old
/// (marks = VO bits, LOS marked), every logged host is GREY and listed again, the verifier
/// is clean, and the next GC works.
#[test]
fn a_caught_panic_keeps_every_cell_old_and_every_log() {
    let heap = gen_heap(|_| {});
    heap.register_kind(
        PANICKY,
        CellKind {
            name: "test-s1-panicky",
            size: CellSize::Fixed(24),
            trace: panicky_trace,
        },
    )
    .unwrap();
    let host = new_node(&heap, 1, 1);
    let big = new_node(&heap, 2, 1100);
    let p = heap.alloc(PANICKY, 24).unwrap();
    let gs = [host, big, p].map(|c| heap.new_global(c.to_value()));
    precise_gc(&heap);
    let young = new_node(&heap, 3, 0);
    store(&heap, host, 0, young.to_value());
    store(&heap, big, 7, young.to_value());
    assert_eq!(state(host), gc_state::GREY);

    PANIC_NOW.set(true);
    let r = catch_unwind(AssertUnwindSafe(|| precise_gc(&heap)));
    PANIC_NOW.set(false);
    assert!(r.is_err(), "the trace function did not panic");
    assert_eq!(state(host), gc_state::GREY, "GREY again");
    assert_eq!(remembered(&heap), vec![host.addr()], "the list stays");
    assert_eq!(cards(&heap, big).0, 1, "the cards stay");
    assert!(
        marked(&heap, young) && marked(&heap, host) && marked(&heap, big),
        "all old"
    );
    heap.verify().expect("clean after the caught panic");

    precise_gc(&heap);
    assert!(heap.is_cell(young.addr()));
    assert!(remembered(&heap).is_empty());
    heap.verify().expect("verifier");
    for g in gs {
        heap.drop_global(g);
    }
}
