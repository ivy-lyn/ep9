//! Calls and constructs from bytecode. A bytecode callee's call is counted here, and the call
//! that reaches the feedback threshold makes its vector before the frame push (`feedback.rs`).
//!
//! A bound function callee (§10.4.1) is replaced by its innermost target FUNCTION before
//! anything else: its bound arguments go in front of the call's (`bound::unwrap`,
//! `bound::chain_arg`), its bound `this` replaces the receiver, and for `[[Construct]]`
//! new.target becomes the target while it is the bound function. The bound function stays in
//! the caller's callee register, so everything its chain holds stays rooted until the frame
//! push.

use ep9_value::Throw;

use crate::bound;
use crate::call::{self, CallSite};
use crate::conv::{Res, describe};
use crate::error::{self, not_implemented, stack_overflow, throw_error};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::{CONSTRUCT_RETURN_STUB, Cx};
use crate::kinds::{is_bound, is_callable};
use crate::object::{self, is_array, is_string};
use crate::runtime::{ErrorKind, frame_top};
use crate::unit::FunctionInfo;

use super::iter::{KeepAlive, not_iterable, push_code_points};
use super::{enter, interrupt};

// ---- calls ---------------------------------------------------------------------------------

/// The call or construct instruction at `pc` (all `Call*` and `Construct*` opcodes). `acc` is
/// `new.target` for `Construct`. A bytecode callee gets a new frame and the run continues in it
/// (no Rust recursion); a host callee runs here. Sets `cx.out_fp`/`cx.out_pc`.
#[cold]
#[inline(never)]
pub(crate) fn call(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> u64 {
    enter(cx, fp, pc);
    if cx.budget <= 0 && interrupt(cx, fp) == 0 {
        return error::unwind(cx, fp, pc);
    }
    let site = call::decode_call(fp, pc);
    match do_call(cx, fp, &site, acc) {
        Ok(v) => v,
        Err(Throw) => error::unwind(cx, fp, pc),
    }
}

fn do_call(cx: &mut Cx, fp: *mut u64, s: &CallSite, acc: u64) -> Res<u64> {
    let rt = cx.rt();
    // The argument values stay rooted in the caller's registers while this copy exists.
    // SAFETY: the offsets come from register operands of verified code in the frame `fp`.
    let mut args: Vec<u64> = s.args.iter().map(|&o| unsafe { *fp.offset(o) }).collect();
    // A string spread gets a new array of code point strings; this keeps it (and so the
    // strings in `args`) alive until the call returns.
    let mut keep = KeepAlive(UNDEFINED);
    if s.spread {
        let last = args.pop().unwrap_or(UNDEFINED);
        let list = if is_array(last) {
            // The array stays rooted in the caller's last argument register.
            last
        } else if is_string(last) {
            keep.0 = object::new_array(rt, 0);
            push_code_points(rt, keep.0, 0, last);
            keep.0
        } else {
            return Err(not_iterable(cx, last));
        };
        for i in 0..object::array_len(list) {
            args.push(object::array_get(list, i).unwrap_or(UNDEFINED));
        }
    }
    let mut callee = s.callee;
    if s.construct {
        if !object::is_constructor(callee) {
            let msg = format!("{} is not a constructor", describe(callee));
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        // `new.target` is the accumulator if it is a constructor (BytecodeGen sets it), else
        // the callee. v0 keeps it in the callee frame's feedback slot (push_frame).
        let mut new_target = if object::is_constructor(acc) {
            acc
        } else {
            callee
        };
        if is_bound(callee) {
            // §10.4.1.2: Construct(target, boundArgs ++ args, newTarget == F ? target :
            // newTarget).
            let u = prepend_bound_args(cx, callee, new_target, &mut args)?;
            callee = u.target;
            new_target = u.new_target;
        }
        let Ok(argc) = u32::try_from(args.len()) else {
            return stack_overflow(cx);
        };
        let fi = object::function_info(callee);
        if fi.host.is_some() {
            let r = call::call_host(
                cx,
                callee,
                fi,
                UNDEFINED,
                argc,
                |i| args[i as usize],
                new_target,
                fp,
                s.next_pc,
            )?;
            cx.out_fp = fp;
            cx.out_pc = s.next_pc;
            return Ok(r);
        }
        if !runs_bytecode(fi) {
            return Err(not_implemented(cx, "calling code of this dispatch kind"));
        }
        // GC point, before `this` exists: the callee and the arguments are in caller
        // registers (a bound callee's target and bound arguments are reachable from it),
        // new.target is a Rust local (as at the GC point below).
        feedback::count_call(rt, cx.feedback_threshold, fi);
        let this = if fi.is_derived() {
            HOLE
        } else {
            // GC point; `this` then lives in a Rust local until the stub frame holds it.
            call::ordinary_create_from_constructor(cx, new_target)?
        };
        // SAFETY: `fp` is the live top frame; `this` is valid Value bits.
        let Some(stub) = (unsafe { call::push_construct_stub(cx, fp, this, callee, s.next_pc) })
        else {
            return stack_overflow(cx);
        };
        // SAFETY: `stub` is the new top frame; the callee frame goes right above it.
        let nfp = unsafe {
            call::push_frame(
                frame_top(stub),
                cx.stack_limit,
                fi,
                callee,
                this,
                argc,
                |i| args[i as usize],
                stub,
                CONSTRUCT_RETURN_STUB.as_ptr(),
                object::function_context(callee),
                new_target,
            )
        };
        let Some(nfp) = nfp else {
            return stack_overflow(cx);
        };
        cx.budget -= 1;
        cx.out_fp = nfp;
        cx.out_pc = fi.code;
        return Ok(UNDEFINED);
    }
    if !is_callable(callee) {
        let msg = format!("{} is not a function", describe(callee));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let mut this = s.this;
    if is_bound(callee) {
        // §10.4.1.1: Call(target, boundThis, boundArgs ++ args).
        let u = prepend_bound_args(cx, callee, UNDEFINED, &mut args)?;
        callee = u.target;
        this = u.this;
    }
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let fi = object::function_info(callee);
    if fi.host.is_some() {
        let r = call::call_host(
            cx,
            callee,
            fi,
            this,
            argc,
            |i| args[i as usize],
            UNDEFINED,
            fp,
            s.next_pc,
        )?;
        cx.out_fp = fp;
        cx.out_pc = s.next_pc;
        return Ok(r);
    }
    if fi.is_class_constructor() {
        let msg = format!(
            "Class constructor {} cannot be invoked without 'new'",
            fi.name
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if !runs_bytecode(fi) {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    // GC point: the callee, the receiver and the arguments are in caller registers (for a
    // bound callee: its target, bound `this` and bound arguments are reachable from it).
    feedback::count_call(rt, cx.feedback_threshold, fi);
    let this = call::bind_this(fi, this, rt.global());
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame.
    let nfp = unsafe {
        call::push_frame(
            frame_top(fp),
            cx.stack_limit,
            fi,
            callee,
            this,
            argc,
            |i| args[i as usize],
            fp,
            s.next_pc,
            object::function_context(callee),
            UNDEFINED,
        )
    };
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    cx.budget -= 1;
    cx.out_fp = nfp;
    cx.out_pc = fi.code;
    Ok(UNDEFINED)
}

/// Walks the chain of the bound callee `f` (`bound::unwrap`; `new_target` undefined for a
/// call) and puts the chain's bound arguments in front of `args`. RangeError if the count
/// does not fit. Not a GC point; `f` must stay rooted (it is in the caller's callee register)
/// while `args` holds the bound arguments.
fn prepend_bound_args(
    cx: &mut Cx,
    f: u64,
    new_target: u64,
    args: &mut Vec<u64>,
) -> Res<bound::Unwrapped> {
    let Some(u) = bound::unwrap(f, new_target) else {
        return stack_overflow(cx);
    };
    if u.bound > 0 {
        args.splice(0..0, (0..u.bound).map(|i| bound::chain_arg(f, u.bound, i)));
    }
    Ok(u)
}

#[inline]
fn runs_bytecode(fi: &FunctionInfo) -> bool {
    fi.is_bytecode() && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
}
