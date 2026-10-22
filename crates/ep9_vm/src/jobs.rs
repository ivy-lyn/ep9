//! The microtask job queue and the per-isolate Promise state (owner P4-PROMISE; RT-9). The
//! promise algorithms themselves are `Cx` methods in `abi/jobs.rs`; this file holds the state
//! they share and the layouts of their heap records.
//!
//! ```text
//! Runtime::jobs: JobState
//!   roots (a heap root source, as symbol.rs):  queue head ──> job ──> job ──> ... <── queue tail
//!                                              rejected head ──> record ──> ... <── rejected tail
//!   closures: one FunctionInfo per (behaviour, name, length), boxed, never freed
//!   tracker:  IsolateConfig::promise_rejection_tracker
//!
//! a Promise    SLOT_OBJECT, ObjectClass::Promise, slots [`promise`]
//! a record     SLOT_OBJECT, ObjectClass::PromiseReaction, null prototype; JS never sees one:
//!              a reaction ([`reaction`]), a job ([`job`]), the state of a pair of resolving
//!              functions ([`resolving`]), the state of a capability executor ([`executor`]),
//!              an entry of the rejected list ([`rejected`]), or closure data of the builtins
//! ```
//!
//! Every list is a chain of heap records (plan rule 8): no Rust `Vec` or `VecDeque` holds a
//! Value. The roots are `Cell<u64>`s that a root source traces, and a store into one takes no
//! barrier (E3); a store into a record goes through `object::slot_set` (barriered).
//!
//! A reaction record holds both halves of one `then` (§27.2.5.4.1 PerformPromiseThen appends
//! a fulfill and a reject reaction together, so one list of pairs keeps both lists' order).

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use ep9_heap::{Heap, Tracer};

use crate::abi::{HostFn, PromiseRejectionTracker};
use crate::frame::UNDEFINED;
use crate::unit::FunctionInfo;

/// The internal slots of a Promise object (§27.2.6 table 89).
pub(crate) mod promise {
    /// `[[PromiseState]]`: an int32, [`PENDING`], [`FULFILLED`] or [`REJECTED`].
    pub const STATE: usize = 0;
    /// `[[PromiseResult]]`.
    pub const RESULT: usize = 1;
    /// `[[PromiseFulfillReactions]]` and `[[PromiseRejectReactions]]`: the first reaction
    /// record of the chain (undefined: none).
    pub const REACTIONS: usize = 2;
    /// The last reaction record (undefined: none), for the append.
    pub const REACTIONS_TAIL: usize = 3;
    /// `[[PromiseIsHandled]]`: a boolean.
    pub const IS_HANDLED: usize = 4;
    pub const COUNT: usize = 5;
    pub const PENDING: i32 = 0;
    pub const FULFILLED: i32 = 1;
    pub const REJECTED: i32 = 2;
}

/// A PromiseReaction Record pair (§27.2.1.2): the capability (undefined in all three slots
/// for none) and the two handlers (undefined = empty).
pub(crate) mod reaction {
    /// `[[Capability]].[[Promise]]` (undefined: no capability).
    pub const PROMISE: usize = 0;
    /// `[[Capability]].[[Resolve]]`.
    pub const RESOLVE: usize = 1;
    /// `[[Capability]].[[Reject]]`.
    pub const REJECT: usize = 2;
    /// `[[Handler]]` of the Fulfill reaction (undefined: empty).
    pub const ON_FULFILLED: usize = 3;
    /// `[[Handler]]` of the Reject reaction (undefined: empty).
    pub const ON_REJECTED: usize = 4;
    /// The next reaction record of the promise (undefined: last).
    pub const NEXT: usize = 5;
    pub const COUNT: usize = 6;
}

/// A queued job (HostEnqueuePromiseJob, §9.5.5): its kind, three operands and the next job.
pub(crate) mod job {
    /// An int32: [`REACTION`], [`THENABLE`] or [`CALLBACK`].
    pub const KIND: usize = 0;
    pub const A: usize = 1;
    pub const B: usize = 2;
    pub const C: usize = 3;
    /// The next job of the queue (undefined: last).
    pub const NEXT: usize = 4;
    pub const COUNT: usize = 5;
    /// NewPromiseReactionJob (§27.2.2.1): A = the reaction record, B = the argument, C = true
    /// for the Reject reaction.
    pub const REACTION: i32 = 0;
    /// NewPromiseResolveThenableJob (§27.2.2.2): A = the promise to resolve, B = the thenable,
    /// C = its `then`.
    pub const THENABLE: i32 = 1;
    /// An embedder job (`Cx::enqueue_job`): A = the callback, B = its argument.
    pub const CALLBACK: i32 = 2;
}

/// The state that a pair of resolving functions shares (CreateResolvingFunctions,
/// §27.2.1.3): the promise and `[[AlreadyResolved]]`.
pub(crate) mod resolving {
    pub const PROMISE: usize = 0;
    /// A boolean.
    pub const ALREADY_RESOLVED: usize = 1;
    pub const COUNT: usize = 2;
}

/// The state of a GetCapabilitiesExecutor function (NewPromiseCapability, §27.2.1.5).
pub(crate) mod executor {
    pub const RESOLVE: usize = 0;
    pub const REJECT: usize = 1;
    pub const COUNT: usize = 2;
}

/// An entry of the rejected list: a promise that was rejected while it had no handler
/// (HostPromiseRejectionTracker "reject"), and the next entry.
pub(crate) mod rejected {
    pub const PROMISE: usize = 0;
    pub const NEXT: usize = 1;
    pub const COUNT: usize = 2;
}

/// The roots of the job state: the two lists (head and tail each).
struct JobRoots {
    queue_head: Cell<u64>,
    queue_tail: Cell<u64>,
    rejected_head: Cell<u64>,
    rejected_tail: Cell<u64>,
}

impl JobRoots {
    fn trace(&self, t: &mut Tracer<'_>) {
        #[cfg(test)]
        if NEGATIVE_CONTROL_UNTRACED_QUEUE.with(Cell::get) {
            return;
        }
        t.visit_bits(self.queue_head.get());
        t.visit_bits(self.queue_tail.get());
        t.visit_bits(self.rejected_head.get());
        t.visit_bits(self.rejected_tail.get());
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only negative control (`tests::jobs`): the root source skips the lists, so the
    /// queued jobs are not roots.
    pub(crate) static NEGATIVE_CONTROL_UNTRACED_QUEUE: Cell<bool> = const { Cell::new(false) };
}

/// The key of a built-in closure's FunctionInfo: the behaviour's address, its `length` and
/// its name.
type ClosureKey = (usize, u32, String);

/// Per-isolate job and Promise state (`Runtime::jobs`).
pub(crate) struct JobState {
    roots: Rc<JobRoots>,
    /// The FunctionInfos of built-in closures (`Cx::new_builtin_closure`): one per behaviour,
    /// name and length, boxed so function cells can point at them; never freed while the
    /// isolate lives (`realm::new_host_function` would leak one per call).
    closures: RefCell<HashMap<ClosureKey, Box<FunctionInfo>>>,
    /// The embedder's HostPromiseRejectionTracker hook.
    tracker: Cell<Option<PromiseRejectionTracker>>,
}

impl JobState {
    /// The empty state, with its root source registered on `heap`. Runs in `Runtime::new`.
    pub fn new(heap: &Heap) -> JobState {
        let roots = Rc::new(JobRoots {
            queue_head: Cell::new(UNDEFINED),
            queue_tail: Cell::new(UNDEFINED),
            rejected_head: Cell::new(UNDEFINED),
            rejected_tail: Cell::new(UNDEFINED),
        });
        let source = Rc::clone(&roots);
        heap.add_root_source(move |t| source.trace(t));
        JobState {
            roots,
            closures: RefCell::new(HashMap::new()),
            tracker: Cell::new(None),
        }
    }

    pub fn queue_head(&self) -> u64 {
        self.roots.queue_head.get()
    }

    pub fn set_queue_head(&self, v: u64) {
        self.roots.queue_head.set(v);
    }

    pub fn queue_tail(&self) -> u64 {
        self.roots.queue_tail.get()
    }

    pub fn set_queue_tail(&self, v: u64) {
        self.roots.queue_tail.set(v);
    }

    pub fn rejected_head(&self) -> u64 {
        self.roots.rejected_head.get()
    }

    pub fn set_rejected_head(&self, v: u64) {
        self.roots.rejected_head.set(v);
    }

    pub fn rejected_tail(&self) -> u64 {
        self.roots.rejected_tail.get()
    }

    pub fn set_rejected_tail(&self, v: u64) {
        self.roots.rejected_tail.set(v);
    }

    pub fn tracker(&self) -> Option<PromiseRejectionTracker> {
        self.tracker.get()
    }

    pub fn set_tracker(&self, t: Option<PromiseRejectionTracker>) {
        self.tracker.set(t);
    }

    /// The FunctionInfo of the built-in closure `behaviour` with `name` and `length` (made on
    /// first use; the box never moves or goes while the isolate lives).
    pub fn closure_info(&self, behaviour: HostFn, name: &str, length: u32) -> *const FunctionInfo {
        let key = (behaviour as usize, length, name.to_string());
        let mut map = self.closures.borrow_mut();
        let info = map
            .entry(key)
            .or_insert_with(|| Box::new(FunctionInfo::host(behaviour, name, length, false)));
        &**info
    }
}
