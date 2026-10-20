//! Frame push and calls (H14). JS→JS calls never recurse in Rust: the interpreter pushes the
//! callee frame and jumps to its code. Rust recursion happens only at the edges:
//!
//! | Path | How | Native stack |
//! |---|---|---|
//! | JS → JS (`Call*`, `Construct`) | push frame, jump (handler or `rt_call`) | none |
//! | JS → host function | push host frame, call the Rust fn | one Rust call, checked |
//! | Rust → JS (`call_function`, host callbacks, ToPrimitive) | push entry frame (return pc = EXIT stub), run the dispatch | one dispatch run, checked |
//! | `[[Construct]]` from JS | push a stub frame (return pc = construct stub) + callee frame | none |

use ep9_value::Throw;

use crate::conv::Res;
use crate::error::{FrameRef, not_implemented, stack_overflow, throw_error};
use crate::frame::{CONSTRUCT_BIT, HEADER, NULL, UNDEFINED, int32_bits, native_of, slot};
use crate::host::{Args, HostCx};
use crate::interp::{self, Cx, EXIT_STUB};
use crate::object::{self, function_context, function_info, is_function, is_object};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, frame_top, val};
use crate::unit::FunctionInfo;

/// Pushes the argument area, header and register file of a frame for `fi` at `base`. Returns
/// the new `fp`, or `None` if the frame would end above `limit`.
///
/// `new_target` is undefined for a call. For a `[[Construct]]` it is the new.target function:
/// the frame gets the construct bit, and v0 keeps new.target in the feedback header slot,
/// which is unused until H16 (H14-Q1 decides the real home of new.target).
///
/// # Safety
/// `base` is the first free slot of the VM stack (`frame_top` of the caller, or the stack
/// base), `limit` is at most the stack end, and `arg(i)` returns valid Value bits for
/// `i < argc`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn push_frame(
    base: *mut u64,
    limit: *mut u64,
    fi: &FunctionInfo,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    caller_fp: *mut u64,
    return_pc: *const u8,
    context: u64,
    new_target: u64,
) -> Option<*mut u64> {
    let construct = new_target != UNDEFINED;
    let n = argc.max(fi.parameter_count) as usize;
    let fp = base.wrapping_add(n + 1 + HEADER);
    let top = fp.wrapping_add(fi.register_count as usize);
    if top.addr() > limit.addr() {
        return None;
    }
    // SAFETY: [base, top) is inside the stack (top <= limit <= end, caller contract) and above
    // every live frame, so these writes clobber nothing live.
    unsafe {
        for i in 0..argc {
            *base.add(n - 1 - i as usize) = arg(i);
        }
        for i in argc as usize..n {
            *base.add(n - 1 - i) = UNDEFINED;
        }
        *base.add(n) = this;
        *fp.offset(slot::CALLER_FP) = native_of(caller_fp);
        *fp.offset(slot::RETURN_PC) = native_of(return_pc);
        *fp.offset(slot::CALLEE) = callee;
        let argc_word = if construct {
            argc | CONSTRUCT_BIT
        } else {
            argc
        };
        *fp.offset(slot::ARGC) = int32_bits(argc_word as i32);
        *fp.offset(slot::FUNCTION_INFO) = native_of(fi);
        *fp.offset(slot::FEEDBACK) = new_target;
        *fp.offset(slot::CONTEXT) = context;
        *fp.offset(slot::BYTECODE_OFFSET) = int32_bits(0);
        for r in 0..fi.register_count as usize {
            *fp.add(r) = UNDEFINED;
        }
    }
    Some(fp)
}

/// OrdinaryCallBindThis (§10.2.1.2) for the receiver a caller passes. v0: a sloppy callee
/// gets the global object for undefined/null; primitives are not wrapped.
#[inline(always)]
pub(crate) fn bind_this(fi: &FunctionInfo, this: u64, global: u64) -> u64 {
    if !fi.is_strict() && (this == UNDEFINED || this == NULL) {
        global
    } else {
        this
    }
}

/// The first free slot above the frame `fp` (the stack base if `fp` is null).
pub(crate) fn top_of(cx: &Cx, fp: *mut u64) -> *mut u64 {
    if fp.is_null() {
        cx.rt().stack().base()
    } else {
        frame_top(fp)
    }
}

/// Calls the host function `f` with a host frame on the VM stack (its arguments are the
/// `Args` view). `caller_fp` and `return_pc` link the frame for stack walks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn call_host(
    cx: &mut Cx,
    f: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
    caller_fp: *mut u64,
    return_pc: *const u8,
) -> Res<u64> {
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let base = top_of(cx, caller_fp);
    // SAFETY: `base` is the first free slot above `caller_fp`; the limit is the stack end.
    let fp = unsafe {
        push_frame(
            base,
            cx.stack_limit,
            fi,
            f,
            this,
            argc,
            arg,
            caller_fp,
            return_pc,
            UNDEFINED,
            new_target,
        )
    };
    let Some(fp) = fp else {
        return stack_overflow(cx);
    };
    let h = fi.host.expect("a host function");
    rt.stack().set_fp(fp);
    // SAFETY: the host frame stays live until `h` returns; its argument slots hold `argc`
    // Values from PARAM0 downward.
    let args = unsafe { Args::new(fp.offset(slot::PARAM0), argc) };
    let r = {
        let mut hcx = HostCx { cx: &mut *cx };
        h(&mut hcx, val(this), args, val(new_target))
    };
    rt.stack().set_fp(caller_fp);
    r.map(|v| v.to_bits())
}

/// Call(F, thisArg, args) from Rust (§7.3.14). Runs the interpreter for a bytecode callee.
pub(crate) fn call_function(cx: &mut Cx, f: u64, this: u64, args: &[u64]) -> Res<u64> {
    call_internal(cx, f, this, args, UNDEFINED)
}

fn call_internal(cx: &mut Cx, f: u64, this: u64, args: &[u64], new_target: u64) -> Res<u64> {
    let rt = cx.rt();
    if !is_function(f) {
        let msg = format!("{} is not a function", crate::conv::describe(f));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let fi = function_info(f);
    let caller = rt.stack().fp();
    let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
    if fi.host.is_some() {
        return call_host(
            cx,
            f,
            fi,
            this,
            argc,
            |i| args[i as usize],
            new_target,
            caller,
            EXIT_STUB.as_ptr(),
        );
    }
    if !fi.is_bytecode() || fi.slot.kind_byte() != crate::dispatch::DispatchKind::Interp as u8 {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    poll(cx, caller)?;
    let this = bind_this(fi, this, rt.global());
    let base = top_of(cx, caller);
    // SAFETY: `base` is the first free slot above the current top frame.
    let fp = unsafe {
        push_frame(
            base,
            cx.stack_limit,
            fi,
            f,
            this,
            argc,
            |i| args[i as usize],
            caller,
            EXIT_STUB.as_ptr(),
            function_context(f),
            new_target,
        )
    };
    let Some(fp) = fp else {
        return stack_overflow(cx);
    };
    rt.stack().set_fp(fp);
    cx.budget -= 1;
    // SAFETY: `fp` is a frame for `fi` that `push_frame` just wrote; its code is verified.
    let r = unsafe { interp::run(cx, fi.code, fp) };
    rt.stack().set_fp(caller);
    if r == 0 { Err(Throw) } else { Ok(r) }
}

/// Construct(F, args, newTarget) from Rust (§7.3.15). v0: `new_target` must be `f`.
pub(crate) fn construct(cx: &mut Cx, f: u64, args: &[u64]) -> Res<u64> {
    if !object::is_constructor(f) {
        let msg = format!("{} is not a constructor", crate::conv::describe(f));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let fi = function_info(f);
    if fi.host.is_some() {
        return call_internal(cx, f, UNDEFINED, args, f);
    }
    let this = ordinary_create_from_constructor(cx, f)?;
    let r = call_internal(cx, f, this, args, f)?;
    Ok(if is_object(r) { r } else { this })
}

/// OrdinaryCreateFromConstructor(F, "%Object.prototype%") (§10.1.13).
pub(crate) fn ordinary_create_from_constructor(cx: &mut Cx, f: u64) -> Res<u64> {
    let rt = cx.rt();
    let p = property::get(cx, f, rt.atom(Atom::Prototype))?;
    let proto = if is_object(p) {
        p
    } else {
        rt.root(Root::ObjectPrototype)
    };
    Ok(object::new_object(rt, proto))
}

/// Interrupt poll at a function entry from Rust.
fn poll(cx: &mut Cx, fp: *mut u64) -> Res<()> {
    if cx.budget <= 0 && interp::slow::interrupt(cx, fp) == 0 {
        return Err(Throw);
    }
    Ok(())
}

/// The arguments of a call instruction, decoded from its bytes (slow paths only).
pub(crate) struct CallSite {
    pub callee: u64,
    pub this: u64,
    /// Register slot offsets (from fp) of the arguments, in order.
    pub args: Vec<isize>,
    pub next_pc: *const u8,
    pub construct: bool,
    pub spread: bool,
}

/// Decodes the `Call*`/`Construct*` instruction at `pc` of the frame `fp`.
pub(crate) fn decode_call(fp: *mut u64, pc: *const u8) -> CallSite {
    use ep9_bytecode::Opcode as O;
    let fi = FrameRef(fp).info();
    let off = fi.offset_of(pc).expect("pc in the frame's code");
    // SAFETY: the code of a bytecode FunctionInfo is `code_len` bytes of verified bytecode.
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let insn = ep9_bytecode::decode_at(code, off).expect("verified bytecode decodes");
    let reg = |i: usize| -> isize {
        let o = insn.operand(i) as isize;
        if o < 0 { o - HEADER as isize } else { o }
    };
    // SAFETY: register operands of verified code name slots of the live frame `fp`.
    let rd = |s: isize| unsafe { *fp.offset(s) };
    let list = |i: usize, n: usize| -> Vec<isize> {
        let first = insn.operand(i) as isize;
        (0..n as isize).map(|k| first + k).collect()
    };
    let next_pc = pc.wrapping_add(insn.size() as usize);
    let callee = rd(reg(0));
    let (this, args, construct, spread) = match insn.opcode {
        O::CallAnyReceiver | O::CallProperty | O::CallWithSpread => {
            let n = insn.operand(2) as usize;
            let l = list(1, n);
            let this = l.first().map_or(UNDEFINED, |&s| rd(s));
            let args = l.into_iter().skip(1).collect();
            (this, args, false, insn.opcode == O::CallWithSpread)
        }
        O::CallProperty0 => (rd(reg(1)), vec![], false, false),
        O::CallProperty1 => (rd(reg(1)), vec![reg(2)], false, false),
        O::CallProperty2 => (rd(reg(1)), vec![reg(2), reg(3)], false, false),
        O::CallUndefinedReceiver => {
            let n = insn.operand(2) as usize;
            (UNDEFINED, list(1, n), false, false)
        }
        O::CallUndefinedReceiver0 => (UNDEFINED, vec![], false, false),
        O::CallUndefinedReceiver1 => (UNDEFINED, vec![reg(1)], false, false),
        O::CallUndefinedReceiver2 => (UNDEFINED, vec![reg(1), reg(2)], false, false),
        O::Construct | O::ConstructWithSpread => {
            let n = insn.operand(2) as usize;
            (
                UNDEFINED,
                list(1, n),
                true,
                insn.opcode == O::ConstructWithSpread,
            )
        }
        op => unreachable!("decode_call on {op:?}"),
    };
    CallSite {
        callee,
        this,
        args,
        next_pc,
        construct,
        spread,
    }
}

/// Pushes a `[[Construct]]` stub frame (receiver = `this`) above `fp` and returns its fp.
/// `ConstructReturn` (the stub's code) replaces a non-object result with `this`.
///
/// # Safety
/// `fp` is the live top frame and `this` is valid Value bits.
pub(crate) unsafe fn push_construct_stub(
    cx: &Cx,
    fp: *mut u64,
    this: u64,
    ctor: u64,
    next_pc: *const u8,
) -> Option<*mut u64> {
    let rt = cx.rt();
    // SAFETY: caller contract; the stub frame has no arguments and no registers.
    unsafe {
        push_frame(
            frame_top(fp),
            cx.stack_limit,
            &rt.construct_stub,
            ctor,
            this,
            0,
            |_| UNDEFINED,
            fp,
            next_pc,
            UNDEFINED,
            UNDEFINED,
        )
    }
}
