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
//! A suspended-start generator resumed with return or throw runs its prologue's resume
//! point, which returns or throws at once (as V8 does): the same result as GeneratorResumeAbrupt
//! (§27.5.3.4) step 2, since no user code runs before the first suspend's resume point.

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
/// function one that inherits from %GeneratorPrototype%. Hot: inlined into every property
/// lookup on a function, so it tests flag bits only (no unit pointer chase).
#[inline(always)]
pub(crate) fn lazy_prototype_kind(fi: &FunctionInfo) -> Option<LazyPrototype> {
    if fi.flags & (fi_flags::CONSTRUCTOR | fi_flags::GENERATOR) == 0 {
        return None;
    }
    Some(if fi.is_generator() {
        LazyPrototype::Instances(IntrinsicId::GeneratorPrototype)
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
/// `mode` as its resume mode, then its frame runs (`call::run_resumed`) until it suspends
/// again (the accumulator is the result as it is) or completes (closed; a return gives
/// `{value, done: true}`, a throw propagates). An error before the frame runs (a full
/// stack) leaves the generator suspended.
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
    let state = object::gen_get_i32(g, genf::STATE);
    if state == genf::STATE_EXECUTING {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Generator is already running",
        ));
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
            } else if let Some(regs) = object::gen_regs(g) {
                // The frame never ran (a full stack): still suspended; drop the input.
                regs.set(rt, genf::REGS_INPUT, UNDEFINED);
            }
            Err(Throw)
        }
    }
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

/// `GeneratorFunction(...params, body)` (§27.3.1.1). Not implemented: needs the compiler
/// hook (P3-FNCTOR).
pub(crate) fn generator_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("GeneratorFunction constructor"))
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

/// `AsyncFunction(...params, body)` (§27.7.1.1). Stub (month 3).
pub(crate) fn async_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("AsyncFunction constructor"))
}
