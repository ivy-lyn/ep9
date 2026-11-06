//! loom: the shared work structure of the parallel mark (parallel.rs [`Work`] and [`run`])
//! with 2 markers, and the page claims of the concurrent sweep ([`Claims`]) with the sweeper
//! thread and the heap's thread (10-quality: loom on concurrent GC structures; plan v12 rule
//! 8). Under `--cfg loom` both use the loom primitives, so loom runs every interleaving of the
//! two threads up to the preemption bound:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release -p ep9_heap --lib parallel::loom_tests
//! ```
//!
//! The markers trace a small synthetic graph: an item is a node id; a node's "mark bit" is an
//! atomic swap (as the mark bit's atomic OR in trace.rs: only the marker that sets it traces
//! the node). Each model checks that every reachable node is traced exactly once, that no
//! node outside the reachable set is traced, that both markers end (termination), and that no
//! marker ends with work left. `share_every` is 1, so a marker looks at the shared counters
//! after every node and segments really move between the markers.
//!
//! The sweep models: a page is a `loom::cell::UnsafeCell` that the sweeping thread writes
//! (its sweep). The sweeper thread claims with `claim_open` and ends each claim with `done`;
//! the heap's thread claims the rest (a join) or takes them all with `close` (a dropped heap),
//! then waits with `wait_settled` and reads every page. loom reports a data race if the end
//! wait could return while the sweeper still writes a page; the models also check that each
//! page is swept at most once, and exactly once in a join.
//!
//! [`Work`]: crate::parallel::Work
//! [`run`]: crate::parallel::run
//! [`Claims`]: crate::parallel::Claims

use loom::sync::Arc;
use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

use crate::parallel::{Claims, Marker, Work, run};

struct Graph {
    edges: Vec<Vec<usize>>,
    marked: Vec<AtomicBool>,
    traced: Vec<AtomicUsize>,
}

struct TestMarker {
    stack: Vec<usize>,
    graph: Arc<Graph>,
}

impl Marker for TestMarker {
    type Item = usize;

    fn pop(&mut self) -> Option<usize> {
        self.stack.pop()
    }

    fn trace(&mut self, n: usize) {
        self.graph.traced[n].fetch_add(1, Relaxed);
        for &c in &self.graph.edges[n] {
            if !self.graph.marked[c].swap(true, Relaxed) {
                self.stack.push(c);
            }
        }
    }

    fn len(&self) -> usize {
        self.stack.len()
    }

    fn take_oldest(&mut self, n: usize) -> Vec<usize> {
        self.stack.drain(..n).collect()
    }

    fn refill(&mut self, seg: Vec<usize>) {
        self.stack.extend(seg);
    }
}

fn reachable(edges: &[Vec<usize>], root: usize) -> Vec<bool> {
    let mut seen = vec![false; edges.len()];
    let mut work = vec![root];
    seen[root] = true;
    while let Some(n) = work.pop() {
        for &c in &edges[n] {
            if !seen[c] {
                seen[c] = true;
                work.push(c);
            }
        }
    }
    seen
}

/// Marker 0 starts with `root` (marked); marker 1 starts idle, as a helper does.
fn check_two_markers(edges: Vec<Vec<usize>>, root: usize, bound: usize) {
    let want = reachable(&edges, root);
    let mut b = loom::model::Builder::new();
    b.preemption_bound = Some(bound);
    b.check(move || {
        let n = edges.len();
        let g = Arc::new(Graph {
            edges: edges.clone(),
            marked: (0..n).map(|i| AtomicBool::new(i == root)).collect(),
            traced: (0..n).map(|_| AtomicUsize::new(0)).collect(),
        });
        let work = Arc::new(Work::new(2, 1));
        let helper = {
            let (work, g) = (Arc::clone(&work), Arc::clone(&g));
            loom::thread::spawn(move || {
                let mut m = TestMarker {
                    stack: Vec::new(),
                    graph: g,
                };
                run(&*work, 1, &mut m, false);
                assert!(m.stack.is_empty(), "marker 1 ended with work left");
            })
        };
        let mut m = TestMarker {
            stack: vec![root],
            graph: Arc::clone(&g),
        };
        run(&*work, 0, &mut m, true);
        assert!(m.stack.is_empty(), "marker 0 ended with work left");
        helper.join().unwrap();
        for (i, &w) in want.iter().enumerate() {
            assert_eq!(
                g.traced[i].load(Relaxed),
                usize::from(w),
                "node {i}: traced once if reachable, else never"
            );
        }
    });
}

/// A root with two children that share a grandchild (the mark race), and a node that nothing
/// reaches.
#[test]
fn two_markers_trace_a_diamond_once() {
    check_two_markers(vec![vec![1, 2], vec![3], vec![3], vec![], vec![0]], 0, 3);
}

/// A wide root: the first look publishes half of marker 0's stack, so marker 1 steals.
#[test]
fn two_markers_share_a_wide_root() {
    check_two_markers(
        vec![vec![1, 2, 3, 4], vec![], vec![5], vec![], vec![5], vec![]],
        0,
        2,
    );
}

/// A chain: one node at a time, so the markers pass the work back and forth or one does it all.
#[test]
fn two_markers_end_on_a_chain() {
    check_two_markers(vec![vec![1], vec![2, 0], vec![3], vec![]], 0, 3);
}

/// Marker 0 stops the mark (the unwind of the heap's thread): marker 1, idle or not, ends.
#[test]
fn a_stop_ends_an_idle_marker() {
    loom::model(|| {
        let work: Arc<Work<usize>> = Arc::new(Work::new(2, 1));
        let helper = {
            let work = Arc::clone(&work);
            loom::thread::spawn(move || {
                let mut m = TestMarker {
                    stack: Vec::new(),
                    graph: Arc::new(Graph {
                        edges: vec![vec![]],
                        marked: vec![AtomicBool::new(true)],
                        traced: vec![AtomicUsize::new(0)],
                    }),
                };
                run(&*work, 1, &mut m, false);
            })
        };
        work.stop();
        helper.join().unwrap();
    });
}

/// The sweeper thread and the heap's thread on `pages` pages; `close`: the heap's thread takes
/// every page left at once (a dropped heap), else it claims and sweeps them (a join).
fn check_claims(pages: usize, close: bool, bound: usize) {
    let mut b = loom::model::Builder::new();
    b.preemption_bound = Some(bound);
    b.check(move || {
        let claims = Arc::new(Claims::new(pages));
        let page: Arc<Vec<loom::cell::UnsafeCell<u32>>> =
            Arc::new((0..pages).map(|_| loom::cell::UnsafeCell::new(0)).collect());
        let sweeper = {
            let (claims, page) = (Arc::clone(&claims), Arc::clone(&page));
            loom::thread::spawn(move || {
                while let Some(i) = claims.claim_open() {
                    // SAFETY: the claim gives page `i` to this thread until `done`.
                    page[i].with_mut(|p| unsafe { *p += 1 });
                    claims.done();
                }
            })
        };
        if close {
            claims.close();
        } else {
            while let Some(i) = claims.claim() {
                // SAFETY: the claim gives page `i` to this thread.
                page[i].with_mut(|p| unsafe { *p += 1 });
            }
        }
        claims.wait_settled();
        // After the end wait the heap's thread owns every page (no race with the sweeper).
        for (i, p) in page.iter().enumerate() {
            // SAFETY: no claim is open and none can succeed (`Claims` doc).
            let n = p.with(|p| unsafe { *p });
            assert!(n <= 1, "page {i} swept {n} times");
            assert!(close || n == 1, "page {i} not swept in a join");
        }
        sweeper.join().unwrap();
    });
}

/// A join: the heap's thread sweeps the pages the sweeper has not taken, then waits.
#[test]
fn a_join_sweeps_each_page_once_and_owns_them_after_the_wait() {
    check_claims(2, false, 3);
}

/// A dropped heap: `close` takes every page left; after the wait no page is touched again.
#[test]
fn a_close_ends_the_sweepers_claims() {
    check_claims(2, true, 3);
}
