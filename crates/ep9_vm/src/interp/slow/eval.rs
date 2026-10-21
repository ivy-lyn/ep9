#![allow(dead_code)] // m2-stub
//! Direct eval in the VM (owner P3-EVAL-VM): runtime id 7 `ResolvePossiblyDirectEval`, and
//! (ABI 3) ids 27-28 `DeclareEvalVar` and `DeclareEvalFunction`. The compiler is reached
//! through `IsolateConfig::compiler` (`crate::CompilerHooks`, C2).
//!
//! P0 stubs: `%eval%` is not installed (C19), so the callee is never %eval% and the call goes
//! ahead with the callee (today's behaviour). Ids 27-28 throw
//! `not implemented in ep9 v0: <name>`, the text of the `CallRuntime` catch-all arm before
//! they were routed.

use crate::conv::Res;
use crate::error::not_implemented;
use crate::interp::Cx;

/// `ResolvePossiblyDirectEval(callee, source, function, scope, position)` (runtime id 7):
/// the closure of the direct eval code if `callee` is %eval% and `source` a string, else
/// `callee`. Stub: `callee`.
pub(crate) fn resolve_possibly_direct_eval(
    _cx: &mut Cx,
    _fp: *mut u64,
    callee: u64,
    _source: u64,
    _function: u64,
    _scope: u64,
    _position: u64,
) -> Res<u64> {
    Ok(callee)
}

/// `DeclareEvalVar(name)` (runtime id 27): a `var` of sloppy direct eval code
/// (§19.2.1.3 EvalDeclarationInstantiation), in the `.eval_ext` object of the nearest
/// function var scope (`ep9_bytecode::EVAL_EXT_BINDING`) or on the global object. Stub.
pub(crate) fn declare_eval_var(cx: &mut Cx, _fp: *mut u64, _name: u64) -> Res<u64> {
    Err(not_implemented(cx, "DeclareEvalVar"))
}

/// `DeclareEvalFunction(name, function)` (runtime id 28): a function declaration of sloppy
/// direct eval code, stored like [`declare_eval_var`]. Stub.
pub(crate) fn declare_eval_function(
    cx: &mut Cx,
    _fp: *mut u64,
    _name: u64,
    _function: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "DeclareEvalFunction"))
}
