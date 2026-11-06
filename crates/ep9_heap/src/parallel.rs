//! S2a part 1: the parallel mark (03-gc §4 S2a). The world stays stopped: the heap's thread and
//! the helpers (at most [`MAX_HELPERS`] by default) drain the mark stack together, also after
//! each ephemeron round; the heap's thread alone resolves the ephemerons, then the weak slots.
//!
//! ```text
//! drain_to_fixpoint -> parallel::drain(t)
//!   helpers() == 0 (EP9_GC_HELPERS=0), a checked tracer in verify mode, Miri, loom:
//!       t.drain()                               the sequential mark, unchanged
//!   else: the heap's thread traces START_AFTER cells alone; work left and the pool free:
//!       Job { Work, mem, kinds } on the heap's thread's stack -> the pool wakes the helpers
//!       marker 0 (heap's thread) + markers 1..=n (helpers), each its own Tracer (Shared mode):
//!         pop a cell -> trace it -> every share_every cells: some marker idle and no segment
//!         queued -> publish the oldest half of the local stack to the own queue
//!         local stack empty -> find(): own queue, then steal from the other queues
//!       termination: `active` counts the markers that hold or may take work; a marker with
//!         an empty stack and nothing to steal leaves it, and one that sees a queued segment
//!         joins it again before it steals; `active == 0` = every stack and queue is empty
//!       the heap's thread closes the job, waits until every helper that joined is done,
//!       then takes each helper's harvest (weak slots, pending ephemerons, counts, errors)
//! ```
//!
//! Rules (03-gc §4 S2a; rule 8 of the plan: markers reach cells only through the Tracer visit
//! paths):
//!
//! | Rule | How |
//! |---|---|
//! | A cell is traced once | the marker that sets its mark bit traces it: an atomic OR on the same mark bitmap (a LOS cell: the `swap` of its header mark byte, as before) |
//! | No lost mark | every mark-bit write of a Shared tracer is an atomic RMW; the sequential mark keeps its load and store |
//! | Fields seen | the mutator is stopped; the job hand-off and every segment pass through a lock, so each marker sees every store made before the GC (H11: GC-visible fields are Relaxed atomics) |
//! | Trace functions | run on any marker thread: they read only cells and data that lives as long as the isolate, never thread-local state, and never call into the heap (trace.rs) |
//! | Roots, CSS, S1 logs, ephemeron resolve steps, weak slots | the heap's thread only, before or after a parallel drain (the drain after an ephemeron round is parallel too) |
//! | A panic | a helper's panic is caught and raised again on the heap's thread after every helper is done; a panic of the heap's thread stops the helpers first; the unwind then resets the marks (heap.rs `UnmarkOnUnwind`) |
//!
//! The control `EP9_GC_HELPERS` (V12-D47): unset or not a number gives min(available cores - 1,
//! 4); a number gives that many helpers (at most [`MAX_EXPLICIT_HELPERS`]); 0 is the sequential
//! mark. The instruction counters count every thread of the process, so instruction floors read
//! `EP9_GC_HELPERS=0`; the parallel gain is a wall-time reading.
//!
//! The helper threads are one pool per process, started at the first parallel mark and kept
//! for the life of the process (parked between marks). One heap at a time uses the pool; a
//! second heap that marks meanwhile on another thread marks alone.
//!
//! [`Work`] is the shared work structure. Under `--cfg loom` it uses the loom primitives
//! (tests/loom.rs: 2 markers), and the pool is not built.
//!
//! S2a part 2: the concurrent sweep (03-gc §4 S2a; the part-2 note there is the design note).
//! One sweeper thread per process sweeps the small pages of a GC while the mutator runs:
//!
//! ```text
//! end of a GC (heap.rs): helpers() > 0 and >= SWEEP_START_PAGES pages queued (1 page in a
//!   GC-stress GC that STRESS_PARALLEL_EVERY picks), else the lazy sweep as before
//!   Job { cage base, a copy of the kind table, the page list, Claims } -> publish
//! sweeper thread: claim_open -> sweep the page (heap.rs Sweeper) -> result, counts and
//!   errors into the job -> done; until every page is taken
//! heap's thread, an allocation that needs a page: file the sweeper's results (recyclable
//!   lists; a free page back to Memory: only the heap's thread changes Memory), else claim
//!   and sweep a page itself, else a fresh page
//! heap's thread, the next GC / finish_sweeping / a verify / is_cell / the drop: claim and
//!   sweep every page left (the drop: close), wait until no sweeper claim is open, file
//! ```
//!
//! | Rule (TSan-free) | How |
//! |---|---|
//! | One thread per page | a page belongs to the thread whose claim took it ([`Claims`]: one atomic counter) until its result is filed; the mutator allocates only in pages it took from its own lists |
//! | No sweep during a GC | every GC ends the last sweep before its root and CSS scan (`finish_sweeping` first); the sweeper never overlaps the roots, the CSS or the markers |
//! | What the sweeper reads of a live cell | the kind byte (an atomic) and the size fields of its kind, which the mutator never writes after the cell's first GC (a THIN string changes its kind, but CONS, SLICED and THIN are all 32 B) |
//! | What the sweeper writes | the page's bitmaps, line marks and dead cells (zap); never a live cell, a page header, `Memory`, the heap's stats or kind table |
//! | Hand-offs | the job through the sweeper's mutex; each result through the job's mutex; the end wait reads `open` with SeqCst after every index is taken ([`Claims`] doc) |
//! | A panic | caught on the sweeper thread and raised again on the heap's thread at the next filing (release builds abort, `panic = "abort"`) |
//!
//! The control: `EP9_GC_HELPERS=0` (no helpers) is also the lazy sweep, so the instruction
//! floors read the sequential mark and the sequential sweep (V12-D47).

// `cfg(loom)` is set only by the loom test command (RUSTFLAGS="--cfg loom").
#![allow(unexpected_cfgs)]

#[cfg(loom)]
use loom::sync::Mutex;
#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicUsize};
#[cfg(not(loom))]
use std::sync::Mutex;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicUsize};

use core::sync::atomic::Ordering::{Relaxed, SeqCst};

use crate::cell::CellPtr;
use crate::trace::Tracer;

/// Most helper threads by default (the gate host needs at least 2 isolated cores per engine,
/// V12-D8).
pub(crate) const MAX_HELPERS: usize = 4;

/// Most helper threads an explicit `EP9_GC_HELPERS` value gives.
pub(crate) const MAX_EXPLICIT_HELPERS: usize = 16;

/// Cells the heap's thread traces alone in each drain before it asks for helpers (about 1 ms
/// on the M5 Max). A smaller mark ends sequentially: waking and ending the helpers costs more
/// than it saves (measured: earley-boyer's minor GCs of 30-60 K cells were slower in parallel
/// with a 32 K threshold on a loaded host). A new realm marks about 2.3 K cells; splay's GCs
/// mark 175 K to 1.5 M.
pub(crate) const START_AFTER: usize = 64 * 1024;

/// Cells a marker traces between two looks at the shared counters.
pub(crate) const SHARE_EVERY: u32 = 64;

/// The helper count of this process: `EP9_GC_HELPERS`, read once (module doc).
pub(crate) fn helpers() -> usize {
    #[cfg(test)]
    if let Some(n) = test_controls::HELPERS.get() {
        return n;
    }
    if cfg!(any(miri, loom)) {
        return 0;
    }
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        helpers_from(std::env::var("EP9_GC_HELPERS").ok().as_deref(), cores)
    })
}

/// The helper count for an `EP9_GC_HELPERS` value and a core count (module doc).
pub(crate) fn helpers_from(var: Option<&str>, cores: usize) -> usize {
    match var.and_then(|v| v.trim().parse::<usize>().ok()) {
        Some(n) => n.min(MAX_EXPLICIT_HELPERS),
        None => cores.saturating_sub(1).min(MAX_HELPERS),
    }
}

fn start_after() -> usize {
    #[cfg(test)]
    if let Some(n) = test_controls::START_AFTER.get() {
        return n;
    }
    if STRESS_PARALLEL.get() {
        return STRESS_START_AFTER;
    }
    START_AFTER
}

/// Tests only: the helper count and the start threshold for marks on this thread.
#[cfg(test)]
pub(crate) mod test_controls {
    use core::cell::Cell;

    std::thread_local! {
        pub(crate) static HELPERS: Cell<Option<usize>> = const { Cell::new(None) };
        pub(crate) static START_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
        /// A mark on this thread waits for a busy pool instead of marking alone (tests run
        /// on many threads at once). Set by [`with`].
        pub(crate) static WAIT_FOR_POOL: Cell<bool> = const { Cell::new(false) };
        /// Parallel drains run on this thread so far, and the cells their helpers traced.
        pub(crate) static PARALLEL_DRAINS: Cell<u64> = const { Cell::new(0) };
        pub(crate) static HELPER_CELLS: Cell<usize> = const { Cell::new(0) };
        /// The GCs of this thread's heaps sweep concurrently from this many queued pages
        /// ([`with_sweeper`]); `None`: the lazy sweep (the default of the unit tests, which
        /// read its queues).
        pub(crate) static SWEEPER: Cell<Option<usize>> = const { Cell::new(None) };
        /// A join of a concurrent sweep first waits (at most 10 s) until the sweeper thread
        /// has taken every page, so the sweeper sweeps them all.
        pub(crate) static SWEEPER_FIRST: Cell<bool> = const { Cell::new(false) };
    }

    /// Runs `f` with the concurrent sweep on for this thread's GCs from `start` queued pages;
    /// `first`: a join waits until the sweeper thread has taken every page.
    pub(crate) fn with_sweeper<R>(start: usize, first: bool, f: impl FnOnce() -> R) -> R {
        let old = (SWEEPER.replace(Some(start)), SWEEPER_FIRST.replace(first));
        struct Reset(Option<usize>, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                SWEEPER.set(self.0);
                SWEEPER_FIRST.set(self.1);
            }
        }
        let _reset = Reset(old.0, old.1);
        f()
    }

    /// Runs `f` with `helpers` helpers and the default start threshold (the GC-stress
    /// control picks it); a mark waits for a busy pool.
    pub(crate) fn with_helpers<R>(helpers: usize, f: impl FnOnce() -> R) -> R {
        let old = (HELPERS.replace(Some(helpers)), WAIT_FOR_POOL.replace(true));
        struct Reset(Option<usize>, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                HELPERS.set(self.0);
                WAIT_FOR_POOL.set(self.1);
            }
        }
        let _reset = Reset(old.0, old.1);
        f()
    }

    /// Runs `f` with `helpers` helpers and the start threshold `start`; a mark waits for a
    /// busy pool.
    pub(crate) fn with<R>(helpers: usize, start: usize, f: impl FnOnce() -> R) -> R {
        with_wait(helpers, start, true, f)
    }

    /// [`with`], and `wait`: whether a mark waits for a busy pool or marks alone.
    pub(crate) fn with_wait<R>(
        helpers: usize,
        start: usize,
        wait: bool,
        f: impl FnOnce() -> R,
    ) -> R {
        let old = (
            HELPERS.replace(Some(helpers)),
            START_AFTER.replace(Some(start)),
            WAIT_FOR_POOL.replace(wait),
        );
        struct Reset(Option<usize>, Option<usize>, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                HELPERS.set(self.0);
                START_AFTER.set(self.1);
                WAIT_FOR_POOL.set(self.2);
            }
        }
        let _reset = Reset(old.0, old.1, old.2);
        f()
    }
}

/// Drains the mark stack of `t`: in parallel when it is worth it (module doc), else as
/// [`Tracer::drain`].
pub(crate) fn drain(t: &mut Tracer<'_>) {
    let n = helpers();
    if n == 0 || !t.may_share() {
        t.drain();
        return;
    }
    if !t.drain_up_to(start_after()) {
        return;
    }
    #[cfg(not(loom))]
    if let Some(claim) = pool::Claim::take(n) {
        pool::run_job(t, &claim);
        return;
    }
    t.drain();
}

/// A grey cell in a shared segment.
#[repr(transparent)]
pub(crate) struct Grey(pub(crate) CellPtr);

// SAFETY: a `Grey` is a marked cell start of the heap being marked. It moves between the marker
// threads of one mark only, while the mutator is stopped; every marker reads the cell through
// atomic accesses (H11), and no cell is freed while the mark runs.
unsafe impl Send for Grey {}

/// One marker's side of the parallel mark: its local stack and its trace step.
pub(crate) trait Marker {
    type Item: Send;
    /// Takes the newest item of the local stack.
    fn pop(&mut self) -> Option<Self::Item>;
    /// Traces one item; it may push new items onto the local stack.
    fn trace(&mut self, item: Self::Item);
    fn len(&self) -> usize;
    /// Takes the `n` oldest items of the local stack (`n <= len`).
    fn take_oldest(&mut self, n: usize) -> Vec<Self::Item>;
    /// Adds a segment to the local stack.
    fn refill(&mut self, seg: Vec<Self::Item>);
}

/// The shared work structure of one parallel mark: a queue of segments per marker, and the
/// counts that find work and end the mark (module doc).
pub(crate) struct Work<T> {
    queues: Box<[Mutex<Vec<Vec<T>>>]>,
    /// Segments in all queues. Changed under the queue's lock, read without it as a hint.
    queued: AtomicUsize,
    /// Markers that hold work or may take work. 0 = the mark is done.
    active: AtomicUsize,
    /// Markers looking for work.
    idle: AtomicUsize,
    /// The heap's thread is unwinding: every marker stops.
    abort: AtomicBool,
    share_every: u32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuardOf<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(loom)]
type MutexGuardOf<'a, T> = loom::sync::MutexGuard<'a, T>;
#[cfg(not(loom))]
type MutexGuardOf<'a, T> = std::sync::MutexGuard<'a, T>;

impl<T: Send> Work<T> {
    /// The work of `markers` markers. Marker 0 starts with work and counts as active; the
    /// others start idle ([`run`] with `has_work` false).
    pub(crate) fn new(markers: usize, share_every: u32) -> Work<T> {
        Work {
            queues: (0..markers).map(|_| Mutex::new(Vec::new())).collect(),
            queued: AtomicUsize::new(0),
            active: AtomicUsize::new(1),
            idle: AtomicUsize::new(0),
            abort: AtomicBool::new(false),
            share_every: share_every.max(1),
        }
    }

    /// Some marker waits and no segment is queued for it.
    fn hungry(&self) -> bool {
        self.idle.load(Relaxed) > self.queued.load(Relaxed)
    }

    fn publish(&self, me: usize, seg: Vec<T>) {
        let mut q = lock(&self.queues[me]);
        q.push(seg);
        self.queued.fetch_add(1, SeqCst);
    }

    /// A segment from the own queue, else from the next ones. Only an active marker calls it.
    fn steal(&self, me: usize) -> Option<Vec<T>> {
        if self.queued.load(SeqCst) == 0 {
            return None;
        }
        let n = self.queues.len();
        for k in 0..n {
            let mut q = lock(&self.queues[(me + k) % n]);
            if let Some(seg) = q.pop() {
                self.queued.fetch_sub(1, SeqCst);
                return Some(seg);
            }
        }
        None
    }

    /// The next segment for marker `me` (its local stack is empty), or `None` when the mark
    /// is done or stopped. `counted`: the marker counts in `active` now (it had work); a
    /// marker that starts idle does not, so a helper that joins after the end finds 0.
    ///
    /// Why `active == 0` means done: a marker leaves `active` only with an empty stack after
    /// a failed steal, and joins it again before it steals; only an active marker publishes.
    /// So the last marker to leave saw every queue empty after the last publish, and no
    /// marker holds work.
    fn find(&self, me: usize, counted: bool) -> Option<Vec<T>> {
        if counted {
            if let Some(seg) = self.steal(me) {
                return Some(seg);
            }
            self.active.fetch_sub(1, SeqCst);
        }
        self.idle.fetch_add(1, SeqCst);
        let mut spins = 0;
        loop {
            if self.abort.load(Relaxed) {
                return None;
            }
            if self.queued.load(SeqCst) > 0 {
                self.active.fetch_add(1, SeqCst);
                if let Some(seg) = self.steal(me) {
                    self.idle.fetch_sub(1, SeqCst);
                    return Some(seg);
                }
                self.active.fetch_sub(1, SeqCst);
            }
            if self.active.load(SeqCst) == 0 {
                return None;
            }
            pause(&mut spins);
        }
    }

    /// Stops every marker (the heap's thread unwinds).
    pub(crate) fn stop(&self) {
        self.abort.store(true, SeqCst);
    }

    fn stopped(&self) -> bool {
        self.abort.load(Relaxed)
    }
}

/// One wait step of an idle marker: spin, then yield, then sleep. The sleep is a
/// `park_timeout` (an unpark only ends it early: the caller reads the counters again), not a
/// `thread::sleep`: the pool uses no libc function that the rest of `ep9` does not (pool doc).
fn pause(spins: &mut u32) {
    #[cfg(loom)]
    {
        let _ = spins;
        loom::thread::yield_now();
    }
    #[cfg(not(loom))]
    {
        *spins = spins.saturating_add(1);
        if *spins < 64 {
            core::hint::spin_loop();
        } else if *spins < 128 {
            std::thread::yield_now();
        } else {
            std::thread::park_timeout(std::time::Duration::from_micros(20));
        }
    }
}

/// Marker `me`'s loop: trace, share when another marker waits, steal when empty, until the
/// mark is done (module doc). `has_work`: the marker starts with work and counts as active
/// (marker 0); else it starts idle.
pub(crate) fn run<M: Marker>(work: &Work<M::Item>, me: usize, m: &mut M, mut has_work: bool) {
    loop {
        if has_work {
            // The first look comes after the first cell: idle markers get work at once.
            let mut since = work.share_every - 1;
            while let Some(item) = m.pop() {
                m.trace(item);
                since += 1;
                if since >= work.share_every {
                    since = 0;
                    if work.stopped() {
                        return;
                    }
                    let n = m.len();
                    if n >= 2 && work.hungry() {
                        work.publish(me, m.take_oldest(n / 2));
                    }
                }
            }
        }
        match work.find(me, has_work) {
            Some(seg) => {
                m.refill(seg);
                has_work = true;
            }
            None => return,
        }
    }
}

/// The loom tests of [`Work`] and [`run`], and of [`Claims`] (declared here, where
/// `cfg(loom)` is allowed).
#[cfg(all(test, loom))]
#[path = "tests/loom.rs"]
mod loom_tests;

/// The marker pool of the process (module doc).
///
/// Its waits are `thread::park` loops on `STATE`, not `Condvar`s: on macOS std's `Condvar`
/// and `thread::sleep` import `pthread_cond_*` and `nanosleep`, and dyld binds each import at
/// every process start, a run with no GC too (the 4 imports: about 250K instructions on the
/// M5 Max, also for `ep9 --version`). `park` uses the dispatch semaphores std already imports.
/// A park can return early (an unpark token, or a spurious wake), so each wait reads the
/// state again.
#[cfg(not(loom))]
mod pool {
    use core::sync::atomic::AtomicBool;
    use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::sync::{Mutex, MutexGuard};
    use std::thread::Thread;

    use super::{Grey, SHARE_EVERY, Work, run};
    use crate::cell::KindTable;
    use crate::memory::Memory;
    use crate::trace::{Harvest, Tracer};

    /// The job of one parallel drain. It lives on the heap's thread's stack; the helpers reach
    /// it through [`State::job`] until they are done.
    struct Job<'a> {
        work: Work<Grey>,
        mem: &'a Memory,
        kinds: &'a KindTable,
        check: bool,
        harvests: Mutex<Vec<Harvest>>,
        panic: Mutex<Option<Box<dyn Any + Send>>>,
    }

    // SAFETY: the helpers only read `mem` and `kinds` (no `&mut` to them exists while the job
    // is open: the heap's thread is inside the mark), and every write a marker makes to the
    // heap is an atomic access (mark bits, gc_state). The other fields are `Sync`.
    unsafe impl Sync for Job<'_> {}

    /// A job pointer in the pool state.
    #[derive(Clone, Copy)]
    struct JobPtr(*const Job<'static>);

    // SAFETY: the heap's thread keeps the job alive until every helper that took the pointer
    // is done (`Close`), and `Job` is `Sync`.
    unsafe impl Send for JobPtr {}

    struct State {
        /// One more per job, so a parked helper sees a new job once.
        epoch: u64,
        /// The open job; `None` once the heap's thread closes it.
        job: Option<JobPtr>,
        /// Helpers the open job wants, helpers that joined it, helpers done with it.
        want: usize,
        joined: usize,
        finished: usize,
        /// Helper threads started so far, and their handles (the job start unparks each).
        spawned: usize,
        threads: Vec<Thread>,
        /// The heap's thread of the open job: a helper that is done unparks it.
        waiter: Option<Thread>,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        epoch: 0,
        job: None,
        want: 0,
        joined: 0,
        finished: 0,
        spawned: 0,
        threads: Vec::new(),
        waiter: None,
    });
    /// A heap's mark holds the pool.
    static BUSY: AtomicBool = AtomicBool::new(false);

    fn state() -> MutexGuard<'static, State> {
        STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The pool, held by one mark, with `helpers` running helper threads.
    pub(super) struct Claim {
        helpers: usize,
    }

    impl Claim {
        /// Takes the pool if no other heap holds it, and starts helper threads up to `n`.
        /// `None` if the pool is busy or no helper thread could start.
        pub(super) fn take(n: usize) -> Option<Claim> {
            while BUSY
                .compare_exchange(false, true, Acquire, Relaxed)
                .is_err()
            {
                if !wait_for_pool() {
                    return None;
                }
                std::thread::yield_now();
            }
            let mut s = state();
            while s.spawned < n {
                let started = std::thread::Builder::new()
                    .name(format!("ep9-gc-marker-{}", s.spawned + 1))
                    .spawn(helper_main);
                let Ok(handle) = started else {
                    break;
                };
                s.threads.push(handle.thread().clone());
                s.spawned += 1;
            }
            let helpers = n.min(s.spawned);
            drop(s);
            if helpers == 0 {
                BUSY.store(false, Release);
                return None;
            }
            Some(Claim { helpers })
        }
    }

    /// A mark waits for a busy pool only in the tests that ask for it.
    fn wait_for_pool() -> bool {
        #[cfg(test)]
        return super::test_controls::WAIT_FOR_POOL.get();
        #[cfg(not(test))]
        false
    }

    impl Drop for Claim {
        fn drop(&mut self) {
            BUSY.store(false, Release);
        }
    }

    /// A helper thread: waits for a job, joins it if it still wants helpers, marks, reports.
    fn helper_main() {
        let mut seen = 0;
        loop {
            let (job, me) = {
                let mut s = state();
                loop {
                    if s.epoch != seen {
                        seen = s.epoch;
                        if let Some(job) = s.job
                            && s.joined < s.want
                        {
                            s.joined += 1;
                            break (job, s.joined);
                        }
                    }
                    // Wait for the next job: `run_job` unparks every helper after it opens one.
                    drop(s);
                    std::thread::park();
                    s = state();
                }
            };
            // SAFETY: the heap's thread keeps the job alive until `finished == joined`, and
            // this helper counts itself finished only after its last use of `job`.
            let job = unsafe { &*job.0 };
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| help(job, me))) {
                job.work.stop();
                job.panic
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_or_insert(p);
            }
            let mut s = state();
            s.finished += 1;
            if let Some(w) = &s.waiter {
                w.unpark();
            }
        }
    }

    /// Marker `me` of `job` on a helper thread.
    fn help(job: &Job<'_>, me: usize) {
        let mut t = Tracer::new_helper(job.mem, job.kinds, job.check);
        run(&job.work, me, &mut t, false);
        let h = t.harvest();
        job.harvests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(h);
    }

    /// Closes the job and waits for every helper that joined it, also on an unwind of the
    /// heap's thread (which first stops the helpers).
    struct Close<'j, 'a> {
        job: &'j Job<'a>,
    }

    impl Drop for Close<'_, '_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.job.work.stop();
            }
            let mut s = state();
            s.job = None;
            while s.finished < s.joined {
                // A helper that is done unparks `waiter` (this thread) under the lock.
                drop(s);
                std::thread::park();
                s = state();
            }
            s.waiter = None;
        }
    }

    /// The parallel drain of `t` with the claimed pool (module doc).
    pub(super) fn run_job(t: &mut Tracer<'_>, claim: &Claim) {
        let job = Job {
            work: Work::new(claim.helpers + 1, SHARE_EVERY),
            mem: t.mem(),
            kinds: t.kinds(),
            check: t.checking(),
            harvests: Mutex::new(Vec::new()),
            panic: Mutex::new(None),
        };
        {
            let mut s = state();
            s.epoch += 1;
            s.job = Some(JobPtr(core::ptr::from_ref(&job).cast()));
            s.want = claim.helpers;
            s.joined = 0;
            s.finished = 0;
            s.waiter = Some(std::thread::current());
            for h in &s.threads {
                h.unpark();
            }
        }
        #[cfg(test)]
        super::test_controls::PARALLEL_DRAINS.set(super::test_controls::PARALLEL_DRAINS.get() + 1);
        {
            let _close = Close { job: &job };
            t.set_shared(true);
            run(&job.work, 0, t, true);
            t.set_shared(false);
        }
        let Job {
            harvests, panic, ..
        } = job;
        if let Some(p) = panic.into_inner().unwrap_or_else(|e| e.into_inner()) {
            resume_unwind(p);
        }
        for h in harvests.into_inner().unwrap_or_else(|e| e.into_inner()) {
            #[cfg(test)]
            super::test_controls::HELPER_CELLS
                .set(super::test_controls::HELPER_CELLS.get() + h.marked_cells());
            t.absorb(h);
        }
    }
}

// ---- S2a part 2: the concurrent sweep ---------------------------------------------------------

/// Small pages a GC must queue before its sweep runs on the sweeper thread (4 MiB of pages).
/// A smaller heap sweeps lazily on the heap's thread: waking the sweeper costs more than the
/// sweep of a few pages. (est.: a page sweep is 1-20 us on the M5 Max, a wake 5-20 us.)
pub(crate) const SWEEP_START_PAGES: usize = 16;

/// Under GC-stress the first GC, then a major GC once this many GCs (less one) passed since
/// the last such GC, use the parallel paths: the parallel mark from [`STRESS_START_AFTER`]
/// cells and the concurrent sweep from one page (route GCPAR-STRESS: GC-stress marks never
/// reach [`START_AFTER`]). With S1's default `stress_major_every` (64) that is every major
/// stress GC; in S0 every 63rd GC. The other stress GCs keep the default thresholds, so a
/// GC-stress run does not pay a wake per allocation.
pub(crate) const STRESS_PARALLEL_EVERY: u32 = 64;

/// The start threshold of a parallel mark in a GC that [`STRESS_PARALLEL_EVERY`] picks.
pub(crate) const STRESS_START_AFTER: usize = 256;

std::thread_local! {
    /// The GC of this thread's heap is a GC-stress GC that uses the parallel paths
    /// ([`StressParallel`]).
    static STRESS_PARALLEL: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// While it lives, the marks on this thread start in parallel after [`STRESS_START_AFTER`]
/// cells (a GC-stress GC that [`STRESS_PARALLEL_EVERY`] picks). Dropped at the mark's end, or
/// by the unwind of a panic.
pub(crate) struct StressParallel(bool);

impl StressParallel {
    pub(crate) fn new(on: bool) -> StressParallel {
        StressParallel(STRESS_PARALLEL.replace(on))
    }
}

impl Drop for StressParallel {
    fn drop(&mut self) {
        STRESS_PARALLEL.set(self.0);
    }
}

/// The concurrent sweep may run for a GC on this thread: there are helpers (so not with
/// `EP9_GC_HELPERS=0`, not under Miri or loom), and in this crate's unit tests only inside
/// `test_controls::with_sweeper`.
pub(crate) fn sweeper_on() -> bool {
    #[cfg(test)]
    if test_controls::SWEEPER.get().is_none() {
        return false;
    }
    helpers() > 0
}

/// Queued pages from which a GC sweeps concurrently ([`SWEEP_START_PAGES`]; a unit test may
/// set it, `test_controls::with_sweeper`).
pub(crate) fn sweep_start_pages() -> usize {
    #[cfg(test)]
    if let Some(n) = test_controls::SWEEPER.get() {
        return n;
    }
    SWEEP_START_PAGES
}

/// The page claims of one concurrent sweep (module doc, part 2). Each index in `0..len` goes
/// to exactly one claim: the sweeper thread's or the heap's thread's.
///
/// The end rule: the sweeper thread adds 1 to `open` before it claims, and takes it off after
/// the claimed page's result is in the job's done list (or after a claim that failed). The
/// heap's thread first sees every index taken (a claim that fails, or [`Claims::close`]),
/// then waits until `open` is 0. Every access is SeqCst, so a sweeper claim that got an index
/// comes before the heap's thread's failed claim in the one total order, and so does the add
/// to `open` before it: the heap's thread then reads `open` after that add, and waits for the
/// take-off (a release; the wait reads with acquire). After the wait no sweeper claim is open
/// and none can succeed, so the sweeper thread touches no page of the job again.
pub(crate) struct Claims {
    next: AtomicUsize,
    len: usize,
    open: AtomicUsize,
}

impl Claims {
    pub(crate) fn new(len: usize) -> Claims {
        Claims {
            next: AtomicUsize::new(0),
            len,
            open: AtomicUsize::new(0),
        }
    }

    /// The next index for the heap's thread, or `None` when every index is taken.
    pub(crate) fn claim(&self) -> Option<usize> {
        if self.next.load(SeqCst) >= self.len {
            return None;
        }
        let i = self.next.fetch_add(1, SeqCst);
        (i < self.len).then_some(i)
    }

    /// The next index for the sweeper thread, with `open` counted; `None` when every index is
    /// taken (then `open` is as before). After a `Some`, the sweeper calls [`Claims::done`].
    pub(crate) fn claim_open(&self) -> Option<usize> {
        self.open.fetch_add(1, SeqCst);
        let got = self.claim();
        if got.is_none() {
            self.open.fetch_sub(1, SeqCst);
        }
        got
    }

    /// The sweeper thread is done with the page of its last claim.
    pub(crate) fn done(&self) {
        self.open.fetch_sub(1, SeqCst);
    }

    /// Takes every index left, so no claim succeeds again (a heap that is dropped).
    pub(crate) fn close(&self) {
        self.next.fetch_max(self.len, SeqCst);
    }

    /// Every index is taken.
    pub(crate) fn taken(&self) -> bool {
        self.next.load(SeqCst) >= self.len
    }

    /// No sweeper claim is open. Read after [`Claims::taken`] is true, a `true` here means
    /// the sweeper thread has finished every page it took (struct doc).
    pub(crate) fn settled(&self) -> bool {
        self.open.load(SeqCst) == 0
    }

    /// Waits until [`Claims::settled`] (struct doc: call it only once every index is taken).
    /// The wait is at most the sweep of one page.
    pub(crate) fn wait_settled(&self) {
        debug_assert!(self.taken());
        let mut spins = 0;
        while !self.settled() {
            pause(&mut spins);
        }
    }
}

pub(crate) use sweeper::{Job, publish};

/// The sweeper thread and its jobs (module doc, part 2). It builds under loom too (the heap
/// names [`Job`]), but no job is made there: [`sweeper_on`] is false.
mod sweeper {
    use core::ptr::NonNull;
    use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
    use std::any::Any;
    use std::collections::VecDeque;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread::Thread;

    use super::Claims;
    use crate::cell::{KindId, KindTable};
    use crate::heap::{Sweeper, Swept};

    /// One concurrent sweep: the small pages that one GC queued, and what the sweep of a page
    /// reads. The heap's thread makes it at the end of the GC; the sweeper thread and the
    /// heap's thread then sweep its pages ([`Claims`]) while the mutator runs.
    pub(crate) struct Job {
        /// The cage base, with the provenance of the reservation.
        base: NonNull<u8>,
        /// A copy of the heap's kind table at the GC: the sweeper never reads the heap's
        /// own (the mutator may register a kind meanwhile; no cell of a new kind is in a
        /// page of this job).
        kinds: Box<KindTable>,
        sticky: bool,
        zap: bool,
        /// The checked sweep (the verify after a GC): errors and kept cells are collected.
        check: bool,
        /// The pages: small-page chunk indices, ordinary pages first (low first), then shape
        /// pages.
        pub(crate) pages: Box<[u32]>,
        pub(crate) claims: Claims,
        /// Results of the pages the sweeper thread swept, for the heap's thread to file.
        done: Mutex<Vec<(u32, Swept)>>,
        /// The sweeper thread's counts: freed cells and bytes, kept cells (checked sweep).
        freed_cells: AtomicU64,
        freed_bytes: AtomicU64,
        kept: AtomicUsize,
        errors: Mutex<Vec<String>>,
        /// A panic of the sweeper thread, raised again on the heap's thread.
        panicked: AtomicBool,
        panic: Mutex<Option<Box<dyn Any + Send>>>,
    }

    // SAFETY: `base` is the address of the cage, which lives until the heap is dropped; the
    // heap's thread closes the job and waits until it settles before that (heap.rs
    // `ConcurrentSweep`), and the sweeper thread touches a page only between a claim and its
    // `done` (`Claims`). Each page goes to one thread. The cells of a page are read only for
    // their kind byte (an atomic) and the size fields of their kind, which the mutator never
    // writes after the cell's first GC (03-gc S2a part-2 note); dead cells are zapped, and no
    // thread reaches a dead cell. The other fields are atomics, mutexes, or not written after
    // the job is made.
    unsafe impl Send for Job {}
    // SAFETY: as for `Send`.
    unsafe impl Sync for Job {}

    impl Job {
        /// A job for `pages` with a copy of `kinds`.
        pub(crate) fn new(
            base: NonNull<u8>,
            kinds: &KindTable,
            sticky: bool,
            zap: bool,
            check: bool,
            pages: Box<[u32]>,
        ) -> Job {
            let mut copy = Box::new(KindTable::new());
            for k in 1..=u8::MAX {
                if let (Some(kind), Some(id)) = (kinds.get(k), KindId::new(k)) {
                    copy.register_in(id, *kind, kinds.space(k))
                        .expect("the heap's table registered it");
                }
            }
            Job {
                base,
                kinds: copy,
                sticky,
                zap,
                check,
                claims: Claims::new(pages.len()),
                pages,
                done: Mutex::new(Vec::new()),
                freed_cells: AtomicU64::new(0),
                freed_bytes: AtomicU64::new(0),
                kept: AtomicUsize::new(0),
                errors: Mutex::new(Vec::new()),
                panicked: AtomicBool::new(false),
                panic: Mutex::new(None),
            }
        }

        pub(crate) fn check(&self) -> bool {
            self.check
        }

        /// Takes the results of the pages the sweeper thread swept so far.
        pub(crate) fn take_done(&self, into: &mut Vec<(u32, Swept)>) {
            into.append(&mut lock(&self.done));
        }

        /// Takes the sweeper thread's counts (freed cells, freed bytes, kept cells) and errors.
        pub(crate) fn take_counts(&self, errors: &mut Vec<String>) -> (u64, u64, usize) {
            errors.append(&mut lock(&self.errors));
            (
                self.freed_cells.swap(0, Relaxed),
                self.freed_bytes.swap(0, Relaxed),
                self.kept.swap(0, Relaxed),
            )
        }

        /// The freed cells and bytes the sweeper thread counted and the heap has not taken.
        pub(crate) fn freed_so_far(&self) -> (u64, u64) {
            (
                self.freed_cells.load(Relaxed),
                self.freed_bytes.load(Relaxed),
            )
        }

        /// The sweeper thread panicked in this job (its panic is not taken yet).
        #[cfg(test)]
        pub(crate) fn panicked(&self) -> bool {
            self.panicked.load(Relaxed)
        }

        /// The panic of the sweeper thread, if it panicked.
        pub(crate) fn take_panic(&self) -> Option<Box<dyn Any + Send>> {
            if !self.panicked.load(Relaxed) {
                return None;
            }
            lock(&self.panic).take()
        }

        /// The sweeper thread's part: claims and sweeps pages until none is left. Each page's
        /// counts, errors and result are in the job before its claim ends (`Claims::done`):
        /// the heap's thread reads them after the end wait.
        fn run(&self) {
            let mut sw = Sweeper::new(self.base, &self.kinds, self.sticky, self.zap);
            let mut errors = Vec::new();
            let mut open = false;
            let r = catch_unwind(AssertUnwindSafe(|| {
                while let Some(i) = self.claims.claim_open() {
                    open = true;
                    let idx = self.pages[i];
                    let mut kept = 0;
                    let swept = if self.check {
                        sw.sweep_page_with::<true>(idx as usize, &mut errors, &mut kept)
                    } else {
                        sw.sweep_page_with::<false>(idx as usize, &mut errors, &mut kept)
                    };
                    self.freed_cells.fetch_add(sw.freed_cells, Relaxed);
                    self.freed_bytes.fetch_add(sw.freed_bytes, Relaxed);
                    (sw.freed_cells, sw.freed_bytes) = (0, 0);
                    self.kept.fetch_add(kept, Relaxed);
                    if !errors.is_empty() {
                        lock(&self.errors).append(&mut errors);
                    }
                    lock(&self.done).push((idx, swept));
                    open = false;
                    self.claims.done();
                }
            }));
            if let Err(p) = r {
                // The page of the open claim: its marks were reset by the unwind
                // (`UnmarkPageOnUnwind`); it goes to no list until the next GC, as after a
                // panic of the lazy sweep. The heap's thread sweeps the pages left.
                lock(&self.panic).get_or_insert(p);
                self.panicked.store(true, Relaxed);
                if open {
                    self.claims.done();
                }
            }
        }
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct State {
        /// Jobs not started yet, oldest first: at most one per heap that has a job (each
        /// publish drops the jobs whose pages are all taken).
        jobs: VecDeque<Arc<Job>>,
        thread: Option<Thread>,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        jobs: VecDeque::new(),
        thread: None,
    });

    /// Gives `job` to the sweeper thread (started at the first job). False if the thread
    /// could not start: the heap's thread then sweeps lazily.
    pub(crate) fn publish(job: &Arc<Job>) -> bool {
        let mut s = lock(&STATE);
        s.jobs.retain(|j| !j.claims.taken());
        if s.thread.is_none() {
            let started = std::thread::Builder::new()
                .name("ep9-gc-sweeper".into())
                .spawn(sweeper_main);
            let Ok(handle) = started else {
                return false;
            };
            s.thread = Some(handle.thread().clone());
        }
        s.jobs.push_back(Arc::clone(job));
        if let Some(t) = &s.thread {
            t.unpark();
        }
        true
    }

    /// The sweeper thread: takes the oldest job and sweeps its pages, then waits (parked) for
    /// the next one. It lives as long as the process.
    fn sweeper_main() {
        loop {
            let job = {
                let mut s = lock(&STATE);
                loop {
                    if let Some(j) = s.jobs.pop_front() {
                        break j;
                    }
                    drop(s);
                    std::thread::park();
                    s = lock(&STATE);
                }
            };
            job.run();
        }
    }
}
