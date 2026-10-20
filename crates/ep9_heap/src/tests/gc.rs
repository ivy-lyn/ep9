//! Mark-sweep with precise roots, weak slots, lazy sweep, zapping, GC-stress and the verifier.
//!
//! Most tests collect with [`precise_gc`]: a full collection whose conservative scan reads an
//! empty word range instead of the stack, so stale stack words cannot keep a cell and every
//! check is exact. Tests that use the real `collect` allow the cells that the scan found
//! ([`expected_live`]). Under Miri there is no scan at all.

use std::cell::RefCell;
use std::hint::black_box;
use std::rc::Rc;

use ep9_value::Value;

use super::*;
use crate::layout::{PAGE_SIZE, ZAP_WORD};
use crate::{RootStrength, gc_state, threshold};

/// A full collection with precise roots only (the conservative scan reads no words).
fn precise_gc(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(heap.last_conservative_roots().is_empty());
}

/// Allocates `n` unrooted nodes in a frame of its own and returns their addresses (as
/// integers in malloc memory, which the conservative scan never reads).
#[inline(never)]
fn garbage(heap: &Heap, n: u32, slots: usize) -> Vec<usize> {
    (0..n)
        .map(|i| new_node(heap, 1000 + i, slots).addr())
        .collect()
}

/// Asserts: every address in `live` is a live cell, and every address in `dead` is freed
/// unless the last conservative scan kept it.
fn check(heap: &Heap, live_roots: &[usize], dead: &[usize]) {
    let expect = expected_live(heap, live_roots);
    for &d in dead {
        if !expect.contains(&d) {
            assert!(!heap.is_cell(d), "cell {d:#x} is unreachable but survived");
        }
    }
    #[cfg(miri)]
    assert!(heap.last_conservative_roots().is_empty());
}

#[test]
fn reachable_survive_and_unreachable_are_freed() {
    let heap = test_heap(|_| {});
    let root = new_node(&heap, 1, 3);
    let g = heap.new_global(root.to_value());
    let a = new_node(&heap, 2, 1);
    let b = new_node(&heap, 3, 0);
    store(&heap, root, 0, a.to_value());
    store(&heap, a, 0, b.to_value());
    let dead = garbage(&heap, 50, 2);
    precise_gc(&heap);
    check(&heap, &[root.addr()], &dead);
    assert_eq!(node_id(b), 3);
    let s = heap.stats();
    assert_eq!(s.collections, 1);
    assert!(s.last_marked_cells >= 3);
    // Cut the edge to `a`: `a` and `b` die (unless the scan sees them).
    store(&heap, root, 0, Value::NULL);
    let (a_addr, b_addr) = (a.addr(), b.addr());
    precise_gc(&heap);
    check(&heap, &[root.addr()], &[a_addr, b_addr]);
    heap.drop_global(g);
    heap.verify().expect("verifier");
}

#[test]
fn survivors_turn_black_and_new_cells_are_white() {
    let heap = test_heap(|_| {});
    let old = new_node(&heap, 1, 1);
    let g = heap.new_global(old.to_value());
    precise_gc(&heap);
    let young = new_node(&heap, 2, 1);
    // SAFETY: both cells are live (rooted / just allocated).
    unsafe {
        assert_eq!(old.header().gc_state(), gc_state::BLACK);
        assert_eq!(young.header().gc_state(), gc_state::WHITE);
    }
    // S1 preview: with threshold GEN only the BLACK host takes the slow path.
    heap.barrier().set_threshold(threshold::GEN);
    let before = crate::barrier::slow_counter::get();
    store(&heap, old, 0, young.to_value());
    store(&heap, young, 0, Value::NULL);
    assert_eq!(crate::barrier::slow_counter::get(), before + 1);
    heap.barrier().set_threshold(threshold::OFF);
    assert_eq!(slot(old, 0).get().to_bits(), young.to_value().to_bits());
    heap.drop_global(g);
}

#[test]
fn handles_strong_and_weak() {
    let heap = test_heap(|_| {});
    let kept = new_node(&heap, 1, 0);
    let g = heap.new_global(kept.to_value());
    let w_live = heap.new_weak(kept.to_value());
    let addrs = garbage(&heap, 1, 0);
    let w_dead_cell = heap.new_weak(cell_at(&heap, addrs[0]).to_value());
    let w_int = heap.new_weak(Value::int32(42));
    let w_undef = heap.new_weak(Value::UNDEFINED);
    precise_gc(&heap);
    check(&heap, &[kept.addr()], &addrs);
    assert_eq!(
        heap.weak(w_live).map(|v| v.to_bits()),
        Some(kept.to_value().to_bits())
    );
    assert_eq!(heap.weak(w_int).and_then(Value::as_int32), Some(42));
    assert!(heap.weak(w_undef).is_some_and(|v| v.is_undefined()));
    assert!(!heap.is_cell(addrs[0]));
    assert!(
        heap.weak(w_dead_cell).is_none(),
        "a dead cell's weak handle is cleared"
    );
    assert_eq!(heap.stats().last_weak_cleared, 1);
    assert_eq!(heap.global(g).to_bits(), kept.to_value().to_bits());
    assert_eq!(heap.handle_count(), 5);
    heap.drop_global(g);
    heap.drop_weak(w_live);
    assert_eq!(heap.handle_count(), 3);
}

/// A CellPtr for an address returned by `garbage` while it is still live.
fn cell_at(heap: &Heap, addr: usize) -> CellPtr {
    assert!(heap.is_cell(addr));
    // SAFETY: a live cell starts at `addr` (checked).
    unsafe { CellPtr::from_raw(core::ptr::NonNull::new(heap_ptr(heap, addr)).unwrap()) }
}

#[test]
fn scratch_roots_are_scoped() {
    let heap = test_heap(|_| {});
    let addrs = garbage(&heap, 3, 1);
    {
        let scope = heap.scratch_roots();
        for &a in &addrs {
            scope.push(cell_at(&heap, a).to_value());
        }
        assert_eq!(scope.len(), 3);
        {
            let inner = heap.scratch_roots();
            inner.push(Value::int32(1));
            assert_eq!(inner.get(0).as_int32(), Some(1));
        }
        assert_eq!(scope.len(), 3, "the inner scope's entry is gone");
        precise_gc(&heap);
        for &a in &addrs {
            assert!(heap.is_cell(a), "scratch roots keep their cells");
        }
        scope.set(1, Value::NULL);
        assert!(scope.get(1).is_null());
    }
    precise_gc(&heap);
    check(&heap, &[], &addrs);
}

#[test]
#[should_panic(expected = "not the innermost")]
fn scratch_push_into_outer_scope_panics() {
    let heap = test_heap(|_| {});
    let outer = heap.scratch_roots();
    let _inner = heap.scratch_roots();
    outer.push(Value::NULL);
}

#[test]
fn root_sources_and_ranges() {
    let heap = test_heap(|_| {});
    // A VM register stack: raw Value words, visited precisely by a root source.
    let vm_stack: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let src_stack = Rc::clone(&vm_stack);
    let id = heap.add_root_source(move |t| {
        for &w in src_stack.borrow().iter() {
            t.visit_bits(w);
        }
    });
    let addrs = garbage(&heap, 4, 0);
    vm_stack
        .borrow_mut()
        .push(cell_at(&heap, addrs[0]).to_value().to_bits());
    vm_stack.borrow_mut().push(Value::UNDEFINED.to_bits());
    // Unit RW data: link cells (strong) and IC cells (weak).
    let link = Box::new([Slot::new(cell_at(&heap, addrs[1]).to_value())]);
    let ic = Box::new([
        Slot::new(cell_at(&heap, addrs[2]).to_value()),
        Slot::new(Value::int32(7)),
    ]);
    // SAFETY: both boxes outlive their registration (removed below before they drop).
    let (link_id, ic_id) = unsafe {
        (
            heap.add_root_range(core::ptr::NonNull::from(&link[..]), RootStrength::Strong),
            heap.add_root_range(core::ptr::NonNull::from(&ic[..]), RootStrength::Weak),
        )
    };
    precise_gc(&heap);
    assert!(heap.is_cell(addrs[0]), "VM stack slot keeps its cell");
    assert!(heap.is_cell(addrs[1]), "link cell keeps its cell");
    assert!(!heap.is_cell(addrs[2]));
    assert!(
        ic[0].get().is_undefined(),
        "a weak IC cell to a dead cell is cleared"
    );
    assert_eq!(ic[1].get().as_int32(), Some(7));
    check(&heap, &[addrs[0], addrs[1]], &addrs[3..]);
    assert!(heap.remove_root(id) && heap.remove_root(link_id) && heap.remove_root(ic_id));
    assert!(!heap.remove_root(id));
    vm_stack.borrow_mut().clear();
    precise_gc(&heap);
    check(&heap, &[], &addrs[..2]);
}

#[test]
fn weak_slots_in_cells_and_weak_sources() {
    let heap = test_heap(|_| {});
    let boxc = heap.alloc(WEAKBOX, 24).unwrap();
    let g = heap.new_global(boxc.to_value());
    let strong_target = new_node(&heap, 1, 0);
    let addrs = garbage(&heap, 2, 0);
    // SAFETY: `boxc` is a live WEAKBOX: weak slot at +8, strong slot at +16.
    unsafe {
        boxc.slot(8).init(cell_at(&heap, addrs[0]).to_value());
        boxc.slot(16).init(strong_target.to_value());
    }
    // A weak table outside the heap.
    let table: Rc<Vec<Slot>> = Rc::new(vec![
        Slot::new(cell_at(&heap, addrs[1]).to_value()),
        Slot::new(strong_target.to_value()),
    ]);
    let t2 = Rc::clone(&table);
    let wid = heap.add_weak_source(move |w| {
        for s in t2.iter() {
            w.visit(s);
        }
    });
    precise_gc(&heap);
    assert!(
        heap.is_cell(strong_target.addr()),
        "the strong slot keeps its cell"
    );
    // SAFETY: `boxc` is rooted by `g`.
    let weak_slot = unsafe { boxc.slot(8) };
    assert!(!heap.is_cell(addrs[0]) && !heap.is_cell(addrs[1]));
    assert!(
        weak_slot.get().is_undefined(),
        "weak slot to a dead cell is cleared"
    );
    assert!(
        table[0].get().is_undefined(),
        "weak source entry to a dead cell is cleared"
    );
    assert_eq!(heap.stats().last_weak_cleared, 2);
    assert_eq!(
        table[1].bits(),
        strong_target.addr() as u64,
        "live cell stays"
    );
    assert!(heap.remove_root(wid));
    heap.drop_global(g);
}

#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn lazy_sweep_zaps_dead_cells() {
    let heap = test_heap(|c| c.zap = true);
    let keep = new_node(&heap, 1, 0);
    let g = heap.new_global(keep.to_value());
    let addrs = garbage(&heap, 20, 3);
    precise_gc(&heap);
    // The sweep is lazy: after the GC every small page waits for it.
    assert_eq!(heap.unswept_pages(), heap.stats().small_pages);
    let freed_before = heap.stats().freed_cells;
    heap.finish_sweeping();
    assert_eq!(heap.unswept_pages(), 0);
    let expect = expected_live(&heap, &[keep.addr()]);
    let mut zapped = 0;
    for &a in &addrs {
        if expect.contains(&a) {
            continue;
        }
        assert!(!heap.is_cell(a));
        let p = heap_ptr(&heap, a).cast::<u64>();
        // SAFETY: the freed cell lies in a committed small page (the page stays committed:
        // it still holds `keep`); nothing reuses it before this read.
        let words = unsafe { [p.read(), p.add(1).read(), p.add(4).read()] };
        assert_eq!(words, [ZAP_WORD; 3], "freed cell {a:#x} is zapped");
        zapped += 1;
    }
    assert_eq!(zapped, 20, "all garbage is freed");
    assert!(heap.stats().freed_cells >= freed_before + zapped);
    heap.drop_global(g);
}

#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn empty_pages_and_large_objects_are_released() {
    let heap = test_heap(|_| {});
    // ~2 MiB of small garbage (8+ pages) and 3 large cells.
    let small = garbage(&heap, 2000, 120);
    let large = garbage(&heap, 3, 2000);
    let before = heap.stats();
    assert!(before.small_pages >= 8 && before.large_cells == 3);
    heap.collect();
    heap.finish_sweeping();
    let expect = expected_live(&heap, &[]);
    let after = heap.stats();
    let kept_large = large.iter().filter(|a| expect.contains(a)).count();
    assert_eq!(after.large_cells, kept_large);
    // Stale stack words may keep a cell (and so its page) for one more cycle.
    assert!(
        after.small_pages <= 3,
        "empty pages go back to the chunk pool ({} left)",
        after.small_pages
    );
    assert!(after.committed_bytes < before.committed_bytes);
    check(&heap, &[], &small);
    // Freed pages are reused before the reservation grows.
    let high = heap.inner.borrow().mem.high();
    black_box(garbage(&heap, 500, 120));
    assert_eq!(heap.inner.borrow().mem.high(), high);
}

#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn budget_triggers_collections() {
    let heap = test_heap(|c| c.initial_budget = PAGE_SIZE);
    for _ in 0..80 {
        black_box(garbage(&heap, 100, 30)); // 25 KiB each round, 2 MiB in all
    }
    let s = heap.stats();
    assert!(s.collections >= 2, "{} collections", s.collections);
    assert!(
        s.small_pages <= 4,
        "{} pages for 1 MiB of garbage",
        s.small_pages
    );
}

/// GC-stress with N = 1 and N = 7: a linked list built under constant collection stays intact,
/// and the verifier runs after every collection.
#[test]
fn gc_stress_every_allocation_and_every_seventh() {
    for n in [1u32, 7] {
        let heap = test_heap(|c| {
            c.gc_stress = n;
            c.verify_after_gc = true;
        });
        let head = heap.new_global(Value::NULL);
        let count: u32 = if cfg!(miri) { 12 } else { 150 };
        for i in 0..count {
            let node = new_node(&heap, i, 2);
            store(&heap, node, 0, heap.global(head));
            store(&heap, node, 1, Value::int32(i as i32));
            heap.set_global(head, node.to_value());
            black_box(garbage(&heap, 1, 1));
        }
        let mut cur = heap.global(head);
        for i in (0..count).rev() {
            let c = CellPtr::from_value(cur).expect("list cell");
            assert_eq!(node_id(c), i);
            assert_eq!(slot(c, 1).get().as_int32(), Some(i as i32));
            cur = slot(c, 0).get();
        }
        assert!(cur.is_null());
        let s = heap.stats();
        assert_eq!(s.collections, u64::from(2 * count / n), "N = {n}");
        heap.drop_global(head);
    }
}

#[test]
fn verifier_catches_corruption() {
    let heap = test_heap(|_| {});
    let a = new_node(&heap, 1, 2);
    let b = new_node(&heap, 2, 0);
    let g = heap.new_global(a.to_value());
    store(&heap, a, 0, b.to_value());
    heap.verify().expect("clean heap");
    let raw = |v: u64| {
        // SAFETY: slot 1 of the live node `a` is a Value slot; the test restores it.
        unsafe { a.field::<u64>(NODE_HEADER + 8).write(v) }
    };
    // An interior pointer is not a cell reference.
    raw((b.addr() + 8) as u64);
    let e = heap.verify().unwrap_err();
    assert!(e.errors[0].contains("not the start of a live cell"), "{e}");
    // The bits 0 and invalid Value bits.
    raw(0);
    assert!(heap.verify().unwrap_err().errors[0].contains("bits 0"));
    raw(ZAP_WORD);
    assert!(heap.verify().unwrap_err().errors[0].contains("invalid Value bits"));
    raw(Value::UNDEFINED.to_bits());
    heap.verify().expect("restored");
    // An unregistered kind byte.
    // SAFETY: `b` is live; the test restores its kind.
    unsafe { b.header() }.set_kind(200);
    assert!(heap.verify().unwrap_err().errors[0].contains("unregistered kind 200"));
    // SAFETY: as above.
    unsafe { b.header() }.set_kind(NODE.get());
    // A bad gc_state.
    // SAFETY: as above.
    unsafe { b.header() }.set_gc_state(9);
    assert!(heap.verify().unwrap_err().errors[0].contains("gc_state 9"));
    // SAFETY: as above.
    unsafe { b.header() }.set_gc_state(gc_state::WHITE);
    // A bad root.
    let bad = heap.new_global(Value::UNDEFINED);
    // SAFETY: the bits form a cell pattern (8-aligned, below 2^47) that is not a live cell;
    // only the verifier reads it, and it is removed before any GC.
    heap.set_global(bad, unsafe {
        Value::from_bits_unchecked((b.addr() + 16) as u64)
    });
    assert!(heap.verify().unwrap_err().errors[0].contains("global handle"));
    heap.drop_global(bad);
    let r = heap.verify().expect("restored");
    assert!(r.small_cells >= 2);
    heap.drop_global(g);
}
