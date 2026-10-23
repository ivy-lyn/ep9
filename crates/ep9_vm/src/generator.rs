//! Generator objects (§27.5; owner P1-GEN-VM): the GENERATOR cell (kind 23, `object::genf`),
//! `%GeneratorFunction%`, `%GeneratorPrototype%` and its `next`/`return`/`throw`, and the
//! lazy `prototype` of generator functions. One cell kind serves async functions and async
//! generators too (the mode field, month 3).
//!
//! ```text
//! function* g() {}       FunctionInfo flag GENERATOR (fi_flags), [[Prototype]]
//!                        %GeneratorFunction.prototype% (object::new_function)
//! g.prototype            exotic::function_prototype -> lazy_prototype_kind(fi)
//!                          Ordinary:            {constructor: g}      (MakeConstructor)
//!                          Instances(%GeneratorPrototype%): {} with that [[Prototype]]
//! g()                    a normal call: the prologue runs CreateGeneratorObject (runtime id
//!                        19), then SuspendGenerator gives the generator to the caller
//! it.next(v)             resume: call::run_resumed -> call::push_resume_frame (fp-3 = the
//!                        generator object), dispatch from the first instruction:
//!                        SwitchOnGeneratorState jumps to the suspend point
//! ```
//!
//! The V8 model (M2-P1-GEN-NUMBERS): a `yield` builds its `{value, done: false}` in bytecode
//! (runtime id 3) and suspends with it in the accumulator, which [`resume`] returns as it is;
//! `yield*` passes the inner result object through. When the frame returns (or throws) the
//! generator is completed, and only then does [`resume`] wrap the value as
//! `{value, done: true}`.
//!
//! | State (`genf::STATE`) | `next(v)` | `return(v)` | `throw(v)` |
//! |---|---|---|---|
//! | suspended (id >= 0; also the start) | resume, mode next | resume, mode return | resume, mode throw |
//! | executing (-2) | TypeError | TypeError | TypeError |
//! | closed (-1) | `{undefined, true}` | `{v, true}` | throw `v` |
//!
//! A suspended-start generator resumed with return or throw is completed at once and runs no
//! frame (GeneratorResumeAbrupt, §27.5.3.4 step 2; M2-P1-X3-DEFER closed in M3-P1), so it
//! needs no VM stack. An async function object (mode `MODE_ASYNC_FUNCTION`) is resumed only
//! by its await closures ([`resume_async_function`], `interp/slow/async_fn.rs`).

use ep9_value::{JsResult, Throw};

use crate::abi::{Args, Cx as HostCx, Local};
use crate::builtins::IntrinsicId;
use crate::call;
use crate::conv::{self, Res};
use crate::error::throw_error;
use crate::frame::{FALSE, TRUE, UNDEFINED, int32_bits};
use crate::function_info::fi_flags;
use crate::interp::Cx;
use crate::kinds;
use crate::object::{self, attr, genf, own_define};
use crate::runtime::{Atom, ErrorKind, Runtime};
use crate::unit::FunctionInfo;

/// What the lazy `prototype` of a function is made from (`exotic::function_prototype`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LazyPrototype {
    /// §10.2.5 MakeConstructor: a new ordinary object with a `constructor` link back.
    Ordinary,
    /// A generator (or async generator) function (§15.5.4, §15.6.4): a new ordinary object
    /// whose `[[Prototype]]` is this intrinsic, with no `constructor`.
    Instances(IntrinsicId),
}

/// The lazy `prototype` the function `fi` has, if any: `exotic::is_lazy_prototype` and
/// `exotic::function_prototype` ask this. A constructor gets an ordinary one, a generator
/// function one that inherits from %GeneratorPrototype%, an async generator function one that
/// inherits from %AsyncGeneratorPrototype% (§15.6.4). Hot: inlined into every property lookup
/// on a function, so it tests flag bits only (no unit pointer chase).
#[inline(always)]
pub(crate) fn lazy_prototype_kind(fi: &FunctionInfo) -> Option<LazyPrototype> {
    if fi.flags & (fi_flags::CONSTRUCTOR | fi_flags::GENERATOR | fi_flags::ASYNC_GENERATOR) == 0 {
        return None;
    }
    Some(if fi.is_generator() {
        LazyPrototype::Instances(IntrinsicId::GeneratorPrototype)
    } else if fi.is_async_generator() {
        LazyPrototype::Instances(IntrinsicId::AsyncGeneratorPrototype)
    } else {
        LazyPrototype::Ordinary
    })
}

/// True if `g` is a GENERATOR cell.
#[inline]
pub(crate) fn is_generator(g: u64) -> bool {
    kinds::cell_kind(g) == kinds::GENERATOR.get()
}

/// Completes the generator `g`: state closed, and its saved values are dropped (nothing can
/// resume it again).
pub(crate) fn close(rt: &Runtime, g: u64) {
    object::gen_set(rt, g, genf::STATE, int32_bits(genf::STATE_CLOSED));
    object::gen_release(rt, g);
}

/// CreateIteratorResultObject(value, done) (§7.4.14). GC point: `value` lives in a Rust local.
fn iter_result(rt: &Runtime, value: u64, done: bool) -> u64 {
    let result = object::new_plain_object(rt);
    own_define(rt, result, rt.atom(Atom::ValueKey), value, attr::DEFAULT);
    let done = if done { TRUE } else { FALSE };
    own_define(rt, result, rt.atom(Atom::Done), done, attr::DEFAULT);
    result
}

/// GeneratorResume / GeneratorResumeAbrupt (§27.5.3.3, §27.5.3.4) for the receiver `g` of
/// `%GeneratorPrototype%.<method>` with resume mode `mode` (`genf::RESUME_*`) and the
/// value `v`. See the module table. A suspended generator gets `v` as its sent value and
/// `mode` as its resume mode, then its frame runs ([`resume_frame`]) until it suspends
/// again (the accumulator is the result as it is) or completes (closed; a return gives
/// `{value, done: true}`, a throw propagates). A suspended-start generator resumed with
/// return or throw is completed first and runs no frame (§27.5.3.4 step 2,
/// M2-P1-X3-DEFER). An error before the frame runs (a full stack) leaves the generator
/// suspended.
pub(crate) fn resume(cx: &mut Cx, g: u64, mode: i32, v: u64, method: &str) -> Res<u64> {
    // GeneratorValidate (§27.5.3.2): a sync GENERATOR (brand empty).
    if !is_generator(g) || object::gen_get_i32(g, genf::MODE) != genf::MODE_SYNC {
        let msg = format!(
            "Method [Generator].prototype.{method} called on incompatible receiver {}",
            conv::describe(g)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let rt = cx.rt();
    let mut state = object::gen_get_i32(g, genf::STATE);
    if state == genf::STATE_EXECUTING {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Generator is already running",
        ));
    }
    if state == SUSPENDED_START && mode != genf::RESUME_NEXT {
        // §27.5.3.4 step 2: a suspended-start generator is completed before the abrupt
        // completion is handled, so its body (and its prologue's resume point) never runs.
        close(rt, g);
        state = genf::STATE_CLOSED;
    }
    if state < 0 {
        // Completed (§27.5.3.3 step 4, §27.5.3.4 step 3).
        return match mode {
            genf::RESUME_THROW => {
                rt.throw_value(v);
                Err(Throw)
            }
            genf::RESUME_RETURN => Ok(iter_result(rt, v, true)),
            _ => Ok(iter_result(rt, UNDEFINED, true)),
        };
    }
    let r = resume_frame::<false>(cx, g, mode, v);
    let state = object::gen_get_i32(g, genf::STATE);
    match r {
        // Suspended again: the iterator result the bytecode built (or `yield*`'s inner one).
        Ok(acc) if state >= 0 => Ok(acc),
        // Returned (executing, or closed by `GeneratorClose`): §27.5.3.3 step 9 via
        // GeneratorStart step 4.g-j.
        Ok(acc) => {
            close(rt, g);
            Ok(iter_result(rt, acc, true))
        }
        Err(Throw) => {
            if state < 0 {
                // The body threw: the generator is completed.
                close(rt, g);
            }
            Err(Throw)
        }
    }
}

/// The suspend id of a sync generator before its first `next` (the initial suspend,
/// `bytecodegen/generator.rs`): GeneratorState suspended-start (§27.5).
const SUSPENDED_START: i32 = 0;

/// Runs the frame of the suspended generator or async function object `g` (state >= 0) with
/// resume mode `mode` and the sent value `v`: the frame is pushed with the saved parameters,
/// `this` and context (`call::run_resumed`) and runs until it suspends again or leaves.
/// Returns the accumulator of that `SuspendGenerator` or `Return`. An error before the frame
/// runs (a full stack, an interrupt) leaves `g` suspended, with no input. GC point; runs JS.
///
/// `ASYNC` only makes one copy per caller: each copy has its own closure type, so each gets
/// its own `call::run_resumed`, which LLVM inlines into the sync `resume` as before (t0-gen;
/// one shared copy was not inlined and cost about 0.4% there).
#[inline(always)]
fn resume_frame<const ASYNC: bool>(cx: &mut Cx, g: u64, mode: i32, v: u64) -> Res<u64> {
    let rt = cx.rt();
    let f = object::gen_get(g, genf::FUNCTION);
    let regs = object::gen_regs(g).filter(|r| {
        kinds::is_function(f) && (object::function_info(f).parameter_count as usize) < r.len()
    });
    let Some(regs) = regs else {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: a suspended generator without its saved values",
        ));
    };
    regs.set(rt, genf::REGS_INPUT, v);
    object::gen_set(rt, g, genf::RESUME_MODE, int32_bits(mode));
    let this = object::gen_get(g, genf::RECEIVER);
    let context = object::gen_get(g, genf::CONTEXT);
    let params = object::function_info(f).parameter_count;
    // GC points in `run_resumed` before the frame push (the interrupt poll): `g` and the
    // values it holds live in Rust locals and in the native's frame; `regs` is a view of a
    // cell that `g` holds, and cells do not move.
    let r = call::run_resumed(
        cx,
        f,
        this,
        params,
        |i| regs.get(1 + i as usize),
        context,
        g,
    );
    if r.is_err()
        && object::gen_get_i32(g, genf::STATE) >= 0
        && let Some(regs) = object::gen_regs(g)
    {
        // The frame never ran: still suspended; drop the input.
        regs.set(rt, genf::REGS_INPUT, UNDEFINED);
    }
    r
}

/// Resumes the suspended async function object `g` (`genf::MODE_ASYNC_FUNCTION`) from an
/// await closure (`interp/slow/async_fn.rs`): mode next with the fulfilled value, or throw
/// with the rejection reason (§27.7.5.3 Await steps 3.c-f, 5.c-f). The frame runs until its
/// next `await` or its end, where `AsyncFunctionResolve` or `AsyncFunctionReject` completed
/// `g`. An `Err` means that the frame never ran or that a throw left it (the caller rejects).
pub(crate) fn resume_async_function(cx: &mut Cx, g: u64, mode: i32, v: u64) -> Res<()> {
    debug_assert!(
        is_generator(g) && object::gen_get_i32(g, genf::MODE) == genf::MODE_ASYNC_FUNCTION
    );
    if object::gen_get_i32(g, genf::STATE) < 0 {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: an async function resumed while it is not suspended",
        ));
    }
    resume_frame::<true>(cx, g, mode, v)?;
    if object::gen_get_i32(g, genf::STATE) == genf::STATE_EXECUTING {
        // The frame left without settling the promise (a BytecodeGen bug).
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: an async function returned without settling its promise",
        ));
    }
    Ok(())
}

/// The host side of `next`/`return`/`throw`.
fn resume_native<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    mode: i32,
    method: &str,
) -> JsResult<Local<'s>> {
    let v = args.get(0).bits();
    resume(cx.inner(), this.bits(), mode, v, method).map(Local::from_bits)
}

/// `GeneratorFunction(...params, body)` (§27.3.1.1): CreateDynamicFunction with kind
/// generator, through the compiler hook like `Function` (`abi/script.rs`, P3-EVAL-VM).
pub(crate) fn generator_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.create_dynamic_function(args, nt, ep9_bytecode::FunctionCtorKind::Generator)
}

/// `%GeneratorPrototype%.next(value)` (§27.5.1.2): GeneratorResume(this, value).
pub(crate) fn generator_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume_native(cx, this, args, genf::RESUME_NEXT, "next")
}

/// `%GeneratorPrototype%.return(value)` (§27.5.1.3): GeneratorResumeAbrupt(this, return
/// completion of value).
pub(crate) fn generator_return<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume_native(cx, this, args, genf::RESUME_RETURN, "return")
}

/// `%GeneratorPrototype%.throw(exception)` (§27.5.1.4): GeneratorResumeAbrupt(this, throw
/// completion of exception).
pub(crate) fn generator_throw<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume_native(cx, this, args, genf::RESUME_THROW, "throw")
}

/// `AsyncFunction(...params, body)` (§27.7.1.1): CreateDynamicFunction with kind async,
/// through the compiler hook like `Function` and `GeneratorFunction` (`abi/script.rs`).
pub(crate) fn async_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.create_dynamic_function(args, nt, ep9_bytecode::FunctionCtorKind::Async)
}

// ---- async generators (§27.6; M3-P2 ASYNCGEN) ----------------------------------------------

/// The async generator record: an ELEMENTS list (plan rule 8: a heap cell, never a Rust
/// buffer) in `genf::ASYNC` of a GENERATOR in mode `genf::MODE_ASYNC_GENERATOR`. It holds
/// `[[AsyncGeneratorState]]` and `[[AsyncGeneratorQueue]]` as a singly linked list of request
/// cells (each an ELEMENTS list of [`agf::REQUEST_LEN`]).
///
/// ```text
/// record:  [STATE (int32), HEAD (request or undefined), TAIL (request or undefined)]
/// request: [MODE (int32 genf::RESUME_*), VALUE, PROMISE (its capability's promise), NEXT]
/// ```
///
/// The record's state is the spec state. `genf::STATE` stays the frame's resume point: an
/// async generator that waits in an `await` is spec state executing with a suspend id in
/// `genf::STATE`.
pub(crate) mod agf {
    /// `[[AsyncGeneratorState]]` (int32: one of the `STATE_*` values).
    pub const STATE: usize = 0;
    /// The first request of the queue, or undefined.
    pub const HEAD: usize = 1;
    /// The last request of the queue, or undefined.
    pub const TAIL: usize = 2;
    pub const RECORD_LEN: usize = 3;
    /// suspended-start (§27.6.2: the body has not run).
    pub const STATE_SUSPENDED_START: i32 = 0;
    /// suspended-yield: the frame waits at a `yield`.
    pub const STATE_SUSPENDED_YIELD: i32 = 1;
    /// executing: the frame runs, or waits in an `await`.
    pub const STATE_EXECUTING: i32 = 2;
    /// draining-queue (ES2024; "awaiting-return" before): the body completed and the queue
    /// is being settled.
    pub const STATE_DRAINING: i32 = 3;
    /// completed.
    pub const STATE_COMPLETED: i32 = 4;
    /// A request's completion type (int32 `genf::RESUME_NEXT`, `RESUME_RETURN`, `RESUME_THROW`).
    pub const MODE: usize = 0;
    /// A request's completion value.
    pub const VALUE: usize = 1;
    /// A request's promise (NewPromiseCapability(%Promise%): the VM settles the promise
    /// itself, so the capability needs no resolving functions).
    pub const PROMISE: usize = 2;
    /// The next request, or undefined.
    pub const NEXT: usize = 3;
    pub const REQUEST_LEN: usize = 4;
}

/// `genf::MODE` of an async-from-sync iterator object (§27.1.6): a GENERATOR cell that runs no
/// frame. It holds the sync Iterator Record: the iterator in `genf::RECEIVER` and its `next`
/// method in `genf::FUNCTION`. JS code never sees one (§27.1.6.1 NOTE); its prototype is
/// %AsyncFromSyncIteratorPrototype%.
pub(crate) const MODE_ASYNC_FROM_SYNC: i32 = 3;

/// True if `g` is an async generator object.
#[inline]
pub(crate) fn is_async_generator(g: u64) -> bool {
    is_generator(g) && object::gen_get_i32(g, genf::MODE) == genf::MODE_ASYNC_GENERATOR
}

/// A new async generator record in state suspended-start, with an empty queue. GC point.
pub(crate) fn new_async_generator_record(rt: &Runtime) -> u64 {
    let r = object::new_list(rt, agf::RECORD_LEN, UNDEFINED);
    object::list_set(rt, r, agf::STATE, int32_bits(agf::STATE_SUSPENDED_START));
    r
}

/// The record of the async generator `g`.
fn ag_record(g: u64) -> u64 {
    debug_assert!(is_async_generator(g));
    object::gen_get(g, genf::ASYNC)
}

/// `[[AsyncGeneratorState]]` of `g`.
pub(crate) fn ag_state(g: u64) -> i32 {
    crate::frame::int32_of(object::list_get(ag_record(g), agf::STATE))
}

/// Sets `[[AsyncGeneratorState]]` of `g`.
pub(crate) fn set_ag_state(rt: &Runtime, g: u64, state: i32) {
    object::list_set(rt, ag_record(g), agf::STATE, int32_bits(state));
}

/// The first request of the queue of `g`, if any.
fn ag_head(g: u64) -> Option<u64> {
    let h = object::list_get(ag_record(g), agf::HEAD);
    (h != UNDEFINED).then_some(h)
}

/// The completion of the first request of `g`: (mode, value).
fn ag_head_completion(g: u64) -> Option<(i32, u64)> {
    ag_head(g).map(|h| {
        (
            crate::frame::int32_of(object::list_get(h, agf::MODE)),
            object::list_get(h, agf::VALUE),
        )
    })
}

/// AsyncGeneratorEnqueue (§27.6.3.3): appends a request (`mode`, `value`, `promise`) to the
/// queue of `g`. GC point: the values live in Rust locals.
pub(crate) fn ag_enqueue(rt: &Runtime, g: u64, mode: i32, value: u64, promise: u64) {
    let req = object::new_list(rt, agf::REQUEST_LEN, UNDEFINED);
    object::list_set(rt, req, agf::MODE, int32_bits(mode));
    object::list_set(rt, req, agf::VALUE, value);
    object::list_set(rt, req, agf::PROMISE, promise);
    let record = ag_record(g);
    let tail = object::list_get(record, agf::TAIL);
    if tail == UNDEFINED {
        object::list_set(rt, record, agf::HEAD, req);
    } else {
        object::list_set(rt, tail, agf::NEXT, req);
    }
    object::list_set(rt, record, agf::TAIL, req);
}

/// Removes the first request of `g` and returns its promise. Not a GC point.
pub(crate) fn ag_dequeue(rt: &Runtime, g: u64) -> Option<u64> {
    let record = ag_record(g);
    let head = ag_head(g)?;
    let next = object::list_get(head, agf::NEXT);
    object::list_set(rt, record, agf::HEAD, next);
    if next == UNDEFINED {
        object::list_set(rt, record, agf::TAIL, UNDEFINED);
    }
    Some(object::list_get(head, agf::PROMISE))
}

/// %Promise% of the current realm, or not implemented (an embedder without ep9_builtins).
fn promise_constructor(cx: &mut Cx, what: &str) -> Res<u64> {
    let c = crate::builtins::intrinsic(cx.rt(), IntrinsicId::Promise);
    if kinds::is_object(c) {
        Ok(c)
    } else {
        Err(crate::error::not_implemented(
            cx,
            &format!("{what} without %Promise%"),
        ))
    }
}

/// A new pending promise of %Promise% (NewPromiseCapability(%Promise%): only the VM settles
/// it). GC point.
fn new_pending_promise(cx: &mut Cx, what: &str) -> Res<u64> {
    promise_constructor(cx, what)?;
    let proto = crate::builtins::intrinsic(cx.rt(), IntrinsicId::PromisePrototype);
    Ok(HostCx::new(cx)
        .new_promise(Local::<crate::abi::JsValue>::from_bits(proto))
        .bits())
}

/// Rejects the pending promise `p` with `reason` (a promise only the VM settles).
fn reject_pending(cx: &mut Cx, p: u64, reason: u64) {
    let mut acx = HostCx::new(cx);
    let p = Local::<crate::abi::JsObject>::from_bits(p);
    if acx.promise_state(p).ok() == Some(crate::abi::PromiseState::Pending) {
        acx.reject_promise(p, Local::<crate::abi::JsValue>::from_bits(reason));
    }
}

/// Resolves the pending promise `p` with `value` (the resolve function's steps: a thenable
/// takes a job, a throwing `then` getter rejects). GC point; may run JS (a `then` getter).
fn resolve_pending(cx: &mut Cx, p: u64, value: u64) -> Res<()> {
    let mut acx = HostCx::new(cx);
    let p = Local::<crate::abi::JsObject>::from_bits(p);
    if acx.promise_state(p).ok() == Some(crate::abi::PromiseState::Pending) {
        acx.resolve_promise_once(p, Local::<crate::abi::JsValue>::from_bits(value))?;
    }
    Ok(())
}

/// AsyncGeneratorCompleteStep(generator, completion, done) (§27.6.3.4): removes the first
/// request and settles its promise: a throw completion (`throw`) rejects it with `value`, a
/// normal one resolves it with CreateIterResultObject(value, done) of the current realm (the
/// spec makes it in the realm of the caller of the generator's context; ep9 makes it in the
/// current one). GC point; may run JS (a `then` getter on %Object.prototype%).
fn ag_complete_step(cx: &mut Cx, g: u64, throw: bool, value: u64, done: bool) -> Res<()> {
    let rt = cx.rt();
    let Some(p) = ag_dequeue(rt, g) else {
        return Ok(());
    };
    if throw {
        reject_pending(cx, p, value);
        Ok(())
    } else {
        let r = iter_result(rt, value, done);
        resolve_pending(cx, p, r)
    }
}

/// AsyncGeneratorDrainQueue (§27.6.3.10): settles the queued requests of the completed `g`
/// (state draining-queue) in order: a return request awaits its value
/// ([`ag_await_return`], which drains the rest later), a throw request rejects, a next
/// request gets `{undefined, true}`. Then the state is completed. GC point; may run JS.
fn ag_drain_queue(cx: &mut Cx, g: u64) -> Res<()> {
    debug_assert_eq!(ag_state(g), agf::STATE_DRAINING);
    loop {
        let Some((mode, value)) = ag_head_completion(g) else {
            set_ag_state(cx.rt(), g, agf::STATE_COMPLETED);
            return Ok(());
        };
        match mode {
            genf::RESUME_RETURN => {
                if ag_await_return(cx, g)? {
                    return Ok(());
                }
            }
            genf::RESUME_THROW => ag_complete_step(cx, g, true, value, true)?,
            _ => ag_complete_step(cx, g, false, UNDEFINED, true)?,
        }
    }
}

/// AsyncGeneratorAwaitReturn (§27.6.3.9) for the first request of `g` (a return request):
/// PromiseResolve(%Promise%, value), then the closures that complete the request with the
/// settled value. Returns true when it subscribed (the closures drain the rest), false
/// when PromiseResolve threw and the request was rejected now (the caller drains on). GC
/// point; may run JS (a `constructor` getter of a promise).
fn ag_await_return(cx: &mut Cx, g: u64) -> Res<bool> {
    let Some((_, value)) = ag_head_completion(g) else {
        return Ok(false);
    };
    let c = promise_constructor(cx, "async generators")?;
    let mut acx = HostCx::new(cx);
    // 5. Let promiseCompletion be Completion(PromiseResolve(%Promise%, completion.[[Value]])).
    let promise = match acx.promise_resolve(
        Local::<crate::abi::JsObject>::from_bits(c),
        Local::<crate::abi::JsValue>::from_bits(value),
    ) {
        Ok(p) => p,
        Err(t) => {
            // 6. An abrupt completion: complete the step with it.
            let e = acx.catch_exception(t)?.bits();
            ag_complete_step(cx, g, true, e, true)?;
            return Ok(false);
        }
    };
    let Some(promise) = promise.as_object() else {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: PromiseResolve gave a non-object",
        ));
    };
    // 9-13. The closures (length 1, name "") complete the step and drain the queue.
    let data = Local::<crate::abi::JsValue>::from_bits(g);
    let on_fulfilled = acx.new_builtin_closure(await_return_fulfilled, "", 1, data);
    let on_rejected = acx.new_builtin_closure(await_return_rejected, "", 1, data);
    acx.perform_promise_then(promise, on_fulfilled, on_rejected, None);
    Ok(true)
}

/// AsyncGeneratorAwaitReturn step 9 (fulfilledClosure).
fn await_return_fulfilled<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    await_return_settled(cx, args, false)
}

/// AsyncGeneratorAwaitReturn step 11 (rejectedClosure).
fn await_return_rejected<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    await_return_settled(cx, args, true)
}

/// The two closures of [`ag_await_return`]: complete the first request with the settled
/// value (`throw` for a rejection), then drain the queue. Returns undefined.
fn await_return_settled<'s>(
    cx: &mut HostCx<'s>,
    args: Args<'s>,
    throw: bool,
) -> JsResult<Local<'s>> {
    let g = cx.closure_data(args.callee()).bits();
    let v = args.get(0).bits();
    if !is_async_generator(g) || ag_state(g) != agf::STATE_DRAINING {
        return Err(throw_error(
            cx.inner(),
            ErrorKind::Error,
            "ep9 internal error: an await-return closure without its draining async generator",
        ));
    }
    ag_complete_step(cx.inner(), g, throw, v, true)?;
    ag_drain_queue(cx.inner(), g)?;
    Ok(Local::undefined())
}

/// The body of `g` completed (AsyncGeneratorStart step 4, §27.6.3.2): the frame is dropped,
/// the state is draining-queue, the first request completes with the body's result (`throw`:
/// a throw completion of `value`; else `{value, done: true}`), then the queue drains. Runtime
/// ids 39 (done) and 40 call it from the frame's last instructions; a frame that could not
/// run calls it too. GC point; may run JS.
pub(crate) fn async_generator_complete(cx: &mut Cx, g: u64, throw: bool, value: u64) -> Res<()> {
    let rt = cx.rt();
    close(rt, g);
    set_ag_state(rt, g, agf::STATE_DRAINING);
    ag_complete_step(cx, g, throw, value, true)?;
    ag_drain_queue(cx, g)
}

/// AsyncGeneratorYield (§27.6.3.8) steps 5-12.a for `value`, run by runtime id 38 inside the
/// frame: the first request completes with `{value, done: false}`, then the state is
/// suspended-yield and the frame suspends. When the queue is not empty, the runner resumes
/// the frame at once with the next request ([`async_generator_run`]): nothing runs between
/// the suspend and that resume, so this is the spec's "execution continues without
/// suspending" (step 11). GC point; may run JS.
pub(crate) fn async_generator_yield(cx: &mut Cx, g: u64, value: u64) -> Res<()> {
    ag_complete_step(cx, g, false, value, false)?;
    set_ag_state(cx.rt(), g, agf::STATE_SUSPENDED_YIELD);
    Ok(())
}

/// A throw left the frame of `g` (the frame could not be pushed: a full stack, an interrupt;
/// or a BytecodeGen bug): the body completes with that throw (the queue settles). A
/// termination passes on.
fn ag_frame_failed(cx: &mut Cx, g: u64, t: Throw) -> Res<()> {
    let e = HostCx::new(cx).catch_exception(t)?.bits();
    async_generator_complete(cx, g, true, e)
}

/// AsyncGeneratorResume (§27.6.3.6) and the await resumption of `g`: runs its frame with
/// resume mode `mode` and the value `v` until it waits in an `await`, suspends at a `yield`
/// or completes. A `yield` with requests queued resumes the frame again with the next one
/// (AsyncGeneratorYield step 11), in this loop: one Rust level per resume, never one per
/// request. Throws only a termination (any other throw settles the queue). GC point; runs JS.
pub(crate) fn async_generator_run(cx: &mut Cx, g: u64, mode: i32, v: u64) -> Res<()> {
    let (mut mode, mut v) = (mode, v);
    loop {
        if object::gen_get_i32(g, genf::STATE) < 0 {
            return Err(throw_error(
                cx,
                ErrorKind::Error,
                "ep9 internal error: an async generator resumed while it is not suspended",
            ));
        }
        set_ag_state(cx.rt(), g, agf::STATE_EXECUTING);
        if let Err(t) = resume_frame::<true>(cx, g, mode, v) {
            return ag_frame_failed(cx, g, t);
        }
        if ag_state(g) != agf::STATE_SUSPENDED_YIELD {
            // Waits in an `await` (executing), or completed (draining or completed).
            return Ok(());
        }
        match ag_head_completion(g) {
            Some((m, x)) => (mode, v) = (m, x),
            None => return Ok(()),
        }
    }
}

/// The steps that `%AsyncGeneratorPrototype%.next`, `return` and `throw` share (§27.6.1.2-4):
/// a new promise; AsyncGeneratorValidate (a wrong receiver rejects it); then the request.
fn async_generator_request(cx: &mut Cx, g: u64, mode: i32, v: u64, method: &str) -> Res<u64> {
    // 2. Let promiseCapability be ! NewPromiseCapability(%Promise%).
    let p = new_pending_promise(cx, "async generators")?;
    // 3. AsyncGeneratorValidate (§27.6.3.2); IfAbruptRejectPromise.
    if !is_async_generator(g) {
        let msg = format!(
            "Method [AsyncGenerator].prototype.{method} called on incompatible receiver {}",
            conv::describe(g)
        );
        let e = crate::error::new_error(cx, ErrorKind::TypeError, &msg);
        reject_pending(cx, p, e);
        return Ok(p);
    }
    let rt = cx.rt();
    let mut state = ag_state(g);
    match mode {
        genf::RESUME_THROW if state == agf::STATE_SUSPENDED_START => {
            // throw step 5: a suspended-start generator is completed first.
            close(rt, g);
            set_ag_state(rt, g, agf::STATE_COMPLETED);
            state = agf::STATE_COMPLETED;
        }
        _ => {}
    }
    if state == agf::STATE_COMPLETED {
        match mode {
            // next step 5: `{undefined, true}`.
            genf::RESUME_NEXT => {
                let r = iter_result(rt, UNDEFINED, true);
                resolve_pending(cx, p, r)?;
                return Ok(p);
            }
            // throw step 6: reject with the value.
            genf::RESUME_THROW => {
                reject_pending(cx, p, v);
                return Ok(p);
            }
            // return: enqueue, then await the value (step 8).
            _ => {}
        }
    }
    // AsyncGeneratorEnqueue.
    ag_enqueue(cx.rt(), g, mode, v, p);
    match (mode, state) {
        (genf::RESUME_NEXT, agf::STATE_SUSPENDED_START | agf::STATE_SUSPENDED_YIELD)
        | (genf::RESUME_RETURN | genf::RESUME_THROW, agf::STATE_SUSPENDED_YIELD) => {
            async_generator_run(cx, g, mode, v)?;
        }
        (genf::RESUME_RETURN, agf::STATE_SUSPENDED_START | agf::STATE_COMPLETED) => {
            // return step 8: draining-queue, then AsyncGeneratorAwaitReturn.
            if state == agf::STATE_SUSPENDED_START {
                close(cx.rt(), g);
            }
            set_ag_state(cx.rt(), g, agf::STATE_DRAINING);
            if !ag_await_return(cx, g)? {
                ag_drain_queue(cx, g)?;
            }
        }
        // Executing or draining-queue: the request waits in the queue.
        _ => {}
    }
    Ok(p)
}

/// The host side of the async generator `next`/`return`/`throw`.
fn async_generator_method<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    mode: i32,
    method: &str,
) -> JsResult<Local<'s>> {
    let v = args.get(0).bits();
    async_generator_request(cx.inner(), this.bits(), mode, v, method).map(Local::from_bits)
}

/// `%AsyncGeneratorPrototype%.next(value)` (§27.6.1.2).
pub(crate) fn async_generator_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_generator_method(cx, this, args, genf::RESUME_NEXT, "next")
}

/// `%AsyncGeneratorPrototype%.return(value)` (§27.6.1.3).
pub(crate) fn async_generator_return<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_generator_method(cx, this, args, genf::RESUME_RETURN, "return")
}

/// `%AsyncGeneratorPrototype%.throw(exception)` (§27.6.1.4).
pub(crate) fn async_generator_throw<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_generator_method(cx, this, args, genf::RESUME_THROW, "throw")
}

/// `AsyncGeneratorFunction(...params, body)` (§27.4.1.1): CreateDynamicFunction with kind
/// async generator, through the compiler hook like `Function` (`abi/script.rs`).
pub(crate) fn async_generator_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.create_dynamic_function(args, nt, ep9_bytecode::FunctionCtorKind::AsyncGenerator)
}

/// `%AsyncIteratorPrototype%[@@asyncIterator]()` (§27.1.3.1): returns the `this` value.
pub(crate) fn async_iterator_iterator<'s>(
    _cx: &mut HostCx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

// ---- async-from-sync iterators (§27.1.6) ----------------------------------------------------

/// CreateAsyncFromSyncIterator (§27.1.6.1) of the sync Iterator Record (`iterator`, `next`):
/// a new async-from-sync iterator object ([`MODE_ASYNC_FROM_SYNC`]). GC point.
pub(crate) fn new_async_from_sync_iterator(rt: &Runtime, iterator: u64, next: u64) -> u64 {
    let proto = crate::builtins::intrinsic(rt, IntrinsicId::AsyncFromSyncIteratorPrototype);
    let proto = if kinds::is_object(proto) {
        proto
    } else {
        crate::frame::NULL
    };
    let o = object::new_generator(
        rt,
        proto,
        next,
        iterator,
        UNDEFINED,
        MODE_ASYNC_FROM_SYNC,
        1,
    );
    // It runs no frame: no saved values, never resumed.
    close(rt, o);
    o
}

/// The sync iterator and its `next` method of the async-from-sync iterator `o`, if it is one.
fn sync_iterator_record(o: u64) -> Option<(u64, u64)> {
    (is_generator(o) && object::gen_get_i32(o, genf::MODE) == MODE_ASYNC_FROM_SYNC).then(|| {
        (
            object::gen_get(o, genf::RECEIVER),
            object::gen_get(o, genf::FUNCTION),
        )
    })
}

/// GetMethod(v, key) (§7.3.10): undefined for undefined or null, else the callable value;
/// TypeError for any other value. GC point; may run JS (a getter).
fn get_method(cx: &mut Cx, v: u64, key: u64) -> Res<u64> {
    let m = crate::property::get(cx, v, key)?;
    if m == UNDEFINED || m == crate::frame::NULL {
        return Ok(UNDEFINED);
    }
    if !kinds::is_callable(m) {
        let msg = format!("{} is not a function", conv::describe(m));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    Ok(m)
}

/// IteratorClose(iteratorRecord, completion) (§7.4.11) of the sync iterator `iter`, for a
/// normal completion (`throw` false: errors of `return` and a non-object result throw) or a
/// throw completion (`throw` true: they are ignored; the caller keeps its own throw).
fn sync_iterator_close(cx: &mut Cx, iter: u64, throw: bool) -> Res<()> {
    let r = (|| {
        let key = crate::atoms::intern_str(cx.rt(), "return");
        let m = get_method(cx, iter, key)?;
        if m == UNDEFINED {
            return Ok(None);
        }
        call::call_function(cx, m, iter, &[]).map(Some)
    })();
    if throw {
        if let Err(t) = r {
            // A termination passes on; any other throw is dropped.
            HostCx::new(cx).catch_exception(t)?;
        }
        return Ok(());
    }
    match r? {
        Some(res) if !kinds::is_object(res) => {
            let msg = format!("Iterator result {} is not an object", conv::describe(res));
            Err(throw_error(cx, ErrorKind::TypeError, &msg))
        }
        _ => Ok(()),
    }
}

/// The promise `p` rejected with the pending throw `t` (IfAbruptRejectPromise); a
/// termination passes on.
fn reject_with_throw(cx: &mut Cx, p: u64, t: Throw) -> Res<u64> {
    let e = HostCx::new(cx).catch_exception(t)?.bits();
    reject_pending(cx, p, e);
    Ok(p)
}

/// AsyncFromSyncIteratorContinuation(result, promiseCapability, syncIteratorRecord,
/// closeOnRejection) (§27.1.6.4). `result` is an object. GC point; may run JS.
fn async_from_sync_continuation(
    cx: &mut Cx,
    result: u64,
    p: u64,
    resolve: u64,
    reject: u64,
    iter: u64,
    close_on_rejection: bool,
) -> Res<u64> {
    let rt = cx.rt();
    // 2-3. done = Completion(IteratorComplete(result)); IfAbruptRejectPromise.
    let done = match crate::property::get(cx, result, rt.atom(Atom::Done)) {
        Ok(d) => conv::to_boolean(d),
        Err(t) => return reject_with_throw(cx, p, t),
    };
    // 4-5. value = Completion(IteratorValue(result)); IfAbruptRejectPromise.
    let value = match crate::property::get(cx, result, rt.atom(Atom::ValueKey)) {
        Ok(v) => v,
        Err(t) => return reject_with_throw(cx, p, t),
    };
    // 6. valueWrapper = Completion(PromiseResolve(%Promise%, value)).
    let c = promise_constructor(cx, "async-from-sync iterators")?;
    let wrapper = HostCx::new(cx).promise_resolve(
        Local::<crate::abi::JsObject>::from_bits(c),
        Local::<crate::abi::JsValue>::from_bits(value),
    );
    let wrapper = match wrapper {
        Ok(w) => w.bits(),
        Err(t) => {
            // 7. Not done and closeOnRejection: IteratorClose(syncIteratorRecord,
            // valueWrapper) keeps the throw. 8. IfAbruptRejectPromise.
            let e = HostCx::new(cx).catch_exception(t)?.bits();
            if !done && close_on_rejection {
                sync_iterator_close(cx, iter, true)?;
            }
            reject_pending(cx, p, e);
            return Ok(p);
        }
    };
    let mut acx = HostCx::new(cx);
    // 9-10. onFulfilled: CreateIterResultObject(v, done) (length 1, name "").
    let on_fulfilled = acx.new_builtin_closure(async_from_sync_unwrap, "", 1, Local::boolean(done));
    // 12-13. onRejected: undefined, or the closure that closes the sync iterator.
    let on_rejected: Local<'_> = if done || !close_on_rejection {
        Local::undefined()
    } else {
        acx.new_builtin_closure(
            async_from_sync_close,
            "",
            1,
            Local::<crate::abi::JsValue>::from_bits(iter),
        )
        .into()
    };
    // 14. PerformPromiseThen(valueWrapper, onFulfilled, onRejected, promiseCapability).
    let Some(wrapper) = Local::<crate::abi::JsValue>::from_bits(wrapper).as_object() else {
        return Err(throw_error(
            acx.inner(),
            ErrorKind::Error,
            "ep9 internal error: PromiseResolve gave a non-object",
        ));
    };
    let cap = crate::abi::PromiseCapability {
        promise: Local::from_bits(p),
        resolve: Local::from_bits(resolve),
        reject: Local::from_bits(reject),
    };
    acx.perform_promise_then(wrapper, on_fulfilled, on_rejected, Some(cap));
    Ok(p)
}

/// AsyncFromSyncIteratorContinuation step 9 (unwrap): `{v, done}`.
fn async_from_sync_unwrap<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let done = cx.closure_data(args.callee()).as_bool() == Some(true);
    let v = args.get(0).bits();
    let rt = cx.inner().rt();
    Ok(Local::from_bits(iter_result(rt, v, done)))
}

/// AsyncFromSyncIteratorContinuation step 13.a (closeIterator): IteratorClose of the sync
/// iterator with a throw completion of `error`, which it then throws.
fn async_from_sync_close<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let iter = cx.closure_data(args.callee()).bits();
    let error = args.get(0);
    sync_iterator_close(cx.inner(), iter, true)?;
    Err(cx.throw(error))
}

/// The steps that `%AsyncFromSyncIteratorPrototype%.next`, `return` and `throw` share
/// (§27.1.6.2.1-3).
fn async_from_sync_method<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    mode: i32,
) -> JsResult<Local<'s>> {
    let present = !args.is_empty();
    let value = args.get(0).bits();
    let o = this.bits();
    let cx = cx.inner();
    // 2. Let promiseCapability be ! NewPromiseCapability(%Promise%).
    let c = promise_constructor(cx, "async-from-sync iterators")?;
    let cap = HostCx::new(cx).new_promise_capability(Local::<crate::abi::JsValue>::from_bits(c))?;
    let (p, resolve, reject) = (cap.promise.bits(), cap.resolve.bits(), cap.reject.bits());
    // 1. Assert: O has a [[SyncIteratorRecord]] internal slot (JS never sees O).
    let Some((iter, next)) = sync_iterator_record(o) else {
        let msg = format!(
            "Method [AsyncFromSyncIterator].prototype method called on incompatible receiver {}",
            conv::describe(o)
        );
        let e = crate::error::new_error(cx, ErrorKind::TypeError, &msg);
        reject_pending(cx, p, e);
        return Ok(Local::from_bits(p));
    };
    let args1 = [value];
    let call_args: &[u64] = if present { &args1 } else { &[] };
    let result = match mode {
        genf::RESUME_NEXT => {
            // 4-5. IteratorNext(syncIteratorRecord, value); IfAbruptRejectPromise.
            match call::call_function(cx, next, iter, call_args) {
                Ok(r) if kinds::is_object(r) => r,
                Ok(r) => {
                    let msg = format!("Iterator result {} is not an object", conv::describe(r));
                    let e = crate::error::new_error(cx, ErrorKind::TypeError, &msg);
                    reject_pending(cx, p, e);
                    return Ok(Local::from_bits(p));
                }
                Err(t) => return reject_with_throw(cx, p, t).map(Local::from_bits),
            }
        }
        _ => {
            let name = if mode == genf::RESUME_RETURN {
                "return"
            } else {
                "throw"
            };
            // 5-6. GetMethod(syncIterator, name); IfAbruptRejectPromise.
            let key = crate::atoms::intern_str(cx.rt(), name);
            let m = match get_method(cx, iter, key) {
                Ok(m) => m,
                Err(t) => return reject_with_throw(cx, p, t).map(Local::from_bits),
            };
            if m == UNDEFINED {
                if mode == genf::RESUME_RETURN {
                    // return step 7: `{value, true}`.
                    let r = iter_result(cx.rt(), value, true);
                    resolve_pending(cx, p, r)?;
                } else {
                    // throw step 7: close the sync iterator (a normal completion), then a
                    // TypeError for the protocol violation.
                    if let Err(t) = sync_iterator_close(cx, iter, false) {
                        return reject_with_throw(cx, p, t).map(Local::from_bits);
                    }
                    let e = crate::error::new_error(
                        cx,
                        ErrorKind::TypeError,
                        "The iterator does not provide a 'throw' method",
                    );
                    reject_pending(cx, p, e);
                }
                return Ok(Local::from_bits(p));
            }
            // 8-10. Call(method, syncIterator, « value »); IfAbruptRejectPromise; an object.
            match call::call_function(cx, m, iter, call_args) {
                Ok(r) if kinds::is_object(r) => r,
                Ok(r) => {
                    let msg = format!("Iterator result {} is not an object", conv::describe(r));
                    let e = crate::error::new_error(cx, ErrorKind::TypeError, &msg);
                    reject_pending(cx, p, e);
                    return Ok(Local::from_bits(p));
                }
                Err(t) => return reject_with_throw(cx, p, t).map(Local::from_bits),
            }
        }
    };
    // 11. AsyncFromSyncIteratorContinuation(result, promiseCapability, syncIteratorRecord,
    // closeOnRejection: true for next and throw, false for return).
    let close = mode != genf::RESUME_RETURN;
    async_from_sync_continuation(cx, result, p, resolve, reject, iter, close).map(Local::from_bits)
}

/// `%AsyncFromSyncIteratorPrototype%.next(value)` (§27.1.6.2.1).
pub(crate) fn async_from_sync_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_from_sync_method(cx, this, args, genf::RESUME_NEXT)
}

/// `%AsyncFromSyncIteratorPrototype%.return(value)` (§27.1.6.2.2).
pub(crate) fn async_from_sync_return<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_from_sync_method(cx, this, args, genf::RESUME_RETURN)
}

/// `%AsyncFromSyncIteratorPrototype%.throw(value)` (§27.1.6.2.3).
pub(crate) fn async_from_sync_throw<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    async_from_sync_method(cx, this, args, genf::RESUME_THROW)
}
