//! The waiter list of `Atomics.wait`, `Atomics.waitAsync` and `Atomics.notify` (ECMA-262
//! §25.4.3 WaiterList Records; TypedArray TA-4 R2 ATOMICS; design
//! `docs/plan/notes/typedarray-design.md` §5.6). Rust values only: it holds no JS value and
//! runs no JS, so any thread may call it.
//!
//! ```text
//! LIST: Mutex<map (block serial, byte index) -> FIFO of waiters>      one per process
//!   Sync  { slot: its own Condvar (waits on the LIST lock), notified }  Atomics.wait
//!   Async { agent id, waiter id, the agent's inbox }                    Atomics.waitAsync
//!
//! wait (sync)   lock; compare (a SeqCst load, under the lock: the critical section);
//!               push; wait in slices of <= SLICE until notified, the deadline or `stop()`
//! notify(n)     lock; pop the first n of the key in order: Sync -> notified, its Condvar;
//!               Async -> push the waiter id into its agent's inbox (the inbox lock)
//! Agent         one per isolate: its async waiters (id, key, deadline) and its inbox; the
//!               isolate drains it between jobs ([`Agent::take_ready`]); [`Agent::block`]
//!               sleeps until an inbox item, the next deadline or `stop()`; the drop
//!               removes its async waiters from the list
//! ```
//!
//! | Rule | Statement |
//! |---|---|
//! | Keys | A block's serial number (one per SharedArrayBuffer block of the process) and a byte index: never an address (H12) |
//! | Lock order | LIST, then an inbox. Never the reverse: [`Agent::take_ready`] takes LIST (for the timeouts) and drops it before it takes its inbox; [`Agent::block`] takes only its inbox |
//! | No thread-local destructor | Nothing here runs from a thread-local destructor (the Windows exit lesson, design §5.6). [`Agent`]'s drop takes LIST only when the agent still has async waiters |
//! | Termination | A sync wait and [`Agent::block`] wake at least every [`SLICE`] to call `stop()` (the caller reads its isolate's terminate word), so a watchdog stops a blocked `Atomics.wait` and a host block for async waiters |
//! | Poison | A panic while a lock is held does not break the list: every lock takes the inner value of a poisoned mutex |

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The longest sleep of a sync wait between two `stop()` checks.
pub const SLICE: Duration = Duration::from_millis(10);

/// A waiter list key: the serial number of a shared block and a byte index in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub block: u64,
    pub byte_index: u64,
}

/// The state of a sync waiter. `notified` is written under the LIST lock.
#[derive(Debug, Default)]
struct SyncSlot {
    cv: Condvar,
    notified: AtomicBool,
}

/// The inbox of an agent: the ids of its async waiters that a notify woke.
#[derive(Debug, Default)]
struct Inbox {
    ids: Mutex<VecDeque<u64>>,
    cv: Condvar,
    /// The number of ids in `ids` (read with no lock).
    len: AtomicUsize,
}

#[derive(Debug)]
enum Waiter {
    Sync(Arc<SyncSlot>),
    Async {
        agent: u64,
        id: u64,
        inbox: Arc<Inbox>,
    },
}

type Map = BTreeMap<Key, VecDeque<Waiter>>;

/// The one waiter list of the process.
static LIST: Mutex<Map> = Mutex::new(BTreeMap::new());

/// The next agent id (never 0).
static NEXT_AGENT: AtomicU64 = AtomicU64::new(1);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
thread_local! {
    /// Tests: the number of waiters that [`remove_where`] inspected on this thread.
    static SCANNED: Cell<usize> = const { Cell::new(0) };
}

/// Removes the waiters of `key` for which `f` is true; drops the key when its queue is empty.
/// One pass over the queue of `key`.
fn remove_where(map: &mut Map, key: Key, mut f: impl FnMut(&Waiter) -> bool) -> usize {
    let Some(q) = map.get_mut(&key) else {
        return 0;
    };
    let before = q.len();
    q.retain(|w| {
        #[cfg(test)]
        SCANNED.with(|n| n.set(n.get() + 1));
        !f(w)
    });
    let removed = before - q.len();
    if q.is_empty() {
        map.remove(&key);
    }
    removed
}

/// The end of a sync wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncWait {
    /// The value was not the expected one: no wait ("not-equal").
    NotEqual,
    /// A notify woke it ("ok").
    Ok,
    /// The timeout passed ("timed-out").
    TimedOut,
    /// `stop()` answered true (the isolate terminates); the waiter left the list.
    Stopped,
}

/// DoWait with mode sync (`sec-dowait`) from the critical section on: under the list lock,
/// `equal()` (the SeqCst load and compare) false gives [`SyncWait::NotEqual`]; else the
/// waiter goes at the end of the queue of `key` and the thread sleeps until a notify, the
/// `timeout` (`None`: no timeout) or `stop()`. `equal` and `stop` run with the list lock held:
/// they must not take it (they read memory only).
pub fn wait(
    key: Key,
    equal: impl FnOnce() -> bool,
    timeout: Option<Duration>,
    stop: impl Fn() -> bool,
) -> SyncWait {
    let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
    let mut g = lock(&LIST);
    if !equal() {
        return SyncWait::NotEqual;
    }
    let slot = Arc::new(SyncSlot::default());
    g.entry(key)
        .or_default()
        .push_back(Waiter::Sync(Arc::clone(&slot)));
    loop {
        if slot.notified.load(Ordering::Relaxed) {
            return SyncWait::Ok;
        }
        let end = if stop() {
            SyncWait::Stopped
        } else if deadline.is_some_and(|d| Instant::now() >= d) {
            SyncWait::TimedOut
        } else {
            // A timeout too long for `Instant` (`deadline` None) waits as no timeout does.
            let wait = deadline.map_or(SLICE, |d| {
                d.saturating_duration_since(Instant::now()).min(SLICE)
            });
            g = slot
                .cv
                .wait_timeout(g, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        };
        remove_where(
            &mut g,
            key,
            |w| matches!(w, Waiter::Sync(s) if Arc::ptr_eq(s, &slot)),
        );
        return end;
    }
}

/// `Atomics.notify` (`sec-atomics.notify`) from the critical section on: wakes the first
/// `count` waiters of `key` (`u64::MAX`: all) in FIFO order and returns how many it woke. A
/// sync waiter's Condvar is signalled; an async waiter's id goes into its agent's inbox.
pub fn notify(key: Key, count: u64) -> u64 {
    let mut g = lock(&LIST);
    let Some(q) = g.get_mut(&key) else {
        return 0;
    };
    let mut woken = 0u64;
    while woken < count {
        let Some(w) = q.pop_front() else { break };
        match w {
            Waiter::Sync(s) => {
                s.notified.store(true, Ordering::Relaxed);
                s.cv.notify_one();
            }
            Waiter::Async { id, inbox, .. } => {
                let mut ids = lock(&inbox.ids);
                ids.push_back(id);
                inbox.len.store(ids.len(), Ordering::Release);
                drop(ids);
                inbox.cv.notify_all();
            }
        }
        woken += 1;
    }
    if q.is_empty() {
        g.remove(&key);
    }
    woken
}

/// The result of [`Agent::wait_async`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncWait {
    /// The value was not the expected one ("not-equal", `async` false).
    NotEqual,
    /// The timeout is 0 ("timed-out", `async` false).
    TimedOut,
    /// The waiter is in the list with this id: a promise waits for it.
    Waiting(u64),
}

/// How an async waiter ended ([`Agent::take_ready`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A notify woke it ("ok").
    Ok,
    /// Its timeout passed ("timed-out").
    TimedOut,
}

/// One pending async waiter of an agent.
#[derive(Debug)]
struct Pending {
    id: u64,
    key: Key,
    deadline: Option<Instant>,
}

/// The waiter-list state of one agent (an isolate; ECMA-262 Agent Records): its async
/// waiters, its inbox and `[[CanBlock]]`. Not `Sync`: only its isolate's thread uses it; the
/// list and other threads reach only the inbox.
#[derive(Debug)]
pub struct Agent {
    id: u64,
    inbox: Arc<Inbox>,
    pending: RefCell<Vec<Pending>>,
    /// `pending.len()`, readable with no borrow (one plain load in the job loop).
    count: Cell<usize>,
    next_waiter: Cell<u64>,
    can_block: Cell<bool>,
}

impl Default for Agent {
    fn default() -> Self {
        Agent::new()
    }
}

impl Agent {
    /// A new agent with no waiter; `[[CanBlock]]` true.
    pub fn new() -> Agent {
        Agent {
            id: NEXT_AGENT.fetch_add(1, Ordering::Relaxed),
            inbox: Arc::default(),
            pending: RefCell::new(Vec::new()),
            count: Cell::new(0),
            next_waiter: Cell::new(1),
            can_block: Cell::new(true),
        }
    }

    /// `[[CanBlock]]` (AgentCanSuspend).
    pub fn can_block(&self) -> bool {
        self.can_block.get()
    }

    /// Sets `[[CanBlock]]`.
    pub fn set_can_block(&self, can: bool) {
        self.can_block.set(can);
    }

    /// The number of async waiters of this agent that have not ended (a notify may have woken
    /// some: [`Agent::take_ready`] collects them). One plain load.
    #[inline]
    pub fn pending(&self) -> usize {
        self.count.get()
    }

    /// DoWait with mode async from the critical section on: under the list lock, `equal()`
    /// false gives [`AsyncWait::NotEqual`]; a `timeout` of zero gives
    /// [`AsyncWait::TimedOut`]; else a new waiter of this agent goes at the end of the queue
    /// of `key`, with the deadline `timeout` (`None`: none).
    pub fn wait_async(
        &self,
        key: Key,
        equal: impl FnOnce() -> bool,
        timeout: Option<Duration>,
    ) -> AsyncWait {
        let mut g = lock(&LIST);
        if !equal() {
            return AsyncWait::NotEqual;
        }
        if timeout == Some(Duration::ZERO) {
            return AsyncWait::TimedOut;
        }
        let id = self.next_waiter.get();
        self.next_waiter.set(id + 1);
        g.entry(key).or_default().push_back(Waiter::Async {
            agent: self.id,
            id,
            inbox: Arc::clone(&self.inbox),
        });
        drop(g);
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        self.pending
            .borrow_mut()
            .push(Pending { id, key, deadline });
        self.count.set(self.count.get() + 1);
        AsyncWait::Waiting(id)
    }

    /// The async waiters of this agent that ended since the last call, in the order they
    /// ended (the timeouts of this call first, then the notifies): each one leaves the agent.
    /// Reads the clock only if a waiter has a deadline; takes the list lock only if one is
    /// due, and the inbox lock only if the inbox is not empty.
    pub fn take_ready(&self) -> Vec<(u64, Outcome)> {
        let mut out = Vec::new();
        if self.count.get() == 0 {
            return out;
        }
        let mut pending = self.pending.borrow_mut();
        if pending.iter().any(|p| p.deadline.is_some()) {
            let now = Instant::now();
            let due = |p: &Pending| p.deadline.is_some_and(|d| now >= d);
            if pending.iter().any(due) {
                // The due waiters by (key, id), sorted before the lock: one pass over the
                // queue of each key with the list lock held (not one per waiter).
                let mut keys: Vec<(Key, u64)> = pending
                    .iter()
                    .filter(|p| due(p))
                    .map(|p| (p.key, p.id))
                    .collect();
                keys.sort_unstable();
                let mut removed = Vec::with_capacity(keys.len());
                let mut g = lock(&LIST);
                for run in keys.chunk_by(|a, b| a.0 == b.0) {
                    let key = run[0].0;
                    remove_where(&mut g, key, |w| match w {
                        Waiter::Async { agent, id, .. }
                            if *agent == self.id && run.binary_search(&(key, *id)).is_ok() =>
                        {
                            removed.push(*id);
                            true
                        }
                        _ => false,
                    });
                }
                drop(g);
                removed.sort_unstable();
                // A due waiter that is not in the list any more was taken by a notify first:
                // its id is in the inbox (pushed under the list lock, before this lock), so
                // it stays pending. The others end as TimedOut, in the order they were made.
                pending.retain(|p| {
                    if removed.binary_search(&p.id).is_ok() {
                        out.push((p.id, Outcome::TimedOut));
                        false
                    } else {
                        true
                    }
                });
            }
        }
        if self.inbox.len.load(Ordering::Acquire) > 0 {
            let ids: Vec<u64> = {
                let mut q = lock(&self.inbox.ids);
                self.inbox.len.store(0, Ordering::Relaxed);
                q.drain(..).collect()
            };
            for id in ids {
                if let Some(i) = pending.iter().position(|p| p.id == id) {
                    pending.remove(i);
                    out.push((id, Outcome::Ok));
                }
            }
        }
        self.count.set(pending.len());
        out
    }

    /// Blocks this thread until a notify puts an id into the inbox, the next deadline of an
    /// async waiter passes, `max` passes or `stop()` answers true: at once if the agent has no
    /// async waiter or the inbox is not empty. It wakes at least every [`SLICE`] to call
    /// `stop()` (the caller reads its isolate's terminate word), as a sync [`wait`] does. Runs
    /// no JS and takes only the inbox lock; `stop` runs with it held and must not take it. The
    /// caller then drains ([`Agent::take_ready`], through a microtask checkpoint).
    pub fn block(&self, max: Duration, stop: impl Fn() -> bool) {
        if self.count.get() == 0 {
            return;
        }
        let now = Instant::now();
        let next = self
            .pending
            .borrow()
            .iter()
            .filter_map(|p| p.deadline)
            .min();
        let until = match now.checked_add(max) {
            Some(m) => next.map_or(m, |d| d.min(m)),
            None => next.unwrap_or(now),
        };
        let mut q = lock(&self.inbox.ids);
        while q.is_empty() && !stop() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            q = self
                .inbox
                .cv
                .wait_timeout(q, (until - now).min(SLICE))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Agent {
    /// Removes the async waiters of this agent from the list (V8
    /// `FutexEmulation::IsolateDeinit`, JSC `WaiterListManager::unregister`): a later notify
    /// does not count them. Takes the list lock only if the agent has async waiters, and then
    /// makes one pass over the queue of each of their keys (V8 walks each list once too).
    fn drop(&mut self) {
        let pending = self.pending.get_mut();
        if pending.is_empty() {
            return;
        }
        let mut keys: Vec<Key> = pending.drain(..).map(|p| p.key).collect();
        keys.sort_unstable();
        keys.dedup();
        let me = self.id;
        let mut g = lock(&LIST);
        for key in keys {
            // The agent goes away: every one of its async waiters leaves the list.
            remove_where(
                &mut g,
                key,
                |w| matches!(w, Waiter::Async { agent, .. } if *agent == me),
            );
        }
    }
}

/// Tests: the number of waiters of `key` in the list.
#[cfg(test)]
fn waiting(key: Key) -> usize {
    lock(&LIST).get(&key).map_or(0, VecDeque::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// A key no other test uses (the list is one per process; tests run in parallel).
    fn key() -> Key {
        static NEXT: AtomicU64 = AtomicU64::new(1 << 40);
        Key {
            block: NEXT.fetch_add(1, Ordering::Relaxed),
            byte_index: 4,
        }
    }

    /// A deadline for each wait of a test thread for another (not a time bound of the test).
    const DEADLINE: Duration = Duration::from_secs(120);

    /// Waits until `key` has `n` waiters (the deadline: a test bug).
    fn until_waiting(key: Key, n: usize) {
        let end = Instant::now() + DEADLINE;
        while waiting(key) != n {
            assert!(Instant::now() < end, "{n} waiters never came");
            std::thread::yield_now();
        }
    }

    #[test]
    fn not_equal_does_not_wait() {
        let k = key();
        assert_eq!(wait(k, || false, None, || false), SyncWait::NotEqual);
        assert_eq!(waiting(k), 0);
        let a = Agent::new();
        assert_eq!(a.wait_async(k, || false, None), AsyncWait::NotEqual);
        assert_eq!(
            a.wait_async(k, || true, Some(Duration::ZERO)),
            AsyncWait::TimedOut
        );
        assert_eq!((a.pending(), waiting(k)), (0, 0));
    }

    /// Three sync waiters in a known order; notify(1) wakes the first, notify(all) the other
    /// two; a notify of an empty key gives 0.
    #[test]
    fn notify_wakes_in_fifo_order_and_counts() {
        let k = key();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut hs = Vec::new();
        for i in 0..3 {
            let order = Arc::clone(&order);
            hs.push(std::thread::spawn(move || {
                let r = wait(k, || true, Some(DEADLINE), || false);
                lock(&order).push(i);
                r
            }));
            until_waiting(k, i + 1);
        }
        assert_eq!(notify(k, 1), 1);
        let end = Instant::now() + DEADLINE;
        while lock(&order).is_empty() {
            assert!(Instant::now() < end);
            std::thread::yield_now();
        }
        assert_eq!(*lock(&order), [0], "the first waiter woke first");
        assert_eq!(notify(k, u64::MAX), 2);
        for h in hs {
            assert_eq!(h.join().unwrap(), SyncWait::Ok);
        }
        assert_eq!(notify(k, u64::MAX), 0);
        assert_eq!(waiting(k), 0);
    }

    #[test]
    fn a_timeout_ends_the_wait_and_leaves_the_list() {
        let k = key();
        let r = wait(k, || true, Some(Duration::from_millis(1)), || false);
        assert_eq!(r, SyncWait::TimedOut);
        assert_eq!(waiting(k), 0);
        assert_eq!(notify(k, u64::MAX), 0);
    }

    /// The stop word (an isolate's terminate bit) ends a wait with no timeout.
    #[test]
    fn stop_wakes_a_wait_with_no_timeout() {
        let k = key();
        let word = Arc::new(AtomicU32::new(0));
        let w = Arc::clone(&word);
        let h =
            std::thread::spawn(move || wait(k, || true, None, || w.load(Ordering::Relaxed) != 0));
        until_waiting(k, 1);
        word.store(1, Ordering::Relaxed);
        assert_eq!(h.join().unwrap(), SyncWait::Stopped);
        assert_eq!(waiting(k), 0);
    }

    /// A notify and a timeout race: the waiter ends exactly once, and the notify count says
    /// whether the notify won (1) or the timeout did (0).
    #[test]
    fn notify_and_timeout_race_count_once() {
        let rounds = if cfg!(miri) { 3 } else { 200 };
        for _ in 0..rounds {
            let k = key();
            let h = std::thread::spawn(move || {
                wait(k, || true, Some(Duration::from_micros(50)), || false)
            });
            let mut n = 0;
            while !h.is_finished() {
                n += notify(k, 1);
                std::thread::yield_now();
            }
            let r = h.join().unwrap();
            n += notify(k, 1);
            match r {
                SyncWait::Ok => assert_eq!(n, 1),
                SyncWait::TimedOut => assert_eq!(n, 0),
                other => panic!("{other:?}"),
            }
            assert_eq!(waiting(k), 0);
        }
    }

    /// waitAsync: a notify from another thread puts the id into the inbox; `block` returns
    /// for it; `take_ready` gives it once as Ok. A second waiter times out through
    /// `take_ready`, and a notify after that does not count it.
    #[test]
    fn async_waiters_end_through_the_inbox() {
        let k = key();
        let a = Agent::new();
        let AsyncWait::Waiting(first) = a.wait_async(k, || true, None) else {
            panic!("waits")
        };
        let AsyncWait::Waiting(second) = a.wait_async(k, || true, Some(Duration::from_millis(1)))
        else {
            panic!("waits")
        };
        assert_eq!((a.pending(), waiting(k)), (2, 2));
        let h = std::thread::spawn(move || notify(k, 1));
        assert_eq!(h.join().unwrap(), 1, "the first (FIFO)");
        a.block(DEADLINE, || false);
        let end = Instant::now() + DEADLINE;
        let mut got = Vec::new();
        while got.len() < 2 {
            assert!(Instant::now() < end);
            got.extend(a.take_ready());
            a.block(Duration::from_millis(1), || false);
        }
        // The timeout may pass before the first drain: the order of the two is not fixed.
        got.sort_unstable_by_key(|&(id, _)| id);
        assert_eq!(got, [(first, Outcome::Ok), (second, Outcome::TimedOut)]);
        assert_eq!((a.pending(), waiting(k)), (0, 0));
        assert_eq!(notify(k, u64::MAX), 0);
        assert!(a.take_ready().is_empty());
    }

    /// The drop of an agent removes its async waiters: a later notify counts 0 (the brief's
    /// "an agent does waitAsync, ends, then notify returns 0"), and the waiters of another
    /// agent on the same key stay.
    #[test]
    fn an_agent_drop_removes_its_async_waiters() {
        let k = key();
        let other = Agent::new();
        let h = std::thread::spawn(move || {
            let a = Agent::new();
            assert!(matches!(
                a.wait_async(k, || true, None),
                AsyncWait::Waiting(_)
            ));
            assert!(matches!(
                a.wait_async(k, || true, None),
                AsyncWait::Waiting(_)
            ));
        });
        h.join().unwrap();
        assert!(matches!(
            other.wait_async(k, || true, None),
            AsyncWait::Waiting(_)
        ));
        assert_eq!(waiting(k), 1);
        assert_eq!(notify(k, u64::MAX), 1, "only the live agent's waiter");
        assert_eq!(other.take_ready().len(), 1);
    }

    /// The drop of an agent with many async waiters on one key inspects each waiter of the
    /// list once (one pass per key), not once per waiter of the agent (n(n+1)/2 checks with
    /// the list lock held, Codex TA-4 r1 #1). The waiter of another agent stays.
    #[test]
    fn bulk_drop_scans_each_waiter_once() {
        const N: usize = 2_000;
        let (k, k2) = (key(), key());
        let other = Agent::new();
        let a = Agent::new();
        for _ in 0..N {
            assert!(matches!(
                a.wait_async(k, || true, None),
                AsyncWait::Waiting(_)
            ));
        }
        assert!(matches!(
            other.wait_async(k, || true, None),
            AsyncWait::Waiting(_)
        ));
        for _ in 0..3 {
            assert!(matches!(
                a.wait_async(k2, || true, None),
                AsyncWait::Waiting(_)
            ));
        }
        assert_eq!((waiting(k), waiting(k2)), (N + 1, 3));
        SCANNED.with(|n| n.set(0));
        drop(a);
        let scanned = SCANNED.with(Cell::get);
        assert!(
            scanned <= N + 1 + 3,
            "{scanned} waiters inspected for {} in the list",
            N + 4
        );
        assert_eq!((waiting(k), waiting(k2)), (1, 0));
        assert_eq!(notify(k2, u64::MAX), 0);
        assert_eq!(notify(k, u64::MAX), 1, "only the live agent's waiter");
        assert_eq!(other.take_ready().len(), 1);
    }

    /// `take_ready` removes many due async waiters of one key in one pass over the key's
    /// queue (Codex TA-4 r1 #1), and gives them as TimedOut in the order they were made.
    #[test]
    fn bulk_expiry_scans_each_waiter_once() {
        const N: usize = 2_000;
        let k = key();
        let a = Agent::new();
        let mut ids = Vec::with_capacity(N);
        for _ in 0..N {
            let AsyncWait::Waiting(id) = a.wait_async(k, || true, Some(Duration::from_millis(1)))
            else {
                panic!("waits")
            };
            ids.push(id);
        }
        std::thread::sleep(Duration::from_millis(5));
        SCANNED.with(|n| n.set(0));
        let got = a.take_ready();
        let scanned = SCANNED.with(Cell::get);
        assert!(
            scanned <= N + 1,
            "{scanned} waiters inspected for {N} in the list"
        );
        let want: Vec<_> = ids.iter().map(|&id| (id, Outcome::TimedOut)).collect();
        assert_eq!(got, want);
        assert_eq!((a.pending(), waiting(k)), (0, 0));
    }

    #[test]
    fn can_block_defaults_to_true() {
        let a = Agent::new();
        assert!(a.can_block());
        a.set_can_block(false);
        assert!(!a.can_block());
    }
}
