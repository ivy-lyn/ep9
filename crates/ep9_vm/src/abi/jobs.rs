//! Jobs and promises on the builtin ABI (owner P4-PROMISE; RT-9, H20): the job queue
//! (`enqueue_job`, `perform_microtask_checkpoint`), built-in closures (`new_builtin_closure`,
//! `closure_data`), the Promise abstract operations of §27.2.1-§27.2.5 and
//! HostPromiseRejectionTracker. `ep9_builtins` (`promise.rs`) builds the Promise constructor
//! and its methods on these; the VM finds `%Promise%` through `IntrinsicId::Promise`, so
//! VM code (month 3: async functions) uses them without `ep9_builtins`.
//!
//! ```text
//! new Promise(executor)          new_promise + create_resolving_functions, Call(executor)
//! resolve(x) (a closure)         x a thenable? enqueue a THENABLE job : fulfill_promise
//! p.then(f, r)                   perform_promise_then: pending? append a reaction
//!                                                      : enqueue a REACTION job
//! fulfill_promise / reject_promise   state, result, then one REACTION job per reaction
//! perform_microtask_checkpoint   run the queue in order until it is empty (one interrupt
//!                                budget unit per job; termination stops it and clears it)
//! ```
//!
//! | Method | Spec |
//! |---|---|
//! | `enqueue_job(callback, argument)` | HostEnqueuePromiseJob (§9.5.5) of a job that calls `callback(argument)` |
//! | `perform_microtask_checkpoint()` | runs every queued job (HTML "perform a microtask checkpoint"); only with no JS on the stack |
//! | `new_builtin_closure(behaviour, name, length, data)`, `closure_data(f)` | CreateBuiltinFunction (§10.3.4) of an Abstract Closure: `data` is what it captures |
//! | `is_promise(v)` | IsPromise (§27.2.1.6) |
//! | `new_promise(proto)` | the object of `new Promise` steps 3-7 (state pending) |
//! | `promise_state(p)`, `promise_result(p)` | `[[PromiseState]]`, `[[PromiseResult]]` |
//! | `create_resolving_functions(p)` | CreateResolvingFunctions (§27.2.1.3) |
//! | `fulfill_promise(p, v)`, `reject_promise(p, r)` | FulfillPromise (§27.2.1.4), RejectPromise (§27.2.1.7) |
//! | `new_promise_capability(c)` | NewPromiseCapability (§27.2.1.5) |
//! | `promise_resolve(c, x)` | PromiseResolve (§27.2.4.7.1) |
//! | `perform_promise_then(p, f, r, capability)` | PerformPromiseThen (§27.2.5.4.1) |
//! | `new_error_object(proto, message)` | an Error instance (`[[ErrorData]]`) with a stack, for `AggregateError` |
//! | `catch_exception(t)` | the `[[Value]]` of a throw completion, taken (a `catch`; a termination passes on) |
//!
//! Errors from a job: a reaction job turns a throw of its handler into a rejection; a throw
//! that still leaves a job (a capability's `resolve` or `reject`, an embedder callback) stops
//! the checkpoint with that exception pending, and the jobs behind it stay queued.

use ep9_value::{JsResult, Throw};

use super::{Cx, HostFn, JsFunction, JsObject, JsString, Local, ObjectClass};
use crate::builtins::IntrinsicId;
use crate::conv::describe;
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::jobs::{executor, job, promise, reaction, rejected, resolving};
use crate::kinds;
use crate::object;
use crate::runtime::{Atom, ErrorKind};

/// The state of a promise (`[[PromiseState]]`, §27.2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

/// The operation of HostPromiseRejectionTracker (§27.2.1.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromiseRejectionOperation {
    /// A promise was rejected while it had no handler.
    Reject,
    /// A handler was added to a rejected promise that had none.
    Handle,
}

/// An embedder's HostPromiseRejectionTracker (`IsolateConfig::promise_rejection_tracker`):
/// called with the promise and the operation. It cannot throw: an exception it leaves pending
/// is dropped. The VM keeps its own list of unhandled rejections either way
/// (`Isolate::take_unhandled_rejections`).
pub type PromiseRejectionTracker =
    for<'s> fn(&mut Cx<'s>, Local<'s, JsObject>, PromiseRejectionOperation);

/// A PromiseCapability Record (§27.2.1.1). Holds `Local`s, so it is stack-only (H20 R2).
#[derive(Clone, Copy, Debug)]
pub struct PromiseCapability<'s> {
    pub promise: Local<'s, JsObject>,
    pub resolve: Local<'s>,
    pub reject: Local<'s>,
}

impl<'s> Cx<'s> {
    // ---- records -------------------------------------------------------------------------

    /// A new internal record (a PromiseReaction-class object, null prototype) of `n` slots.
    fn record(&mut self, n: usize) -> Local<'s, JsObject> {
        self.new_object_with_slots(Local::null(), ObjectClass::PromiseReaction, n)
    }

    /// Slot `i` of a record or a promise (no class check: VM-made objects only).
    fn rec_get(&self, o: Local<'s, JsObject>, i: usize) -> Local<'s> {
        Self::local(object::slot_get(o.bits(), i))
    }

    fn rec_set(&mut self, o: Local<'s, JsObject>, i: usize, v: impl Into<Local<'s>>) {
        object::slot_set(self.rt(), o.bits(), i, v.into().bits());
    }

    /// Catches the throw completion `t`: takes its pending exception and returns the thrown
    /// value (the `[[Value]]` of a `Completion(...)` the spec then handles, as a JS `catch`
    /// does). While the isolate terminates it gives `Err(t)` back: a termination cannot be
    /// caught.
    pub fn catch_exception(&mut self, t: Throw) -> JsResult<Local<'s>> {
        let rt = self.rt();
        if rt.terminating.get() {
            return Err(t);
        }
        let e = rt.take_pending();
        rt.shared.throw_site.take();
        debug_assert!(e != 0, "a throw completion without a pending exception");
        Ok(Self::local(if e == 0 { UNDEFINED } else { e }))
    }

    // ---- built-in closures ------------------------------------------------------------------

    /// CreateBuiltinFunction (§10.3.4) of an Abstract Closure: a new function object that runs
    /// `behaviour` (a host function), with the `name` and `length` properties, no `prototype`
    /// and no `[[Construct]]`, and `data` as what the closure captures ([`Cx::closure_data`]
    /// of `args.callee()` reads it back). One FunctionInfo per behaviour, name and length per
    /// isolate. GC point.
    pub fn new_builtin_closure(
        &mut self,
        behaviour: HostFn,
        name: &str,
        length: u32,
        data: impl Into<Local<'s>>,
    ) -> Local<'s, JsFunction> {
        let data = data.into();
        let rt = self.rt();
        let info = rt.jobs.closure_info(behaviour, name, length);
        // A host function's context word is not read by calls (host frames get no context),
        // so it holds the closure data; the function cell traces it.
        Self::local(object::new_function(rt, info, data.bits()))
    }

    /// The data of a built-in closure made by [`Cx::new_builtin_closure`] (undefined for any
    /// other value).
    pub fn closure_data(&self, f: impl Into<Local<'s>>) -> Local<'s> {
        let f = f.into().bits();
        if !kinds::is_function(f) || object::function_info(f).host.is_none() {
            return Local::undefined();
        }
        Self::local(object::function_context(f))
    }

    // ---- the job queue --------------------------------------------------------------------

    /// Appends a job record of `kind` to the queue (HostEnqueuePromiseJob, §9.5.5).
    fn enqueue(&mut self, kind: i32, a: Local<'s>, b: Local<'s>, c: Local<'s>) {
        let j = self.record(job::COUNT);
        self.rec_set(j, job::KIND, Local::int32(kind));
        self.rec_set(j, job::A, a);
        self.rec_set(j, job::B, b);
        self.rec_set(j, job::C, c);
        let rt = self.rt();
        let tail = rt.jobs.queue_tail();
        if tail == UNDEFINED {
            rt.jobs.set_queue_head(j.bits());
        } else {
            object::slot_set(rt, tail, job::NEXT, j.bits());
        }
        rt.jobs.set_queue_tail(j.bits());
    }

    /// Takes the first job of the queue.
    fn dequeue(&mut self) -> Option<Local<'s, JsObject>> {
        let rt = self.rt();
        let head = rt.jobs.queue_head();
        if head == UNDEFINED {
            return None;
        }
        let next = object::slot_get(head, job::NEXT);
        rt.jobs.set_queue_head(next);
        if next == UNDEFINED {
            rt.jobs.set_queue_tail(UNDEFINED);
        }
        Some(Self::local(head))
    }

    /// HostEnqueuePromiseJob (§9.5.5) for an embedder: a job that calls `callback` with
    /// `this` undefined and the one argument `argument`. Runs at the next microtask checkpoint,
    /// after the jobs queued before it. TypeError if `callback` is not callable.
    pub fn enqueue_job(
        &mut self,
        callback: impl Into<Local<'s>>,
        argument: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let (f, a) = (callback.into(), argument.into());
        if !f.is_callable() {
            let msg = format!("{} is not a function", describe(f.bits()));
            return Err(self.throw_type_error(&msg));
        }
        self.enqueue(job::CALLBACK, f, a, Local::undefined());
        Ok(())
    }

    /// Performs a microtask checkpoint: runs the queued jobs in order, and the jobs they
    /// queue, until the queue is empty. Each job takes one interrupt budget unit and polls
    /// (M2-P2-POLL), so an interrupt callback runs and `TerminateHandle` stops a job loop; a
    /// termination also clears the queue. A job that throws stops the checkpoint with its
    /// exception pending (`Err`); the jobs behind it stay queued. Then the rejected list drops
    /// any promise that got a handler ("handle" unlinks one at once already).
    ///
    /// Only with no JS on the stack (the spec runs it when the execution context stack is
    /// empty): `Isolate::with_cx_checkpoint` runs it after its scope, and an embedder inside
    /// `Isolate::with_cx` may call it there. From a host function it throws an Error and runs
    /// nothing.
    pub fn perform_microtask_checkpoint(&mut self) -> JsResult<()> {
        let rt = self.rt();
        if !rt.stack().fp().is_null() {
            return Err(self.throw_error(
                ErrorKind::Error,
                "a microtask checkpoint needs an empty JS stack",
            ));
        }
        loop {
            if rt.jobs.queue_head() == UNDEFINED {
                break;
            }
            if let Err(t) = self.poll_interrupt() {
                self.clear_queue();
                return Err(t);
            }
            let Some(j) = self.dequeue() else { break };
            if let Err(t) = self.run_job(j) {
                if rt.terminating.get() {
                    self.clear_queue();
                }
                return Err(t);
            }
        }
        self.prune_rejected();
        Ok(())
    }

    /// Drops every queued job (a termination: no job runs after it, as V8 drops its
    /// microtask queue then).
    fn clear_queue(&mut self) {
        self.rt().jobs.clear_queue();
    }

    fn run_job(&mut self, j: Local<'s, JsObject>) -> JsResult<()> {
        let kind = self.rec_get(j, job::KIND).as_int32();
        let (a, b, c) = (
            self.rec_get(j, job::A),
            self.rec_get(j, job::B),
            self.rec_get(j, job::C),
        );
        match kind {
            Some(job::REACTION) => {
                let r = a.as_object().expect("a reaction record");
                self.promise_reaction_job(r, b, c.as_bool() == Some(true))
            }
            Some(job::THENABLE) => {
                let p = a.as_object().expect("a promise");
                self.promise_resolve_thenable_job(p, b, c)
            }
            Some(job::CALLBACK) => self.call(a, Local::undefined(), &[b]).map(|_| ()),
            k => unreachable!("job kind {k:?}"),
        }
    }

    /// The Job of NewPromiseReactionJob (§27.2.2.1) for the Fulfill or the Reject half of the
    /// reaction record `r`.
    fn promise_reaction_job(
        &mut self,
        r: Local<'s, JsObject>,
        argument: Local<'s>,
        reject: bool,
    ) -> JsResult<()> {
        // a. Let promiseCapability be reaction.[[Capability]].
        let cap_promise = self.rec_get(r, reaction::PROMISE);
        // b-c. Let type be reaction.[[Type]]; let handler be reaction.[[Handler]].
        let handler = self.rec_get(
            r,
            if reject {
                reaction::ON_REJECTED
            } else {
                reaction::ON_FULFILLED
            },
        );
        // d-f. handlerResult: the argument as a normal (Fulfill) or throw (Reject) completion
        // when the handler is empty, else Completion(HostCallJobCallback(handler, undefined,
        // « argument »)).
        let (normal, value) = if handler.is_undefined() {
            (!reject, argument)
        } else {
            match self.call(handler, Local::undefined(), &[argument]) {
                Ok(v) => (true, v),
                Err(t) => (false, self.catch_exception(t)?),
            }
        };
        // g. If promiseCapability is undefined, then (assert: handlerResult is not abrupt)
        // return empty.
        if cap_promise.is_undefined() {
            debug_assert!(normal, "a reaction without a capability got a throw");
            return Ok(());
        }
        // h-i. Call(promiseCapability.[[Reject]] or .[[Resolve]], undefined, « value »).
        let f = self.rec_get(
            r,
            if normal {
                reaction::RESOLVE
            } else {
                reaction::REJECT
            },
        );
        self.call(f, Local::undefined(), &[value]).map(|_| ())
    }

    /// The Job of NewPromiseResolveThenableJob (§27.2.2.2). Steps 2-5: the job's realm is
    /// GetFunctionRealm(then), so it runs there (`realm::switch`), and the resolving functions
    /// of step a are that realm's (X-REALM3). The realm of the checkpoint is current again
    /// after the job, also after a throw.
    fn promise_resolve_thenable_job(
        &mut self,
        promise: Local<'s, JsObject>,
        thenable: Local<'s>,
        then: Local<'s>,
    ) -> JsResult<()> {
        let then_realm = crate::realm::get_function_realm(self.rt(), then.bits());
        let saved = self.cx.realm;
        if then_realm == saved {
            return self.promise_resolve_thenable_job_in(promise, thenable, then);
        }
        crate::realm::switch(self.cx, then_realm);
        let r = self.promise_resolve_thenable_job_in(promise, thenable, then);
        crate::realm::switch(self.cx, saved);
        r
    }

    /// Steps a-c of the Job of NewPromiseResolveThenableJob, in the job's realm.
    fn promise_resolve_thenable_job_in(
        &mut self,
        promise: Local<'s, JsObject>,
        thenable: Local<'s>,
        then: Local<'s>,
    ) -> JsResult<()> {
        // a. Let resolvingFunctions be CreateResolvingFunctions(promiseToResolve).
        let (resolve, reject) = self.create_resolving_functions(promise);
        // b. Let thenCallResult be Completion(HostCallJobCallback(then, thenable, « resolve,
        // reject »)).
        match self.call(then, thenable, &[resolve.into(), reject.into()]) {
            Ok(_) => Ok(()),
            // c. If thenCallResult is an abrupt completion, return ? Call(reject, undefined,
            // « thenCallResult.[[Value]] »).
            Err(t) => {
                let e = self.catch_exception(t)?;
                self.call(reject, Local::undefined(), &[e]).map(|_| ())
            }
        }
    }

    // ---- promises -------------------------------------------------------------------------

    /// IsPromise (§27.2.1.6): an object with a `[[PromiseState]]` slot. Never throws.
    pub fn is_promise(&self, v: impl Into<Local<'s>>) -> bool {
        self.object_class(v) == Some(ObjectClass::Promise)
    }

    /// A new pending promise with prototype `proto` (`new Promise` steps 3-7, §27.2.3.1:
    /// state pending, no reactions, not handled); `proto` is an object or null. GC point.
    pub fn new_promise(&mut self, proto: impl Into<Local<'s>>) -> Local<'s, JsObject> {
        let p = self.new_object_with_slots(proto.into(), ObjectClass::Promise, promise::COUNT);
        self.rec_set(p, promise::STATE, Local::int32(promise::PENDING));
        self.rec_set(p, promise::IS_HANDLED, Local::boolean(false));
        p
    }

    /// The bits of `p` if it is a promise, else a TypeError.
    fn require_promise(&mut self, p: Local<'s>) -> JsResult<Local<'s, JsObject>> {
        match p.as_object() {
            Some(o) if self.is_promise(o) => Ok(o),
            _ => {
                let msg = format!("{} is not a promise", describe(p.bits()));
                Err(self.throw_type_error(&msg))
            }
        }
    }

    fn state_of(&self, p: Local<'s, JsObject>) -> i32 {
        self.rec_get(p, promise::STATE)
            .as_int32()
            .expect("[[PromiseState]] is an int32")
    }

    /// `[[PromiseState]]` of the promise `p`; TypeError if `p` is not a promise.
    pub fn promise_state(&mut self, p: impl Into<Local<'s>>) -> JsResult<PromiseState> {
        let p = self.require_promise(p.into())?;
        Ok(match self.state_of(p) {
            promise::PENDING => PromiseState::Pending,
            promise::FULFILLED => PromiseState::Fulfilled,
            _ => PromiseState::Rejected,
        })
    }

    /// `[[PromiseResult]]` of the promise `p` (undefined while pending); TypeError if `p` is
    /// not a promise.
    pub fn promise_result(&mut self, p: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        let p = self.require_promise(p.into())?;
        Ok(self.rec_get(p, promise::RESULT))
    }

    /// `[[PromiseIsHandled]]` of the promise `p`; TypeError if `p` is not a promise.
    pub fn promise_is_handled(&mut self, p: impl Into<Local<'s>>) -> JsResult<bool> {
        let p = self.require_promise(p.into())?;
        Ok(self.rec_get(p, promise::IS_HANDLED).as_bool() == Some(true))
    }

    /// CreateResolvingFunctions(promise) (§27.2.1.3): the resolve and reject functions
    /// (length 1, name "", not constructors) that share one `[[AlreadyResolved]]`. GC point.
    pub fn create_resolving_functions(
        &mut self,
        promise: Local<'s, JsObject>,
    ) -> (Local<'s, JsFunction>, Local<'s, JsFunction>) {
        let state = self.record(resolving::COUNT);
        self.rec_set(state, resolving::PROMISE, promise);
        self.rec_set(state, resolving::ALREADY_RESOLVED, Local::boolean(false));
        let resolve = self.new_builtin_closure(resolve_function, "", 1, state);
        let reject = self.new_builtin_closure(reject_function, "", 1, state);
        (resolve, reject)
    }

    /// The steps of a promise resolve function after its `[[AlreadyResolved]]` check
    /// (§27.2.1.3.2 steps 7-16): resolves `promise` with `resolution`.
    fn resolve_promise(
        &mut self,
        promise: Local<'s, JsObject>,
        resolution: Local<'s>,
    ) -> JsResult<()> {
        // 7. If SameValue(resolution, promise) is true, then reject with a TypeError.
        if self.same_value(resolution, promise) {
            let e = error::new_error(
                self.cx,
                ErrorKind::TypeError,
                "Chaining cycle detected for promise #<Promise>",
            );
            self.reject_promise(promise, Self::local::<super::JsValue>(e));
            return Ok(());
        }
        // 8. If resolution is not an Object, then fulfill.
        let Some(thenable) = resolution.as_object() else {
            self.fulfill_promise(promise, resolution);
            return Ok(());
        };
        // 9. Let then be Completion(Get(resolution, "then")).
        let key = self.key("then");
        let then = match self.get(thenable, key) {
            Ok(v) => v,
            // 10. If then is an abrupt completion, then reject with then.[[Value]].
            Err(t) => {
                let e = self.catch_exception(t)?;
                self.reject_promise(promise, e);
                return Ok(());
            }
        };
        // 12. If IsCallable(thenAction) is false, then fulfill.
        if !then.is_callable() {
            self.fulfill_promise(promise, resolution);
            return Ok(());
        }
        // 13-15. HostMakeJobCallback(thenAction); NewPromiseResolveThenableJob(promise,
        // resolution, thenJobCallback); HostEnqueuePromiseJob.
        self.enqueue(job::THENABLE, promise.into(), resolution, then);
        Ok(())
    }

    /// The steps of a promise resolve function after its `[[AlreadyResolved]]` check
    /// (§27.2.1.3.2 steps 7-16), for a VM-made pending promise that one resolve at most
    /// reaches: an async function's own promise (§27.7.5.2 AsyncBlockStart step 3.f;
    /// `interp/slow/async_fn.rs`). A thenable takes a job; a throw of its `then` getter
    /// rejects. GC point; may run JS (the getter).
    pub(crate) fn resolve_promise_once(
        &mut self,
        promise: Local<'s, JsObject>,
        resolution: Local<'s>,
    ) -> JsResult<()> {
        debug_assert!(self.is_promise(promise) && self.state_of(promise) == promise::PENDING);
        self.resolve_promise(promise, resolution)
    }

    /// FulfillPromise(promise, value) (§27.2.1.4). `promise` is pending.
    pub fn fulfill_promise(&mut self, promise: Local<'s, JsObject>, value: impl Into<Local<'s>>) {
        self.settle(promise, promise::FULFILLED, value.into());
    }

    /// RejectPromise(promise, reason) (§27.2.1.7), with HostPromiseRejectionTracker(promise,
    /// "reject") when it has no handler. `promise` is pending.
    pub fn reject_promise(&mut self, promise: Local<'s, JsObject>, reason: impl Into<Local<'s>>) {
        self.settle(promise, promise::REJECTED, reason.into());
    }

    /// The shared steps of FulfillPromise and RejectPromise, then TriggerPromiseReactions
    /// (§27.2.1.8).
    fn settle(&mut self, p: Local<'s, JsObject>, state: i32, value: Local<'s>) {
        debug_assert!(self.is_promise(p) && self.state_of(p) == promise::PENDING);
        // 2. Let reactions be promise.[[PromiseFulfillReactions]] (or Reject).
        let mut r = self.rec_get(p, promise::REACTIONS);
        // 3-6. Result, both reaction lists cleared, state.
        self.rec_set(p, promise::RESULT, value);
        self.rec_set(p, promise::REACTIONS, Local::undefined());
        self.rec_set(p, promise::REACTIONS_TAIL, Local::undefined());
        self.rec_set(p, promise::STATE, Local::int32(state));
        let reject = state == promise::REJECTED;
        // RejectPromise 7. If promise.[[PromiseIsHandled]] is false, perform
        // HostPromiseRejectionTracker(promise, "reject").
        if reject && self.rec_get(p, promise::IS_HANDLED).as_bool() != Some(true) {
            self.track_rejection(p, PromiseRejectionOperation::Reject);
        }
        // TriggerPromiseReactions: one job per reaction, in list order.
        while let Some(rec) = r.as_object() {
            self.enqueue(job::REACTION, rec.into(), value, Local::boolean(reject));
            r = self.rec_get(rec, reaction::NEXT);
        }
    }

    /// HostPromiseRejectionTracker(promise, operation) (§27.2.1.9): "reject" appends the
    /// promise to the rejected list (`Isolate::take_unhandled_rejections` reads it) and
    /// "handle" unlinks it at once (O(1), through [`promise::REJECTED_ENTRY`]), as node drops
    /// a handled promise from its list; then the embedder's hook runs.
    fn track_rejection(&mut self, p: Local<'s, JsObject>, op: PromiseRejectionOperation) {
        let rt = self.rt();
        match op {
            PromiseRejectionOperation::Reject => {
                let e = self.record(rejected::COUNT);
                let tail = Self::local::<super::JsValue>(rt.jobs.rejected_tail());
                self.rec_set(e, rejected::PROMISE, p);
                self.rec_set(e, rejected::PREV, tail);
                self.rec_set(e, rejected::NEXT, Local::undefined());
                match tail.as_object() {
                    None => rt.jobs.set_rejected_head(e.bits()),
                    Some(t) => self.rec_set(t, rejected::NEXT, e),
                }
                rt.jobs.set_rejected_tail(e.bits());
                self.rec_set(p, promise::REJECTED_ENTRY, e);
            }
            PromiseRejectionOperation::Handle => {
                // No entry if `take_unhandled` took it already.
                if let Some(e) = self.rec_get(p, promise::REJECTED_ENTRY).as_object() {
                    self.unlink_rejected(e);
                }
            }
        }
        if let Some(hook) = rt.jobs.tracker() {
            hook(self, p, op);
            // The hook cannot throw (the host operation returns unused).
            if rt.pending() != 0 && !rt.terminating.get() {
                rt.take_pending();
                rt.shared.throw_site.take();
            }
        }
    }

    /// Unlinks the entry `e` from the rejected list, and clears its promise's back-link.
    fn unlink_rejected(&mut self, e: Local<'s, JsObject>) {
        let rt = self.rt();
        let prev = self.rec_get(e, rejected::PREV);
        let next = self.rec_get(e, rejected::NEXT);
        match prev.as_object() {
            None => rt.jobs.set_rejected_head(next.bits()),
            Some(pr) => self.rec_set(pr, rejected::NEXT, next),
        }
        match next.as_object() {
            None => rt.jobs.set_rejected_tail(prev.bits()),
            Some(nx) => self.rec_set(nx, rejected::PREV, prev),
        }
        let p = self
            .rec_get(e, rejected::PROMISE)
            .as_object()
            .expect("a promise");
        self.rec_set(p, promise::REJECTED_ENTRY, Local::undefined());
    }

    /// Drops the entries of the rejected list whose promise got a handler. "handle" unlinks
    /// its entry already, so this finds one only if `[[PromiseIsHandled]]` was set some other
    /// way.
    fn prune_rejected(&mut self) {
        let rt = self.rt();
        let mut e = Self::local::<super::JsValue>(rt.jobs.rejected_head());
        while let Some(rec) = e.as_object() {
            let next = self.rec_get(rec, rejected::NEXT);
            let p = self
                .rec_get(rec, rejected::PROMISE)
                .as_object()
                .expect("a promise");
            if self.rec_get(p, promise::IS_HANDLED).as_bool() == Some(true) {
                self.unlink_rejected(rec);
            }
            e = next;
        }
    }

    /// The reasons of the rejected promises that still have no handler, in rejection order,
    /// into `out`; the rejected list is emptied. Used by `Isolate::take_unhandled_rejections`.
    pub(crate) fn take_unhandled(&mut self, out: &ep9_heap::ScratchRoots<'_>) {
        let rt = self.rt();
        let mut e = Self::local::<super::JsValue>(rt.jobs.rejected_head());
        rt.jobs.set_rejected_head(UNDEFINED);
        rt.jobs.set_rejected_tail(UNDEFINED);
        while let Some(rec) = e.as_object() {
            let p = self
                .rec_get(rec, rejected::PROMISE)
                .as_object()
                .expect("a promise");
            // The entry leaves the list: a later "handle" finds no back-link.
            self.rec_set(p, promise::REJECTED_ENTRY, Local::undefined());
            if self.rec_get(p, promise::IS_HANDLED).as_bool() != Some(true) {
                out.push(crate::runtime::val(self.rec_get(p, promise::RESULT).bits()));
            }
            e = self.rec_get(rec, rejected::NEXT);
        }
    }

    /// NewPromiseCapability(C) (§27.2.1.5). TypeError if `c` is not a constructor, or if it
    /// did not call the executor with two callable functions. For `%Promise%` itself the
    /// promise and its resolving functions are made directly: the executor is not observable,
    /// and `%Promise%.prototype` is a non-configurable, non-writable data property.
    pub fn new_promise_capability(
        &mut self,
        c: impl Into<Local<'s>>,
    ) -> JsResult<PromiseCapability<'s>> {
        let c = c.into();
        // 1. If IsConstructor(C) is false, throw a TypeError exception.
        if !self.is_constructor(c) {
            let msg = format!("{} is not a constructor", describe(c.bits()));
            return Err(self.throw_type_error(&msg));
        }
        if self.has_intrinsic(IntrinsicId::Promise)
            && c.is_same_bits(&self.intrinsic(IntrinsicId::Promise))
        {
            let proto = self.intrinsic(IntrinsicId::PromisePrototype);
            let p = self.new_promise(proto);
            let (resolve, reject) = self.create_resolving_functions(p);
            return Ok(PromiseCapability {
                promise: p,
                resolve: resolve.into(),
                reject: reject.into(),
            });
        }
        // 2-4. The executor closure and its record { [[Resolve]], [[Reject]] }.
        let state = self.record(executor::COUNT);
        let exec = self.new_builtin_closure(capability_executor, "", 2, state);
        // 5. Let promise be ? Construct(C, « executor »).
        let p = self.construct(c, &[exec.into()], c)?;
        // 6-7. resolve and reject must be callable.
        let resolve = self.rec_get(state, executor::RESOLVE);
        let reject = self.rec_get(state, executor::REJECT);
        if !resolve.is_callable() || !reject.is_callable() {
            return Err(self.throw_type_error("Promise resolve or reject function is not callable"));
        }
        // 8. Return the PromiseCapability Record.
        Ok(PromiseCapability {
            promise: p,
            resolve,
            reject,
        })
    }

    /// PromiseResolve(C, x) (§27.2.4.7.1): `x` if it is a promise whose `constructor` is `c`,
    /// else a new promise of `c` resolved with `x`.
    pub fn promise_resolve(
        &mut self,
        c: impl Into<Local<'s, JsObject>>,
        x: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        let (c, x) = (c.into(), x.into());
        // 1. If IsPromise(x) is true, then
        if let Some(xo) = x.as_object()
            && self.is_promise(xo)
        {
            // a. Let xConstructor be ? Get(x, "constructor").
            let key = self.atom(Atom::Constructor);
            let xc = self.get(xo, key)?;
            // b. If SameValue(xConstructor, C) is true, return x.
            if self.same_value(xc, c) {
                return Ok(x);
            }
        }
        // 2. Let promiseCapability be ? NewPromiseCapability(C).
        let cap = self.new_promise_capability(c)?;
        // 3. Perform ? Call(promiseCapability.[[Resolve]], undefined, « x »).
        self.call(cap.resolve, Local::undefined(), &[x])?;
        // 4. Return promiseCapability.[[Promise]].
        Ok(cap.promise.into())
    }

    /// PerformPromiseThen(promise, onFulfilled, onRejected, resultCapability) (§27.2.5.4.1):
    /// `resultCapability.[[Promise]]`, or undefined without a capability. A handler that is not
    /// callable is empty. GC point; runs no JS (the embedder's tracker hook aside).
    pub fn perform_promise_then(
        &mut self,
        p: Local<'s, JsObject>,
        on_fulfilled: impl Into<Local<'s>>,
        on_rejected: impl Into<Local<'s>>,
        capability: Option<PromiseCapability<'s>>,
    ) -> Local<'s> {
        debug_assert!(self.is_promise(p));
        let (f, r) = (on_fulfilled.into(), on_rejected.into());
        let empty = |v: Local<'s>| {
            if v.is_callable() {
                v
            } else {
                Local::undefined()
            }
        };
        // 1-8. The reaction records.
        let rec = self.record(reaction::COUNT);
        if let Some(cap) = capability {
            self.rec_set(rec, reaction::PROMISE, cap.promise);
            self.rec_set(rec, reaction::RESOLVE, cap.resolve);
            self.rec_set(rec, reaction::REJECT, cap.reject);
        }
        self.rec_set(rec, reaction::ON_FULFILLED, empty(f));
        self.rec_set(rec, reaction::ON_REJECTED, empty(r));
        match self.state_of(p) {
            // 9. Pending: append the reactions.
            promise::PENDING => {
                let tail = self.rec_get(p, promise::REACTIONS_TAIL);
                match tail.as_object() {
                    None => self.rec_set(p, promise::REACTIONS, rec),
                    Some(t) => self.rec_set(t, reaction::NEXT, rec),
                }
                self.rec_set(p, promise::REACTIONS_TAIL, rec);
            }
            // 10. Fulfilled: a job for the fulfill reaction.
            promise::FULFILLED => {
                let v = self.rec_get(p, promise::RESULT);
                self.enqueue(job::REACTION, rec.into(), v, Local::boolean(false));
            }
            // 11. Rejected: "handle" if it had no handler, then a job for the reject reaction.
            _ => {
                let v = self.rec_get(p, promise::RESULT);
                if self.rec_get(p, promise::IS_HANDLED).as_bool() != Some(true) {
                    self.track_rejection(p, PromiseRejectionOperation::Handle);
                }
                self.enqueue(job::REACTION, rec.into(), v, Local::boolean(true));
            }
        }
        // 12. Set promise.[[PromiseIsHandled]] to true.
        self.rec_set(p, promise::IS_HANDLED, Local::boolean(true));
        // 13-14. The capability's promise, or undefined.
        capability.map_or(Local::undefined(), |c| c.promise.into())
    }

    // ---- errors ---------------------------------------------------------------------------

    /// A new Error instance (an object with `[[ErrorData]]`, §20.5.1.1) with prototype
    /// `proto`, an own `message` {w, c} when `message` is given, and an own `stack` captured
    /// now (as the native error constructors make them). For constructors such as
    /// `AggregateError` (§20.5.7.1). GC point.
    pub fn new_error_object(
        &mut self,
        proto: impl Into<Local<'s, JsObject>>,
        message: Option<Local<'s, JsString>>,
    ) -> Local<'s, JsObject> {
        let p = proto.into().bits();
        debug_assert!(p == NULL || kinds::is_object(p));
        let e = error::new_error_value(self.cx, p, message.map(|m| m.bits()));
        Self::local(e)
    }
}

// ---- the closures' behaviours -------------------------------------------------------------

/// The record of a resolving function (its closure data).
fn resolving_state<'s>(cx: &Cx<'s>, f: Local<'s, JsFunction>) -> Local<'s, JsObject> {
    cx.closure_data(f)
        .as_object()
        .expect("a resolving function has its record")
}

/// Takes `[[AlreadyResolved]]` of the record: true if it was set (step 4), else sets it
/// (step 5).
fn already_resolved<'s>(cx: &mut Cx<'s>, state: Local<'s, JsObject>) -> bool {
    if cx.rec_get(state, resolving::ALREADY_RESOLVED).as_bool() == Some(true) {
        return true;
    }
    cx.rec_set(state, resolving::ALREADY_RESOLVED, Local::boolean(true));
    false
}

/// A promise resolve function (§27.2.1.3.2).
fn resolve_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: super::Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let state = resolving_state(cx, args.callee());
    if !already_resolved(cx, state) {
        let p = cx
            .rec_get(state, resolving::PROMISE)
            .as_object()
            .expect("a promise");
        cx.resolve_promise(p, args.get(0))?;
    }
    Ok(Local::undefined())
}

/// A promise reject function (§27.2.1.3.1).
fn reject_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: super::Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let state = resolving_state(cx, args.callee());
    if !already_resolved(cx, state) {
        let p = cx
            .rec_get(state, resolving::PROMISE)
            .as_object()
            .expect("a promise");
        cx.reject_promise(p, args.get(0));
    }
    Ok(Local::undefined())
}

/// A GetCapabilitiesExecutor function (NewPromiseCapability step 2, §27.2.1.5).
fn capability_executor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: super::Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let state = cx
        .closure_data(args.callee())
        .as_object()
        .expect("an executor has its record");
    // a-b. If resolvingFunctions.[[Resolve]] or [[Reject]] is not undefined, throw a
    // TypeError exception.
    if !cx.rec_get(state, executor::RESOLVE).is_undefined()
        || !cx.rec_get(state, executor::REJECT).is_undefined()
    {
        return Err(cx.throw_type_error(
            "Promise executor has already been invoked with non-undefined arguments",
        ));
    }
    // c-d. Set them.
    cx.rec_set(state, executor::RESOLVE, args.get(0));
    cx.rec_set(state, executor::REJECT, args.get(1));
    Ok(Local::undefined())
}
