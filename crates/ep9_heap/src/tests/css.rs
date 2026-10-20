//! Conservative stack scan (H10, probe cases S1-S7 of 03-gc §2). Not run under Miri.
//!
//! Two kinds of test:
//! - **Rule tests** run a full collection whose conservative scan reads a word array the test
//!   builds (`collect_with_words`). They are exact: no stale stack word can interfere.
//! - **Stack tests** run the real path (`collect`: spill trampoline, scan [sp, stack high)).

use std::hint::black_box;

use ep9_value::Value;

use super::*;
use crate::layout::{FIRST_CELL_OFFSET, PAGE_MASK, PAGE_SIZE};

/// Three adjacent nodes of 32 bytes, plus 20 unrooted nodes after them. Returns the addresses
/// in malloc memory (invisible to the word-array scans).
#[inline(never)]
fn cells(heap: &Heap) -> Vec<usize> {
    (0..23).map(|i| new_node(heap, i, 2).addr()).collect()
}

/// Runs a word-array collection and returns which of `addrs` survived.
fn survivors(heap: &Heap, words: &[usize], addrs: &[usize]) -> Vec<bool> {
    heap.collect_with_words(words);
    heap.finish_sweeping();
    addrs.iter().map(|&a| heap.is_cell(a)).collect()
}

#[test]
fn exact_interior_and_end_pointers() {
    let heap = test_heap(|_| {});
    let a = cells(&heap);
    assert_eq!(a[1], a[0] + 32);
    // Exact base of cell 0, an interior pointer into cell 3, nothing else.
    let s = survivors(&heap, &[a[0], a[3] + 20], &a);
    assert!(s[0] && s[3], "exact and interior pointers keep their cells");
    assert_eq!(s.iter().filter(|x| **x).count(), 2, "{s:?}");
    assert_eq!(heap.stats().last_conservative_roots, 2);
    // Cells 1, 2, 4.. are free now. A one-past-end pointer of cell 0 (= the base of freed
    // cell 1) keeps cell 0 through the end rule (word - 1 lies inside cell 0).
    let s = survivors(&heap, &[a[0] + 32], &a);
    assert!(s[0], "one-past-end pointer keeps its cell");
    assert_eq!(s.iter().filter(|x| **x).count(), 1);
    heap.verify().expect("verifier");
}

#[test]
fn end_rule_also_keeps_the_next_cell() {
    let heap = test_heap(|_| {});
    let a = cells(&heap);
    // a[1] is both "one past a[0]" and "the base of a[1]": both stay (conservative).
    let s = survivors(&heap, &[a[1]], &a);
    assert!(s[0] && s[1]);
    assert_eq!(s.iter().filter(|x| **x).count(), 2);
}

#[test]
fn non_pointers_are_never_roots() {
    let heap = test_heap(|_| {});
    let a = cells(&heap);
    let page = a[0] & PAGE_MASK;
    let words = [
        Value::double(a[0] as f64).to_bits() as usize, // doubles start at 2^49 (probe S6)
        Value::int32(a[0] as i32).to_bits() as usize,
        Value::UNDEFINED.to_bits() as usize,
        page + 64,                        // the page's mark bitmap
        page + FIRST_CELL_OFFSET - 8,     // metadata, just below the first cell
        a[22] + 32 + 8,                   // free memory after the last cell
        (a[0] as u64 | 1 << 48) as usize, // a high-tagged pointer (probe S6)
        a[0] >> 3,                        // a compressed-looking pointer (probe S4)
    ];
    let s = survivors(&heap, &words, &a);
    assert!(s.iter().all(|x| !x), "{s:?}");
    assert_eq!(heap.stats().last_conservative_roots, 0);
}

#[test]
fn interior_pointer_into_a_large_object_tail() {
    let heap = test_heap(|_| {});
    let n = (PAGE_SIZE + 4096) / 8; // two chunks
    let addr = new_node(&heap, 1, n).addr();
    let tail_word = addr + NODE_HEADER + 8 * (n - 2);
    assert!(
        tail_word - (addr & PAGE_MASK) > PAGE_SIZE,
        "points into the second chunk"
    );
    assert_eq!(survivors(&heap, &[tail_word], &[addr]), [true]);
    assert_eq!(survivors(&heap, &[], &[addr]), [false]);
    assert_eq!(heap.stats().large_cells, 0);
}

#[test]
fn conservatively_found_cells_are_traced() {
    let heap = test_heap(|_| {});
    let a = cells(&heap);
    let ptr = |x: usize| {
        // SAFETY: `x` is a live cell of this heap (just allocated).
        unsafe { CellPtr::from_raw(core::ptr::NonNull::new(heap_ptr(&heap, x)).unwrap()) }
    };
    store(&heap, ptr(a[5]), 0, ptr(a[9]).to_value());
    store(&heap, ptr(a[9]), 1, ptr(a[12]).to_value());
    let s = survivors(&heap, &[a[5] + 8], &a);
    let live: Vec<usize> = (0..a.len()).filter(|&i| s[i]).collect();
    assert_eq!(live, [5, 9, 12]);
}

// ---- the real stack ----

#[test]
fn cell_in_a_local_survives() {
    let heap = test_heap(|_| {});
    let c = new_node(&heap, 41, 3);
    store(&heap, c, 2, Value::int32(5));
    let v = black_box(c.to_value());
    for _ in 0..3 {
        black_box(garbage_frame(&heap));
        heap.collect();
    }
    let c2 = CellPtr::from_value(black_box(v)).unwrap();
    assert!(heap.is_cell(c2.addr()));
    assert_eq!(node_id(c2), 41);
    assert_eq!(slot(c2, 2).get().as_int32(), Some(5));
    assert!(heap.stats().last_stack_words > 0);
    heap.verify().expect("verifier");
}

#[inline(never)]
fn garbage_frame(heap: &Heap) -> usize {
    (0..10).map(|i| new_node(heap, 500 + i, 4).addr()).sum()
}

/// Probe S7: a cell held only in a Rust `Vec` is not a root. Every such cell is freed unless
/// a stale stack word kept it, and the scan reports each one it kept.
#[test]
fn cell_only_in_a_vec_is_not_a_root() {
    let heap = test_heap(|_| {});
    let addrs: Vec<usize> = (0..64).map(|i| new_node(&heap, i, 1).addr()).collect();
    heap.collect();
    let expect = expected_live(&heap, &[]);
    let mut freed = 0;
    for &a in &addrs {
        if heap.is_cell(a) {
            assert!(expect.contains(&a), "{a:#x} survived without a root");
        } else {
            freed += 1;
        }
    }
    assert!(freed >= 32, "only {freed} of 64 freed");
}

/// The trampoline path while values sit in many locals: 40 rooted-by-local cells across a GC
/// in a deeper frame.
#[test]
fn many_locals_across_a_deep_collection() {
    let heap = test_heap(|_| {});
    let mut locals = [None::<CellPtr>; 40];
    for (i, l) in locals.iter_mut().enumerate() {
        *l = Some(new_node(&heap, i as u32, 1));
    }
    black_box(&mut locals);
    deep(&heap, 20);
    for (i, l) in black_box(&locals).iter().enumerate() {
        let c = l.unwrap();
        assert!(heap.is_cell(c.addr()));
        assert_eq!(node_id(c), i as u32);
    }
}

#[inline(never)]
fn deep(heap: &Heap, n: u32) {
    if n == 0 {
        heap.collect();
    } else {
        black_box(new_node(heap, 9000 + n, 1));
        deep(heap, n - 1);
    }
}
