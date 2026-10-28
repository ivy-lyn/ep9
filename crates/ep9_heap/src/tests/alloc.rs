//! Allocation: TLAB bump, holes, overflow, LOS, pre-fill, kinds, limits, fixed offsets.

use std::hint::black_box;

use ep9_value::encoding::VALUE_UNDEFINED;
use ep9_value::{ShapeId, Value};

use super::*;
use crate::layout::{FIRST_CELL_OFFSET, LARGE_CELL_OFFSET, LINE_SIZE, PAGE_MASK, PAGE_SIZE};
use crate::{AllocError, KindError, gc_state};

/// Reads the heap's raw TLAB fields at their fixed offsets, as JIT code would.
fn raw_tlab(heap: &Heap) -> (usize, usize) {
    let base = core::ptr::from_ref(heap).cast::<u8>();
    // SAFETY: offsets 8 and 16 of the heap are `Cell<usize>` fields (asserted at compile
    // time); this thread owns the heap and nothing writes them during the read.
    unsafe {
        (
            base.add(Heap::TLAB_CURSOR_OFFSET).cast::<usize>().read(),
            base.add(Heap::TLAB_LIMIT_OFFSET).cast::<usize>().read(),
        )
    }
}

#[test]
fn new_cell_is_white_prefilled_and_findable() {
    let heap = test_heap(|_| {});
    let c = new_node(&heap, 77, 5);
    // SAFETY: `c` is live (a local on this stack; no GC point since).
    let h = unsafe { c.header() };
    assert_eq!(h.shape_id(), ShapeId::new(0));
    assert_eq!(h.gc_state(), gc_state::WHITE);
    assert_eq!(h.kind(), NODE.get());
    assert_eq!(h.flags(), 0);
    assert_eq!(node_id(c), 77);
    for i in 0..5 {
        assert_eq!(slot(c, i).bits(), VALUE_UNDEFINED, "slot {i} pre-filled");
    }
    assert!(heap.is_cell(c.addr()));
    assert!(!heap.is_cell(c.addr() + 8));
    assert_eq!(heap.find_cell(c.addr() + 8 * 3), Some(c));
    // SAFETY: `c` is live.
    assert_eq!(unsafe { heap.cell_size(c) }, NODE_HEADER + 40);
    assert_eq!(CellPtr::from_value(c.to_value()), Some(c));
}

#[test]
fn tlab_bumps_and_fixed_offsets_match() {
    let heap = test_heap(|_| {});
    assert_eq!(
        raw_tlab(&heap),
        (0, 0),
        "no TLAB before the first allocation"
    );
    let a = new_node(&heap, 1, 0); // 16 B
    let b = new_node(&heap, 2, 1); // 24 B
    let c = new_node(&heap, 3, 30); // 256 B
    assert_eq!(
        a.addr() & !PAGE_MASK,
        FIRST_CELL_OFFSET,
        "first cell of a fresh page"
    );
    assert_eq!(b.addr(), a.addr() + 16);
    assert_eq!(c.addr(), b.addr() + 24);
    let (cursor, limit) = raw_tlab(&heap);
    assert_eq!(cursor, c.addr() + 256);
    assert_eq!(
        limit,
        (a.addr() & PAGE_MASK) + PAGE_SIZE,
        "the fresh page is one hole"
    );
    black_box((a, b, c));
}

#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn large_cells_get_their_own_chunks() {
    let heap = test_heap(|_| {});
    let n = 1100; // 16 + 8800 B > 8 KiB
    let big = new_node(&heap, 5, n);
    assert_eq!(big.addr() & !PAGE_MASK, LARGE_CELL_OFFSET);
    assert_eq!(heap.stats().large_cells, 1);
    assert!(heap.is_cell(big.addr()));
    assert_eq!(
        heap.find_cell(big.addr() + NODE_HEADER + 8 * (n - 1)),
        Some(big)
    );
    assert_eq!(
        heap.find_cell(big.addr() + NODE_HEADER + 8 * n),
        None,
        "past the end"
    );
    for i in [0, n / 2, n - 1] {
        assert_eq!(slot(big, i).bits(), VALUE_UNDEFINED);
    }
    // A 3-chunk object: interior pointers into the last chunk find it.
    let huge_n = (2 * PAGE_SIZE + 4096) / 8;
    let huge = new_node(&heap, 6, huge_n);
    let last = huge.addr() + NODE_HEADER + 8 * (huge_n - 1);
    assert!(last - huge.addr() > 2 * PAGE_SIZE - LARGE_CELL_OFFSET);
    assert_eq!(heap.find_cell(last), Some(huge));
    assert_eq!(slot(huge, huge_n - 1).bits(), VALUE_UNDEFINED);
    assert_eq!(heap.stats().large_cells, 2);
    heap.verify().expect("verifier");
    black_box((big, huge));
}

#[test]
fn errors() {
    let heap = test_heap(|_| {});
    let unknown = KindId::new(99).unwrap();
    assert_eq!(
        heap.alloc(unknown, 16),
        Err(AllocError::UnknownKind(unknown))
    );
    assert_eq!(heap.alloc(NODE, usize::MAX), Err(AllocError::TooLarge));
    let dup = heap.register_kind(
        NODE,
        CellKind {
            name: "dup",
            size: CellSize::Fixed(16),
            trace: |_, _| {},
        },
    );
    assert_eq!(dup, Err(KindError::Taken(NODE)));
}

/// A kind with a fixed size takes only sizes that round to it, on every path: the sweep uses
/// the kind's size, so a 16 B LEAF (24 B kind) would overlap the next cell, and the zap of the
/// dead one would overwrite the live neighbour's header.
#[test]
fn fixed_size_kind_rejects_other_sizes() {
    let heap = test_heap(|_| {});
    let first = heap.alloc(LEAF, 24).unwrap();
    let tlab = raw_tlab(&heap);
    // TLAB, overflow TLAB (> 256 B) and LOS (> 8 KiB) sizes.
    for size in [16, 32, 1024, 9000] {
        assert_eq!(
            heap.alloc(LEAF, size),
            Err(AllocError::WrongSize(LEAF)),
            "size {size}"
        );
    }
    assert_eq!(raw_tlab(&heap), tlab, "a rejected size takes no memory");
    assert_eq!(heap.stats().large_cells, 0);
    // A size that rounds to the fixed size is the same cell.
    let next = heap.alloc(LEAF, 20).unwrap();
    assert_eq!(next.addr(), first.addr() + 24);
    // A dynamic kind takes any size.
    assert!(heap.alloc(NODE, 1024).is_ok());
    black_box((first, next));
}

/// Fill pages with 256 B cells, keep every other one: each page then has 1-line holes. A 1 KiB
/// cell does not fit a hole and goes to the overflow allocator (a fresh page); a 256 B cell
/// reuses a hole.
#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn holes_and_overflow() {
    let heap = test_heap(|_| {});
    let keep = heap.new_global(Value::UNDEFINED);
    // The list is a LOS cell, so the small page holds only the 256 B cells.
    let list = new_node(&heap, 0, 1100);
    heap.set_global(keep, list.to_value());
    let mut first_page = 0;
    for i in 0..600 {
        let c = new_node(&heap, i + 1, 30); // exactly one line
        assert_eq!(c.addr() % LINE_SIZE, 0, "256 B cells stay line aligned");
        if i == 0 {
            first_page = c.addr() & PAGE_MASK;
        }
        if i % 2 == 0 {
            store(&heap, list, i as usize, c.to_value());
        }
    }
    heap.collect();
    heap.finish_sweeping();
    // Small cells now reuse holes in the first page (low pages are swept first).
    let small = new_node(&heap, 9000, 30);
    assert_eq!(
        small.addr() & PAGE_MASK,
        first_page,
        "a hole of the old page"
    );
    // A medium cell of 1 KiB does not fit a 1-line hole: overflow page.
    let medium = new_node(&heap, 9001, 126);
    assert_ne!(medium.addr() & PAGE_MASK, first_page);
    assert_eq!(
        medium.addr() & !PAGE_MASK,
        FIRST_CELL_OFFSET,
        "a fresh overflow page"
    );
    // The kept cells are intact.
    for i in (0..600).step_by(2) {
        let c = CellPtr::from_value(slot(list, i).get()).unwrap();
        assert_eq!(node_id(c), i as u32 + 1);
    }
    heap.verify().expect("verifier");
    heap.drop_global(keep);
    black_box((small, medium));
}

#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn out_of_memory_after_one_collection() {
    let heap = test_heap(|c| c.max_heap_bytes = 4 * PAGE_SIZE);
    let mut handles = Vec::new();
    let err = loop {
        match heap.alloc(NODE, 100 * 1024) {
            Ok(c) => {
                // SAFETY: a new NODE cell; meta word written before the next GC point.
                unsafe { c.field::<u64>(8).write((100 * 1024 - 16) / 8) };
                handles.push(heap.new_global(c.to_value()));
            }
            Err(e) => break e,
        }
        assert!(handles.len() < 100, "the limit must stop allocation");
    };
    assert_eq!(err, AllocError::OutOfMemory);
    assert!(
        heap.stats().collections >= 1,
        "a collection ran before the failure"
    );
    assert!(heap.stats().committed_bytes <= 4 * PAGE_SIZE);
    for h in handles.drain(..) {
        heap.drop_global(h);
    }
    // With the roots gone, the next allocation collects and succeeds.
    let c = heap.alloc(LEAF, 24).expect("memory is free again");
    black_box(c);
}

/// A LOS cell in chunk 0 and a small page in chunk 1 both die: chunk 0 is decommitted, chunk 1
/// stays cached. Under a limit of one page + 128 KiB, a new small page must reuse the cached
/// chunk instead of committing chunk 0.
#[test]
#[cfg_attr(miri, ignore = "slow under Miri")]
fn cached_page_is_reused_under_the_limit() {
    let heap = test_heap(|c| c.max_heap_bytes = PAGE_SIZE + 128 * 1024);
    let big = new_node(&heap, 1, 12_000);
    let small = heap.alloc(LEAF, 24).unwrap();
    assert!(small.addr() > big.addr());
    black_box((big, small));
    heap.collect_with_words(&[]);
    heap.finish_sweeping();
    let s = heap.stats();
    assert_eq!((s.large_cells, s.small_pages), (0, 0));
    assert_eq!(s.committed_bytes, PAGE_SIZE, "one cached page");
    let c = heap.alloc(LEAF, 24).expect("the cached page is reused");
    assert_eq!(c.addr() & PAGE_MASK, small.addr() & PAGE_MASK);
    assert_eq!(heap.stats().committed_bytes, PAGE_SIZE);
}

#[test]
fn gc_stress_keeps_the_limit_at_zero() {
    let heap = test_heap(|c| c.gc_stress = 1);
    let before = heap.stats().collections;
    for i in 0..10 {
        let c = new_node(&heap, i, 2);
        assert_eq!(
            raw_tlab(&heap).1,
            0,
            "stress: the inline path always misses"
        );
        black_box(c);
    }
    assert_eq!(
        heap.stats().collections - before,
        10,
        "one GC per allocation"
    );
    heap.set_gc_stress(0);
    let c = new_node(&heap, 99, 2);
    assert_ne!(raw_tlab(&heap).1, 0, "stress off: the TLAB limit is back");
    black_box(c);
}

/// One unrooted leaf, the only cell of a fresh small page, allocated in a frame of its own.
/// Returns its address in malloc memory, which the conservative scan never reads.
#[inline(never)]
fn leaf_garbage(heap: &Heap) -> Vec<usize> {
    vec![heap.alloc(LEAF, 24).expect("leaf").addr()]
}

/// As [`leaf_garbage`], with the leaf rooted by a global handle.
#[inline(never)]
fn rooted_leaf(heap: &Heap) -> (crate::GlobalHandle, Vec<usize>) {
    let c = heap.alloc(LEAF, 24).expect("leaf");
    (heap.new_global(c.to_value()), vec![c.addr()])
}

/// A large cell needs the commit room of a small page that dies in the large allocation's own
/// GC. The page waits in the lazy sweep queue, so the allocation must sweep it before it gives
/// OutOfMemory. Verifier off: it finishes the sweep and would hide the failure.
#[test]
fn a_large_cell_sweeps_dead_pages_before_out_of_memory() {
    let heap = test_heap(|c| c.max_heap_bytes = PAGE_SIZE);
    let dead = leaf_garbage(&heap);
    paint_stack(&dead);
    clear_stack();
    assert_eq!(heap.stats().committed_bytes, PAGE_SIZE);
    let before = heap.stats().collections;
    let big = heap.alloc(NODE, NODE_HEADER + 8 * 2000);
    if let Ok(c) = big {
        // SAFETY: a new NODE cell; meta word written before the next GC point.
        unsafe { c.field::<u64>(8).write(2000) };
    }
    let st = heap.stats();
    assert_eq!(st.last_marked_cells, 0, "the GC found no live cell");
    assert_eq!(st.collections, before + 1, "one GC");
    let big = big.expect("the dead page makes room");
    assert_eq!((st.small_pages, st.large_cells), (0, 1));
    assert!(st.committed_bytes <= PAGE_SIZE);
    assert!(!heap.is_cell(dead[0]) && heap.is_cell(big.addr()));
    heap.verify().expect("verifier");
}

/// As above for a medium cell (> 256 B), which takes only fresh pages (the overflow TLAB).
/// A first GC retires the TLAB while the leaf is still rooted.
#[test]
fn a_medium_cell_sweeps_dead_pages_before_out_of_memory() {
    let heap = test_heap(|c| c.max_heap_bytes = PAGE_SIZE);
    let (g, dead) = rooted_leaf(&heap);
    paint_stack(&dead);
    heap.collect_with_words(&[]);
    heap.drop_global(g);
    clear_stack();
    let before = heap.stats().collections;
    let m = heap.alloc(NODE, NODE_HEADER + 8 * 128);
    if let Ok(c) = m {
        // SAFETY: a new NODE cell; meta word written before the next GC point.
        unsafe { c.field::<u64>(8).write(128) };
    }
    let st = heap.stats();
    assert_eq!(st.last_marked_cells, 0, "the GC found no live cell");
    assert_eq!(st.collections, before + 1, "one GC");
    let m = m.expect("the dead page makes room");
    assert_eq!(st.small_pages, 1);
    assert_eq!(st.committed_bytes, PAGE_SIZE);
    assert_eq!(
        m.addr() & PAGE_MASK,
        dead[0] & PAGE_MASK,
        "the dead page is reused"
    );
    assert!(heap.is_cell(m.addr()));
    heap.verify().expect("verifier");
}
