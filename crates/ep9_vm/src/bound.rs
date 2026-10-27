//! Bound function exotic objects (§10.4.1): kind BOUND_FUNCTION through the fixed-field API.
//!
//! ```text
//!  BOUND_FUNCTION (H17 v0)                     made by BoundFunctionCreate (§10.4.1.3)
//!  +24 fixed 0  [[BoundTargetFunction]]        a FUNCTION or another BOUND_FUNCTION
//!  +32 fixed 1  [[BoundThis]]
//!  +40 fixed 2  [[BoundArguments]]             undefined (none), or an internal dense ARRAY
//!  +48, +56     in-object slots: `length`, `name` (SetFunctionLength/Name, by
//!               Function.prototype.bind through the ABI)
//! ```
//!
//! The fields never change after creation, and a bound function never gets a frame. A call
//! walks the chain of bound functions down to the first FUNCTION (the innermost target) with
//! a loop, never Rust recursion ([`unwrap`]), and that target's frame gets the arguments in
//! spec order: the innermost bound arguments first, then each outer function's, then the call's
//! own. A second walk copies the bound ones in that order ([`copy_chain_args`]), so a call
//! costs O(chain depth + bound arguments). [`unwrap`] takes one interrupt budget unit per
//! level, so termination stops a call through a long chain. The callers are the three call
//! paths: `slow::call` (bytecode), `call::call_with` and `call::construct_with` (Rust).
//!
//! GC: the copy goes to the VM stack's scratch area (`call.rs`), or to the argument `Vec` of
//! `slow::call`, whose values the caller keeps rooted: the outermost bound function is in the
//! callee register, and everything in the chain is reachable from it.

use ep9_value::Throw;

use crate::conv::Res;
use crate::error::{stack_overflow, throw_error};
use crate::frame::UNDEFINED;
use crate::interp::Cx;
use crate::interp::slow::interrupt;
use crate::kinds::{self, is_bound, is_function};
use crate::object::{self, arr, fixed};
use crate::runtime::ErrorKind;

/// Fixed-field indices (`object::fixed_get` / `fixed_set`).
mod field {
    pub const TARGET: usize = 0;
    pub const THIS: usize = 1;
    pub const ARGS: usize = 2;
}

/// `[[BoundTargetFunction]]` of the bound function `f`.
#[inline]
pub(crate) fn target(f: u64) -> u64 {
    object::fixed_get(f, field::TARGET)
}

/// `[[BoundThis]]` of the bound function `f`.
#[inline]
pub(crate) fn bound_this(f: u64) -> u64 {
    object::fixed_get(f, field::THIS)
}

/// The number of `[[BoundArguments]]` of the bound function `f`.
#[inline]
fn bound_len(f: u64) -> u32 {
    let a = object::fixed_get(f, field::ARGS);
    if a == UNDEFINED {
        0
    } else {
        object::array_len(a)
    }
}

/// `[[BoundArguments]]`\[i\] of the bound function `f` (`i < bound_len(f)`). The list is
/// dense: its values are JS values, never the hole.
#[inline]
fn bound_arg(f: u64, i: u32) -> u64 {
    let a = object::fixed_get(f, field::ARGS);
    let v = object::array_get(a, i);
    debug_assert!(v.is_some(), "bound argument {i} is a hole");
    v.unwrap_or(UNDEFINED)
}

/// The innermost target of the bound function `f`: the first FUNCTION down its chain.
fn innermost(f: u64) -> u64 {
    let mut t = target(f);
    while is_bound(t) {
        t = target(t);
    }
    debug_assert!(is_function(t));
    t
}

/// IsConstructor of the bound function `f` (§10.4.1.3 step 6: it has `[[Construct]]` if its
/// target has one). The targets never change, so this asks the innermost target. Not a GC
/// point. `object::is_constructor` asks this for BOUND_FUNCTION cells.
pub(crate) fn is_constructor(f: u64) -> bool {
    object::is_constructor(innermost(f))
}

/// BoundFunctionCreate(targetFunction, boundThis, boundArgs) (§10.4.1.3). `target` is
/// callable; `target`, `this` and `args` are rooted by the caller. GC point.
pub(crate) fn new_bound_function(cx: &mut Cx, target: u64, this: u64, args: &[u64]) -> Res<u64> {
    debug_assert!(kinds::is_callable(target));
    // 1. Let proto be ? targetFunction.[[GetPrototypeOf]]() (a callable proxy asks its own;
    //    GC point).
    let proto = crate::property::try_get_prototype_of(cx, target)?;
    // 2-4. MakeBasicObject with [[Prototype]] proto (below, after the argument list).
    let rt = cx.rt();
    // 9. (the list) GC point first; the list then lives in a Rust local (CSS).
    let list = if args.is_empty() {
        UNDEFINED
    } else {
        let Ok(n) = u32::try_from(args.len()) else {
            return Err(throw_error(cx, ErrorKind::RangeError, "Too many arguments"));
        };
        if n > arr::MAX_LENGTH {
            return Err(throw_error(cx, ErrorKind::RangeError, "Too many arguments"));
        }
        let a = object::new_array(rt, args.len());
        for (i, &v) in args.iter().enumerate() {
            // The capacity is `args.len()`: no growth, so no GC point inside the loop.
            object::array_set(rt, a, i as u32, v);
        }
        a
    };
    // 3-4. GC point: `proto`, `list` and the caller's values live in Rust locals.
    let f = object::new_object_with_fields(rt, kinds::BOUND_FUNCTION, proto, fixed::BOUND);
    // 5-6. [[Call]] and [[Construct]]: `call.rs` and `slow::call` dispatch on the kind;
    //      IsConstructor is the target's ([`is_constructor`]).
    // 7. Set obj.[[BoundTargetFunction]] to targetFunction.
    object::fixed_set(rt, f, field::TARGET, target);
    // 8. Set obj.[[BoundThis]] to boundThis.
    object::fixed_set(rt, f, field::THIS, this);
    // 9. Set obj.[[BoundArguments]] to boundArgs.
    object::fixed_set(rt, f, field::ARGS, list);
    // 10. Return obj.
    Ok(f)
}

/// A call of a bound function after its chain is walked ([`unwrap`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Unwrapped {
    /// The innermost target: a FUNCTION.
    pub target: u64,
    /// The innermost `[[BoundThis]]` (the `this` of `[[Call]]`; `[[Construct]]` ignores it).
    pub this: u64,
    /// The bound arguments of the whole chain, which go before the call's own.
    pub bound: u32,
    /// The new.target for the target: `new_target`, replaced by each bound function's target
    /// while it is that bound function (§10.4.1.2 step 5). Undefined for a call.
    pub new_target: u64,
}

/// Walks the chain of the bound function `f` for `[[Call]]` (§10.4.1.1, `new_target`
/// undefined) or `[[Construct]]` (§10.4.1.2). Steps 1-5 of each level, as a loop: Call(F) =
/// Call(target, boundThis, boundArgs ++ args), Construct(F, args, nt) = Construct(target,
/// boundArgs ++ args, nt == F ? target : nt). Each level takes one interrupt budget unit and
/// polls when the budget runs out: a long chain is a loop with no back edge. RangeError if the
/// bound arguments of the chain do not fit a u32 count. GC point only when it throws.
pub(crate) fn unwrap(cx: &mut Cx, f: u64, new_target: u64) -> Res<Unwrapped> {
    debug_assert!(is_bound(f));
    let mut cur = f;
    let mut u = Unwrapped {
        target: f,
        this: UNDEFINED,
        bound: 0,
        new_target,
    };
    while is_bound(cur) {
        // No back edge polls a long chain: count each level here.
        cx.budget -= 1;
        if cx.budget <= 0 && interrupt(cx, core::ptr::null_mut()) == 0 {
            return Err(Throw);
        }
        // 1. Let target be F.[[BoundTargetFunction]].
        let t = target(cur);
        // 2. Let boundThis be F.[[BoundThis]].
        u.this = bound_this(cur);
        // 3-4. Let args be the list-concatenation of boundArgs and argumentsList.
        let Some(bound) = u.bound.checked_add(bound_len(cur)) else {
            return stack_overflow(cx);
        };
        u.bound = bound;
        // [[Construct]] 5. If SameValue(F, newTarget) is true, set newTarget to target.
        if u.new_target == cur {
            u.new_target = t;
        }
        cur = t;
    }
    u.target = cur;
    Ok(u)
}

/// Writes the bound-argument prefix of a call of the bound function `f` to `out`, whose length
/// is [`Unwrapped::bound`]: the innermost function's bound arguments first, `f`'s last. One
/// walk from `f` inward puts each level's arguments right below the ones it already wrote, so
/// the cost is O(chain depth + `out.len()`). Not a GC point.
pub(crate) fn copy_chain_args(f: u64, out: &mut [u64]) {
    let mut end = out.len();
    let mut cur = f;
    while end > 0 {
        debug_assert!(is_bound(cur), "more bound arguments than the chain has");
        let n = bound_len(cur) as usize;
        let start = end - n;
        for (k, slot) in out[start..end].iter_mut().enumerate() {
            *slot = bound_arg(cur, k as u32);
        }
        end = start;
        cur = target(cur);
    }
}
