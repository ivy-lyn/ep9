//! Async generators (§27.6; owner M3-P2 ASYNCGEN): the runtime ids 36-41 of ABI 5 and the
//! resume closures of `await` in an async generator. `runtime.rs` routes the ids here (since
//! P0-VM3 step 2). The lowering is in the front end (`bytecodegen/async_gen.rs`); the request
//! queue, the prototype methods and the async-from-sync iterator are in `generator.rs`.
//!
//! | Id | Name | Args | Result |
//! |---|---|---|---|
//! | 36 | `AsyncGeneratorEnter` | closure, receiver | the async generator object (a GENERATOR in mode `MODE_ASYNC_GENERATOR`, its record in `genf::ASYNC`) |
//! | 37 | `AsyncGeneratorAwait` | generator, value | undefined; it subscribes the resume |
//! | 38 | `AsyncGeneratorYield` | generator, value | undefined; the first request completes with `{value, done: false}` |
//! | 39 | `AsyncGeneratorResolve` | generator, value, done | undefined; `done` true: the body completed with `value` |
//! | 40 | `AsyncGeneratorReject` | generator, reason | undefined; the body threw `reason` |
//! | 41 | `CreateAsyncFromSyncIterator` | iterator, next | the async-from-sync iterator; with `next` = the hole: GetIterator(iterator, async) |
//!
//! ```text
//! g(args)          the frame runs the parameters, then Enter (the object, state
//!                  suspended-start), then the initial suspend gives the caller the object
//! it.next(v)       generator.rs: a request (a heap cell) joins the queue; a suspended
//!                  generator runs its frame (generator::async_generator_run)
//! await v          Await: promise = PromiseResolve(%Promise%, v); PerformPromiseThen with two
//!                  closures whose data is the object; then SuspendGenerator gives the
//!                  runner control back (spec state stays executing)
//! a reaction job   the closure resumes the frame (mode next or throw) through the runner
//! yield v          bytecode: Await(v), then Yield (id 38), then SuspendGenerator; the
//!                  runner resumes again at once when a request waits (§27.6.3.8 step 11)
//! end / throw      Resolve (id 39, done) / Reject (id 40): the frame is dropped and the
//!                  record keeps the completion; after the frame returned, the runner
//!                  settles the first request and drains the queue in the resumer's realm
//!                  (§27.6.3.2 step 4: the generator context is removed first; X-AG2)
//! ```
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | AG1 | The request queue is a linked list of ELEMENTS cells held by a record cell in `genf::ASYNC` | Plan rule 8: no Rust `Vec`/`VecDeque` of Values; O(1) enqueue and dequeue |
//! | AG2 | AsyncGeneratorYield's "continue without suspending" (step 11) is a suspend plus an immediate resume by the runner loop | No JS runs between the two, so the result is the same; the frame code has one shape per `yield` |
//! | AG3 | Id 41 with the hole as `next` is GetIterator(obj, async) (§7.4.3): `@@asyncIterator`, else CreateAsyncFromSyncIterator over `@@iterator` | ABI 5 has no way to load `@@asyncIterator` from bytecode (no symbol constant, and a global `Symbol` lookup is observable); the hole is never a JS value, so the documented (iterator, next) form keeps its meaning. No new id, no ABI bump (plan §9: try an existing id first) |
//! | AG4 | A throw that leaves the frame (a full stack at the resume push, an interrupt) completes the body with that throw: the queue settles | The spec has no such failure; the alternative leaves the requests pending for ever |
//!
//! The object, its record, its requests and the saved registers are heap cells (plan rule 8);
//! the closures hold the object in their context word (`Cx::new_builtin_closure`).

use ep9_bytecode::FunctionFlags;
use ep9_value::JsResult;

use crate::abi::{self, Args, JsObject, JsValue, Local};
use crate::builtins::{IntrinsicId, intrinsic};
use crate::call;
use crate::conv::{Res, describe};
use crate::error::{not_implemented, throw_error};
use crate::frame::{HOLE, NULL, TRUE, UNDEFINED};
use crate::generator::{self, agf, is_async_generator};
use crate::interp::Cx;
use crate::kinds;
use crate::object::{self, genf};
use crate::property;
use crate::runtime::{Atom, ErrorKind};
use crate::symbol::{self, WellKnownSymbol};

use super::{context, info};

/// Throws the internal error `ep9 internal error: <msg>` (a BytecodeGen or VM bug).
fn internal(cx: &mut Cx, msg: &str) -> ep9_value::Throw {
    throw_error(cx, ErrorKind::Error, &format!("ep9 internal error: {msg}"))
}

/// The object `g` of ids 37-40, which must be an async generator whose body runs, or the
/// internal error.
fn running_generator(cx: &mut Cx, g: u64, id: &str) -> Res<u64> {
    if is_async_generator(g) && generator::ag_state(g) == agf::STATE_EXECUTING {
        Ok(g)
    } else {
        Err(internal(
            cx,
            &format!("{id} outside a running async generator"),
        ))
    }
}

/// Id 36: AsyncGeneratorEnter(closure, receiver) (§15.6.2 EvaluateAsyncGeneratorBody steps
/// 2-3): the async generator object of the running async generator function `closure` in
/// frame `fp`, with `receiver` as its `this`: OrdinaryCreateFromConstructor(closure,
/// "%AsyncGeneratorPrototype%"), state suspended-start, an empty queue. Its REGS cell has room
/// for the input, the parameters and every register of the frame. The frame then makes its
/// initial suspend, which gives the caller the object. GC point.
pub(crate) fn async_generator_enter(
    cx: &mut Cx,
    fp: *mut u64,
    closure: u64,
    receiver: u64,
) -> Res<u64> {
    let fi = info(fp);
    let is_async_gen = fi.loaded_unit().is_some_and(|u| {
        let flags = u.unit.functions[fi.function_id as usize].flags;
        flags.contains(FunctionFlags::ASYNC) && flags.contains(FunctionFlags::GENERATOR)
    });
    if !is_async_gen
        || !kinds::is_function(closure)
        || !core::ptr::eq(object::function_info(closure), fi)
    {
        return Err(internal(
            cx,
            "AsyncGeneratorEnter outside its async generator function",
        ));
    }
    let rt = cx.rt();
    // §10.1.14 GetPrototypeFromConstructor: `prototype` of an async generator function is a
    // data property ({w}), so this runs no JS.
    let p = property::get(cx, closure, rt.atom(Atom::Prototype))?;
    let proto = if kinds::is_object(p) {
        p
    } else {
        intrinsic(rt, IntrinsicId::AsyncGeneratorPrototype)
    };
    let proto = if kinds::is_object(proto) { proto } else { NULL };
    // GC points: the record, then the object; the values live in Rust locals (CSS).
    let record = generator::new_async_generator_record(rt);
    let n = 1 + fi.parameter_count as usize + fi.register_count as usize;
    let g = object::new_generator(
        rt,
        proto,
        closure,
        receiver,
        context(fp),
        genf::MODE_ASYNC_GENERATOR,
        n,
    );
    object::gen_set(rt, g, genf::ASYNC, record);
    Ok(g)
}

/// %Promise% of the current realm; not implemented without it.
fn promise_constructor(cx: &mut Cx) -> Res<u64> {
    let c = intrinsic(cx.rt(), IntrinsicId::Promise);
    if kinds::is_object(c) {
        Ok(c)
    } else {
        Err(not_implemented(cx, "async generators without %Promise%"))
    }
}

/// Id 37: AsyncGeneratorAwait(generator, value): Await(value) (§6.2.3.1) steps 2-7 in an async
/// generator: the resume of `generator` subscribes to the promise of `value`. A throw of
/// PromiseResolve (a `constructor` getter of a promise) is a throw at the `await`. The spec
/// state stays executing. GC point; may run JS.
pub(crate) fn async_generator_await(cx: &mut Cx, g: u64, value: u64) -> Res<u64> {
    let g = running_generator(cx, g, "AsyncGeneratorAwait")?;
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

/// Await step 3 (fulfilledClosure) in an async generator.
fn await_fulfilled<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume(cx, args, genf::RESUME_NEXT)
}

/// Await step 5 (rejectedClosure) in an async generator.
fn await_rejected<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    resume(cx, args, genf::RESUME_THROW)
}

/// Resumes the async generator of the closure `args.callee()` with `mode` and `args[0]`
/// (`generator::async_generator_run`, which settles the queue on any throw but a
/// termination). Returns undefined: the reaction has no capability.
fn resume<'s>(cx: &mut abi::Cx<'s>, args: Args<'s>, mode: i32) -> JsResult<Local<'s>> {
    let g = cx.closure_data(args.callee()).bits();
    let v = args.get(0).bits();
    if !is_async_generator(g) || generator::ag_state(g) != agf::STATE_EXECUTING {
        return Err(internal(
            cx.inner(),
            "an await closure without its running async generator",
        ));
    }
    generator::async_generator_run(cx.inner(), g, mode, v)?;
    Ok(Local::undefined())
}

/// Id 38: AsyncGeneratorYield(generator, value) (§27.6.3.8 steps 5-12.a): the first request
/// completes with `{value, done: false}`, then the state is suspended-yield; the frame then
/// suspends. GC point; may run JS (a `then` getter).
pub(crate) fn async_generator_yield(cx: &mut Cx, g: u64, value: u64) -> Res<u64> {
    let g = running_generator(cx, g, "AsyncGeneratorYield")?;
    generator::async_generator_yield(cx, g, value)?;
    Ok(UNDEFINED)
}

/// Id 39: AsyncGeneratorResolve(generator, value, done). `done` true: the body completed
/// normally with `value` (§27.6.3.2 AsyncGeneratorStart step 4, normal or return completion):
/// the frame is dropped; the runner settles the first request and drains the queue after the
/// frame returned (`generator::async_generator_body_done`). `done` false:
/// AsyncGeneratorCompleteStep with `{value, false}` only (BytecodeGen does not emit it). The
/// frame then returns. GC point; may run JS.
pub(crate) fn async_generator_resolve(cx: &mut Cx, g: u64, value: u64, done: u64) -> Res<u64> {
    let g = running_generator(cx, g, "AsyncGeneratorResolve")?;
    if done == TRUE {
        generator::async_generator_body_done(cx.rt(), g, false, value);
    } else {
        generator::async_generator_yield(cx, g, value)?;
        generator::set_ag_state(cx.rt(), g, agf::STATE_EXECUTING);
    }
    Ok(UNDEFINED)
}

/// Id 40: AsyncGeneratorReject(generator, reason): the body threw `reason` (§27.6.3.2 step 4,
/// throw completion): the frame is dropped; after the frame returned, the runner rejects the
/// first request and drains the queue (`generator::async_generator_body_done`). The frame
/// then returns. GC point only for the check's error.
pub(crate) fn async_generator_reject(cx: &mut Cx, g: u64, reason: u64) -> Res<u64> {
    let g = running_generator(cx, g, "AsyncGeneratorReject")?;
    generator::async_generator_body_done(cx.rt(), g, true, reason);
    Ok(UNDEFINED)
}

/// The TypeError of an async GetIterator for a value that is not async iterable.
fn not_async_iterable(cx: &mut Cx, v: u64) -> ep9_value::Throw {
    let msg = format!("{} is not async iterable", describe(v));
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// GetMethod(v, @@key) for GetIterator (§7.4.3): undefined for undefined or null, the method
/// if it is callable, else the "not async iterable" TypeError. GC point (a getter).
fn iterator_method(cx: &mut Cx, v: u64, which: WellKnownSymbol) -> Res<u64> {
    let key = symbol::well_known(cx.rt(), which);
    let m = property::get(cx, v, key)?;
    if m == UNDEFINED || m == NULL {
        return Ok(UNDEFINED);
    }
    if !kinds::is_callable(m) {
        return Err(not_async_iterable(cx, v));
    }
    Ok(m)
}

/// GetIterator(obj, async) (§7.4.3): `obj[@@asyncIterator]()`, else
/// CreateAsyncFromSyncIterator(GetIteratorFromMethod(obj, `obj[@@iterator]`)). BytecodeGen loads
/// `next` from the result (unobservable for an async-from-sync iterator). GC point; runs JS.
fn get_async_iterator(cx: &mut Cx, obj: u64) -> Res<u64> {
    if obj == UNDEFINED || obj == NULL {
        return Err(not_async_iterable(cx, obj));
    }
    let method = iterator_method(cx, obj, WellKnownSymbol::AsyncIterator)?;
    if method == UNDEFINED {
        let sync = iterator_method(cx, obj, WellKnownSymbol::Iterator)?;
        if sync == UNDEFINED {
            return Err(not_async_iterable(cx, obj));
        }
        let it = call::call_function(cx, sync, obj, &[])?;
        if !kinds::is_object(it) {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Result of the Symbol.iterator method is not an object",
            ));
        }
        let next = property::get(cx, it, cx.rt().atom(Atom::Next))?;
        return Ok(generator::new_async_from_sync_iterator(cx.rt(), it, next));
    }
    let it = call::call_function(cx, method, obj, &[])?;
    if !kinds::is_object(it) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Result of the Symbol.asyncIterator method is not an object",
        ));
    }
    Ok(it)
}

/// Id 41: CreateAsyncFromSyncIterator(iterator, next) (§27.1.6.1) of the sync Iterator Record
/// (`iterator`, `next`): a new async-from-sync iterator. With the hole as `next` (AG3) it is
/// GetIterator(`iterator`, async) instead. GC point; the hole form runs JS.
pub(crate) fn create_async_from_sync_iterator(cx: &mut Cx, iterator: u64, next: u64) -> Res<u64> {
    if next == HOLE {
        return get_async_iterator(cx, iterator);
    }
    if !kinds::is_object(iterator) {
        return Err(internal(
            cx,
            "CreateAsyncFromSyncIterator of a non-object iterator",
        ));
    }
    Ok(generator::new_async_from_sync_iterator(
        cx.rt(),
        iterator,
        next,
    ))
}
