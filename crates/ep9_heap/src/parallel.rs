//! S2a part 1: the parallel mark (03-gc §4 S2a). The world stays stopped: the heap's thread and
//! up to [`MAX_HELPERS`] helper threads drain the mark stack together, then the heap's thread
//! goes on alone (the ephemeron rounds, weak slots, sweep).
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
//! | Roots, CSS, S1 logs, ephemeron rounds, weak slots | the heap's thread only, before or after the parallel drain |
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

/// One wait step of an idle marker: spin, then yield, then sleep.
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
            std::thread::sleep(std::time::Duration::from_micros(20));
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

/// The loom test of [`Work`] and [`run`] (declared here, where `cfg(loom)` is allowed).
#[cfg(all(test, loom))]
#[path = "tests/loom.rs"]
mod loom_tests;

/// The marker pool of the process (module doc).
#[cfg(not(loom))]
mod pool {
    use core::sync::atomic::AtomicBool;
    use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::sync::{Condvar, Mutex, MutexGuard};

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
        /// Helper threads started so far.
        spawned: usize,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        epoch: 0,
        job: None,
        want: 0,
        joined: 0,
        finished: 0,
        spawned: 0,
    });
    /// Helpers wait here for a job; the heap's thread waits on `END` for the helpers.
    static START: Condvar = Condvar::new();
    static END: Condvar = Condvar::new();
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
                if started.is_err() {
                    break;
                }
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
                    s = START.wait(s).unwrap_or_else(|e| e.into_inner());
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
            END.notify_all();
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
                s = END.wait(s).unwrap_or_else(|e| e.into_inner());
            }
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
        }
        START.notify_all();
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
