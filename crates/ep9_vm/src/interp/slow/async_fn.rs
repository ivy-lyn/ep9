//! Async functions (§27.7; owner M3-P1 ASYNC): the runtime ids 32-35 of ABI 5 and the
//! resume closures of `await`. `runtime.rs` routes the ids here (since P0-VM3 step 2). The
//! lowering is in the front end (`bytecodegen/async_fn.rs`).
//!
//! | Id | Name | Args | Result |
//! |---|---|---|---|
//! | 32 | `AsyncFunctionEnter` | closure, receiver | the async function object (a GENERATOR in mode `MODE_ASYNC_FUNCTION`, its promise in `genf::ASYNC`) |
//! | 33 | `AsyncFunctionAwait` | async object, value | undefined; it subscribes the resume |
//! | 34 | `AsyncFunctionResolve` | async object, value | the promise |
//! | 35 | `AsyncFunctionReject` | async object, reason | the promise |
//!
//! ```text
//! f(args)          the frame runs: Enter makes the object and a pending promise
//! await v          Await: promise = PromiseResolve(%Promise%, v); PerformPromiseThen(promise,
//!                  onFulfilled, onRejected) with two builtin closures whose data is the object
//!                  (no extra job for a native promise: the ES2019 await rule); then
//!                  SuspendGenerator pops the frame and gives the caller the promise
//! a reaction job   onFulfilled(x) / onRejected(e): generator::resume_async_function, mode
//!                  next / throw: one resumed frame, one Rust level per job (never one per
//!                  `await`), until the next `await` or the end
//! return v         Resolve: the object is closed, then the promise's resolve steps (a
//!                  thenable takes a job, §27.2.1.3.2)
//! throw e          the body's Promise handler: Reject
//! ```
//!
//! The object, its saved registers and its promise are heap cells (plan rule 8); the closures
//! hold the object in their context word (`Cx::new_builtin_closure`).

use ep9_value::{JsResult, Throw};

use crate::abi::{self, Args, JsObject, JsValue, Local};
use crate::builtins::{IntrinsicId, intrinsic};
use crate::conv::Res;
use crate::error::{not_implemented, throw_error};
use crate::exotic::probe;
use crate::frame::{NULL, UNDEFINED};
use crate::generator::is_generator;
use crate::interp::Cx;
use crate::kinds;
use crate::object::{self, genf};
use crate::runtime::ErrorKind;

use super::{context, info};

/// Throws the internal error `ep9 internal error: <msg>` (a BytecodeGen or VM bug).
fn internal(cx: &mut Cx, msg: &str) -> Throw {
    throw_error(cx, ErrorKind::Error, &format!("ep9 internal error: {msg}"))
}

/// True if `g` is the object of an async function (a GENERATOR in async-function mode).
fn is_async_object(g: u64) -> bool {
    is_generator(g) && object::gen_get_i32(g, genf::MODE) == genf::MODE_ASYNC_FUNCTION
}

/// The object `g` of ids 33-35, or the internal error.
fn async_object(cx: &mut Cx, g: u64, id: &str) -> Res<u64> {
    if is_async_object(g) {
        Ok(g)
    } else {
        Err(internal(
            cx,
            &format!("{id} of a non-async-function object"),
        ))
    }
}

/// `%Promise%` of the current realm; not implemented without it (an embedder with no
/// ep9_builtins tables).
fn promise_constructor(cx: &mut Cx) -> Res<u64> {
    let c = intrinsic(cx.rt(), IntrinsicId::Promise);
    if kinds::is_object(c) {
        Ok(c)
    } else {
        Err(not_implemented(cx, "async functions without %Promise%"))
    }
}

/// Id 32: AsyncFunctionEnter(closure, receiver) (§15.8.4 EvaluateAsyncFunctionBody step 1,
/// §27.7.5.1 AsyncFunctionStart): the async function object of the running async function
/// `closure` in frame `fp`, with `receiver` as its `this`, and its promise (NewPromiseCapability
/// of %Promise%: the promise alone, since only the body resolves it). Its REGS cell has room for
/// the input, the parameters and every register of the frame. Executing until the first
/// `await`. The object is never visible to JS (null prototype). GC point.
pub(crate) fn async_function_enter(
    cx: &mut Cx,
    fp: *mut u64,
    closure: u64,
    receiver: u64,
) -> Res<u64> {
    probe::note("async_fn AsyncFunctionEnter");
    let fi = info(fp);
    let is_async = fi.loaded_unit().is_some_and(|u| {
        let flags = u.unit.functions[fi.function_id as usize].flags;
        flags.contains(ep9_bytecode::FunctionFlags::ASYNC)
            && !flags.contains(ep9_bytecode::FunctionFlags::GENERATOR)
    });
    if !is_async
        || !kinds::is_function(closure)
        || !core::ptr::eq(object::function_info(closure), fi)
    {
        return Err(internal(
            cx,
            "AsyncFunctionEnter outside its async function",
        ));
    }
    promise_constructor(cx)?;
    let rt = cx.rt();
    let proto = intrinsic(rt, IntrinsicId::PromisePrototype);
    // GC points: the promise, then the object; the values live in Rust locals (CSS).
    let p = abi::Cx::new(cx).new_promise(Local::<JsValue>::from_bits(proto));
    let n = 1 + fi.parameter_count as usize + fi.register_count as usize;
    let g = object::new_generator(
        rt,
        NULL,
        closure,
        receiver,
        context(fp),
        genf::MODE_ASYNC_FUNCTION,
        n,
    );
    object::gen_set(rt, g, genf::ASYNC, p.bits());
    Ok(g)
}

/// Id 33: AsyncFunctionAwait(object, value): Await(value) (§27.7.5.3) steps 2-7. A throw of
/// PromiseResolve (a `constructor` getter of a promise) is a throw at the `await`. GC point;
/// may run JS.
pub(crate) fn async_function_await(cx: &mut Cx, object: u64, value: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionAwait");
    let g = async_object(cx, object, "AsyncFunctionAwait")?;
    let c = promise_constructor(cx)?;
    let mut acx = abi::Cx::new(cx);
    // 2. Let promise be ? PromiseResolve(%Promise%, value).
    let promise = acx.promise_resolve(
        Local::<JsObject>::from_bits(c),
        Local::<JsValue>::from_bits(value),
    )?;
    let Some(promise) = promise.as_object() else {
        return Err(internal(acx.inner(), "PromiseResolve gave a non-object"));
    };
    // 3-6. The closures that resume the async context (length 1, name "").
    let data = Local::<JsValue>::from_bits(g);
    let on_fulfilled = acx.new_builtin_closure(await_fulfilled, "", 1, data);
    let on_rejected = acx.new_builtin_closure(await_rejected, "", 1, data);
    // 7. Perform PerformPromiseThen(promise, onFulfilled, onRejected).
    acx.perform_promise_then(promise, on_fulfilled, on_rejected, None);
    Ok(UNDEFINED)
}

/// Id 34: AsyncFunctionResolve(object, value) (§27.7.5.2 AsyncBlockStart step 3.f): the
/// object is completed, then `Call(promiseCapability.[[Resolve]], undefined, « value »)`.
/// Returns the promise. GC point; may run JS (a `then` getter).
pub(crate) fn async_function_resolve(cx: &mut Cx, object: u64, value: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionResolve");
    let g = async_object(cx, object, "AsyncFunctionResolve")?;
    let p = object::gen_get(g, genf::ASYNC);
    crate::generator::close(cx.rt(), g);
    abi::Cx::new(cx).resolve_promise_once(
        Local::<JsObject>::from_bits(p),
        Local::<JsValue>::from_bits(value),
    )?;
    Ok(p)
}

/// Id 35: AsyncFunctionReject(object, reason) (§27.7.5.2 step 3.g): the object is completed,
/// then `Call(promiseCapability.[[Reject]], undefined, « reason »)`. Returns the promise.
pub(crate) fn async_function_reject(cx: &mut Cx, object: u64, reason: u64) -> Res<u64> {
    probe::note("async_fn AsyncFunctionReject");
    let g = async_object(cx, object, "AsyncFunctionReject")?;
    let p = object::gen_get(g, genf::ASYNC);
    crate::generator::close(cx.rt(), g);
    reject_once(&mut abi::Cx::new(cx), p, reason);
    Ok(p)
}

/// Rejects the async function's promise `p` with `reason` if it is still pending (only the
/// body settles it, so it is pending unless a resume failed after a settle).
fn reject_once(acx: &mut abi::Cx<'_>, p: u64, reason: u64) {
    let p = Local::<JsObject>::from_bits(p);
    if acx.promise_state(p).ok() == Some(abi::PromiseState::Pending) {
        acx.reject_promise(p, Local::<JsValue>::from_bits(reason));
    }
}

/// Await step 3 (fulfilledClosure): resume the async context with a normal completion.
fn await_fulfilled<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume(cx, args, genf::RESUME_NEXT)
}

/// Await step 5 (rejectedClosure): resume the async context with a throw completion.
fn await_rejected<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume(cx, args, genf::RESUME_THROW)
}

/// Resumes the object of the closure `args.callee()` with `mode` and `args[0]`. Returns
/// undefined: the reaction has no capability, so the closure must not throw. A throw that
/// leaves the resume (the frame could not be pushed: a full stack, or an interrupt) rejects
/// the promise and completes the object; a termination passes on.
fn resume<'s>(cx: &mut abi::Cx<'s>, args: Args<'s>, mode: i32) -> JsResult<Local<'s>> {
    let g = cx.closure_data(args.callee()).bits();
    let v = args.get(0).bits();
    if !is_async_object(g) {
        return Err(internal(
            cx.inner(),
            "an await closure without its async function",
        ));
    }
    if let Err(t) = crate::generator::resume_async_function(cx.inner(), g, mode, v) {
        let e = cx.catch_exception(t)?;
        let p = object::gen_get(g, genf::ASYNC);
        crate::generator::close(cx.inner().rt(), g);
        reject_once(cx, p, e.bits());
    }
    Ok(Local::undefined())
}
