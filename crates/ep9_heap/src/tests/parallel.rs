//! S2a part 1: the parallel mark (parallel.rs). Each test sets the helper count and a start
//! threshold of 0 for its own thread (`test_controls::with`), so even small marks run in
//! parallel; the default threshold keeps the other tests sequential.
//!
//! | Test | Covers |
//! |---|---|
//! | `the_helper_count_*` | `EP9_GC_HELPERS`: unset, 0, a number, a bad value, the cap |
//! | `a_parallel_mark_*` | S0 and S1 major GCs: the cells kept = the model's reachable set; marked cells and bytes = the sequential mark's; weak slots; word0 shapes; the checked mark (pushed = kept) |
//! | `a_parallel_minor_gc_*` | S1 minor GCs: young cells reached from roots and the S1 logs stay, young garbage goes, V1 runs before every minor GC |
//! | `ephemerons_*`, `the_random_mutator_*` | the ephemeron model fixpoint and the random mutators of stress.rs, every drain in parallel |
//! | `a_panic_*` | a trace function panics on some marker: the panic reaches `collect`, the marks are reset, the next GC keeps every live cell, the pool still works |
//! | `two_heaps_*` | two heaps on two threads mark at once: one holds the pool, the other marks alone; both keep the right cells |
//! | `the_helpers_really_mark` | some helper traces cells (a mark repeated until one does) |
//! | `no_helper_*` | 0 helpers, or a mark below the start threshold: no parallel drain |

#![cfg(not(miri))]

use core::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::collections::BTreeSet;

use ep9_value::Value;

use super::*;
use crate::parallel::{MAX_EXPLICIT_HELPERS, helpers_from, test_controls};
use crate::{GlobalHandle, Space, set_shape};

/// Parallel drains run on this thread so far.
fn drains() -> u64 {
    test_controls::PARALLEL_DRAINS.get()
}

/// Runs `f` with `helpers` helpers and every drain in parallel.
fn parallel<R>(helpers: usize, f: impl FnOnce() -> R) -> R {
    test_controls::with(helpers, 0, f)
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

#[test]
fn the_helper_count_follows_ep9_gc_helpers() {
    // Unset (or not a number): min(cores - 1, 4).
    assert_eq!(helpers_from(None, 1), 0);
    assert_eq!(helpers_from(None, 2), 1);
    assert_eq!(helpers_from(None, 5), 4);
    assert_eq!(helpers_from(None, 18), 4);
    assert_eq!(helpers_from(Some("many"), 18), 4);
    assert_eq!(helpers_from(Some(""), 3), 2);
    assert_eq!(helpers_from(Some("-1"), 3), 2);
    // A number: that many; 0 is the sequential mark.
    assert_eq!(helpers_from(Some("0"), 18), 0);
    assert_eq!(helpers_from(Some("1"), 1), 1);
    assert_eq!(helpers_from(Some(" 6 "), 2), 6);
    assert_eq!(helpers_from(Some("1000"), 2), MAX_EXPLICIT_HELPERS);
}

/// A random graph for the major-GC tests: nodes with 0-4 slots (edges to random nodes),
/// weak boxes (a weak slot and a strong slot), roots by global handles, and (with a shape
/// space) a word0 shape on some nodes.
struct Graph {
    cells: Vec<CellPtr>,
    /// Strong edges and weak edges by cell index.
    strong: Vec<Vec<usize>>,
    weak: Vec<Option<usize>>,
    roots: Vec<usize>,
    handles: Vec<GlobalHandle>,
    /// Shape cells (in `cells`) and the shape index of each cell, if any.
    shape_of: Vec<Option<usize>>,
    /// Cell address -> index.
    index: std::collections::HashMap<usize, usize>,
    /// Cells a GC freed: their memory may hold a new cell now, so no check reads them.
    gone: BTreeSet<usize>,
}

/// A shape-space kind: word0 and one strong slot at +8. 16 bytes.
const PSHAPE: KindId = KindId::new(40).unwrap();

fn pshape_trace(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: a live PSHAPE cell has a Value slot at +8.
    t.visit(unsafe { c.slot(8) });
}

fn graph_heap(generational: bool, checked: bool, shapes: bool) -> Heap {
    let heap = test_heap(|c| {
        c.generational = generational;
        c.verify_after_gc = checked;
        if shapes {
            c.shape_space_bytes = 1 << 20;
        }
    });
    if shapes {
        heap.register_kind_in(
            PSHAPE,
            CellKind {
                name: "test-parallel-shape",
                size: CellSize::Fixed(16),
                trace: pshape_trace,
            },
            Space::Shape,
        )
        .unwrap();
    }
    heap
}

fn build(heap: &Heap, rng: &mut Rng, n: usize, shapes: bool) -> Graph {
    let mut g = Graph {
        cells: Vec::new(),
        strong: Vec::new(),
        weak: Vec::new(),
        roots: Vec::new(),
        handles: Vec::new(),
        shape_of: Vec::new(),
        index: std::collections::HashMap::new(),
        gone: BTreeSet::new(),
    };
    let n_shapes = if shapes { 16 } else { 0 };
    for _ in 0..n_shapes {
        g.push(heap.alloc_shape(PSHAPE, 16).expect("alloc shape"));
    }
    for i in n_shapes..n {
        let c = if rng.below(8) == 0 {
            heap.alloc(WEAKBOX, 24).expect("alloc weak box")
        } else {
            new_node(heap, i as u32, rng.below(5))
        };
        g.push(c);
        let shape = (shapes && rng.below(2) == 0).then(|| rng.below(n_shapes));
        if let Some(s) = shape {
            let id = heap.shape_id_of(g.cells[s]);
            // SAFETY: `c` is a new live cell.
            set_shape(unsafe { c.header() }, id, heap.barrier());
        }
        g.shape_of[i] = shape;
    }
    // Edges: mostly to nearby cells (deep chains and wide fans), some anywhere.
    for i in n_shapes..n {
        let c = g.cells[i];
        let to = |rng: &mut Rng| {
            if rng.below(4) == 0 {
                n_shapes + rng.below(n - n_shapes)
            } else {
                (i + 1 + rng.below(8)).min(n - 1)
            }
        };
        // SAFETY: `c` is live.
        if unsafe { c.header() }.kind() == WEAKBOX.get() {
            let w = to(rng);
            let s = to(rng);
            // SAFETY: a live WEAKBOX has Value slots at +8 (weak) and +16 (strong).
            unsafe {
                c.slot(8)
                    .set(c.header(), g.cells[w].to_value(), heap.barrier());
                c.slot(16)
                    .set(c.header(), g.cells[s].to_value(), heap.barrier());
            }
            g.weak[i] = Some(w);
            g.strong[i].push(s);
        } else {
            for k in 0..node_slots_of(c) {
                if rng.below(5) > 0 {
                    let t = to(rng);
                    store(heap, c, k, g.cells[t].to_value());
                    g.strong[i].push(t);
                }
            }
        }
    }
    if shapes {
        // Each shape's slot names a node.
        for s in 0..n_shapes {
            let t = n_shapes + rng.below(n - n_shapes);
            let sc = g.cells[s];
            // SAFETY: a live PSHAPE cell has a Value slot at +8.
            unsafe {
                sc.slot(8)
                    .set(sc.header(), g.cells[t].to_value(), heap.barrier())
            };
            g.strong[s].push(t);
        }
    }
    for _ in 0..1 + n / 2000 {
        let r = n_shapes + rng.below(n - n_shapes);
        g.roots.push(r);
        g.handles.push(heap.new_global(g.cells[r].to_value()));
    }
    g
}

fn node_slots_of(c: CellPtr) -> usize {
    // SAFETY: callers pass live NODE cells; +8 holds the slot count in its low half.
    (unsafe { c.field::<u64>(8).read() } & 0xFFFF_FFFF) as usize
}

impl Graph {
    fn push(&mut self, c: CellPtr) {
        self.index.insert(c.addr(), self.cells.len());
        self.cells.push(c);
        self.strong.push(Vec::new());
        self.weak.push(None);
        self.shape_of.push(None);
    }

    /// Reads the strong edges of node `i` again from its slots.
    fn reread(&mut self, i: usize) {
        let c = self.cells[i];
        self.strong[i] = (0..node_slots_of(c))
            .filter_map(|s| self.index.get(&(slot(c, s).bits() as usize)).copied())
            .collect();
    }

    /// The model: the cells reachable from the roots (strong edges and word0 shapes).
    fn reachable(&self) -> BTreeSet<usize> {
        let mut live = BTreeSet::new();
        let mut work = self.roots.clone();
        while let Some(i) = work.pop() {
            if live.insert(i) {
                work.extend(&self.strong[i]);
                work.extend(self.shape_of[i]);
            }
        }
        live
    }

    /// Checks the heap against the model after a GC: exactly the reachable cells are live,
    /// and a live weak box's weak slot holds its target if the target lives, else undefined.
    fn check(&mut self, heap: &Heap, live: &BTreeSet<usize>, what: &str) {
        for (i, c) in self.cells.iter().enumerate() {
            if self.gone.contains(&i) {
                continue;
            }
            assert_eq!(
                heap.is_cell(c.addr()),
                live.contains(&i),
                "{what}: cell {i}"
            );
        }
        self.gone
            .extend((0..self.cells.len()).filter(|i| !live.contains(i)));
        for &i in live {
            if let Some(w) = self.weak[i] {
                // SAFETY: cell `i` is live (checked above); +8 is its weak slot.
                let bits = unsafe { self.cells[i].slot(8) }.bits();
                let want = if live.contains(&w) {
                    self.cells[w].to_value().to_bits()
                } else {
                    Value::UNDEFINED.to_bits()
                };
                assert_eq!(bits, want, "{what}: weak slot of cell {i}");
            }
        }
    }

    fn drop_roots(self, heap: &Heap) {
        for h in self.handles {
            heap.drop_global(h);
        }
    }
}

/// One major-GC case: build, mark in parallel (the cells kept = the model), then mark again
/// sequentially and in parallel: the same marked cells and bytes.
fn major_case(seed: u64, n: usize, generational: bool, checked: bool, shapes: bool) {
    let heap = graph_heap(generational, checked, shapes);
    let mut rng = Rng(seed);
    let mut g = build(&heap, &mut rng, n, shapes);
    let live = g.reachable();
    let before = drains();
    parallel(3, || heap.collect_with_words(&[]));
    assert!(drains() > before, "seed {seed:#x}: no parallel drain");
    let what = format!("seed {seed:#x} n {n} gen {generational} checked {checked}");
    g.check(&heap, &live, &what);
    let par = heap.stats();
    test_controls::with(0, 0, || heap.collect_with_words(&[]));
    let seq = heap.stats();
    parallel(2, || heap.collect_with_words(&[]));
    let par2 = heap.stats();
    g.check(&heap, &live, &what);
    for s in [&par, &par2] {
        assert_eq!(
            (s.last_marked_cells, s.last_marked_bytes),
            (seq.last_marked_cells, seq.last_marked_bytes),
            "{what}: marked cells and bytes"
        );
    }
    assert_eq!(
        seq.last_marked_cells,
        live.len(),
        "{what}: marked = reachable"
    );
    heap.verify().expect("verifier");
    g.drop_roots(&heap);
}

#[test]
fn a_parallel_mark_keeps_exactly_the_reachable_cells() {
    for seed in 0..6u64 {
        major_case(
            0x5A2A_0000 + seed,
            20_000 + 7_000 * seed as usize,
            false,
            false,
            false,
        );
    }
}

/// The helpers really mark: a helper may wake only after a short mark has ended (a loaded
/// host), so the test marks a 100 K-cell graph until some helper traced cells (at most 200
/// times), and checks the result each time.
#[test]
fn the_helpers_really_mark() {
    let heap = graph_heap(false, false, false);
    let mut rng = Rng(0x5A2A_9000);
    let mut g = build(&heap, &mut rng, 100_000, false);
    let live = g.reachable();
    let helped = test_controls::HELPER_CELLS.get();
    for round in 0..200 {
        parallel(3, || heap.collect_with_words(&[]));
        g.check(&heap, &live, &format!("round {round}"));
        if test_controls::HELPER_CELLS.get() > helped {
            break;
        }
    }
    assert!(
        test_controls::HELPER_CELLS.get() > helped,
        "no helper traced a cell in 200 marks"
    );
    g.drop_roots(&heap);
}

#[test]
fn a_parallel_mark_of_a_generational_heap_and_a_checked_mark() {
    for seed in 0..4u64 {
        major_case(0x5A2A_1000 + seed, 30_000, true, seed % 2 == 0, false);
        major_case(0x5A2A_2000 + seed, 30_000, false, true, false);
    }
}

#[test]
fn a_parallel_mark_marks_word0_shapes_once() {
    for seed in 0..4u64 {
        major_case(0x5A2A_3000 + seed, 25_000, seed % 2 == 0, seed < 2, true);
    }
}

/// S1 minor GCs in parallel: an old graph (one major GC), then rounds of young chains, each
/// hung from an old host (through the barrier logs), and young garbage. The verify after each
/// GC is on, so V1 runs before every minor GC. A minor GC keeps every old cell, so the model
/// after it is the reachable set plus the cells live before it.
#[test]
fn a_parallel_minor_gc_keeps_young_cells_reached_from_old_hosts() {
    let heap = graph_heap(true, true, false);
    let mut rng = Rng(0x5A2A_4000);
    let mut g = build(&heap, &mut rng, 20_000, false);
    parallel(3, || heap.collect_with_words(&[]));
    let mut alive = g.reachable();
    g.check(&heap, &alive, "first major");
    let old_nodes: Vec<usize> = alive
        .iter()
        .copied()
        // SAFETY: the cells of `alive` are live.
        .filter(|&i| unsafe { g.cells[i].header() }.kind() == NODE.get())
        .filter(|&i| node_slots_of(g.cells[i]) > 0)
        .collect();
    let v1_before = heap.inner.borrow().v1_runs;
    let mut minors = 0;
    for round in 0..6 {
        for _ in 0..4 {
            let host = old_nodes[rng.below(old_nodes.len())];
            let len = 500 + rng.below(3000);
            let first = g.cells.len();
            for k in 0..len {
                let c = new_node(&heap, (first + k) as u32, 2);
                g.push(c);
                if k > 0 {
                    store(&heap, g.cells[first + k - 1], 0, c.to_value());
                    g.reread(first + k - 1);
                }
            }
            store(&heap, g.cells[host], 0, g.cells[first].to_value());
            g.reread(host);
        }
        for _ in 0..2000 {
            new_node(&heap, 0, 1);
        }
        let mut want = g.reachable();
        want.extend(alive.iter().copied());
        parallel(2 + round % 3, || heap.collect_minor_with_words(&[]));
        minors += 1;
        assert!(heap.stats().last_minor);
        g.check(&heap, &want, &format!("minor round {round}"));
        alive = want;
    }
    assert_eq!(
        heap.inner.borrow().v1_runs - v1_before,
        minors,
        "V1 before every minor GC"
    );
    // A major GC frees the old cells the young chains cut off.
    let live = g.reachable();
    parallel(3, || heap.collect_with_words(&[]));
    g.check(&heap, &live, "final major");
    heap.verify().expect("verifier");
    g.drop_roots(&heap);
}

#[test]
fn ephemerons_match_the_model_fixpoint_in_parallel() {
    parallel(3, || {
        for seed in 0..120u64 {
            super::ephemeron::random_graph(0x5A2A_E000 + seed, seed % 2 == 1);
        }
    });
}

#[test]
fn the_random_mutators_with_every_drain_in_parallel() {
    let before = drains();
    parallel(2, || {
        super::stress::run_with(0x5A2A_5000, 1500, 3, false);
        super::stress::run_with(0x5A2A_5001, 1500, 1, true);
        super::stress::run_with(0x5A2A_5002, 6000, 0, true);
    });
    assert!(drains() > before);
}

/// A kind whose trace function panics while `PANIC_ALL` is set, on whichever marker traces it.
const PANICKER: KindId = KindId::new(41).unwrap();
static PANIC_ALL: AtomicBool = AtomicBool::new(false);

fn panicker_trace(c: CellPtr, t: &mut Tracer<'_>) {
    if PANIC_ALL.load(Relaxed) {
        std::panic::resume_unwind(Box::new("test parallel trace panic"));
    }
    // SAFETY: a live PANICKER cell has a Value slot at +8.
    t.visit(unsafe { c.slot(8) });
}

#[test]
fn a_panic_in_a_trace_function_on_any_marker_reaches_collect() {
    for generational in [false, true] {
        let heap = graph_heap(generational, false, false);
        heap.register_kind(
            PANICKER,
            CellKind {
                name: "test-parallel-panicker",
                size: CellSize::Fixed(24),
                trace: panicker_trace,
            },
        )
        .unwrap();
        let mut rng = Rng(0x5A2A_6000 + u64::from(generational));
        let mut g = build(&heap, &mut rng, 30_000, false);
        // Panicking cells hang from many live nodes, so some marker meets one soon.
        let live_nodes: Vec<usize> = g
            .reachable()
            .into_iter()
            // SAFETY: the reachable cells are live.
            .filter(|&i| unsafe { g.cells[i].header() }.kind() == NODE.get())
            .filter(|&i| node_slots_of(g.cells[i]) > 0)
            .collect();
        for k in 0..64 {
            let host = live_nodes[(k * 7919) % live_nodes.len()];
            let p = heap.alloc(PANICKER, 24).expect("alloc");
            g.push(p);
            let s = node_slots_of(g.cells[host]) - 1;
            store(&heap, g.cells[host], s, p.to_value());
            g.reread(host);
        }
        let live = g.reachable();
        PANIC_ALL.store(true, Relaxed);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parallel(3, || heap.collect_with_words(&[]))
        }));
        PANIC_ALL.store(false, Relaxed);
        let p = r.expect_err("the trace panic reaches collect");
        assert_eq!(p.downcast_ref::<&str>(), Some(&"test parallel trace panic"));
        // Nothing was freed, and the next GC (in parallel) keeps exactly the live cells.
        let before = drains();
        parallel(3, || heap.collect_with_words(&[]));
        assert!(drains() > before, "the pool works after a panic");
        g.check(
            &heap,
            &live,
            &format!("after the panic, generational {generational}"),
        );
        heap.verify().expect("verifier");
        g.drop_roots(&heap);
    }
}

#[test]
fn two_heaps_on_two_threads_mark_at_once() {
    let threads: Vec<_> = (0..2u64)
        .map(|k| {
            std::thread::Builder::new()
                .stack_size(8 << 20)
                .spawn(move || {
                    let heap = graph_heap(k == 1, false, false);
                    let mut rng = Rng(0x5A2A_7000 + k);
                    let mut g = build(&heap, &mut rng, 25_000, false);
                    let live = g.reachable();
                    for round in 0..12 {
                        // Thread 0 marks alone when the pool is busy; thread 1 waits for it,
                        // so at least one mark of this test holds the pool.
                        test_controls::with_wait(3, 0, k == 1, || heap.collect_with_words(&[]));
                        g.check(&heap, &live, &format!("thread {k} round {round}"));
                    }
                    g.drop_roots(&heap);
                    drains()
                })
                .unwrap()
        })
        .collect();
    let total: u64 = threads.into_iter().map(|t| t.join().unwrap()).sum();
    assert!(total > 0, "some mark held the pool");
}

#[test]
fn no_helper_or_a_small_mark_means_no_parallel_drain() {
    let heap = graph_heap(false, false, false);
    let mut rng = Rng(0x5A2A_8000);
    let mut g = build(&heap, &mut rng, 5_000, false);
    let live = g.reachable();
    let before = drains();
    test_controls::with(0, 0, || heap.collect_with_words(&[]));
    test_controls::with(4, usize::MAX, || heap.collect_with_words(&[]));
    // The default threshold: a mark of a few thousand cells stays sequential.
    test_controls::with(4, crate::parallel::START_AFTER, || {
        heap.collect_with_words(&[]);
    });
    assert!(live.len() < crate::parallel::START_AFTER);
    assert_eq!(drains(), before);
    g.check(&heap, &live, "sequential");
    g.drop_roots(&heap);
}
