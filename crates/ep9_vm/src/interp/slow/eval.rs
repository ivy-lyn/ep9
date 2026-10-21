#![allow(dead_code)] // m2-stub
//! Direct eval in the VM (owner P3-EVAL-VM): runtime id 7 `ResolvePossiblyDirectEval`, and
//! (ABI 3, with the P0-LANG merge) ids 27-28 `DeclareEvalVar` and `DeclareEvalFunction`.
//! The compiler is reached through `IsolateConfig::compiler` (C2).
//!
//! P0 stub: `%eval%` is not installed (C19), so the callee is never %eval% and the call goes
//! ahead with the callee (today's behaviour).

use crate::conv::Res;
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
