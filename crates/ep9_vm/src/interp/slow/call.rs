//! Calls and constructs from bytecode.

use ep9_value::Throw;

use crate::call::{self, CallSite};
use crate::conv::{Res, describe};
use crate::error::{self, not_implemented, stack_overflow, throw_error};
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::{CONSTRUCT_RETURN_STUB, Cx};
use crate::kinds::is_callable;
use crate::object::{self, is_array, is_function, is_string};
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
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let callee = s.callee;
    if s.construct {
        if !object::is_constructor(callee) {
            let msg = format!("{} is not a constructor", describe(callee));
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        if !is_function(callee) {
            // A bound function (`bound::is_constructor`): its [[Construct]] comes with
            // Function.prototype.bind.
            return Err(not_implemented(cx, "[[Construct]] of a bound function"));
        }
        // `new.target` is the accumulator if it is a constructor (BytecodeGen sets it), else
        // the callee. v0 keeps it in the callee frame's feedback slot (push_frame).
        let new_target = if object::is_constructor(acc) {
            acc
        } else {
            callee
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
    if !is_function(callee) {
        // A bound function: its [[Call]] comes with Function.prototype.bind.
        return Err(not_implemented(cx, "[[Call]] of a bound function"));
    }
    let fi = object::function_info(callee);
    if fi.host.is_some() {
        let r = call::call_host(
            cx,
            callee,
            fi,
            s.this,
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
    let this = call::bind_this(fi, s.this, rt.global());
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

#[inline]
fn runs_bytecode(fi: &FunctionInfo) -> bool {
    fi.is_bytecode() && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
}
