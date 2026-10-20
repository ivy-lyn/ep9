//! The sized-stack worker: every parse, semantic and scope pass runs on a thread whose stack is
//! sized for the depth cap (01-front-end §5 "Parse path", §6).
//!
//! ```text
//! caller thread                         front-end worker (one per caller thread, kept alive)
//!   run(job) --Job--> channel --------> loop { job = recv(); job() }   stack = STACK_BYTES
//!      ^                                        |
//!      +---------- result or panic payload -----+
//! ```
//!
//! - One worker per calling thread, kept in a thread local. The caller blocks until the job ends,
//!   so one worker is enough, and isolates on one thread (D14) share it.
//! - A job that runs on a worker thread runs inline: a nested call does not spawn a second worker.
//! - A panic inside the job is caught on the worker and resumed on the caller. The worker lives on.
//! - The stack is a reservation. The OS commits pages only as deep as a job really goes.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

type Job = Box<dyn FnOnce() + Send>;

/// The worker thread could not be created (the OS refused the stack reservation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpawnError {
    pub(crate) message: String,
}

struct Worker {
    stack_bytes: usize,
    jobs: Option<mpsc::Sender<Job>>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    fn spawn(stack_bytes: usize) -> Result<Self, SpawnError> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let handle = thread::Builder::new()
            .name("ep9-frontend".into())
            .stack_size(stack_bytes)
            .spawn(move || {
                ON_WORKER.with(|on| on.set(true));
                // Ends when the owning thread drops its sender.
                while let Ok(job) = inbox.recv() {
                    job();
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
            jobs: Some(jobs),
            handle: Some(handle),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Close the channel first, so the worker loop ends; then wait for it.
        drop(self.jobs.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
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
        // At most two tries: the second one follows a worker that died (its channel is closed).
        for _ in 0..2 {
            // A caller that needs a bigger stack than the current worker has gets a new worker.
            if slot.as_ref().is_some_and(|w| w.stack_bytes < stack_bytes) {
                *slot = None;
            }
            if slot.is_none() {
                *slot = Some(Worker::spawn(stack_bytes)?);
            }
            let worker = slot.as_ref().expect("set above");
            let sender = worker
                .jobs
                .as_ref()
                .expect("open while the worker is in the slot");
            match sender.send(wrapped.take().expect("sent at most once")) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(job)) => {
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

/// True on a front-end worker thread.
#[cfg(test)]
pub(crate) fn on_worker() -> bool {
    ON_WORKER.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
