//! The external-bytes input of the heap (typedarray design §5.4; TA-0 TA-HEAP): the trigger
//! bound and the major GC it runs, no GC below the bound, the bound against
//! `max_heap_bytes`, the count going down when a weak source frees a dead buffer's bytes
//! (minor and major GCs), GC-stress.
//!
//! ```text
//! model BufferTable (Rust memory): [(buffer cell, held weakly; its bytes)], one weak source
//!   new buffer:   alloc the cell -> root it -> add_external_bytes(n)? -> push (cell, n)
//!   weak source:  a dead cell -> remove_external_bytes(n) during the GC, drop the entry
//!   model:        external_bytes() == the sum of the table's bytes, after every step
//! ```
//!
//! A GC that `add_external_bytes` runs scans the stack conservatively, so a check that a
//! dead buffer is freed uses a GC with precise roots only (`collect_with_words(&[])`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::{AllocError, GlobalHandle};

const MIB: usize = 1 << 20;

/// The model BufferTable: each buffer cell (held weakly) and its external bytes.
#[derive(Default)]
struct Table {
    entries: RefCell<Vec<(Value, usize)>>,
    /// Weak-source runs (one per GC, minor and major).
    runs: Cell<u32>,
}

impl Table {
    fn bytes(&self) -> usize {
        self.entries.borrow().iter().map(|&(_, n)| n).sum()
    }
}

/// Registers the model table's weak source: a dead buffer's bytes are removed during the GC.
fn install_table(heap: &Heap) -> Rc<Table> {
    let table = Rc::new(Table::default());
    let t = Rc::clone(&table);
    let heap_ptr: *const Heap = heap;
    heap.add_weak_source(move |w| {
        // SAFETY: the heap owns this weak source and runs it only while it is alive; the
        // tests do not move the heap after `install_table`. `remove_external_bytes` takes no
        // borrow of the heap, so it may run while the GC holds one.
        let heap = unsafe { &*heap_ptr };
        t.runs.set(t.runs.get() + 1);
        t.entries.borrow_mut().retain(|&(v, n)| {
            let live = w.is_live(v);
            if !live {
                heap.remove_external_bytes(n);
            }
            live
        });
    });
    table
}

/// A buffer cell rooted by the returned handle, with `n` external bytes reported. Not
/// inlined: the cell address stays out of the caller's frame (the conservative scan).
#[inline(never)]
fn new_buffer(heap: &Heap, table: &Table, n: usize) -> Result<GlobalHandle, AllocError> {
    let c = heap.alloc(LEAF, 24).expect("a buffer cell");
    let g = heap.new_global(c.to_value());
    match heap.add_external_bytes(n) {
        Ok(()) => {
            table.entries.borrow_mut().push((c.to_value(), n));
            Ok(g)
        }
        Err(e) => {
            heap.drop_global(g);
            Err(e)
        }
    }
}

fn check_model(heap: &Heap, table: &Table) {
    assert_eq!(
        heap.external_bytes(),
        table.bytes(),
        "the external bytes equal the table's bytes"
    );
}

fn majors(heap: &Heap) -> u64 {
    let s = heap.stats();
    s.collections - s.minor_collections
}

/// A generational heap (S1) with a 1 MiB initial budget: the first external bound is 2 MiB.
fn ext_heap(checked: bool, tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    test_heap(|c| {
        c.generational = true;
        c.verify_after_gc = checked;
        c.initial_budget = MIB;
        tweak(c);
    })
}

/// The trigger fires: the add that passes the bound runs one major GC first, and the next
/// bound is twice the external bytes after that GC.
#[test]
fn an_add_past_the_external_bound_runs_a_major_gc() {
    for checked in [false, true] {
        let heap = ext_heap(checked, |_| {});
        let table = install_table(&heap);
        let mut gs = Vec::new();
        gs.push(new_buffer(&heap, &table, MIB).unwrap());
        gs.push(new_buffer(&heap, &table, MIB).unwrap());
        assert_eq!(
            heap.stats().collections,
            0,
            "2 MiB is at the bound, not past it"
        );
        assert_eq!(heap.external_limit(), 2 * MIB);

        gs.push(new_buffer(&heap, &table, 1).unwrap());
        let s = heap.stats();
        assert_eq!(s.collections, 1, "the add past the bound ran a GC");
        assert_eq!(s.minor_collections, 0, "a major GC");
        assert!(!s.last_minor);
        assert_eq!(table.runs.get(), 1, "the weak source ran in that GC");
        check_model(&heap, &table);
        assert_eq!(heap.external_bytes(), 2 * MIB + 1, "every buffer is rooted");
        assert_eq!(heap.external_limit(), 2 * (2 * MIB + 1), "the next bound");

        // Up to the new bound: no GC; past it: one more major GC.
        gs.push(new_buffer(&heap, &table, 2 * MIB + 1).unwrap());
        assert_eq!(heap.stats().collections, 1);
        gs.push(new_buffer(&heap, &table, 1).unwrap());
        assert_eq!(majors(&heap), 2);
        check_model(&heap, &table);
        for g in gs {
            heap.drop_global(g);
        }
    }
}

/// It does not fire below the bound: many small adds up to the bound run no GC, minor or
/// major, and removes never run one.
#[test]
fn adds_below_the_external_bound_run_no_gc() {
    for checked in [false, true] {
        let heap = ext_heap(checked, |_| {});
        for _ in 0..512 {
            heap.add_external_bytes(4096).unwrap();
        }
        assert_eq!(heap.external_bytes(), 2 * MIB);
        heap.remove_external_bytes(MIB);
        heap.add_external_bytes(MIB).unwrap();
        assert_eq!(heap.stats().collections, 0, "no GC at or below the bound");
        heap.add_external_bytes(1).unwrap();
        assert_eq!(heap.stats().collections, 1, "the next byte passes it");
        heap.remove_external_bytes(2 * MIB + 1);
        assert_eq!(heap.external_bytes(), 0);
    }
}

/// The count goes down when bytes are freed: a dead buffer's bytes leave the count in the
/// GC that finds it dead (a young buffer in a minor GC, an old one only in a major GC), and
/// the next bound reads the lower count (the bound does not ratchet up).
#[test]
fn the_count_goes_down_when_a_weak_source_frees_a_dead_buffer() {
    for checked in [false, true] {
        let heap = ext_heap(checked, |_| {});
        let table = install_table(&heap);
        let keep = new_buffer(&heap, &table, 1000).unwrap();
        let old_dead = new_buffer(&heap, &table, 3 * MIB).unwrap();
        assert_eq!(heap.stats().collections, 1, "3 MiB passed the 2 MiB bound");
        heap.collect_with_words(&[]); // both are old now
        heap.drop_global(old_dead);
        let young_dead = new_buffer(&heap, &table, 5000).unwrap();
        heap.drop_global(young_dead);
        check_model(&heap, &table);
        assert_eq!(heap.external_bytes(), 1000 + 3 * MIB + 5000);

        heap.collect_minor_with_words(&[]);
        assert!(heap.stats().last_minor);
        check_model(&heap, &table);
        assert_eq!(
            heap.external_bytes(),
            1000 + 3 * MIB,
            "a minor GC frees the young buffer; the old one stays until a major GC"
        );

        heap.collect_with_words(&[]);
        check_model(&heap, &table);
        assert_eq!(
            heap.external_bytes(),
            1000,
            "the major GC frees the old buffer"
        );
        assert_eq!(
            heap.external_limit(),
            2 * MIB,
            "the bound after the major GC reads the lower count"
        );
        heap.drop_global(keep);
        heap.collect_with_words(&[]);
        assert_eq!(heap.external_bytes(), 0);
        assert!(table.entries.borrow().is_empty());
    }
}

/// A remove right after the GC (the unmap of a dead reservation runs after the GC ends,
/// typedarray design §5.1) also gives the lower bound: the bound is set at the first add
/// after each major GC.
#[test]
fn a_remove_after_the_gc_lowers_the_next_bound() {
    let heap = ext_heap(false, |_| {});
    heap.add_external_bytes(2 * MIB).unwrap();
    heap.add_external_bytes(6 * MIB).unwrap();
    assert_eq!(heap.stats().collections, 1);
    assert_eq!(
        heap.external_limit(),
        16 * MIB,
        "twice the 8 MiB after the GC"
    );
    heap.collect_with_words(&[]);
    heap.remove_external_bytes(8 * MIB);
    heap.add_external_bytes(1).unwrap();
    assert_eq!(heap.stats().collections, 2, "no GC from the add");
    assert_eq!(
        heap.external_limit(),
        2 * MIB,
        "not 16 MiB: the bytes left after the free"
    );
}

/// The bound counts against `max_heap_bytes`: when the committed heap bytes plus the
/// external bytes would pass it, a major GC runs first (below the growth bound), and the GC's
/// weak source frees a dead buffer. Above the limit an add still succeeds (the heap limit
/// bounds only the heap's commits, H23), and the next GC comes one initial budget later, not
/// on every add.
#[test]
fn the_external_bound_counts_against_max_heap_bytes() {
    for checked in [false, true] {
        let heap = ext_heap(checked, |c| {
            c.max_heap_bytes = 8 * MIB;
            c.initial_budget = 4 * MIB; // the growth bound: 8 MiB
        });
        let table = install_table(&heap);
        let keep = new_buffer(&heap, &table, 0).unwrap();
        let committed = heap.stats().committed_bytes;
        assert!(committed > 0 && committed < MIB);
        let room = 8 * MIB - committed;
        let big = new_buffer(&heap, &table, room).unwrap();
        assert_eq!(
            heap.stats().collections,
            0,
            "at the room, not past it: no GC"
        );
        assert_eq!(
            heap.external_limit(),
            room,
            "the room, below the 8 MiB growth bound"
        );

        // Past the room: one major GC first. (It scans the stack, which may keep the dead
        // buffer one more GC; the GC with precise roots then frees it.)
        heap.drop_global(big);
        let small = new_buffer(&heap, &table, 1).unwrap();
        let s = heap.stats();
        assert_eq!(s.collections, 1, "the add past the room ran a GC");
        assert!(!s.last_minor);
        check_model(&heap, &table);
        heap.collect_with_words(&[]);
        check_model(&heap, &table);
        assert_eq!(heap.external_bytes(), 1, "the dead buffer's bytes are gone");

        // Over the limit with live buffers: the adds succeed; one GC per 4 MiB, not per add.
        let mut gs = vec![keep, small];
        gs.push(new_buffer(&heap, &table, room).unwrap());
        assert_eq!(heap.stats().collections, 3, "the room again");
        assert!(heap.external_bytes() + heap.stats().committed_bytes > 8 * MIB);
        for _ in 0..4 {
            gs.push(new_buffer(&heap, &table, MIB).unwrap());
        }
        assert_eq!(
            heap.stats().collections,
            3,
            "4 MiB above the bytes after the GC"
        );
        gs.push(new_buffer(&heap, &table, 1).unwrap());
        assert_eq!(heap.stats().collections, 4, "then the next GC");
        check_model(&heap, &table);
        for g in gs {
            heap.drop_global(g);
        }
    }
}

/// A count that would overflow is `TooLarge`, with no GC and the count unchanged.
#[test]
fn an_add_that_overflows_the_count_is_too_large() {
    let heap = ext_heap(false, |_| {});
    heap.add_external_bytes(1).unwrap();
    assert_eq!(
        heap.add_external_bytes(usize::MAX),
        Err(AllocError::TooLarge)
    );
    assert_eq!(heap.stats().collections, 0);
    assert_eq!(heap.external_bytes(), 1);
}

/// A remove of more bytes than were reported is a VM bug: it panics.
#[test]
#[should_panic(expected = "remove_external_bytes(2) with only 1 external bytes reported")]
fn a_remove_of_more_than_was_reported_panics() {
    let heap = ext_heap(false, |_| {});
    heap.add_external_bytes(1).unwrap();
    heap.remove_external_bytes(2);
}

/// Under GC-stress an add counts as one allocation: with `gc_stress = 2` every second add
/// runs a GC (below the bound too), and with 1 every add does.
#[test]
fn an_add_counts_as_an_allocation_under_gc_stress() {
    for (n, want) in [(2, 5), (1, 10)] {
        let heap = ext_heap(false, |c| c.gc_stress = n);
        for _ in 0..10 {
            heap.add_external_bytes(16).unwrap();
        }
        assert_eq!(heap.stats().collections, want, "gc_stress = {n}");
        assert_eq!(heap.external_bytes(), 160);
    }
}

/// GC-stress 1 (a GC before every allocation and every add; every 4th GC major), with and
/// without the checked mark: buffers made, kept and dropped in a fixed pattern; the count
/// equals the model table after every step; at the end every buffer is dead and the count
/// is 0. Not under Miri (800 GCs); the other cases run the weak source and the stress path
/// there.
#[test]
#[cfg_attr(miri, ignore)]
fn the_count_follows_the_table_under_gc_stress() {
    for checked in [false, true] {
        let heap = ext_heap(checked, |c| {
            c.gc_stress = 1;
            c.stress_major_every = 4;
        });
        let table = install_table(&heap);
        let mut kept = Vec::new();
        let mut made = 0usize;
        for i in 0..200usize {
            let n = (i * 7919) % 50_000 + 1;
            let g = new_buffer(&heap, &table, n).unwrap();
            made += n;
            if i % 3 == 0 {
                kept.push(g);
            } else {
                heap.drop_global(g);
            }
            if i % 10 == 9 && !kept.is_empty() {
                heap.drop_global(kept.remove(0));
            }
            check_model(&heap, &table);
        }
        assert!(
            table.runs.get() >= 400,
            "a GC before each alloc and each add"
        );
        assert!(
            heap.external_bytes() < made,
            "dead buffers left the count: {} of {made}",
            heap.external_bytes()
        );
        for g in kept {
            heap.drop_global(g);
        }
        heap.collect_with_words(&[]);
        check_model(&heap, &table);
        assert_eq!(heap.external_bytes(), 0, "every buffer is dead");
        assert!(table.entries.borrow().is_empty());
    }
}
