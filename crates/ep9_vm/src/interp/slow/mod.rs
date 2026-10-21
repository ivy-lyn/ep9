//! Runtime entries of the cold blocks (H15: every runtime call is in a cold block).
//!
//! Convention: `fn(cx, fp, pc, values..) -> u64`. All arguments are passed by value, so no
//! handler local escapes and the handler keeps its sibling call. The result is the new
//! accumulator, or 0 if an exception is pending. Entries that switch frames (calls, unwinding)
//! also set `cx.out_fp` and `cx.out_pc`, and never return 0 without doing so.
//!
//! Each entry first records the interpreter state (`Runtime::enter`): the top frame for the
//! GC root scan and the bytecode offset for stack traces.
//!
//! | File | Entries |
//! |---|---|
//! | `mod.rs` | shared helpers, unwind, interrupts, throws, bad opcode or context |
//! | `ops_misc.rs` | operators, conversions, typeof, compare, switch |
//! | `property.rs` | globals and property access |
//! | `call.rs` | calls and constructs |
//! | `runtime.rs` | `CallRuntime` |
//! | `literal.rs` | closures, contexts, literals, template objects, `CloneObject` |
//! | `iter.rs` | iteration, spread, arguments, for-in |

use core::sync::atomic::Ordering;

use ep9_value::Throw;

use crate::conv::{Res, describe};
use crate::error::{self, FrameRef, not_implemented, throw_error};
use crate::frame::{TRUE, UNDEFINED, slot};
use crate::interp::{BUDGET, Cx};
use crate::object;
use crate::runtime::{ErrorKind, Site};
use crate::unit::FunctionInfo;

mod call;
mod iter;
mod literal;
mod ops_misc;
mod property;
mod runtime;

pub(crate) use call::call;
pub(crate) use iter::{
    arguments, array_from_iterable, for_in_enumerate, for_in_next, get_iterator,
};
pub(crate) use literal::{CtxKind, Lit, clone_object, create_closure, create_context, literal};
pub(crate) use ops_misc::{
    CmpOp, UnOp, binary, compare, strict_equals, switch_target, to_boolean, type_of, unary,
};
pub(crate) use property::{PropOp, load_global, prop, store_global};
pub(crate) use runtime::call_runtime;

#[inline(always)]
fn enter(cx: &Cx, fp: *mut u64, pc: *const u8) {
    cx.rt().enter(fp, pc);
}

#[inline(always)]
fn ret(r: Res<u64>) -> u64 {
    match r {
        Ok(v) => {
            debug_assert!(v != 0);
            v
        }
        Err(Throw) => 0,
    }
}

fn info<'a>(fp: *mut u64) -> &'a FunctionInfo {
    FrameRef(fp).info()
}

fn strict(fp: *mut u64) -> bool {
    info(fp).is_strict()
}

/// The current context of the frame `fp`.
fn context(fp: *mut u64) -> u64 {
    // SAFETY: header slot 6 of a live frame.
    unsafe { *fp.offset(slot::CONTEXT) }
}

/// Finds the handler for the pending exception (see `error::unwind`).
#[cold]
#[inline(never)]
pub(crate) fn unwind(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    error::unwind(cx, fp, pc)
}

/// A byte that is no opcode. This is a VM bug (the verifier accepts defined opcodes only), so
/// the run stops at once: no JS handler runs, the frames above the entry frame are dropped.
#[cold]
#[inline(never)]
pub(crate) fn bad_opcode(cx: &mut Cx, _fp: *mut u64, pc: *const u8) -> u64 {
    let rt = cx.rt();
    // SAFETY: `pc` is the byte that the dispatch just read.
    let b = unsafe { *pc };
    let s = object::new_string(rt, &format!("ep9 internal error: bad opcode 0x{b:02x}"));
    rt.throw_value(s)
}

/// The interrupt poll (budget exhausted): resets the budget and handles termination. Returns
/// 0 if the isolate must terminate (the pending exception cannot be caught).
#[cold]
#[inline(never)]
pub(crate) fn interrupt(cx: &mut Cx, _fp: *mut u64) -> u64 {
    cx.budget = BUDGET;
    let rt = cx.rt();
    if rt.terminating.get() || rt.terminate.swap(false, Ordering::AcqRel) {
        rt.terminating.set(true);
        return rt.throw_value(UNDEFINED);
    }
    TRUE
}

/// `interrupt` at a `JumpLoop`.
#[cold]
#[inline(never)]
pub(crate) fn interrupt_at(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    interrupt(cx, fp)
}

/// Throws the "not implemented" error of v0.
#[cold]
#[inline(never)]
pub(crate) fn not_impl(cx: &mut Cx, fp: *mut u64, pc: *const u8, what: &'static str) -> u64 {
    enter(cx, fp, pc);
    not_implemented(cx, what);
    0
}

/// Throws a new error of `kind` with a fixed message.
#[cold]
#[inline(never)]
pub(crate) fn throw_msg(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    kind: ErrorKind,
    msg: &'static str,
) -> u64 {
    enter(cx, fp, pc);
    throw_error(cx, kind, msg);
    0
}

/// The message templates that name a binding.
#[derive(Clone, Copy)]
pub(crate) enum NameMsg {
    /// ReferenceError: TDZ read or write.
    Tdz,
    /// ReferenceError: unknown global.
    NotDefined,
}

/// Throws a ReferenceError about the binding `name` (a string value).
#[cold]
#[inline(never)]
pub(crate) fn throw_name(cx: &mut Cx, fp: *mut u64, pc: *const u8, m: NameMsg, name: u64) -> u64 {
    enter(cx, fp, pc);
    throw_name_now(cx, m, name);
    0
}

fn throw_name_now(cx: &mut Cx, m: NameMsg, name: u64) -> Throw {
    let n = object::str_view(name).to_string_lossy();
    let msg = match m {
        NameMsg::Tdz => format!("Cannot access '{n}' before initialization"),
        NameMsg::NotDefined => format!("{n} is not defined"),
    };
    throw_error(cx, ErrorKind::ReferenceError, &msg)
}

/// `throw v` (`rethrow`: a `ReThrow`, which keeps the first throw site).
#[cold]
#[inline(never)]
pub(crate) fn throw(cx: &mut Cx, fp: *mut u64, pc: *const u8, v: u64, rethrow: bool) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if !rethrow {
        let fi = info(fp);
        let site = fi
            .loaded_unit()
            .zip(fi.offset_of(pc).and_then(|o| fi.source_offset(o)))
            .map(|(u, offset)| Site {
                unit: u.index,
                offset,
            });
        rt.shared.throw_site.set(site);
    }
    rt.throw_value(v)
}

fn throw_msg_now(cx: &mut Cx, msg: &str) -> u64 {
    throw_error(cx, ErrorKind::Error, &format!("ep9 internal error: {msg}"));
    0
}

/// A context slot access that the fast path rejected: a VM or BytecodeGen bug (bad depth or
/// slot), reported as an internal error.
#[cold]
#[inline(never)]
pub(crate) fn bad_context(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    throw_msg_now(cx, "context slot out of range")
}

/// `ThrowIfNotSuperConstructor(constructor)` failure.
#[cold]
#[inline(never)]
pub(crate) fn not_super_constructor(cx: &mut Cx, fp: *mut u64, pc: *const u8, ctor: u64) -> u64 {
    enter(cx, fp, pc);
    let msg = format!(
        "Super constructor {} of anonymous class is not a constructor",
        describe(ctor)
    );
    throw_error(cx, ErrorKind::TypeError, &msg);
    0
}
