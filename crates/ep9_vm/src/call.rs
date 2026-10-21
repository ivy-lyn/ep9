//! Frame push and calls (H14). JS→JS calls never recurse in Rust: the interpreter pushes the
//! callee frame and jumps to its code. Rust recursion happens only at the edges:
//!
//! | Path | How | Native stack |
//! |---|---|---|
//! | JS → JS (`Call*`, `Construct`) | push frame, jump (handler or `rt_call`) | none |
//! | JS → host function | push host frame, call the Rust fn | one Rust call, checked |
//! | Rust → JS (`call_function`, host callbacks, ToPrimitive) | push entry frame (return pc = EXIT stub), run the dispatch | one dispatch run, checked |
//! | `[[Construct]]` from JS | push a stub frame (return pc = construct stub) + callee frame | none |
//!
//! Scratch area (frame.rs): Rust code (the builtin ABI's `ArgsBuilder`, H20) may keep Values
//! above the top frame, up to the VM stack's scratch top. A host frame push and an entry
//! frame push from Rust start above the scratch area ([`top_of`]) and clear the scratch top,
//! and the return restores the scratch top the caller had. So bytecode never runs with a
//! scratch area, the JS→JS pushes (T0 fast path, `slow::call`) need not look at it, and a
//! Rust caller's scratch Values survive the nested call (they lie below the nested frames).
//!
//! Host functions use the builtin ABI (H20, abi/mod.rs). [`call_host`] checks the throw
//! protocol in debug builds: `Err` comes with a pending exception and `Ok` without one.
//!
//! Bound functions (§10.4.1, `bound.rs`) never get a frame. [`call_with`] and
//! [`construct_with`] walk the chain to the innermost target FUNCTION and call it with the
//! bound arguments in front of the caller's, as `slow::call` does for bytecode callers. The
//! bound arguments are copied once to the scratch area ([`push_bound_args`]), and the callee
//! frame goes above them:
//!
//! ```text
//! call_with(f) ──── FUNCTION ──────────────────────────────> call_function_object(f)
//!             └──── BOUND: unwrap(f) ─> (target, boundThis, bound ++ args)
//! construct_with(f, nt) ── FUNCTION ───────────────────────> construct_function_object(f, nt)
//!                       └─ BOUND: unwrap(f, nt) ─> (target, bound ++ args, nt == F ? target : nt)
//! ```

use ep9_value::Throw;

use crate::abi::{self, Args, Local};
use crate::bound;
use crate::conv::Res;
use crate::error::{FrameRef, not_implemented, stack_overflow, throw_error};
use crate::frame::{CONSTRUCT_BIT, HEADER, HOLE, NULL, UNDEFINED, int32_bits, native_of, slot};
use crate::interp::{self, Cx, EXIT_STUB};
use crate::kinds::is_callable;
use crate::object::{self, function_context, function_info, is_function, is_object};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Runtime, frame_top};
use crate::unit::FunctionInfo;

/// Pushes the argument area, header and register file of a frame for `fi` at `base`. Returns
/// the new `fp`, or `None` if the frame would end above `limit`.
///
/// `new_target` is undefined for a call. For a `[[Construct]]` it is the new.target function:
/// the frame gets the construct bit, and new.target goes into `slot::NEW_TARGET` (fp - 3,
/// FRAME 2, H14-Q1). The FeedbackVector does not use that slot: T0 reaches it through the
/// FunctionInfo (feedback.rs).
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
        *fp.offset(slot::NEW_TARGET) = new_target;
        *fp.offset(slot::CONTEXT) = context;
        *fp.offset(slot::BYTECODE_OFFSET) = int32_bits(0);
        for r in 0..fi.register_count as usize {
            *fp.add(r) = UNDEFINED;
        }
    }
    Some(fp)
}

/// Pushes the frame of a resumed generator (FRAME 2): [`push_frame`] for a call (the
/// construct bit is clear), then `slot::NEW_TARGET` (fp - 3) holds `generator`, so
/// `LdaNewTarget` in the resumed frame gives the generator object (BytecodeGen emits
/// `LdaUndefined` for `new.target` in a generator body).
///
/// # Safety
/// As for [`push_frame`]; `generator` is valid Value bits.
#[allow(dead_code, clippy::too_many_arguments)] // m2-stub: caller P1-GEN-VM (resume)
pub(crate) unsafe fn push_resume_frame(
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
    generator: u64,
) -> Option<*mut u64> {
    // SAFETY: caller contract (the `push_frame` contract).
    let fp = unsafe {
        push_frame(
            base, limit, fi, callee, this, argc, arg, caller_fp, return_pc, context, UNDEFINED,
        )
    }?;
    // SAFETY: header slot of the frame just pushed.
    unsafe { *fp.offset(slot::NEW_TARGET) = generator };
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

/// The first free slot above the frame `fp` (the stack base if `fp` is null), or the scratch
/// top if Rust code keeps scratch Values above that frame.
pub(crate) fn top_of(cx: &Cx, fp: *mut u64) -> *mut u64 {
    let stack = cx.rt().stack();
    let top = if fp.is_null() {
        stack.base()
    } else {
        frame_top(fp)
    };
    let scratch = stack.scratch_top();
    if scratch.addr() > top.addr() {
        scratch
    } else {
        top
    }
}

/// Puts back the scratch top a Rust caller had before it entered JS or a host function.
fn restore_scratch(rt: &Runtime, saved: *mut u64) {
    if saved.is_null() {
        rt.stack().clear_scratch();
    } else {
        rt.stack().set_scratch_top(saved);
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
    let saved = rt.stack().scratch_top();
    let base = top_of(cx, caller_fp);
    // SAFETY: `base` is the first free slot above `caller_fp` and its scratch area; the limit
    // is the stack end.
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
    rt.stack().clear_scratch();
    // SAFETY: `fp` is the host frame that `push_frame` just wrote; it stays live until `h`
    // returns.
    let args = unsafe { Args::from_fp(fp) };
    let r = {
        let mut hcx = abi::Cx::new(cx);
        h(
            &mut hcx,
            Local::from_bits(this),
            args,
            Local::from_bits(new_target),
        )
    };
    rt.stack().set_fp(caller_fp);
    restore_scratch(rt, saved);
    // H20 R4: the throw protocol.
    debug_assert!(
        r.is_ok() || rt.pending() != 0,
        "host function {:?} returned Err without a pending exception",
        fi.name
    );
    debug_assert!(
        r.is_err() || rt.pending() == 0,
        "host function {:?} returned Ok with a pending exception",
        fi.name
    );
    r.map(|v| v.bits())
}

/// Call(F, thisArg, args) from Rust (§7.3.14). Runs the interpreter for a bytecode callee.
pub(crate) fn call_function(cx: &mut Cx, f: u64, this: u64, args: &[u64]) -> Res<u64> {
    let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
    call_with(cx, f, this, argc, |i| args[i as usize], UNDEFINED)
}

/// Call(F, thisArg, args) or the call step of `[[Construct]]` (`new_target` not undefined)
/// from Rust, with `argc` arguments that `arg(i)` reads. The arguments are copied once, into
/// the callee frame; `arg` may read the caller's frame or scratch area (H20 `call_args`).
/// A bound `f` (§10.4.1.1 `[[Call]]`) calls its innermost target with the bound `this` and
/// the bound arguments in front; its `[[Construct]]` goes through [`construct_with`].
pub(crate) fn call_with(
    cx: &mut Cx,
    f: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    if !is_callable(f) {
        let msg = format!("{} is not a function", crate::conv::describe(f));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if is_function(f) {
        return call_function_object(cx, f, this, argc, arg, new_target);
    }
    // A bound function: `construct_with` unwraps it before the call step.
    debug_assert!(new_target == UNDEFINED, "[[Construct]] of a bound function");
    // §10.4.1.1 steps 1-5: Call(target, boundThis, boundArgs ++ argumentsList).
    let u = bound::unwrap(cx, f, UNDEFINED)?;
    let Some(total) = u.bound.checked_add(argc) else {
        return stack_overflow(cx);
    };
    let bound = u.bound;
    let (pre, saved) = push_bound_args(cx, f, bound)?;
    let r = call_function_object(
        cx,
        u.target,
        u.this,
        total,
        |i| {
            if i < bound {
                // SAFETY: slot `i < bound` of the copy, under the scratch top until the
                // `restore_scratch` below.
                unsafe { *pre.add(i as usize) }
            } else {
                arg(i - bound)
            }
        },
        UNDEFINED,
    );
    restore_scratch(cx.rt(), saved);
    r
}

/// Copies the bound-argument prefix of a call of the bound function `f` (`bound` values,
/// `bound::copy_chain_args`) to the scratch area above the top frame, and raises the scratch
/// top past it: the GC scans the copy, and a frame push or a nested call from Rust starts
/// above it ([`top_of`]). Returns the copy and the scratch top to put back after the call.
/// RangeError if the VM stack is full. Not a GC point unless it throws.
fn push_bound_args(cx: &mut Cx, f: u64, bound: u32) -> Res<(*const u64, *mut u64)> {
    let stack = cx.rt().stack();
    let saved = stack.scratch_top();
    let start = top_of(cx, stack.fp());
    let end = start.wrapping_add(bound as usize);
    if end.addr() > cx.stack_limit.addr() {
        return stack_overflow(cx);
    }
    // SAFETY: [start, end) is inside the stack (end <= limit <= end of the stack) and above
    // the top frame and every scratch Value, so these writes clobber nothing live.
    let out = unsafe { core::slice::from_raw_parts_mut(start, bound as usize) };
    bound::copy_chain_args(f, out);
    // Every slot below `end` now holds a valid Value.
    stack.set_scratch_top(end);
    Ok((start, saved))
}

/// [`call_with`] for a FUNCTION `f` (a host or a bytecode function).
fn call_function_object(
    cx: &mut Cx,
    f: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    debug_assert!(is_function(f));
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let fi = function_info(f);
    let caller = rt.stack().fp();
    if fi.host.is_some() {
        return call_host(
            cx,
            f,
            fi,
            this,
            argc,
            arg,
            new_target,
            caller,
            EXIT_STUB.as_ptr(),
        );
    }
    // [[Call]] of a class constructor (§10.2.1 step 2), as in the `Call*` slow path.
    if new_target == UNDEFINED && fi.is_class_constructor() {
        let msg = format!(
            "Class constructor {} cannot be invoked without 'new'",
            fi.name
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if !fi.is_bytecode() || fi.slot.kind_byte() != crate::dispatch::DispatchKind::Interp as u8 {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    poll(cx, caller)?;
    let this = bind_this(fi, this, rt.global());
    let saved = rt.stack().scratch_top();
    let base = top_of(cx, caller);
    // SAFETY: `base` is the first free slot above the current top frame and its scratch area.
    let fp = unsafe {
        push_frame(
            base,
            cx.stack_limit,
            fi,
            f,
            this,
            argc,
            arg,
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
    rt.stack().clear_scratch();
    cx.budget -= 1;
    // SAFETY: `fp` is a frame for `fi` that `push_frame` just wrote; its code is verified.
    let r = unsafe { interp::run(cx, fi.code, fp) };
    rt.stack().set_fp(caller);
    restore_scratch(rt, saved);
    if r == 0 { Err(Throw) } else { Ok(r) }
}

/// Construct(F, args, newTarget) from Rust (§7.3.15), with `argc` arguments that `arg(i)`
/// reads. `new_target` must be a constructor; it gives the prototype of the new object
/// (OrdinaryCreateFromConstructor) and `new.target` in the callee. A derived class
/// constructor gets no object: its `this` starts uninitialized, as in the `Construct` slow
/// path. A bound `f` (§10.4.1.2) constructs its innermost target with the bound arguments in
/// front, and new.target becomes the target while it is the bound function.
pub(crate) fn construct_with(
    cx: &mut Cx,
    f: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    for c in [f, new_target] {
        if !object::is_constructor(c) {
            let msg = format!("{} is not a constructor", crate::conv::describe(c));
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
    }
    if is_function(f) {
        return construct_function_object(cx, f, argc, arg, new_target);
    }
    // §10.4.1.2 steps 1-6: Construct(target, boundArgs ++ argumentsList, newTarget), with
    // newTarget = target while it is the bound function. The copy of the bound arguments is
    // scanned at the GC point of OrdinaryCreateFromConstructor.
    let u = bound::unwrap(cx, f, new_target)?;
    let Some(total) = u.bound.checked_add(argc) else {
        return stack_overflow(cx);
    };
    let bound = u.bound;
    let (pre, saved) = push_bound_args(cx, f, bound)?;
    let r = construct_function_object(
        cx,
        u.target,
        total,
        |i| {
            if i < bound {
                // SAFETY: slot `i < bound` of the copy, under the scratch top until the
                // `restore_scratch` below.
                unsafe { *pre.add(i as usize) }
            } else {
                arg(i - bound)
            }
        },
        u.new_target,
    );
    restore_scratch(cx.rt(), saved);
    r
}

/// [`construct_with`] for a FUNCTION `f` with `[[Construct]]` (§10.2.2).
fn construct_function_object(
    cx: &mut Cx,
    f: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    let fi = function_info(f);
    if fi.host.is_some() {
        return call_function_object(cx, f, UNDEFINED, argc, arg, new_target);
    }
    let this = if fi.is_derived() {
        HOLE
    } else {
        ordinary_create_from_constructor(cx, new_target)?
    };
    let r = call_function_object(cx, f, this, argc, arg, new_target)?;
    // §10.2.2 steps 10-12, as `interp::construct_return` does for JS callers.
    Ok(if is_object(r) {
        r
    } else if this == HOLE {
        UNDEFINED
    } else {
        this
    })
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
