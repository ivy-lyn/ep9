#![allow(dead_code)] // m2-stub
//! Generator objects (§27.5; owner P1-GEN-VM): the GENERATOR cell (kind 23, `object::genf`),
//! `%GeneratorFunction%`, `%GeneratorPrototype%` and its `next`/`return`/`throw`, and the
//! lazy `prototype` of generator functions. One cell kind serves async functions and async
//! generators too (the mode field, month 3).
//!
//! ```text
//! function* g() {}       FunctionInfo kind Generator
//! g.prototype            exotic::function_prototype -> lazy_prototype_kind(fi)
//!                          Ordinary:            {constructor: g}      (MakeConstructor)
//!                          Instances(%GeneratorPrototype%): {} with that [[Prototype]]
//! g()                    CreateGeneratorObject (runtime id 19) ... SuspendGenerator
//! it.next(v)             call::push_resume_frame (fp-3 = the generator object), dispatch
//! ```
//!
//! P0 stubs: [`lazy_prototype_kind`] gives today's answer (constructors only), and the
//! natives throw "not implemented". Nothing reaches them before generators compile.

use ep9_value::JsResult;

use crate::abi::{Args, Cx as HostCx, Local};
use crate::builtins::IntrinsicId;
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
/// `exotic::function_prototype` ask this. Stub: constructors only (today's answer); P1-GEN-VM
/// adds generator functions. Hot: inlined into every property lookup on a function.
#[inline(always)]
pub(crate) fn lazy_prototype_kind(fi: &FunctionInfo) -> Option<LazyPrototype> {
    fi.is_constructor().then_some(LazyPrototype::Ordinary)
}

/// `GeneratorFunction(...params, body)` (§27.3.1.1). Stub: needs the compiler hook.
pub(crate) fn generator_function<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("GeneratorFunction constructor"))
}

/// `%GeneratorPrototype%.next(value)` (§27.5.1.2). Stub.
pub(crate) fn generator_next<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("generators"))
}

/// `%GeneratorPrototype%.return(value)` (§27.5.1.3). Stub.
pub(crate) fn generator_return<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("generators"))
}

/// `%GeneratorPrototype%.throw(exception)` (§27.5.1.4). Stub.
pub(crate) fn generator_throw<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("generators"))
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
