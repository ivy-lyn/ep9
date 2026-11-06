//! S2a part 2: the concurrent sweep (parallel.rs part 2) and the GC-stress control of the
//! parallel paths (route GCPAR-STRESS). Each test turns the concurrent sweep on for its own
//! thread (`test_controls::with_sweeper`) and sets 2 helpers (`with_helpers`), so it does not
//! depend on the host's core count; the other unit tests keep the lazy sweep.
//!
//! | Test | Covers |
//! |---|---|
//! | `the_concurrent_sweep_frees_what_the_lazy_sweep_frees` | S0, S1 major, S1 minor, a shape space: the cells kept (the model), the freed cells and bytes, the pages and the committed bytes equal the lazy sweep's; the sweeper thread swept the pages |
//! | `allocation_takes_the_pages_the_sweeper_swept` | the allocations file the sweeper's pages and reuse them: no new page |
//! | `a_checked_sweep_on_the_sweeper_*` | the verify after a GC with a checked job: no false error, the kept count; a planted fault on a page the sweeper swept fails the verify |
//! | `a_panic_in_a_size_function_on_the_sweeper_reaches_the_heaps_thread` | the panic comes out of `finish_sweeping`; the heap still works |
//! | `a_heap_dropped_during_a_concurrent_sweep` | the drop waits for the sweeper thread (it never touches the unmapped cage) |
//! | `two_heaps_sweep_concurrently` | two heaps on two threads, one sweeper thread: both keep the right cells |
//! | `the_random_mutators_with_the_concurrent_sweep` | the random mutators of stress.rs (S0 GC-stress, S1 checked GC-stress, S1 budget policy) |
//! | `gc_stress_reaches_the_parallel_mark_and_the_concurrent_sweep` | GCPAR-STRESS: under GC-stress the GCs that `STRESS_PARALLEL_EVERY` picks mark in parallel and sweep concurrently; without GC-stress the same small heap does neither |
//! | `no_concurrent_sweep_*` | 0 helpers; fewer pages than the start threshold |

#![cfg(not(miri))]

use core::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::parallel::{Rng, build, graph_heap};
use super::*;
use crate::parallel::{SWEEP_START_PAGES, test_controls};

/// What one GC and its whole sweep left.
#[derive(Debug, PartialEq, Eq)]
struct Left {
    live: Vec<bool>,
    freed_cells: u64,
    freed_bytes: u64,
    small_pages: usize,
    committed_bytes: usize,
}

/// One GC of a random graph of `n` cells and its whole sweep, lazy or concurrent (the sweeper
/// thread sweeps every page: the test waits for it). Checks the cells kept against the model.
fn gc_and_sweep(
    seed: u64,
    n: usize,
    generational: bool,
    minor: bool,
    shapes: bool,
    concurrent: bool,
) -> (Left, u64) {
    let heap = graph_heap(generational, false, shapes);
    let mut rng = Rng(seed);
    let mut g = build(&heap, &mut rng, n, shapes);
    let live = g.reachable();
    let gc = || {
        if minor {
            heap.collect_minor_with_words(&[]);
        } else {
            heap.collect_with_words(&[]);
        }
    };
    let what = format!(
        "seed {seed:#x} gen {generational} minor {minor} shapes {shapes} concurrent {concurrent}"
    );
    if concurrent {
        test_controls::with_helpers(2, || test_controls::with_sweeper(1, false, gc));
        assert!(heap.sweeping_concurrently(), "{what}: no concurrent sweep");
        assert!(
            heap.wait_for_sweeper(),
            "{what}: the sweeper thread did not end in 10 s"
        );
    } else {
        gc();
        assert!(!heap.sweeping_concurrently(), "{what}");
    }
    heap.finish_sweeping();
    assert!(!heap.sweeping_concurrently(), "{what}: the sweep ended");
    g.check(&heap, &live, &what);
    let st = heap.stats();
    let left = Left {
        live: (0..g.cells.len()).map(|i| live.contains(&i)).collect(),
        freed_cells: st.freed_cells,
        freed_bytes: st.freed_bytes,
        small_pages: st.small_pages,
        committed_bytes: st.committed_bytes,
    };
    let pages = heap.sweeper_pages();
    g.drop_roots(&heap);
    (left, pages)
}

#[test]
fn the_concurrent_sweep_frees_what_the_lazy_sweep_frees() {
    let cases = [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
    ];
    for (k, &(generational, minor, shapes)) in cases.iter().enumerate() {
        let seed = 0x5EE9_0000 + k as u64;
        let (lazy, none) = gc_and_sweep(seed, 40_000, generational, minor, shapes, false);
        assert_eq!(none, 0);
        let (conc, pages) = gc_and_sweep(seed, 40_000, generational, minor, shapes, true);
        assert_eq!(
            lazy, conc,
            "case {k}: the concurrent sweep left another heap"
        );
        assert!(lazy.freed_cells > 0, "case {k}: the GC freed nothing");
        assert_eq!(
            pages as usize,
            lazy_pages(seed, generational, shapes),
            "case {k}: the sweeper swept every page"
        );
    }
}

/// The small pages of the graph of [`gc_and_sweep`] before its GC.
fn lazy_pages(seed: u64, generational: bool, shapes: bool) -> usize {
    let heap = graph_heap(generational, false, shapes);
    let g = build(&heap, &mut Rng(seed), 40_000, shapes);
    let n = heap.stats().small_pages;
    g.drop_roots(&heap);
    n
}

#[test]
fn allocation_takes_the_pages_the_sweeper_swept() {
    let heap = test_heap(|_| {});
    // 8 pages of 32-byte nodes; one in 1024 stays (a global), so every page is recyclable.
    let mut keep = Vec::new();
    for i in 0..8 * 8192u32 {
        let c = new_node(&heap, i, 2);
        if i % 1024 == 0 {
            keep.push(heap.new_global(c.to_value()));
        }
    }
    let before = heap.stats();
    test_controls::with_helpers(2, || {
        test_controls::with_sweeper(1, false, || heap.collect_with_words(&[]))
    });
    assert!(
        heap.wait_for_sweeper(),
        "the sweeper thread did not end in 10 s"
    );
    let freed = heap.stats().freed_bytes;
    assert!(freed > 1 << 20, "{freed} bytes freed");
    // Garbage of half the freed bytes: the allocations file the sweeper's pages and fill them.
    let mut bytes = 0;
    while bytes < freed / 2 {
        new_node(&heap, 0, 2);
        bytes += 32;
    }
    let after = heap.stats();
    assert_eq!(after.collections, 1);
    assert!(
        heap.sweeper_pages() > 0,
        "the allocations filed the sweeper's pages"
    );
    assert!(
        after.small_pages <= before.small_pages && after.committed_bytes <= before.committed_bytes,
        "the allocations took new pages: {before:?} -> {after:?}"
    );
    heap.finish_sweeping();
    heap.verify().expect("verifier");
    for g in keep {
        heap.drop_global(g);
    }
}

/// The verify after a GC joins the checked job at once; `with_sweeper(_, true)` makes the
/// join wait until the sweeper thread has taken every page, so the sweeper's checked sweep
/// gives the kept count and the errors.
#[test]
fn a_checked_sweep_on_the_sweeper_keeps_the_verify_clean() {
    for generational in [false, true] {
        let heap = graph_heap(generational, true, false);
        let mut g = build(
            &heap,
            &mut Rng(0x5EE9_2000 + u64::from(generational)),
            40_000,
            false,
        );
        let live = g.reachable();
        test_controls::with_helpers(2, || {
            test_controls::with_sweeper(1, true, || heap.collect_with_words(&[]))
        });
        assert!(
            heap.sweeper_pages() > 0,
            "gen {generational}: the sweeper swept no page"
        );
        g.check(&heap, &live, "checked");
        g.drop_roots(&heap);
    }
}

#[test]
fn a_checked_sweep_on_the_sweeper_finds_a_planted_fault() {
    let heap = test_heap(|c| c.verify_after_gc = true);
    // As in verify.rs: a dead node whose size reaches into its live neighbour.
    let dead = new_node(&heap, 1, 0);
    let live = new_node(&heap, 2, 0);
    assert_eq!(live.addr(), dead.addr() + 16);
    let g = heap.new_global(live.to_value());
    // SAFETY: `dead` is a NODE cell that nothing reaches; offset 8 is its meta word.
    unsafe { dead.field::<u64>(8).write((1u64 << 32) | 1) };
    let r = catch_unwind(AssertUnwindSafe(|| {
        test_controls::with_helpers(2, || {
            test_controls::with_sweeper(1, true, || heap.collect_with_words(&[]))
        })
    }));
    let p = r.expect_err("the verify after the GC did not fail");
    let msg = p.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("overlaps the next cell"), "{msg}");
    assert_eq!(heap.sweeper_pages(), 1, "the sweeper thread swept the page");
    // SAFETY: `live` is rooted; its word0 is a header.
    let kind = unsafe { live.header() }.kind();
    assert_eq!(kind, NODE.get(), "the live header was zapped");
    heap.drop_global(g);
}

/// A kind whose size function panics on the sweeper thread while `PANIC_ON_SWEEPER` is set.
const SWEEP_PANICKER: KindId = KindId::new(42).unwrap();
static PANIC_ON_SWEEPER: AtomicBool = AtomicBool::new(false);

fn no_trace(_: CellPtr, _: &mut Tracer<'_>) {}

fn sweep_panicker_size(_: CellPtr) -> usize {
    if PANIC_ON_SWEEPER.load(Relaxed) && std::thread::current().name() == Some("ep9-gc-sweeper") {
        std::panic::resume_unwind(Box::new("test sweeper size panic"));
    }
    24
}

#[test]
fn a_panic_in_a_size_function_on_the_sweeper_reaches_the_heaps_thread() {
    let heap = test_heap(|_| {});
    heap.register_kind(
        SWEEP_PANICKER,
        CellKind {
            name: "test-sweep-panicker",
            size: CellSize::Dynamic(sweep_panicker_size),
            trace: no_trace,
        },
    )
    .unwrap();
    let keep = new_node(&heap, 1, 1);
    let g = heap.new_global(keep.to_value());
    // Dead panicking cells on every page: the sweep reads their size to free them.
    for _ in 0..40_000 {
        heap.alloc(SWEEP_PANICKER, 24).expect("alloc");
    }
    PANIC_ON_SWEEPER.store(true, Relaxed);
    let r = test_controls::with_helpers(2, || {
        test_controls::with_sweeper(1, true, || {
            heap.collect_with_words(&[]);
            assert!(heap.sweeping_concurrently());
            catch_unwind(AssertUnwindSafe(|| heap.finish_sweeping()))
        })
    });
    PANIC_ON_SWEEPER.store(false, Relaxed);
    let p = r.expect_err("the sweeper's panic did not reach the heap's thread");
    assert_eq!(p.downcast_ref::<&str>(), Some(&"test sweeper size panic"));
    assert!(!heap.sweeping_concurrently());
    // The heap still works: the next GC keeps the rooted node and frees the rest.
    heap.collect_with_words(&[]);
    assert!(heap.is_cell(keep.addr()));
    heap.verify().expect("verifier");
    heap.drop_global(g);
}

#[test]
fn a_heap_dropped_during_a_concurrent_sweep() {
    for round in 0..20u64 {
        let heap = graph_heap(round % 2 == 1, false, false);
        let g = build(&heap, &mut Rng(0x5EE9_3000 + round), 40_000, false);
        test_controls::with_helpers(2, || {
            test_controls::with_sweeper(1, false, || heap.collect_with_words(&[]))
        });
        assert!(heap.sweeping_concurrently());
        g.drop_roots(&heap);
        // The drop closes the job and waits for the sweeper thread's page.
        drop(heap);
    }
    // The sweeper thread still works.
    let (_, pages) = gc_and_sweep(0x5EE9_3100, 20_000, false, false, false, true);
    assert!(pages > 0);
}

#[test]
fn two_heaps_sweep_concurrently() {
    let run = |seed: u64| {
        move || {
            for round in 0..4u64 {
                let (left, pages) =
                    gc_and_sweep(seed + round, 30_000, round % 2 == 0, false, false, true);
                assert!(
                    pages > 0 && left.freed_cells > 0,
                    "seed {seed:#x} round {round}"
                );
            }
        }
    };
    let a = std::thread::spawn(run(0x5EE9_4000));
    let b = std::thread::spawn(run(0x5EE9_5000));
    a.join().expect("heap A");
    b.join().expect("heap B");
}

#[test]
fn the_random_mutators_with_the_concurrent_sweep() {
    test_controls::with_helpers(2, || {
        test_controls::with_sweeper(1, false, || {
            super::stress::run_with(0x5EE9_6000, 1500, 3, false);
            super::stress::run_with(0x5EE9_6001, 1500, 1, true);
            super::stress::run_with(0x5EE9_6002, 6000, 0, true);
        })
    });
}

/// A small generational heap (2,000 rooted nodes, under `SWEEP_START_PAGES` pages and over
/// `STRESS_START_AFTER` cells): the parallel drains and the concurrent sweeps that `n`
/// allocations start, with GC-stress 1 (every allocation collects) or without it (a full GC
/// every 50 allocations).
fn small_heap_parallel_paths(gc_stress: bool, n: u32) -> (u64, u64) {
    let heap = test_heap(|c| {
        c.generational = true;
        c.verify_after_gc = true;
    });
    let head = new_node(&heap, 0, 1);
    let g = heap.new_global(head.to_value());
    let mut tail = head;
    for i in 1..2_000 {
        let c = new_node(&heap, i, 1);
        store(&heap, tail, 0, c.to_value());
        tail = c;
    }
    assert!(heap.stats().small_pages < SWEEP_START_PAGES);
    if gc_stress {
        heap.set_gc_stress(1);
    }
    let drains = test_controls::PARALLEL_DRAINS.get();
    let jobs = heap.concurrent_jobs();
    test_controls::with_helpers(2, || {
        test_controls::with_sweeper(SWEEP_START_PAGES, false, || {
            for i in 0..n {
                new_node(&heap, 2_000 + i, 0);
                if !gc_stress && i % 50 == 0 {
                    heap.collect();
                }
            }
        })
    });
    heap.set_gc_stress(0);
    let out = (
        test_controls::PARALLEL_DRAINS.get() - drains,
        heap.concurrent_jobs() - jobs,
    );
    heap.verify().expect("verifier");
    heap.drop_global(g);
    out
}

/// Route GCPAR-STRESS: GC-stress marks never reached the 64 K-cell start threshold, so the
/// GC-stress runs (CI test262-gc) did not run the parallel mark. Now every
/// `STRESS_PARALLEL_EVERY`th stress GC (the first one too) uses both parallel paths.
#[test]
fn gc_stress_reaches_the_parallel_mark_and_the_concurrent_sweep() {
    let n = 3 * crate::parallel::STRESS_PARALLEL_EVERY;
    let (drains, jobs) = small_heap_parallel_paths(true, n);
    assert!(drains >= 3, "{drains} parallel drains in {n} GC-stress GCs");
    assert!(jobs >= 3, "{jobs} concurrent sweeps in {n} GC-stress GCs");
    let (drains, jobs) = small_heap_parallel_paths(false, n);
    assert_eq!((drains, jobs), (0, 0), "a small heap without GC-stress");
}

#[test]
fn no_concurrent_sweep_with_0_helpers() {
    let heap = graph_heap(false, false, false);
    let g = build(&heap, &mut Rng(0x5EE9_7000), 40_000, false);
    test_controls::with_helpers(0, || {
        test_controls::with_sweeper(1, false, || heap.collect_with_words(&[]))
    });
    assert!(
        !heap.sweeping_concurrently(),
        "EP9_GC_HELPERS=0 is the lazy sweep"
    );
    assert!(heap.unswept_pages() > 0);
    g.drop_roots(&heap);
}

#[test]
fn no_concurrent_sweep_below_the_start_threshold() {
    let heap = graph_heap(false, false, false);
    let g = build(&heap, &mut Rng(0x5EE9_8000), 2_000, false);
    assert!(heap.stats().small_pages < SWEEP_START_PAGES);
    test_controls::with_helpers(2, || {
        test_controls::with_sweeper(SWEEP_START_PAGES, false, || heap.collect_with_words(&[]))
    });
    assert!(!heap.sweeping_concurrently(), "a small heap sweeps lazily");
    g.drop_roots(&heap);
}
