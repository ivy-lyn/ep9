//! Bound function exotic objects (§10.4.1): kind BOUND_FUNCTION through the fixed-field API
//! (target, bound `this`, bound arguments). Week-4 stub; P3 (`Function.prototype.bind`)
//! builds them.
#![allow(dead_code)] // wk4-stub

use crate::conv::Res;
use crate::error::not_implemented;
use crate::interp::Cx;

/// IsConstructor of the bound function `f`: its target's (false until bound functions
/// exist). `object::is_constructor` asks this for BOUND_FUNCTION cells.
pub(crate) fn is_constructor(_f: u64) -> bool {
    false
}

/// BoundFunctionCreate(target, boundThis, boundArgs) (§10.4.1.3). GC point.
pub(crate) fn new_bound_function(cx: &mut Cx, _target: u64, _this: u64, _args: &[u64]) -> Res<u64> {
    Err(not_implemented(cx, "bound functions"))
}
