//! Frame push and calls (H14). JS→JS calls in one realm never recurse in Rust: the interpreter
//! pushes the callee frame and jumps to its code. Rust recursion happens only at the edges:
//!
//! | Path | How | Native stack |
//! |---|---|---|
//! | JS → JS (`Call*`, `Construct`) | push frame, jump (handler or `rt_call`); a callee of another realm: [`call_function_object`] in its realm (`slow::call`) | none; another realm: one Rust level, checked |
//! | JS → host function | push host frame, call the Rust fn | one Rust call, checked |
//! | Rust → JS (`call_function`, host callbacks, ToPrimitive) | push entry frame (return pc = EXIT stub), run the dispatch | one dispatch run, checked |
//! | generator resume (`next`/`return`/`throw`, [`run_resumed`]) | push a resumed frame (fp-3 = the generator, return pc = EXIT stub), run the dispatch | one dispatch run, checked |
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
//! Realms (H22-Q3): a callee of another realm runs with its realm current
//! (`realm::enter_realm_of` before the call, `realm::leave` after it, also after a throw).
//! [`call_host`], the bytecode path of [`call_function_object`] and [`run_resumed`] switch;
//! `slow::call` sends a bytecode callee of another realm through [`call_function_object`] or
//! [`construct_function_object`], so every cross-realm call is one of these Rust edges.
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
use crate::kinds::{OBJECT, cell_kind, is_bound, is_callable, is_cell_bits};
use crate::object::{self, function_context, function_info, is_function, is_object};
use crate::property;
use crate::realm;
use crate::runtime::{Atom, ErrorKind, Runtime, frame_top};
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
        // The header and the receiver first, so their inputs are dead before the loops.
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
        // The receiver and the arguments at fixed offsets from `fp` too (`base + n` is
        // `fp + RECEIVER`; argument `i` is at `fp + PARAM0 - i`), so the stores need no other
        // pointer register.
        *fp.offset(slot::RECEIVER) = this;
        for i in 0..argc {
            *fp.offset(slot::PARAM0 - i as isize) = arg(i);
        }
        fill_undefined(base, fp.offset(slot::PARAM0 + 1 - argc as isize));
        fill_undefined(fp, top);
    }
    Some(fp)
}

/// Writes undefined to every slot of `[from, to)`: the missing arguments and the registers of
/// a new frame. A plain loop, one store per slot: these ranges are short (most T0 functions
/// have fewer than 10 registers), and the vector loop that LLVM makes of a plain fill costs
/// more set-up instructions and registers than the T0 call handlers have free (M3-P2 PERF3:
/// a64 cachegrind t0-call -1.5% against the vector loop; a loop of 2 stores per pass, or one
/// counted down, costs more). The volatile store only keeps LLVM from making the vector loop
/// again; it is a plain store.
///
/// # Safety
/// `[from, to)` is writable stack memory.
#[inline(always)]
unsafe fn fill_undefined(from: *mut u64, to: *mut u64) {
    let mut p = from;
    while p < to {
        // SAFETY: `p` is in `[from, to)` (caller contract).
        unsafe { p.write_volatile(UNDEFINED) };
        p = p.wrapping_add(1);
    }
}

/// Pushes the frame of a resumed generator (FRAME 2): [`push_frame`] for a call (the
/// construct bit is clear), then `slot::NEW_TARGET` (fp - 3) holds `generator`, so
/// `LdaNewTarget` in the resumed frame gives the generator object (BytecodeGen emits
/// `LdaUndefined` for `new.target` in a generator body). Caller: [`run_resumed`].
///
/// # Safety
/// As for [`push_frame`]; `generator` is valid Value bits.
#[allow(clippy::too_many_arguments)]
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

/// OrdinaryCallBindThis (§10.2.1.2) with no allocation: the this value of a call of `fi`, or
/// `None` when step 6.b makes a wrapper object (a sloppy callee and a number, boolean,
/// string or symbol receiver); the caller then takes [`wrap_this`] (a cold path).
///
/// The receiver is tested before the strictness flag (M3-P2 PERF3): an object receiver (the
/// hot T0 call of a method) costs one cell test, one kind load and one compare, strict or
/// sloppy, with no flag test. A receiver is a language value, so a cell of kind 16 or more is
/// an object (kinds 64 and up are internal cells, never a receiver), and the upper bound of
/// `is_object` is not tested. Any other receiver then costs one flag test for a strict callee.
///
/// The caller wraps, not the callee prologue (H14-Q2):
///
/// ```text
/// call site                          primitive receiver of a sloppy bytecode callee
/// interp/ops.rs call_fast (2 cfgs)   None -> Flow::Cold -> slow::call
/// slow::call, enter_b_cold           None -> wrap_this before push_frame
/// call_function_object (from Rust)   None -> push_frame, then wrap_this into the receiver slot
/// call_host (host callee)            no bind, no wrap: the host function gets the raw receiver
/// native.rs enter_native, run_b      None -> push_frame, then wrap_this into the receiver slot
/// ```
#[inline(always)]
pub(crate) fn try_bind_this(fi: &FunctionInfo, this: u64, global: u64) -> Option<u64> {
    if is_cell_bits(this) {
        // An object goes in as it is; a string or a symbol (kinds below 16) needs the wrapper
        // for a sloppy callee only.
        let kind = cell_kind(this);
        debug_assert!(kind < 64, "an internal cell as a receiver");
        return (kind >= OBJECT.get() || fi.is_strict()).then_some(this);
    }
    if fi.is_strict() {
        return Some(this);
    }
    if this == UNDEFINED || this == NULL {
        Some(global)
    } else if this == HOLE {
        // The hole is not a language value: it goes in as it is (no global object, no wrapper).
        Some(this)
    } else {
        // A number or a boolean.
        None
    }
}

/// ToObject(`this`) for a call that [`try_bind_this`] refused (§10.2.1.2 step 6.b). GC point:
/// the caller keeps the callee and the arguments rooted. `slow::call` and `enter_b_cold` wrap
/// before `push_frame` (the arguments are in caller registers or the spread array);
/// `call_function_object` and the spike-native `enter_native`/`run_b` (`native::wrap_receiver`)
/// wrap after it, with the frame not yet published and under the scratch top, as their `arg`
/// may read memory the GC does not scan. The wrapper stays in a Rust local until it is stored.
#[cold]
#[inline(never)]
pub(crate) fn wrap_this(cx: &mut Cx, this: u64) -> Res<u64> {
    crate::conv::to_object(cx, this)
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
    // H20 R5: a host function runs in its own realm. Not a GC point; the caller's realm stays
    // in `saved_realm` (a Rust local) meanwhile.
    let saved_realm = realm::enter_realm_of(cx, f);
    let r = {
        let mut hcx = abi::Cx::new(cx);
        h(
            &mut hcx,
            Local::from_bits(this),
            args,
            Local::from_bits(new_target),
        )
        .map(|v| v.bits())
    };
    realm::leave(cx, saved_realm);
    // H20-Q3: pops the host frame, or makes the call of `h`'s tail-call request in its place.
    let r = host_return(cx, r, caller_fp, saved);
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
    r
}

/// Call(F, thisArg, args) from Rust (§7.3.14). Runs the interpreter for a bytecode callee.
pub(crate) fn call_function(cx: &mut Cx, f: u64, this: u64, args: &[u64]) -> Res<u64> {
    let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
    call_with(cx, f, this, argc, |i| args[i as usize], UNDEFINED)
}

/// Call(F, thisArg, args) or the call step of `[[Construct]]` (`new_target` not undefined)
/// from Rust, with `argc` arguments that `arg(i)` reads. The arguments are copied once, into
/// the callee frame, before the first GC point; `arg` may read the caller's frame or scratch
/// area (H20 `call_args`), or memory that the GC does not scan (`native::call_from_rust`).
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
    if !is_bound(f) {
        return call_proxy(cx, f, this, argc, arg, new_target); // a callable proxy (§10.5.12)
    }
    let u = bound::unwrap(cx, f, UNDEFINED)?;
    let Some(total) = u.bound.checked_add(argc) else {
        return stack_overflow(cx);
    };
    let bound = u.bound;
    let (pre, saved) = push_bound_args(cx, f, bound)?;
    let r = call_target(
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

/// [`call_with`] for a FUNCTION `f` (a host or a bytecode function). A bytecode `f` of
/// another realm runs with its realm current (the class-constructor TypeError too, §10.2.1
/// step 2: it is made in the callee's realm); `slow::call` sends cross-realm calls here.
pub(crate) fn call_function_object(
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
    // Not a GC point: `arg` may still read memory the GC does not scan (see below).
    let saved_realm = realm::enter_realm_of(cx, f);
    let r = call_bytecode(cx, f, fi, this, argc, arg, new_target, caller);
    realm::leave(cx, saved_realm);
    r
}

/// The bytecode part of [`call_function_object`], in the callee's realm.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn call_bytecode(
    cx: &mut Cx,
    f: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
    caller: *mut u64,
) -> Res<u64> {
    let rt = cx.rt();
    // [[Call]] of a class constructor (§10.2.1 step 2), as in the `Call*` slow path.
    if new_target == UNDEFINED && fi.is_class_constructor() {
        let msg = format!(
            "Class constructor {} cannot be invoked without 'new'",
            fi.name
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if !fi.is_bytecode() || fi.slot.kind_byte() != crate::dispatch::DispatchKind::Interp as u8 {
        // Native code (an AotBaseline unit function or a Baseline slot; MC-w30-2, M7-P5
        // A2-RW): a call from Rust (`Cx::call`, a builtin callback, a forwarded `apply` or
        // `call` that the T0 forward refused, a callee of another realm) enters it through the
        // Rust entry, with an entry frame. Construct stays refused here (MC-w29-8).
        if new_target == UNDEFINED
            && let Some(nfi) = crate::native::native_callee(f)
        {
            return crate::native::enter_from_rust(cx, f, nfi, this, argc, arg);
        }
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    poll(cx, caller)?;
    let bound_this = try_bind_this(fi, this, rt.global());
    let saved = rt.stack().scratch_top();
    let base = top_of(cx, caller);
    // No GC point before the push: `arg` may read memory that the GC does not scan (the
    // embedder slice of `native::call_from_rust`, the tests' `call_raw`), so the frame
    // copies the arguments first. A receiver to wrap goes in as it is, for now.
    // SAFETY: `base` is the first free slot above the current top frame and its scratch area.
    let fp = unsafe {
        push_frame(
            base,
            cx.stack_limit,
            fi,
            f,
            bound_this.unwrap_or(this),
            argc,
            arg,
            caller,
            EXIT_STUB.as_ptr(),
            function_context(f),
            new_target,
        )
    };
    // 1 unit per call, also when the frame does not fit (R3-2): a loop that catches the
    // RangeError of a full VM stack still reaches the next poll (M3-P2-STOP; the T0 and
    // spike-native paths take it the same way, M3-P2-D1).
    cx.budget -= 1;
    let Some(fp) = fp else {
        return stack_overflow(cx);
    };
    if bound_this.is_none() {
        // GC point (the wrap of a primitive receiver of a sloppy callee, H14-Q2). The frame is
        // not published yet: the scratch top covers it, so the GC scans the copied arguments
        // and `f`, and a stack walk does not see the frame.
        rt.stack().set_scratch_top(frame_top(fp));
        match wrap_this(cx, this) {
            // SAFETY: the receiver slot of the frame that `push_frame` just wrote.
            Ok(w) => unsafe { *fp.offset(slot::RECEIVER) = w },
            Err(e) => {
                restore_scratch(rt, saved);
                return Err(e);
            }
        }
    }
    rt.stack().set_fp(fp);
    rt.stack().clear_scratch();
    // SAFETY: `fp` is a frame for `fi` that `push_frame` just wrote; its code is verified.
    let r = unsafe { interp::run(cx, fi.code, fp) };
    rt.stack().set_fp(caller);
    restore_scratch(rt, saved);
    if r == 0 { Err(Throw) } else { Ok(r) }
}

/// Resumes a suspended generator from Rust (GeneratorResume, §27.5.3.3 steps 5-9): pushes a
/// resumed frame of the bytecode generator function `f` (`push_resume_frame`, fp-3 =
/// `generator`) with `this`, the `argc` saved parameters that `arg(i)` reads and the saved
/// `context`, and runs the dispatch from the function's first instruction until the frame
/// suspends again or completes (its return pc is the EXIT stub). One checked native level,
/// as [`call_with`]: a RangeError when the native or the VM stack is full, before the frame
/// exists. Returns the accumulator of the `SuspendGenerator` or `Return` that left the frame.
pub(crate) fn run_resumed(
    cx: &mut Cx,
    f: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    context: u64,
    generator: u64,
) -> Res<u64> {
    debug_assert!(is_function(f));
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let fi = function_info(f);
    if !fi.is_bytecode() || fi.slot.kind_byte() != crate::dispatch::DispatchKind::Interp as u8 {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    let caller = rt.stack().fp();
    // The generator's body runs in the realm of its function (§27.5.3.3 runs it in the
    // generator's own execution context). Not a GC point.
    let saved_realm = realm::enter_realm_of(cx, f);
    let r = resume_here(cx, f, fi, this, argc, arg, context, generator, caller);
    realm::leave(cx, saved_realm);
    r
}

/// The frame push and run of [`run_resumed`], in the generator function's realm.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn resume_here(
    cx: &mut Cx,
    f: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    context: u64,
    generator: u64,
    caller: *mut u64,
) -> Res<u64> {
    let rt = cx.rt();
    poll(cx, caller)?;
    let saved = rt.stack().scratch_top();
    let base = top_of(cx, caller);
    // SAFETY: `base` is the first free slot above the current top frame and its scratch area.
    let fp = unsafe {
        push_resume_frame(
            base,
            cx.stack_limit,
            fi,
            f,
            this,
            argc,
            arg,
            caller,
            EXIT_STUB.as_ptr(),
            context,
            generator,
        )
    };
    let Some(fp) = fp else {
        return stack_overflow(cx);
    };
    rt.stack().set_fp(fp);
    rt.stack().clear_scratch();
    cx.budget -= 1;
    // SAFETY: `fp` is a frame for `fi` that `push_resume_frame` just wrote; its code is
    // verified.
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
    if !is_bound(f) {
        return construct_proxy(cx, f, argc, arg, new_target); // a callable proxy (§10.5.13)
    }
    let u = bound::unwrap(cx, f, new_target)?;
    let Some(total) = u.bound.checked_add(argc) else {
        return stack_overflow(cx);
    };
    let bound = u.bound;
    let (pre, saved) = push_bound_args(cx, f, bound)?;
    let r = construct_target(
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

/// [`construct_with`] for a FUNCTION `f` with `[[Construct]]` (§10.2.2). The receiver of a
/// base constructor is made first, in the caller's context (step 3; its prototype falls back
/// to newTarget's realm), then the call runs in `f`'s realm ([`call_function_object`]);
/// `slow::call` sends a construct of a bytecode function of another realm here.
pub(crate) fn construct_function_object(
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
    // §10.2.2 steps 10-12, as `interp::construct_return` does for JS callers: after the call,
    // in the caller's realm (`call_function_object` switched back).
    if is_object(r) {
        Ok(r)
    } else if this != HOLE {
        Ok(this)
    } else {
        let (kind, msg) = interp::derived_result_error(r);
        Err(throw_error(cx, kind, msg))
    }
}

/// OrdinaryCreateFromConstructor(F, "%Object.prototype%") (§10.1.13): a `prototype` that is
/// not an object falls back to `%Object.prototype%` of GetFunctionRealm(F) (§10.1.14 step 4).
///
/// A FUNCTION `f` whose `prototype` is an own data property already (not the lazy one,
/// `exotic.rs`) reads it with one shape lookup; every other `f` (a lazy `prototype`, an
/// accessor, a bound function, a Proxy) takes the generic `[[Get]]` (M4-P1 OBJ-ALLOC; the
/// probe: `new` was 16% of earley-boyer samples, mostly this `[[Get]]`).
pub(crate) fn ordinary_create_from_constructor(cx: &mut Cx, f: u64) -> Res<u64> {
    let rt = cx.rt();
    let key = rt.atom(Atom::Prototype);
    let own = if is_function(f) {
        object::own_get(f, key).filter(|&(_, a)| a & object::attr::ACCESSOR == 0)
    } else {
        None
    };
    let p = match own {
        Some((v, _)) => v,
        None => property::get(cx, f, key)?,
    };
    if !is_object(p) {
        let r = realm::try_get_function_realm(cx, f)?;
        let rt = cx.rt();
        let proto = realm::intrinsic_in(rt, r, crate::builtins::IntrinsicId::ObjectPrototype);
        return Ok(object::new_object(rt, proto));
    }
    // The in-object capacity from `f`'s own `this.<name> =` stores (`slack.rs`). GC point
    // below: `p` lives in a Rust local.
    Ok(match crate::slack::instance_capacity(rt, f, p) {
        Some(n) => object::new_object_with_capacity(rt, p, n),
        None => object::new_object(rt, p),
    })
}

/// Interrupt poll at a function entry from Rust.
fn poll(cx: &mut Cx, fp: *mut u64) -> Res<()> {
    if cx.budget <= 0 && interp::slow::interrupt(cx, fp) == 0 {
        return Err(Throw);
    }
    Ok(())
}

/// Most arguments a call keeps inline in an [`ArgBuf`] (M4-P1 OBJ-ALLOC, PERF3 step-2 item
/// 1): the cold call path decodes and copies up to this many with no heap allocation.
pub(crate) const INLINE_ARGS: usize = 8;

/// A short list of `T`: up to [`INLINE_ARGS`] items inline, more in a heap `Vec` (a long
/// argument list, a spread, bound arguments). It derefs to `[T]`.
///
/// The cold call path keeps register offsets ([`CallSite::args`]) and the argument copy of
/// `slow::call` in it. That copy holds Value bits that stay rooted elsewhere (the caller's
/// registers, a bound function, a spread array that a stack slot keeps), as the `Vec` before
/// it did; the inline part is on the native stack, which the GC scans conservatively anyway.
pub(crate) struct ArgBuf<T: Copy + Default> {
    len: usize,
    inline: [T; INLINE_ARGS],
    /// All items when `len > INLINE_ARGS`; empty (no allocation) otherwise.
    spill: Vec<T>,
}

impl<T: Copy + Default> ArgBuf<T> {
    #[inline]
    pub(crate) fn new() -> Self {
        ArgBuf {
            len: 0,
            inline: [T::default(); INLINE_ARGS],
            spill: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn push(&mut self, v: T) {
        if self.len < INLINE_ARGS {
            self.inline[self.len] = v;
        } else {
            if self.len == INLINE_ARGS {
                self.spill.reserve(INLINE_ARGS * 2);
                self.spill.extend_from_slice(&self.inline);
            }
            self.spill.push(v);
        }
        self.len += 1;
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let v = self[self.len - 1];
        if self.len > INLINE_ARGS {
            self.spill.pop();
            if self.len - 1 == INLINE_ARGS {
                self.inline.copy_from_slice(&self.spill);
                self.spill.clear();
            }
        }
        self.len -= 1;
        Some(v)
    }

    /// Puts `n` copies of `fill` in front of the items.
    pub(crate) fn prepend(&mut self, n: usize, fill: T) {
        if n == 0 {
            return;
        }
        let total = self.len + n;
        if total <= INLINE_ARGS {
            self.inline.copy_within(0..self.len, n);
            self.inline[..n].fill(fill);
        } else {
            let mut v = Vec::with_capacity(total);
            v.resize(n, fill);
            v.extend_from_slice(&self[..]);
            self.spill = v;
        }
        self.len = total;
    }
}

impl<T: Copy + Default> core::ops::Deref for ArgBuf<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        if self.len <= INLINE_ARGS {
            &self.inline[..self.len]
        } else {
            &self.spill
        }
    }
}

impl<T: Copy + Default> core::ops::DerefMut for ArgBuf<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        if self.len <= INLINE_ARGS {
            &mut self.inline[..self.len]
        } else {
            &mut self.spill
        }
    }
}

impl<T: Copy + Default> FromIterator<T> for ArgBuf<T> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(it: I) -> Self {
        let mut b = ArgBuf::new();
        for v in it {
            b.push(v);
        }
        b
    }
}

/// The arguments of a call instruction, decoded from its bytes (slow paths only).
pub(crate) struct CallSite {
    pub callee: u64,
    pub this: u64,
    /// Register slot offsets (from fp) of the arguments, in order (no heap allocation for up
    /// to [`INLINE_ARGS`]).
    pub args: ArgBuf<isize>,
    pub next_pc: *const u8,
    pub construct: bool,
    pub spread: bool,
    /// The feedback slot operand (the Call slot of a `Construct*` site: `construct_ic.rs`).
    pub slot: u32,
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
    let list = |i: usize, n: usize, skip: usize| -> ArgBuf<isize> {
        let first = insn.operand(i) as isize;
        (skip as isize..n as isize).map(|k| first + k).collect()
    };
    let one = |a: isize| -> ArgBuf<isize> { core::iter::once(a).collect() };
    let next_pc = pc.wrapping_add(insn.size() as usize);
    let callee = rd(reg(0));
    let mut slot = 0;
    let (this, args, construct, spread) = match insn.opcode {
        O::CallAnyReceiver | O::CallProperty | O::CallWithSpread => {
            let n = insn.operand(2) as usize;
            // The list holds the receiver, then the arguments.
            let this = if n > 0 {
                rd(insn.operand(1) as isize)
            } else {
                UNDEFINED
            };
            (this, list(1, n, 1), false, insn.opcode == O::CallWithSpread)
        }
        O::CallProperty0 => (rd(reg(1)), ArgBuf::new(), false, false),
        O::CallProperty1 => (rd(reg(1)), one(reg(2)), false, false),
        O::CallProperty2 => (
            rd(reg(1)),
            [reg(2), reg(3)].into_iter().collect(),
            false,
            false,
        ),
        O::CallUndefinedReceiver => {
            let n = insn.operand(2) as usize;
            (UNDEFINED, list(1, n, 0), false, false)
        }
        O::CallUndefinedReceiver0 => (UNDEFINED, ArgBuf::new(), false, false),
        O::CallUndefinedReceiver1 => (UNDEFINED, one(reg(1)), false, false),
        O::CallUndefinedReceiver2 => (
            UNDEFINED,
            [reg(1), reg(2)].into_iter().collect(),
            false,
            false,
        ),
        O::Construct | O::ConstructWithSpread => {
            let n = insn.operand(2) as usize;
            slot = insn.unsigned(3);
            (
                UNDEFINED,
                list(1, n, 0),
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
        slot,
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

// ---- the x86-64 T0 call fast path (PERF3 item 2) ----------------------------------------

/// The `fp` that [`push_frame`] gives a call frame of `fi` with `argc` arguments at `base`, or
/// `None` if that frame would end above `limit`: the size test of [`push_frame`] alone, with no
/// write. The x86-64 T0 call fast path (`interp/ops.rs` `call_fast`, PERF3 item 2) makes this
/// test before its count and its budget unit, then writes the frame with
/// [`fill_missing_args`], [`write_call_args`] and [`write_call_header`]; the four together
/// write what [`push_frame`] writes (test `tests::calls::the_fast_frame_write_equals_push_frame`).
/// The forwarded call of the T0 call fast path on the other targets ([`forward_frame`], M7-P4
/// LEVER-3) writes its frame with the same four.
///
/// The `& !7` changes no bit (`base` is a slot address, so `fp` is 8-byte aligned). It keeps
/// LLVM from folding `base + 8 * n + 72` into every later address, so the fast path holds the
/// new `fp` in one register, not `base` and `n` in two.
#[inline(always)]
pub(crate) fn call_frame_fp(
    base: *mut u64,
    limit: *mut u64,
    fi: &FunctionInfo,
    argc: u32,
) -> Option<*mut u64> {
    let n = argc.max(fi.parameter_count) as usize;
    let fp = base.wrapping_add(n + 1 + HEADER).map_addr(|a| a & !7);
    let top = fp.wrapping_add(fi.register_count as usize);
    (top.addr() <= limit.addr()).then_some(fp)
}

/// Writes undefined to the missing-argument slots of the call frame at `fp` (from
/// [`call_frame_fp`] of `base`): `[base, fp + PARAM0 + 1 - argc)`, as [`push_frame`] does.
///
/// # Safety
/// `fp` is `call_frame_fp(base, limit, fi, argc)` (not `None`) with the [`push_frame`]
/// contract for `base` and `limit`.
#[inline(always)]
pub(crate) unsafe fn fill_missing_args(base: *mut u64, fp: *mut u64, argc: u32) {
    // SAFETY: the slots lie in the frame `call_frame_fp` placed below `limit`, above every
    // live frame (caller contract).
    unsafe { fill_undefined(base, fp.offset(slot::PARAM0 + 1 - argc as isize)) }
}

/// Writes the receiver and the arguments of the call frame at `fp` (from [`call_frame_fp`])
/// that [`push_frame`] writes; `this` is the receiver after [`try_bind_this`] with the global
/// object (see [`bound_with_global`]). The x86-64 T0 call fast path writes them right after
/// its frame size test on the forms with a receiver operand (M6-P1 CBC), so that their values
/// hold no register across its later tests; the slots lie above the top frame until the frame
/// is complete, where no GC root scan and no reader looks, so a cold exit after this write
/// leaves them as garbage that the cold path's frame push writes again. `LIST` is false for
/// the fixed-count forms (`argc` is a constant up to 2: straight stores) and true for the
/// register-list forms: one pointer loop (a counted `for` loop is unrolled by LLVM, and both
/// copies in one function cost the list forms 1-2 more callee-saved registers on x86-64).
///
/// # Safety
/// `fp` is `call_frame_fp(base, limit, fi, argc)` (not `None`) with the [`push_frame`]
/// contract for `base` and `limit`, and `arg(i)` returns valid Value bits for `i < argc`.
#[inline(always)]
pub(crate) unsafe fn write_call_args<const LIST: bool>(
    fp: *mut u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) {
    // SAFETY: [fp + RECEIVER - argc, fp + RECEIVER] lies in the frame `call_frame_fp` placed
    // below `limit`, above every live frame (caller contract), so these writes clobber nothing
    // live.
    unsafe {
        *fp.offset(slot::RECEIVER) = this;
        if !LIST {
            debug_assert!(argc <= 2);
            if argc != 0 {
                *fp.offset(slot::PARAM0) = arg(0);
                if argc == 2 {
                    *fp.offset(slot::PARAM0 - 1) = arg(1);
                }
            }
            return;
        }
        let mut p = fp.offset(slot::PARAM0);
        let end = p.wrapping_sub(argc as usize);
        let mut i = 0;
        while p > end {
            // Volatile as in `fill_undefined`: a plain store; LLVM makes no vector loop.
            p.write_volatile(arg(i));
            p = p.wrapping_sub(1);
            i += 1;
        }
    }
}

/// Writes the header (new.target undefined) and the registers of the call frame at `fp` that
/// [`push_frame`] writes; [`fill_missing_args`] and [`write_call_args`] write the slots below
/// the header. `context` runs at its store (the fast path loads the callee's context there,
/// not before: one register less for the list forms on x86-64).
///
/// # Safety
/// `fp` is `call_frame_fp(base, limit, fi, argc)` (not `None`) with the [`push_frame`]
/// contract for `base` and `limit`.
#[inline(always)]
pub(crate) unsafe fn write_call_header(
    fp: *mut u64,
    fi: &FunctionInfo,
    callee: u64,
    argc: u32,
    caller_fp: *mut u64,
    return_pc: *const u8,
    context: impl FnOnce() -> u64,
) {
    // SAFETY: [fp + CONTEXT .. fp + register_count) lies in the frame `call_frame_fp` placed
    // below `limit`, above every live frame (caller contract).
    unsafe {
        *fp.offset(slot::CALLER_FP) = native_of(caller_fp);
        *fp.offset(slot::RETURN_PC) = native_of(return_pc);
        *fp.offset(slot::CALLEE) = callee;
        *fp.offset(slot::ARGC) = int32_bits(argc as i32);
        *fp.offset(slot::FUNCTION_INFO) = native_of(fi);
        *fp.offset(slot::NEW_TARGET) = UNDEFINED;
        *fp.offset(slot::CONTEXT) = context();
        *fp.offset(slot::BYTECODE_OFFSET) = int32_bits(0);
        fill_undefined(fp, fp.wrapping_add(fi.register_count as usize));
    }
}

/// The global object step of [`try_bind_this`] after the fact: for `b =
/// try_bind_this(fi, this, UNDEFINED)` (not `None`), `bound_with_global(fi, b, global)` is
/// `try_bind_this(fi, this, global)`. Only a sloppy callee with an undefined or null receiver
/// gives undefined there (an object, a strict receiver and the hole come back as they are, and
/// none of them is undefined for a sloppy callee). The T0 call fast path tests the receiver
/// first and loads the global object only at its frame write (x86-64, PERF3 item 2).
#[cfg_attr(
    any(feature = "spike-native", not(target_arch = "x86_64")),
    allow(dead_code)
)]
#[inline(always)]
pub(crate) fn bound_with_global(fi: &FunctionInfo, bound: u64, global: u64) -> u64 {
    if bound == UNDEFINED && !fi.is_strict() {
        global
    } else {
        bound
    }
}

// ---- Proxy [[Call]] and [[Construct]] (§10.5.12-13; M5-P3 PROXY2) ------------------------------
//
// A callable proxy never gets a frame. [`call_with`] and [`construct_with`] send it here, and so
// does `slow::call` for a bytecode caller (through them). Its `[[Call]]` and `[[Construct]]`
// (`proxy.rs`) run its trap or forward to its target, nested in Rust, with the arguments copied
// once to the scratch area ([`push_args`]), so the GC sees them while the trap's argument array
// is made. The innermost target of a bound function may be a callable proxy too: the bound
// paths call [`call_target`] and [`construct_target`], which take either kind.
//
// ```text
// call_with(f) ──── FUNCTION ─────────> call_function_object(f)
//             ├──── PROXY_CALLABLE ───> call_proxy: args to scratch ─> proxy::call (trap "apply")
//             └──── BOUND: unwrap(f) ─> call_target(target, boundThis, bound ++ args)
// construct_with(f, nt) ── FUNCTION ──> construct_function_object(f, nt)
//             ├──── PROXY_CALLABLE ───> construct_proxy ─> proxy::construct (trap "construct")
//             └──── BOUND: unwrap(f) ─> construct_target(target, bound ++ args, nt)
// ```

/// The call step of a bound function's innermost target (`bound::unwrap`): a FUNCTION
/// ([`call_function_object`]) or a callable proxy ([`call_proxy`]).
#[inline(always)]
fn call_target(
    cx: &mut Cx,
    f: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    if is_function(f) {
        call_function_object(cx, f, this, argc, arg, new_target)
    } else {
        call_proxy(cx, f, this, argc, arg, new_target)
    }
}

/// The construct step of a bound function's innermost target (§10.4.1.2 steps 1-6:
/// Construct(target, boundArgs ++ argumentsList, newTarget), with newTarget = target while it
/// is the bound function; the copy of the bound arguments is scanned at the GC point of
/// OrdinaryCreateFromConstructor): a FUNCTION ([`construct_function_object`]) or a callable
/// proxy ([`construct_proxy`]).
#[inline(always)]
fn construct_target(
    cx: &mut Cx,
    f: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    if is_function(f) {
        construct_function_object(cx, f, argc, arg, new_target)
    } else {
        construct_proxy(cx, f, argc, arg, new_target)
    }
}

/// Copies the `argc` arguments that `arg(i)` reads to the scratch area above the top frame,
/// and raises the scratch top past them, as [`push_bound_args`] does: the GC scans the copy.
/// `arg` may read the caller's scratch Values (below the copy). Returns the copy and the
/// scratch top to put back after the call. RangeError if the VM stack is full. Not a GC point
/// unless it throws.
fn push_args(cx: &mut Cx, argc: u32, arg: impl Fn(u32) -> u64) -> Res<(*const u64, *mut u64)> {
    let stack = cx.rt().stack();
    let saved = stack.scratch_top();
    let start = top_of(cx, stack.fp());
    let end = start.wrapping_add(argc as usize);
    if end.addr() > cx.stack_limit.addr() || end.addr() < start.addr() {
        return stack_overflow(cx);
    }
    for i in 0..argc {
        // SAFETY: [start, end) is inside the stack (end <= limit <= end of the stack) and
        // above the top frame and every scratch Value, so these writes clobber nothing live.
        unsafe { *start.add(i as usize) = arg(i) };
    }
    // Every slot below `end` now holds a valid Value.
    stack.set_scratch_top(end);
    Ok((start, saved))
}

/// `[[Call]]` of the callable proxy `p` (§10.5.12) with `argc` arguments that `arg(i)` reads
/// (any memory, as for [`call_with`]): they go to the scratch area first ([`push_args`]),
/// then `proxy::call` runs the trap or forwards. `new_target` is undefined: a construct takes
/// [`construct_proxy`]. GC point, runs JS.
#[cold]
#[inline(never)]
fn call_proxy(
    cx: &mut Cx,
    p: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    debug_assert!(
        new_target == UNDEFINED,
        "[[Construct]] of a proxy: construct_proxy"
    );
    let (args, saved) = push_args(cx, argc, arg)?;
    // SAFETY: `argc` Values at `args`, under the scratch top until the `restore_scratch`
    // below; nested frames and scratch use start above it ([`top_of`]).
    let list = unsafe { core::slice::from_raw_parts(args, argc as usize) };
    let r = crate::proxy::call(cx, p, this, list);
    restore_scratch(cx.rt(), saved);
    r
}

/// `[[Construct]]` of the callable proxy `p` (§10.5.13; [`construct_with`] checked
/// IsConstructor of `p` and `new_target`), as [`call_proxy`]. GC point, runs JS.
#[cold]
#[inline(never)]
fn construct_proxy(
    cx: &mut Cx,
    p: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
    new_target: u64,
) -> Res<u64> {
    let (args, saved) = push_args(cx, argc, arg)?;
    // SAFETY: as in [`call_proxy`].
    let list = unsafe { core::slice::from_raw_parts(args, argc as usize) };
    let r = crate::proxy::construct(cx, p, list, new_target);
    restore_scratch(cx.rt(), saved);
    r
}

// ---- tail calls from host functions (H20-Q3) ------------------------------------------------
//
// PrepareForTailCall (§9.4.5) for a host function: `Cx::tail_call_args` (abi/mod.rs) makes a
// request ([`request_tail`]) and returns the marker `HOLE`; [`call_host`] then pops the host
// frame and makes the call in its place ([`host_return`], [`tail_call`]), as V8's
// Function.prototype.call shifts its arguments and tail-calls Call. The request is three
// words in header slots of the host frame that a host frame does not use once its function
// has returned (a host frame has no bytecode and no context, and `Args::new_target` reads
// its new.target slot only for a `[[Construct]]`):
//
// ```text
// host frame:  [args .. receiver] CALLER_FP RETURN_PC CALLEE ARGC FUNCTION_INFO
//              NEW_TARGET <- f   CONTEXT <- this   BYTECODE_OFFSET <- marker   | fp
// marker (int32): 0 = no request (as `push_frame` writes it); from + 1 = a request whose
//                 argument k is host argument from + k
// ```
//
// The frame slots are scanned as every frame slot, so `f` and `this` stay rooted until the
// host frame pops. The callee frame then goes at the host frame's base (the comment above
// [`tail_call`]): its arguments stay where they are, and the host frame takes no stack room
// while the callee runs. A bytecode callee still runs in a nested dispatch run, one native
// level, as from `call_function_object`; the host function's Rust frames are gone.

/// Makes a tail-call request of the running host function: Call(`f`, `this`, args) after
/// its host frame is gone, where the `len` args are a tail of its arguments: argument `k` at
/// `a0 - k` (an `Args::rest`). Returns false, and changes nothing, when the request cannot be
/// made: `f` is not a FUNCTION (a bound function or a proxy is called at once, in the host
/// function's realm, as before), the top frame is not a host frame, or the args are not a
/// tail of its arguments. Not a GC point.
#[inline]
pub(crate) fn request_tail(cx: &Cx, f: u64, this: u64, a0: *const u64, len: u32) -> bool {
    let fp = cx.rt().stack().fp();
    if !is_function(f) || fp.is_null() || FrameRef(fp).info().host.is_none() {
        return false;
    }
    // `a0` is argument `from` of this frame: `fp + PARAM0 - from`, `from + len = argc`.
    let gap = fp
        .wrapping_offset(slot::PARAM0)
        .addr()
        .wrapping_sub(a0.addr());
    let from = gap / 8;
    if !gap.is_multiple_of(8) || from + len as usize != FrameRef(fp).argc() as usize {
        return false;
    }
    // SAFETY: header slots of the live host frame `fp`; after its function returns nothing
    // reads them but `host_hole` (module comment above). The words are valid Values.
    unsafe {
        *fp.offset(slot::NEW_TARGET) = f;
        *fp.offset(slot::CONTEXT) = this;
        *fp.offset(slot::BYTECODE_OFFSET) = crate::frame::int32_bits(from as i32 + 1);
    }
    true
}

/// The end of a host call ([`call_host`]): the host frame pops (the top frame goes back to
/// `caller_fp`) and the scratch top goes back to `saved`. When the host function returned
/// the marker `HOLE`, [`host_hole`] runs the call of its tail-call request, if it made one.
/// The host frame is still the top frame here (every call from Rust puts the top frame back
/// when it returns), so this path keeps no more values live across the host function than
/// before.
#[inline(always)]
fn host_return(cx: &mut Cx, r: Res<u64>, caller_fp: *mut u64, saved: *mut u64) -> Res<u64> {
    if matches!(r, Ok(HOLE)) {
        return host_hole(cx, caller_fp, saved);
    }
    let rt = cx.rt();
    rt.stack().set_fp(caller_fp);
    restore_scratch(rt, saved);
    r
}

/// [`host_return`] for the result `HOLE`: the call of the tail-call request in the host frame
/// (the top frame) if it holds one (its marker slot is not 0; [`tail_call`]), else the pop
/// and the result `HOLE`, as before.
#[cold]
#[inline(never)]
fn host_hole(cx: &mut Cx, caller_fp: *mut u64, saved: *mut u64) -> Res<u64> {
    let fp = cx.rt().stack().fp();
    debug_assert!(FrameRef(fp).info().host.is_some() && FrameRef(fp).caller().0 == caller_fp);
    // SAFETY: a header slot of the host frame `fp`, still on the stack.
    if unsafe { *fp.offset(slot::BYTECODE_OFFSET) } != int32_bits(0) {
        return tail_call(cx, fp, caller_fp, saved);
    }
    let rt = cx.rt();
    rt.stack().set_fp(caller_fp);
    restore_scratch(rt, saved);
    Ok(HOLE)
}

/// The call of the tail-call request in the host frame at `fp` (pushed at `base` above
/// `caller_fp`; [`request_tail`]), after the frame is gone: Call(f, this, args) through
/// [`call_function_object`], so a host callee runs in its realm and a bytecode callee in a
/// nested dispatch run, also in its realm (`f` is a FUNCTION). The host function's realm is
/// left already.
///
/// The callee frame fits in place, at `base` (a host frame of a call has `argc` arguments
/// and no registers, so `base = fp - argc - 1 - HEADER`): its `fp` is
/// `base + max(len, parameter_count) + 1 + HEADER`, at or above `fp - from` (as
/// `len = argc - from`), so callee argument `k` goes to a slot at or above host argument
/// `from + k`, the one it is read from. [`push_frame`] writes the header and the receiver
/// first (above every argument it reads), then argument `k` right after it reads it, then the
/// missing arguments and the registers: so a write lands only on an argument read before it.
///
/// GC: the scratch top comes down to `base` and the push starts there. On the success path
/// nothing from here to that `push_frame` allocates (`call_function_object`:
/// `native_stack_ok`, the realm switch, then the `call_host` push, or `call_bytecode`'s
/// checks, the interrupt poll (its callbacks run no JS and make no Value), `try_bind_this` and
/// the push), so the host frame's slots need no root meanwhile (`f` and `this` are Rust
/// locals, which the conservative stack scan sees). The GC-stress tests of `tail_tests` below
/// check it with arguments that only the host frame roots. Every other path throws, and then
/// no argument is read again.
#[inline(always)]
fn tail_call(cx: &mut Cx, fp: *mut u64, caller_fp: *mut u64, saved: *mut u64) -> Res<u64> {
    // SAFETY: header slots of the host frame `fp`, still on the stack (`request_tail`).
    let (f, this, marker) = unsafe {
        (
            *fp.offset(slot::NEW_TARGET),
            *fp.offset(slot::CONTEXT),
            crate::frame::int32_of(*fp.offset(slot::BYTECODE_OFFSET)),
        )
    };
    debug_assert!(is_function(f) && marker > 0);
    let argc = FrameRef(fp).argc();
    let from = (marker - 1) as u32;
    debug_assert!(from <= argc);
    let a0 = fp
        .wrapping_offset(slot::PARAM0 - from as isize)
        .cast_const();
    // The host frame's base: `argc` arguments, the receiver and the header (no registers).
    let base = fp.wrapping_sub(argc as usize + 1 + HEADER);
    let rt = cx.rt();
    // The host frame is gone: a stack walk from here sees the caller as the top frame, and a
    // push from here starts at `base` (`top_of`).
    rt.stack().set_fp(caller_fp);
    rt.stack().set_scratch_top(base);
    // SAFETY: argument `k < argc - from` of the host frame; `call_function_object` reads each
    // one in its push before it writes over it (above).
    let r = call_function_object(
        cx,
        f,
        this,
        argc - from,
        |k| unsafe { *a0.sub(k as usize) },
        UNDEFINED,
    );
    restore_scratch(rt, saved);
    r
}

/// H20-Q3 tail calls through `Cx::tail_call_args`, from hand-assembled bytecode (ep9_vm tests
/// cannot compile JS): the argument shift, missing and extra arguments, a host callee, a
/// chain, the receiver wrap of a sloppy callee (a GC point after the in-place push), the
/// class-constructor TypeError, a value that is not a function, and the VM stack room.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tail_tests {
    use ep9_bytecode::{
        FeedbackSlotKind, FunctionFlags, FunctionId, FunctionKind, RegisterList, VerifiedUnit,
    };

    use crate::tests::{Asm, config, isolate, meta, on_big_stack, take_output, unit};
    use crate::{Args, Cx, Dispatch, IsolateConfig, JsResult, Local};

    /// `fwd(f, thisArg, ...args)`: Call(f, thisArg, args) as a tail call (the form of
    /// Function.prototype.call, with the callee as argument 0).
    fn fwd<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        cx.tail_call_args(args.get(0), args.get(1), args.rest(2))
    }

    /// `fwdNow(f, thisArg, ...args)`: the same call made at once, inside the host frame.
    fn fwd_now<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        cx.call_args(args.get(0), args.get(1), args.rest(2))
    }

    /// `keep(fwd, f, ...args)`: calls `fwd(f, undefined, ...args)` from Rust while an
    /// ArgsBuilder holds 41 under the scratch top; returns "kept" if it still holds 41 after
    /// the call (the tail callee's frame must go above the caller's scratch Values).
    fn keep<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let mut b = cx.args_builder();
        b.push(cx, Local::int32(41))?;
        let mut list = cx.args_builder();
        list.push(cx, args.get(1))?;
        list.push(cx, Local::undefined())?;
        for v in args.rest(2).iter() {
            list.push(cx, v)?;
        }
        cx.call_built(args.get(0), Local::undefined(), list)?;
        let kept = b.get(0).as_number() == Some(41.0);
        drop(b);
        Ok(cx.key(if kept { "kept" } else { "clobbered" }).value())
    }

    /// Runs `make()` with `fwd` and `fwdNow` registered; returns the result text and the
    /// printed lines; checks the heap after the run.
    fn run(
        config: IsolateConfig,
        make: impl FnOnce() -> VerifiedUnit + Send + 'static,
    ) -> (String, Vec<String>) {
        on_big_stack(move || {
            let mut iso = isolate(config);
            iso.register_host_function("fwd", 2, fwd);
            iso.register_host_function("fwdNow", 2, fwd_now);
            iso.register_host_function("keep", 2, keep);
            take_output();
            let r = iso
                .with_cx_checkpoint(|cx| cx.run_script(make(), "tail.js").map(|v| cx.display(v)));
            let out = take_output();
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            (r.unwrap(), out)
        })
    }

    /// `try { body } catch (e) { print(e.message); }`
    fn try_print_message(s: &mut Asm, body: impl FnOnce(&mut Asm)) {
        let (ctx, t) = (s.b.new_register(), s.b.new_register());
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
        s.b.bind(start);
        body(s);
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        s.b.star(t);
        s.get_named(t, "message");
        s.print_acc();
        s.b.bind(done);
    }

    /// `acc = g(l[0], .., l[n - 1])` for the global `g` (clobbers `s.f`).
    fn call_global(s: &mut Asm, g: &str, l: RegisterList, n: u32) {
        s.lda_global(g);
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver(
            f,
            RegisterList::new(l.get(0).local_index().unwrap(), n),
            n,
            s.call,
        );
    }

    fn semantics() -> VerifiedUnit {
        // f1: function show(a, b, c) { "use strict"; print(this); print(a); print(b);
        //     print(c); return "r"; }
        let mut m = meta("show", 3, 0, 0, 0);
        m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut show = Asm::new(m);
        let recv = show.b.receiver();
        show.b.ldar(recv);
        show.print_acc();
        for i in 0..3 {
            let p = show.b.parameter(i);
            show.b.ldar(p);
            show.print_acc();
        }
        show.lda_str("r");
        show.b.return_();
        // f2: function sloppyThis() { return typeof this; }
        let mut sloppy = Asm::new(meta("sloppyThis", 0, 0, 0, 0));
        let ts = sloppy.b.feedback_slot(FeedbackSlotKind::TypeOf);
        let recv = sloppy.b.receiver();
        sloppy.b.ldar(recv).type_of(ts).return_();
        // f3: class A { constructor() {} }
        let mut ma = meta("A", 0, 0, 0, 0);
        ma.kind = FunctionKind::ClassConstructor;
        ma.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut a = Asm::new(ma);
        a.b.lda_undefined().return_();

        let mut s = Asm::script(0);
        for (id, name) in [(1, "show"), (2, "sloppyThis"), (3, "A")] {
            let k = s.b.function_constant(FunctionId(id));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        let l = s.b.new_register_list(7);
        let set = |s: &mut Asm, i: u32, v: i32| {
            s.b.lda_smi(v).star(l.get(i));
        };
        // print(fwd(show, 7, 1, 2)): one argument missing.
        s.lda_global("show");
        s.b.star(l.get(0));
        set(&mut s, 1, 7);
        set(&mut s, 2, 1);
        set(&mut s, 3, 2);
        call_global(&mut s, "fwd", l, 4);
        s.print_acc();
        // print(fwd(show, 7, 1, 2, 3, 4, 5)): more arguments than parameters.
        set(&mut s, 4, 3);
        set(&mut s, 5, 4);
        set(&mut s, 6, 5);
        call_global(&mut s, "fwd", l, 7);
        s.print_acc();
        // print(fwd(fwd, undefined, show, 8, 1)): a host forward of a host forward.
        s.lda_global("fwd");
        s.b.star(l.get(0));
        s.b.lda_undefined().star(l.get(1));
        s.lda_global("show");
        s.b.star(l.get(2));
        set(&mut s, 3, 8);
        set(&mut s, 4, 1);
        call_global(&mut s, "fwd", l, 5);
        s.print_acc();
        // print(fwd(sloppyThis, 5)): the callee wraps the receiver after the push.
        s.lda_global("sloppyThis");
        s.b.star(l.get(0));
        set(&mut s, 1, 5);
        call_global(&mut s, "fwd", l, 2);
        s.print_acc();
        // fwd(print, undefined, 6): a host callee.
        s.lda_global("print");
        s.b.star(l.get(0));
        s.b.lda_undefined().star(l.get(1));
        set(&mut s, 2, 6);
        call_global(&mut s, "fwd", l, 3);
        // print(keep(fwd, show, 3)): a Rust caller with scratch Values.
        s.lda_global("fwd");
        s.b.star(l.get(0));
        s.lda_global("show");
        s.b.star(l.get(1));
        set(&mut s, 2, 3);
        call_global(&mut s, "keep", l, 3);
        s.print_acc();
        // try { fwd(A) } catch (e) { print(e.message) }: the class-constructor TypeError.
        try_print_message(&mut s, |s| {
            s.lda_global("A");
            s.b.star(l.get(0));
            call_global(s, "fwd", l, 1);
        });
        // try { fwd(9) } catch (e) { print(e.message) }: not a function (no request).
        try_print_message(&mut s, |s| {
            s.b.lda_smi(9).star(l.get(0));
            call_global(s, "fwd", l, 1);
        });
        s.lda_str("end");
        s.b.return_();
        unit(
            "",
            vec![s.finish(), show.finish(), sloppy.finish(), a.finish()],
        )
    }

    #[test]
    fn a_tail_call_shifts_the_arguments_in_place() {
        let want = [
            "7",
            "1",
            "2",
            "undefined",
            "r", // missing argument
            "7",
            "1",
            "2",
            "3",
            "r", // extra arguments
            "8",
            "1",
            "undefined",
            "undefined",
            "r",      // a chain
            "object", // the wrap of a primitive receiver
            "6",      // a host callee
            "undefined",
            "3",
            "undefined",
            "undefined",
            "kept", // a Rust caller's scratch
            "Class constructor A cannot be invoked without 'new'",
            "9 is not a function",
        ];
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let (r, out) = run(config(d, stress), semantics);
                assert_eq!(r, "end", "{} stress={stress}", d.name());
                assert_eq!(out, want, "{} stress={stress}", d.name());
            }
        }
    }

    /// `depth = 0; try { rec(); } catch (e) {} print(depth);` where
    /// `rec() { depth = depth + 1; via(rec, undefined); }` (or `rec()` for `via` "").
    fn recursion(via: &'static str) -> VerifiedUnit {
        let mut rec = Asm::new(meta("rec", 0, 0, 0, 0));
        rec.lda_global("depth");
        rec.b.inc(rec.bin);
        rec.sta_global("depth");
        let l = rec.b.new_register_list(2);
        rec.lda_global("rec");
        rec.b.star(l.get(0));
        if via.is_empty() {
            let f = rec.f;
            rec.b.star(f).call_undefined_receiver0(f, rec.call);
        } else {
            rec.b.lda_undefined().star(l.get(1));
            call_global(&mut rec, via, l, 2);
        }
        rec.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        s.b.lda_zero();
        s.sta_global("depth");
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("rec");
        let ctx = s.b.new_register();
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
        s.b.bind(start);
        s.lda_global("rec");
        let f = s.f;
        s.b.star(f).call_undefined_receiver0(f, s.call);
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        s.b.bind(done);
        s.lda_global("depth");
        s.b.return_();
        unit("", vec![s.finish(), rec.finish()])
    }

    #[test]
    fn a_tail_call_takes_no_stack_room_for_its_host_frame() {
        // A small VM stack, so its end, not the native stack, stops each recursion.
        let cfg = || IsolateConfig {
            vm_stack_slots: 16 * 1024,
            ..config(Dispatch::DEFAULT, false)
        };
        let depth =
            |via: &'static str| -> u64 { run(cfg(), move || recursion(via)).0.parse().unwrap() };
        let (direct, tail, now) = (depth(""), depth("fwd"), depth("fwdNow"));
        // The callee frame of `fwd` takes the host frame's slots: each level costs the frame
        // of `rec` alone, as a direct call. `fwdNow` keeps its host frame below the callee.
        assert!(
            tail * 10 >= direct * 9,
            "direct {direct}, tail {tail}, now {now}"
        );
        assert!(
            now * 10 <= direct * 7,
            "direct {direct}, tail {tail}, now {now}"
        );
    }
}

// ---- the forwarded call (M7-P0 CALLHOST; the T0 form M7-P4 LEVER-3) -------------------------
//
// A call of the current realm's %Function.prototype.apply% or %Function.prototype.call% whose
// `this` is a bytecode function of this realm pushes that function's frame from the source,
// in the same dispatch run (H20-Q3 rest). Two places take it:
//
// ```text
// T0 call fast path (interp/ops.rs call_fast): a host callee with a FUNCTION receiver
//   ──> forward_frame: no GC point, nothing thrown, the budget above 0 (else None: cold)
// slow::call ──> host_entry ──> forward: the same steps after `enter`, with the GC point of
//   the feedback count (the call that makes the callee's vector) and the RangeError of a
//   full VM stack
// ```
//
// Both read the same tests ([`forward_target`], [`forward_list`]) and push the frame that
// `do_call` pushes for a bytecode callee.

/// The FunctionInfo of `f` if a forwarded call may push its frame: a FUNCTION with bytecode
/// in an Interp slot (not a host function, not native code), not a class constructor, of the
/// current realm. Not a GC point.
#[inline(always)]
pub(crate) fn forward_target<'a>(cx: &Cx, f: u64) -> Option<&'a FunctionInfo> {
    if !is_function(f) {
        return None;
    }
    let fi = function_info(f);
    (fi.is_bytecode()
        && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
        && !fi.is_class_constructor()
        && object::function_realm(f) == cx.realm)
        .then_some(fi)
}

/// `apply`'s list as a forwarded call reads it in place, with its argument count.
pub(crate) enum ForwardList {
    /// An arguments object with its realm's initial shape and an element at each index below
    /// its `length` (`arguments::forward_source`); also undefined and null, with count 0.
    Arguments(crate::arguments::Source, u32),
    /// An Array with an element at each index below its `length`.
    Array(u32),
}

/// [`ForwardList`] of `apply`'s argArray `list`, or `None`: the builtin's
/// CreateListFromArrayLike (a hole, a changed `length`, an accessor, a proxy, any other
/// object, a primitive). Not a GC point.
#[inline(always)]
pub(crate) fn forward_list(cx: &Cx, list: u64) -> Option<ForwardList> {
    if list == UNDEFINED || list == NULL {
        let none = crate::arguments::Source::EMPTY;
        return Some(ForwardList::Arguments(none, 0));
    }
    if !is_object(list) {
        return None;
    }
    if object::is_array(list) {
        let n = object::array_len(list);
        return (0..n)
            .all(|i| object::array_get(list, i).is_some())
            .then_some(ForwardList::Array(n));
    }
    if cell_kind(list) != crate::kinds::ARGUMENTS.get() {
        return None;
    }
    let (src, n) = crate::arguments::forward_source(cx.rt(), list)?;
    Some(ForwardList::Arguments(src, n))
}

/// The forwarded call of the T0 call fast path (M7-P4 LEVER-3): the steps of `slow::call`'s
/// `forward` for a call at `next` (the return pc) of the live top frame `fp` whose callee is
/// the host function `callee`, receiver `func` and `argc` arguments that `arg` reads from `fp`
/// (`arg(fp, i)`, the reader of `interp::ops::call_fast`). When it
/// takes the call, it counts the call of the callee (the T0 count, below the feedback
/// threshold or with the vector made: no GC point) and pushes its frame right above `fp`, and
/// gives the new frame and the callee's code (else a null frame). The caller then takes the call's budget unit and
/// the sampled call record of `callee`, as the cold path does (`call_record_cold`,
/// `slow::call`), and jumps.
///
/// It makes no call (no panic path either), so the call handlers that inline it keep their
/// register use and their frame: the callee test reads the current REALM record's intrinsics
/// list (the record is a copy of the intrinsic slots, `realm::record_intrinsic`), and the
/// switch test is the process-wide flag alone.
///
/// Not in the x86-64 form of the call fast path (`call_fast`, PERF3 item 2): there, the
/// inlined body costs the call handlers more callee-saved registers, pushed on each call
/// (CallProperty0 2 to 6, CallProperty1 3 to 6, M7-P4 LEVER-3), so x86-64 forwards stay on
/// the cold path.
///
/// A null frame and code, with no state changed (the slots it may have written lie above the top frame,
/// where no GC root scan and no reader looks): the cold path decides. That is every call that
/// is not a forward ([`forward_target`], `try_bind_this`, the list), an `apply` list that is
/// an arguments object with a mapped index or an Array that is not packed ([`forward_list`]
/// there), a budget at 0 or below (the cold path polls first), the call that makes the
/// callee's vector (a GC point), a frame that does not fit (the cold path throws the
/// RangeError), and every call while some isolate has its human fast paths off.
/// Not a GC point.
///
/// # Safety
/// `fp` is the live top frame (bytecode runs with no scratch area), `next` the pc after its
/// current call instruction, and `arg(fp, i)` returns valid Value bits for `i < argc`, rooted
/// in the caller's registers.
#[cfg_attr(
    all(not(feature = "spike-native"), target_arch = "x86_64"),
    allow(dead_code)
)]
// Out of line in debug builds: inline, its unoptimized locals grow the frame of each call
// handler (and of `step::<S2>`, which holds them), and the debug Sibling dispatch and the
// small-stack re-entry test (`tests::interp::scaled_steps_fit_a_small_stack`) run out of
// native stack (as the debug `Flow::recorded` is out of line).
//
// The out-of-line call takes and gives no address of a local of the handler: the reader
// `arg` is a copy (at most 2 words, in registers) that reads `fp`, and the result is a pair
// in registers (not an `Option`, which comes back through a stack slot). A local whose
// address goes into a call makes LLVM mark no call of the handler as a tail call, so in an
// optimized build with debug assertions (the Spike C test build: ep9_vm at opt-level 2) the
// threaded dispatch grows the native stack by one handler frame per instruction, until a
// RangeError (M7-P4 integration fixer: 15 Spike C unit tests on 2 MiB test threads).
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline(always))]
pub(crate) unsafe fn forward_frame(
    cx: &Cx,
    fp: *mut u64,
    next: *const u8,
    callee: u64,
    func: u64,
    argc: u32,
    arg: impl Fn(*mut u64, u32) -> u64 + Copy,
) -> (*mut u64, *const u8) {
    // SAFETY: the contract of this function.
    match unsafe { forward_frame_in(cx, fp, next, callee, func, argc, arg) } {
        Some(pushed) => pushed,
        None => (core::ptr::null_mut(), core::ptr::null()),
    }
}

/// The steps of [`forward_frame`], with `None` for its null frame.
///
/// # Safety
/// As [`forward_frame`].
#[cfg_attr(
    all(not(feature = "spike-native"), target_arch = "x86_64"),
    allow(dead_code)
)]
#[inline(always)]
unsafe fn forward_frame_in(
    cx: &Cx,
    fp: *mut u64,
    next: *const u8,
    callee: u64,
    func: u64,
    argc: u32,
    arg: impl Fn(*mut u64, u32) -> u64 + Copy,
) -> Option<(*mut u64, *const u8)> {
    use crate::builtins::IntrinsicId;
    if cx.budget <= 0 || ANY_OFF.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let apply = callee == realm_intrinsic(cx, IntrinsicId::FunctionPrototypeApply)?;
    if !apply && callee != realm_intrinsic(cx, IntrinsicId::FunctionPrototypeCall)? {
        return None;
    }
    // The source of the arguments: the call's own registers after thisArg (`call`), or the
    // list (`apply`) in the forms with no call in their tests: undefined, null, an unmodified
    // arguments object of this realm with no mapped index, or a packed Array with an
    // ELEMENTS list ([`packed_array`]). Any other list (a mapped arguments object, an Array
    // with a hole or a double list) takes the cold path's `forward_list`.
    let (n, words) = if apply {
        let list = if argc > 1 { arg(fp, 1) } else { UNDEFINED };
        if list == UNDEFINED || list == NULL {
            (0, None)
        } else if !is_cell_bits(list) {
            return None;
        } else if cell_kind(list) == crate::kinds::ARGUMENTS.get() {
            let (src, n) = crate::arguments::forward_source_in(cx.realm, list)?;
            if src.mapped() {
                return None;
            }
            (
                n,
                if n == 0 {
                    None
                } else {
                    Some(src.unmapped_list()?)
                },
            )
        } else if cell_kind(list) == crate::kinds::ARRAY.get() {
            packed_array(list)?
        } else {
            return None;
        }
    } else {
        (argc.saturating_sub(1), None)
    };
    // The tests are in this order (the list first) so that fewer values are live in the
    // list's element scan.
    let fi = forward_target(cx, func)?;
    // The call that makes the callee's vector (`feedback::count_call`, a GC point) goes cold.
    let novector = fi.feedback.get().is_null();
    if novector && fi.invocations.get() + 1 >= cx.feedback_threshold {
        return None;
    }
    let this_arg = if argc > 0 { arg(fp, 0) } else { UNDEFINED };
    let this_arg = try_bind_this(fi, this_arg, cx.global_this)?;
    let base = frame_top(fp);
    // The frame of `push_frame` for a call, in the steps of the x86-64 T0 call fast path: the
    // size test, the missing arguments, the receiver and the arguments, the header and the
    // registers.
    let nfp = call_frame_fp(base, cx.stack_limit, fi, n)?;
    // SAFETY: `nfp` is `call_frame_fp` of `base`, the first free slot above the live top frame
    // `fp` (caller contract), and the VM stack cap; each reader returns Values the caller
    // keeps rooted.
    unsafe {
        fill_missing_args(base, nfp, n);
        match words {
            Some(w) => {
                write_call_args::<true>(nfp, this_arg, n, |i| crate::arguments::unmapped_word(w, i))
            }
            None if apply => write_call_args::<true>(nfp, this_arg, 0, |_| UNDEFINED),
            None => write_call_args::<true>(nfp, this_arg, n, |i| arg(fp, i + 1)),
        }
        write_call_header(nfp, fi, func, n, fp, next, || function_context(func));
    }
    // The count of `feedback::count_call` (no vector: below the threshold, checked above).
    if novector {
        fi.invocations.set(fi.invocations.get() + 1);
    }
    #[cfg(test)]
    FORWARDED_CALLS.with(|h| h.set(h.get() + 1));
    #[cfg(test)]
    T0_FORWARDED_CALLS.with(|h| h.set(h.get() + 1));
    // (`fast_hit` is defined below: called by its path.)
    crate::call::fast_hit!(if apply { 1 } else { 2 });
    Some((nfp, fi.code))
}

/// The length n of the Array `a` and its ELEMENTS list (`None` when n is 0) when each index
/// below n holds an element in an ELEMENTS list (not a hole, not a double list): the case of
/// [`forward_list`]'s Array form that `forward_frame` reads in place, with
/// `arguments::unmapped_word` (each `object::array_get` of an index below n gives that
/// word). `None` for any other Array. Not a GC point; no panic path.
#[cfg_attr(
    all(not(feature = "spike-native"), target_arch = "x86_64"),
    allow(dead_code)
)]
#[inline(always)]
fn packed_array(a: u64) -> Option<(u32, Option<ep9_heap::CellPtr>)> {
    use ep9_value::encoding::NUMBER_TAG;
    let c = object::cell(a);
    // The int32 length test of `object::array_len`.
    let w = object::rd(c, object::arr::LENGTH);
    if w & NUMBER_TAG != NUMBER_TAG {
        return None;
    }
    let n = w as u32;
    if n == 0 {
        return Some((0, None));
    }
    let e = object::rd(c, object::obj::ELEMENTS);
    if !is_cell_bits(e) || cell_kind(e) != crate::kinds::ELEMENTS.get() {
        return None;
    }
    let ec = object::cell(e);
    if n as usize > object::rd_i32(ec, object::elems::CAPACITY) as usize {
        return None;
    }
    let mut i = 0;
    while i < n {
        if crate::arguments::unmapped_word(ec, i) == HOLE {
            return None;
        }
        i += 1;
    }
    Some((n, Some(ec)))
}

/// The intrinsic `id` of the current realm read from its REALM record (`Cx::realm`): the
/// value `builtins::intrinsic` gives, without its bound test (`realm::record_intrinsic` keeps
/// the record's list a copy of the intrinsic slots; `realm::switch` reads it back). `None`
/// when `Cx::realm` is no REALM record or its list is too short. Not a GC point; no panic
/// path.
#[cfg_attr(
    all(not(feature = "spike-native"), target_arch = "x86_64"),
    allow(dead_code)
)]
#[inline(always)]
fn realm_intrinsic(cx: &Cx, id: crate::builtins::IntrinsicId) -> Option<u64> {
    let realm = cx.realm;
    if !is_cell_bits(realm) || cell_kind(realm) != crate::kinds::REALM.get() {
        return None;
    }
    let list = object::record_get(realm, object::realmf::INTRINSICS);
    if !is_cell_bits(list) {
        return None;
    }
    let i = id as usize;
    let c = object::cell(list);
    (i < object::rd_i32(c, object::elems::CAPACITY) as usize)
        .then(|| object::rd(c, object::elems::DATA + 8 * i))
}

// ---- the human fast-path switch (FASTDIFF, M7-P0 CALLHOST; plan V12-D30) --------------------
//
// A human fast path is hand-written code that skips spec steps under a guard, with the spec
// steps beside it in the same build (09-milestones.md:108). The doc-hidden `Cx` method
// `set_human_fast_paths` turns the ones that read [`human_fast_paths`] off for one isolate,
// so `ep9_builtins/tests/fastpath_diff.rs` runs each case with them on and off and compares
// the output and the exceptions.
//
// The switch is per isolate, keyed by `Runtime::id` (no other runtime of the process gets the
// same id). It lives here, not in `Isolate`: the fast paths see a `Runtime`, never the
// `Isolate`. While no isolate has turned them off, the test is one relaxed load.

/// True while some isolate of the process has its human fast paths off.
static ANY_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The `Runtime::id` of every isolate whose human fast paths are off.
static OFF: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// Whether the human fast paths of `rt`'s isolate are on (the default). Not a GC point.
#[inline(always)]
pub(crate) fn human_fast_paths(rt: &Runtime) -> bool {
    !ANY_OFF.load(std::sync::atomic::Ordering::Relaxed) || !is_off(rt)
}

#[cold]
#[inline(never)]
fn is_off(rt: &Runtime) -> bool {
    OFF.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&rt.id)
}

/// Turns the human fast paths of `rt`'s isolate on or off (`abi::Cx::set_human_fast_paths`).
/// Not a GC point.
pub(crate) fn set_human_fast_paths(rt: &Runtime, on: bool) {
    let mut off = OFF
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    off.retain(|&id| id != rt.id);
    if !on {
        off.push(rt.id);
    }
    ANY_OFF.store(!off.is_empty(), std::sync::atomic::Ordering::Relaxed);
}

/// The FASTDIFF hit counters (route P0-FASTHITS; feature `fastdiff-hits`, which only the
/// FASTDIFF test turns on, so no shipped build counts): per Switch human fast path, the times it
/// was taken on this thread. A run with the fast paths off must count 0 for each.
#[cfg(feature = "fastdiff-hits")]
pub(crate) mod fast_hits {
    use core::cell::Cell;

    /// The Switch rows of the FASTDIFF inventory, in counter order.
    pub const NAMES: [&str; 6] = [
        "host-entry",
        "forward-apply",
        "forward-call",
        "args-create",
        "clfal-length",
        "clfal-elements",
    ];

    thread_local! {
        static HITS: [Cell<u64>; NAMES.len()] = const { [const { Cell::new(0) }; NAMES.len()] };
    }

    /// Counts one hit of fast path `i` (an index of [`NAMES`]).
    #[inline(always)]
    pub(crate) fn hit(i: usize) {
        HITS.with(|h| h[i].set(h[i].get() + 1));
    }

    /// The count of each fast path on this thread, then 0 for each.
    pub(crate) fn take() -> Vec<(&'static str, u64)> {
        HITS.with(|h| {
            NAMES
                .iter()
                .zip(h)
                .map(|(n, c)| (*n, c.replace(0)))
                .collect()
        })
    }
}

/// Counts a hit of the FASTDIFF Switch path `$i` (`fast_hits::NAMES`) with feature
/// `fastdiff-hits`; nothing without it.
macro_rules! fast_hit {
    ($i:expr) => {
        #[cfg(feature = "fastdiff-hits")]
        $crate::call::fast_hits::hit($i);
    };
}
pub(crate) use fast_hit;

#[cfg(test)]
thread_local! {
    /// The calls that `interp/slow/call.rs` `host_entry` took on this thread (the tests check
    /// the path ran).
    pub(crate) static HOST_ENTRY_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    /// The calls that `interp/slow/call.rs` `forward` or [`forward_frame`] pushed a frame for
    /// on this thread.
    pub(crate) static FORWARDED_CALLS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    /// The calls of [`FORWARDED_CALLS`] that [`forward_frame`] (the T0 call fast path) pushed.
    #[cfg_attr(all(not(feature = "spike-native"), target_arch = "x86_64"), allow(dead_code))]
    pub(crate) static T0_FORWARDED_CALLS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}
