//! The sized-stack worker: every parse, semantic and scope pass runs on a thread whose stack is
//! sized for the depth cap (01-front-end §5 "Parse path", §6).
//!
//! ```text
//! caller thread                          front-end worker (one per caller thread, kept alive)
//!   run(job) --Job--> mailbox (1 slot) --> loop { take the job or park; job() }  STACK_BYTES
//!      ^                                                                |
//!      +--------- result or panic payload (a std sync channel) ---------+
//! ```
//!
//! - One worker per calling thread, kept in a thread local. The caller blocks until the job ends,
//!   so one worker and one job slot are enough, and isolates on one thread (D14) share it.
//! - A job that runs on a worker thread runs inline: a nested call does not spawn a second worker.
//! - A panic inside the job is caught on the worker and resumed on the caller. The worker lives on.
//!   Under `panic = "abort"` (P2-INFRA2) the `catch_unwind` in `run` never runs: the process
//!   aborts in the job. So nothing relies on it for isolation (ep9-test262 uses worker processes).
//! - The stack is a reservation. The OS commits pages only as deep as a job really goes.
//! - Dropping a worker sets the mailbox close flag and unparks the thread: no join
//!   (M2-P1-WIN-TLSJOIN), no lock, no allocation, no free (M7-P3-WIN-EXITHANG). The thread sees
//!   the flag and ends on its own. The drop usually runs in the caller's thread-local destructor.
//!   On Windows, when the caller thread ends the process (returns from `main`, or
//!   `std::process::exit`), the OS has already ended every other thread, maybe while one held a
//!   lock. A join of such a thread panics, and a panic in a thread-local destructor aborts
//!   (0xC0000409). A lock that a killed thread held stays locked: the old std job channel took
//!   its waker lock in its close, and such an exit waited for ever (CI 42484485512). The one OS
//!   call left in the drop is `Thread::unpark` (`WakeByAddressSingle` on Windows); the old close
//!   made it too (it unparked the parked worker) at every exit before. Nothing is freed: the
//!   worker thread's own references (never released by a killed thread) keep the mailbox and its
//!   `Thread` alive.
//! - No job is left in the slot at the close: `run` waits for its reply, which the job sends after
//!   the thread took it out. The loop reads the flag before the slot, so such a job would run.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

/// The worker thread could not be created (the OS refused the stack reservation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpawnError {
    pub(crate) message: String,
}

/// The one-slot job mailbox of a worker, shared by the [`Worker`] and its thread.
struct Mailbox {
    /// Taken by [`Worker::send`] and by the worker thread, never by [`Worker`]'s drop.
    slot: Mutex<Slot>,
    /// Set by [`Worker`]'s drop: the thread ends when it finds the slot empty.
    closed: AtomicBool,
}

#[derive(Default)]
struct Slot {
    job: Option<Job>,
    /// The worker thread left its loop. Set under the lock, so a send after it is refused.
    gone: bool,
}

/// A front-end worker thread, owned through its mailbox. Dropping it closes the mailbox; the
/// thread is never joined (see the module doc).
struct Worker {
    stack_bytes: usize,
    mailbox: Arc<Mailbox>,
    thread: thread::Thread,
}

thread_local! {
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
    static ON_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Run `job` on this thread's front-end worker, which has a `stack_bytes` stack.
///
/// A panic in `job` resumes on the caller. On a worker thread, `job` runs inline.
///
/// # Errors
///
/// Returns [`SpawnError`] if the worker thread cannot be created.
pub(crate) fn run<R, F>(stack_bytes: usize, job: F) -> Result<R, SpawnError>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    if ON_WORKER.with(Cell::get) {
        return Ok(job());
    }
    let (reply_tx, reply_rx) = mpsc::sync_channel::<thread::Result<R>>(1);
    let mut wrapped: Option<Job> = Some(Box::new(move || {
        let result = panic::catch_unwind(AssertUnwindSafe(job));
        // The caller waits for this reply; it is gone only if the caller itself unwound.
        let _ = reply_tx.send(result);
    }));
    WORKER.with(|cell| -> Result<(), SpawnError> {
        let mut slot = cell.borrow_mut();
        // At most two tries: the second one follows a worker thread that ended (never expected).
        for _ in 0..2 {
            // A caller that needs a bigger stack than the current worker has gets a new worker.
            if slot.as_ref().is_some_and(|w| w.stack_bytes < stack_bytes) {
                *slot = None;
            }
            if slot.is_none() {
                *slot = Some(Worker::spawn(stack_bytes)?);
            }
            let worker = slot.as_ref().expect("set above");
            match worker.send(wrapped.take().expect("sent at most once")) {
                Ok(()) => return Ok(()),
                Err(job) => {
                    wrapped = Some(job);
                    *slot = None;
                }
            }
        }
        Err(SpawnError {
            message: "the front-end worker stopped twice".into(),
        })
    })?;
    match reply_rx.recv() {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(payload)) => resume(payload),
        // The job was dropped without a reply: the worker thread died. Never expected.
        Err(mpsc::RecvError) => Err(SpawnError {
            message: "the front-end worker stopped during a job".into(),
        }),
    }
}

fn resume<R>(payload: Box<dyn Any + Send>) -> R {
    panic::resume_unwind(payload)
}

impl Mailbox {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        // No code panics while it holds the lock; a poisoned lock is still a sound slot.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Owned by the worker thread. On every end of the thread (a return or an unwind), it marks the
/// mailbox gone and drops a job left in the slot, so that job's caller gets no reply and fails.
/// A thread killed by the OS runs no drop; its caller is then the thread that ends the process.
struct Gone(Arc<Mailbox>);

impl Drop for Gone {
    fn drop(&mut self) {
        let job = {
            let mut slot = self.0.lock();
            slot.gone = true;
            slot.job.take()
        };
        drop(job);
    }
}

impl Worker {
    fn spawn(stack_bytes: usize) -> Result<Self, SpawnError> {
        let mailbox = Arc::new(Mailbox {
            slot: Mutex::new(Slot::default()),
            closed: AtomicBool::new(false),
        });
        let inbox = Gone(Arc::clone(&mailbox));
        // The handle is dropped at the end of this function: that detaches the thread.
        let handle = thread::Builder::new()
            .name("ep9-frontend".into())
            .stack_size(stack_bytes)
            .spawn(move || {
                ON_WORKER.with(|on| on.set(true));
                let mailbox = &inbox.0;
                loop {
                    // The flag first, then the slot: a job put before the close (the store
                    // below is Release, this load is Acquire) is then in the slot.
                    let closed = mailbox.closed.load(Ordering::Acquire);
                    let job = mailbox.lock().job.take();
                    match job {
                        Some(job) => job(),
                        None if closed => return,
                        // An unpark before this park leaves a token, so park returns at once.
                        None => thread::park(),
                    }
                }
            })
            .map_err(|err| SpawnError {
                message: format!(
                    "cannot reserve a {} MiB front-end stack: {err}",
                    stack_bytes >> 20
                ),
            })?;
        Ok(Self {
            stack_bytes,
            mailbox,
            thread: handle.thread().clone(),
        })
    }

    /// Put `job` in the slot and wake the thread. `Err(job)` when the thread has left its loop.
    fn send(&self, job: Job) -> Result<(), Job> {
        {
            let mut slot = self.mailbox.lock();
            if slot.gone {
                return Err(job);
            }
            debug_assert!(slot.job.is_none(), "run waits for the reply of each job");
            slot.job = Some(job);
        }
        self.thread.unpark();
        Ok(())
    }
}

impl Drop for Worker {
    /// No join, no lock, no allocation and no free (see the module doc).
    fn drop(&mut self) {
        self.mailbox.closed.store(true, Ordering::Release);
        self.thread.unpark();
        // The fields drop next: two `Arc` decrements that the thread's references keep above 0.
    }
}

/// True on a front-end worker thread.
#[cfg(test)]
pub(crate) fn on_worker() -> bool {
    ON_WORKER.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const SMALL: usize = 1 << 20;

    #[test]
    fn runs_on_a_worker_and_returns_the_value() {
        assert!(!on_worker());
        let value = run(SMALL, || (on_worker(), 40 + 2)).unwrap();
        assert_eq!(value, (true, 42));
    }

    #[test]
    fn reuses_one_worker_per_thread() {
        let first = run(SMALL, || thread::current().id()).unwrap();
        let second = run(SMALL, || thread::current().id()).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, thread::current().id());
    }

    #[test]
    fn nested_calls_run_inline() {
        let ids = run(SMALL, || {
            let outer = thread::current().id();
            let inner = run(SMALL, || thread::current().id()).unwrap();
            (outer, inner)
        })
        .unwrap();
        assert_eq!(ids.0, ids.1);
    }

    #[test]
    fn a_panic_resumes_on_the_caller_and_the_worker_lives_on() {
        let caught = panic::catch_unwind(|| run(SMALL, || panic!("boom")));
        let payload = caught.expect_err("the panic reaches the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
        assert_eq!(run(SMALL, || 7).unwrap(), 7);
    }

    #[test]
    fn a_bigger_request_replaces_the_worker() {
        let first = run(SMALL, || thread::current().id()).unwrap();
        let bigger = run(2 * SMALL, || thread::current().id()).unwrap();
        assert_ne!(first, bigger);
        // A smaller request reuses the bigger worker.
        assert_eq!(run(SMALL, || thread::current().id()).unwrap(), bigger);
    }

    /// Sends one message when dropped. Kept in a thread local, it signals the end of a thread.
    struct SendOnDrop(mpsc::Sender<()>);

    impl Drop for SendOnDrop {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    thread_local! {
        static AT_THREAD_END: RefCell<Option<SendOnDrop>> = const { RefCell::new(None) };
    }

    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn dropping_a_worker_does_not_wait_for_its_thread() {
        // M2-P1-WIN-TLSJOIN. At a Windows process exit the worker thread is already gone when the
        // owner's thread-local destructor drops the worker; a join there panics. A busy worker
        // stands in for that thread on every host: the drop must return without waiting for it.
        let worker = Worker::spawn(SMALL).unwrap();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (ended_tx, ended_rx) = mpsc::channel::<()>();
        let job: Job = Box::new(move || {
            AT_THREAD_END.with(|end| *end.borrow_mut() = Some(SendOnDrop(ended_tx)));
            let _ = gate_rx.recv();
        });
        assert!(worker.send(job).is_ok(), "the worker thread is alive");
        let (dropped_tx, dropped_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            drop(worker);
            let _ = dropped_tx.send(());
        });
        let dropped = dropped_rx.recv_timeout(WAIT);
        drop(gate_tx);
        assert!(dropped.is_ok(), "the drop waited for the worker thread");
        // The worker thread still ends on its own once its job and channel are done.
        assert!(
            ended_rx.recv_timeout(WAIT).is_ok(),
            "the worker thread did not end"
        );
    }

    #[test]
    fn dropping_a_worker_takes_no_lock_its_thread_may_hold() {
        // M7-P3-WIN-EXITHANG. At a Windows process exit the OS can end the worker thread while it
        // holds the mailbox lock, and nothing releases that lock. A helper thread that holds the
        // lock stands in for that thread on every host: the drop must return while it is held.
        let worker = Worker::spawn(SMALL).unwrap();
        let (ended_tx, ended_rx) = mpsc::channel::<()>();
        let (ran_tx, ran_rx) = mpsc::channel::<()>();
        let job: Job = Box::new(move || {
            AT_THREAD_END.with(|end| *end.borrow_mut() = Some(SendOnDrop(ended_tx)));
            let _ = ran_tx.send(());
        });
        assert!(worker.send(job).is_ok(), "the worker thread is alive");
        assert!(ran_rx.recv_timeout(WAIT).is_ok(), "the job did not run");
        let mailbox = Arc::clone(&worker.mailbox);
        let (locked_tx, locked_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = thread::spawn(move || {
            let held = mailbox.lock();
            let _ = locked_tx.send(());
            let _ = release_rx.recv();
            drop(held);
        });
        assert!(
            locked_rx.recv_timeout(WAIT).is_ok(),
            "the helper has no lock"
        );
        let (dropped_tx, dropped_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            drop(worker);
            let _ = dropped_tx.send(());
        });
        let dropped = dropped_rx.recv_timeout(WAIT);
        drop(release_tx);
        holder.join().unwrap();
        assert!(dropped.is_ok(), "the drop waited for the mailbox lock");
        // With the lock free again, the worker thread sees the close and ends.
        assert!(
            ended_rx.recv_timeout(WAIT).is_ok(),
            "the worker thread did not end"
        );
    }

    #[test]
    fn a_caller_thread_that_ends_ends_its_worker_thread() {
        let (ended_tx, ended_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            run(SMALL, move || {
                AT_THREAD_END.with(|end| *end.borrow_mut() = Some(SendOnDrop(ended_tx)));
            })
            .unwrap();
        })
        .join()
        .unwrap();
        assert!(
            ended_rx.recv_timeout(WAIT).is_ok(),
            "the worker thread did not end"
        );
    }

    /// Linux x86-64 user code under Apple's Rosetta translator (Docker on Apple silicon). The
    /// translator aborts the process on an mmap it cannot place, where a real kernel returns
    /// ENOMEM ("rosetta error: could not find free space for allocation size 4000000001000").
    fn under_rosetta_for_linux() -> bool {
        cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && std::fs::read_to_string("/proc/self/maps").is_ok_and(|m| m.contains("[rosetta]"))
    }

    #[test]
    fn an_impossible_stack_is_an_error_not_an_abort() {
        if under_rosetta_for_linux() {
            eprintln!(
                "skipped: the Rosetta translator aborts on a 1 PiB mmap (a real kernel fails it)"
            );
            return;
        }
        // 1 PiB is above every 64-bit user address space (47-48 bits), so no OS can reserve it.
        let result = run(1 << 50, || ());
        let err = result.expect_err("no OS reserves 1 PiB");
        assert!(err.message.contains("front-end stack"), "{err:?}");
        // The next call can still create a worker with a sane size.
        assert_eq!(run(SMALL, || 1).unwrap(), 1);
    }

    #[test]
    fn a_send_after_the_thread_left_its_loop_is_refused() {
        // The retry in `run` relies on this: the job comes back, it is not lost in the slot.
        let worker = Worker::spawn(SMALL).unwrap();
        let mailbox = Arc::clone(&worker.mailbox);
        let thread_end = worker.thread.clone();
        drop(worker);
        let start = std::time::Instant::now();
        while !mailbox.lock().gone {
            assert!(
                start.elapsed() < WAIT,
                "the worker thread did not leave its loop"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let again = Worker {
            stack_bytes: SMALL,
            mailbox,
            thread: thread_end,
        };
        let refused = again.send(Box::new(|| ()));
        assert!(refused.is_err(), "a send to a gone thread was taken");
    }
}
