//! Calls and constructs from bytecode. A bytecode callee's call is counted here, and the call
//! that reaches the feedback threshold makes its vector before the frame push (`feedback.rs`).
//!
//! A bound function callee (§10.4.1) is replaced by its innermost target FUNCTION before
//! anything else: its bound arguments go in front of the call's (`bound::unwrap`,
//! `bound::copy_chain_args`), its bound `this` replaces the receiver, and for `[[Construct]]`
//! new.target becomes the target while it is the bound function. The bound function stays in
//! the caller's callee register, so everything its chain holds stays rooted until the frame
//! push.
//!
//! Spike B (feature `spike-native` only, `native.rs`): a call of a FUNCTION with a Baseline or
//! AotBaseline slot (no construct, no spread, not host, not a class constructor) is entered
//! natively. The default build has no such branch. Two places enter it:
//!
//! ```text
//! slow::call --enter, interrupt poll--> native_entry: unprefixed CallAnyReceiver, CallProperty,
//!   CallProperty0-2, CallUndefinedReceiver, CallUndefinedReceiver0-2 of a native-slotted
//!   callee; operands read at their fixed places, no decode_call, no Vec (M2-P3-D21-ENTRY)
//!   --anything else--> decode_call --> do_call --native-slotted callee, a prefixed call-->
//!   call_native (CallSite, before the args Vec)
//! ```

use ep9_value::Throw;

use crate::bound;
use crate::call::{self, CallSite};
use crate::conv::{Res, describe};
use crate::error::{self, not_implemented, stack_overflow, throw_error};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED};
use crate::interp::{CONSTRUCT_RETURN_STUB, Cx};
use crate::kinds::{is_bound, is_callable};
use crate::object::{self, is_array};
use crate::runtime::{ErrorKind, frame_top};
use crate::unit::FunctionInfo;

use super::iter::{KeepAlive, iterable_to_array};
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
    #[cfg(feature = "spike-native")]
    // SAFETY: `pc` is the call instruction (or its prefix) of verified code in the live top
    // frame `fp`, which bytecode runs with no scratch area (call.rs).
    if let Some(r) = unsafe { native_entry(cx, fp, pc) } {
        return match r {
            Ok(v) => v,
            Err(Throw) => error::unwind(cx, fp, pc),
        };
    }
    let site = call::decode_call(fp, pc);
    match do_call(cx, fp, &site, acc) {
        Ok(v) => v,
        Err(Throw) => error::unwind(cx, fp, pc),
    }
}

fn do_call(cx: &mut Cx, fp: *mut u64, s: &CallSite, acc: u64) -> Res<u64> {
    #[cfg(feature = "spike-native")]
    if !s.construct
        && !s.spread
        && let Some(fi) = crate::native::native_callee(s.callee)
    {
        return call_native(cx, fp, s, fi);
    }
    let rt = cx.rt();
    // The argument values stay rooted in the caller's registers while this copy exists.
    // SAFETY: the offsets come from register operands of verified code in the frame `fp`.
    let mut args: Vec<u64> = s.args.iter().map(|&o| unsafe { *fp.offset(o) }).collect();
    // The spread operand (`CallWithSpread`, `ConstructWithSpread`) is an Array that bytecode
    // made: BytecodeGen ran the iterator protocol already (`arguments_into`), or built the
    // list itself (a spread that is not last, a default derived constructor's rest array).
    // Its elements are the arguments as they are; an `@@iterator` call here would be a
    // second, observable iteration. Any other value goes through IterableToList, and `keep`
    // (a stack slot the conservative scan sees) keeps that new array, and so its values in
    // `args`, alive until the call returns.
    let mut keep = KeepAlive(UNDEFINED);
    if s.spread {
        let last = args.pop().unwrap_or(UNDEFINED);
        let list = if is_array(last) {
            // The array stays rooted in the caller's last argument register.
            last
        } else {
            // GC point, runs JS: the values in `args` stay rooted in caller registers (the
            // heap does not move cells).
            keep.0 = iterable_to_array(cx, last)?;
            keep.0
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
        // the callee. The callee frame keeps it in `slot::NEW_TARGET` (push_frame).
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

/// Spike B (feature `spike-native`, M2-P3-D21-ENTRY): the decode-free native entry at the top
/// of `slow::call`, for the call that T0's `call_fast` sends cold because the callee's slot is
/// not Interp (interp/ops.rs). It takes an unprefixed `CallAnyReceiver`, `CallProperty`,
/// `CallProperty0`-`2`, `CallUndefinedReceiver` or `CallUndefinedReceiver0`-`2` whose callee
/// is native-slotted (`native::native_callee`). It reads the operands at their fixed
/// single-scale places (one byte each, checked at compile time below) and the arguments
/// straight from the caller's registers, with one argument reader per shape (`go`): no
/// `decode_call`, no `CallSite`, no `Vec`.
///
/// Everything else returns `None`, with no state changed, and takes `decode_call` and
/// `do_call` as before: a `Wide`/`ExtraWide` call (`pc` is at the prefix byte), `CallWithSpread`,
/// `Construct`, `ConstructWithSpread`, and any callee that is not native-slotted (T0, host,
/// bound, class constructor, generator, not callable).
///
/// What it keeps from `do_call` -> `call_native`, which a prefixed call still takes:
/// - receiver: the receiver register (`CallProperty*`; the first list register of
///   `CallAnyReceiver`/`CallProperty`, undefined when the count is 0), else undefined;
///   OrdinaryCallBindThis in `enter_native`;
/// - arguments: the same registers in the same order, read once by `push_frame` into the
///   callee frame. No GC point comes between the read and the frame push, the values stay in
///   the caller's registers too, and native code runs with its frame published, so the GC
///   scan sees them in both frames;
/// - state: `slow::call` already published `fp` and the call's offset (`enter`) and ran the
///   interrupt poll; `enter_native` counts one budget unit, and a full VM stack throws the
///   RangeError. No feedback count and no native stack check (`native.rs` module doc);
/// - result: the accumulator and the next pc (`out_fp`/`out_pc`), or `Err` and `slow::call`
///   unwinds from this call instruction.
///
/// # Safety
/// `pc` is the call instruction (or its prefix) of verified code in the live top frame `fp`,
/// and no scratch Values lie above that frame.
#[cfg(feature = "spike-native")]
#[inline(always)]
unsafe fn native_entry(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> Option<Res<u64>> {
    use crate::interp::dsl::R;
    use ep9_bytecode::{Opcode as O, OperandScale};

    // The 9 opcodes are the bytes CallAnyReceiver ..= CallUndefinedReceiver2, in this order.
    // At single scale every operand of each is one byte, so operand `i` is at `pc + 1 + i`.
    const FIRST: u8 = O::CallAnyReceiver as u8;
    const LAST: u8 = O::CallUndefinedReceiver2 as u8;
    const _: () = {
        let ops = [
            O::CallAnyReceiver,
            O::CallProperty,
            O::CallProperty0,
            O::CallProperty1,
            O::CallProperty2,
            O::CallUndefinedReceiver,
            O::CallUndefinedReceiver0,
            O::CallUndefinedReceiver1,
            O::CallUndefinedReceiver2,
        ];
        assert!(LAST - FIRST + 1 == ops.len() as u8);
        let mut i = 0;
        while i < ops.len() {
            let info = ops[i].info();
            assert!(ops[i] as u8 == FIRST + i as u8);
            assert!(info.size(OperandScale::Single) == 1 + info.operands.len());
            i += 1;
        }
    };
    // The instruction length of `O::$o` at single scale.
    macro_rules! len {
        ($o:ident) => {
            const { O::$o.info().size(OperandScale::Single) }
        };
    }

    // SAFETY: caller contract: `pc` is a byte of verified code.
    let op = unsafe { *pc };
    if !(FIRST..=LAST).contains(&op) {
        return None;
    }
    // SAFETY: called only for operand `i` of one of the 9 call opcodes (checked above); the
    // verifier checked that the whole instruction lies in the code.
    let byte = |i: usize| unsafe { *pc.add(1 + i) };
    let reg = |i: usize| R::from_operand(i32::from(byte(i) as i8)).0;
    // SAFETY: called only for slots named by register operands of this verified instruction
    // (and each list with its count), which lie in the live frame `fp`.
    let rd = |s: isize| unsafe { *fp.offset(s) };
    // Register lists hold locals only, so the list operand is the slot offset itself.
    let first = isize::from(byte(1) as i8);
    // Operand 0 is the callee in all 9.
    let callee = rd(reg(0));
    let fi = crate::native::native_callee(callee)?;
    #[cfg(test)]
    crate::native::T0_ENTRY_HITS.with(|h| h.set(h.get() + 1));
    // `go` once per shape, each with its own argument reader: no slot list and no match per
    // argument.
    macro_rules! enter {
        ($this:expr, $len:expr, $argc:expr, $arg:expr) => {
            // SAFETY: caller contract; the reader reads registers of this verified
            // instruction, and `native_callee` checked the native slot.
            unsafe {
                go(
                    cx,
                    fp,
                    pc.wrapping_add($len),
                    callee,
                    fi,
                    $this,
                    $argc,
                    $arg,
                )
            }
        };
    }
    const CALL_ANY_RECEIVER: u8 = O::CallAnyReceiver as u8;
    const CALL_PROPERTY: u8 = O::CallProperty as u8;
    const CALL_PROPERTY0: u8 = O::CallProperty0 as u8;
    const CALL_PROPERTY1: u8 = O::CallProperty1 as u8;
    const CALL_PROPERTY2: u8 = O::CallProperty2 as u8;
    const CALL_UNDEFINED: u8 = O::CallUndefinedReceiver as u8;
    const CALL_UNDEFINED0: u8 = O::CallUndefinedReceiver0 as u8;
    const CALL_UNDEFINED1: u8 = O::CallUndefinedReceiver1 as u8;
    // Argument readers: `n` list registers from `from` up, or one or two registers.
    let list = |from: isize| move |i: u32| rd(from + i as isize);
    let one = |a: isize| move |_: u32| rd(a);
    let two = |a: isize, b: isize| move |i: u32| rd(if i == 0 { a } else { b });
    let none = |_: u32| UNDEFINED;
    Some(match op {
        // The list holds the receiver, then the arguments; count 0: no receiver.
        CALL_ANY_RECEIVER | CALL_PROPERTY => match u32::from(byte(2)) {
            0 => enter!(UNDEFINED, len!(CallProperty), 0, none),
            n => enter!(rd(first), len!(CallProperty), n - 1, list(first + 1)),
        },
        CALL_PROPERTY0 => enter!(rd(reg(1)), len!(CallProperty0), 0, none),
        CALL_PROPERTY1 => enter!(rd(reg(1)), len!(CallProperty1), 1, one(reg(2))),
        CALL_PROPERTY2 => enter!(rd(reg(1)), len!(CallProperty2), 2, two(reg(2), reg(3))),
        CALL_UNDEFINED => {
            let n = u32::from(byte(2));
            enter!(UNDEFINED, len!(CallUndefinedReceiver), n, list(first))
        }
        CALL_UNDEFINED0 => enter!(UNDEFINED, len!(CallUndefinedReceiver0), 0, none),
        CALL_UNDEFINED1 => enter!(UNDEFINED, len!(CallUndefinedReceiver1), 1, one(reg(1))),
        // CallUndefinedReceiver2 (the range check above).
        _ => enter!(
            UNDEFINED,
            len!(CallUndefinedReceiver2),
            2,
            two(reg(1), reg(2))
        ),
    })
}

/// The tail of [`native_entry`] for one call shape: enters native code with return pc `next`
/// and the receiver and arguments that `arg` reads, then sets the next pc. `enter_native` is
/// `#[inline(always)]`, so each shape's reader inlines into the frame push (cachegrind on
/// linux a64, config A of the D21 bench: 262.9 Ir per call with `enter_native` outlined,
/// 237.7 inlined).
///
/// # Safety
/// `fp` is the live top frame with no scratch Values above it, `arg(i)` returns valid Value
/// bits for `i < argc`, and `fi` is the FunctionInfo of the native-slotted `callee`
/// (`native::native_callee`).
#[cfg(feature = "spike-native")]
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn go(
    cx: &mut Cx,
    fp: *mut u64,
    next: *const u8,
    callee: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (no scratch
    // area), and the rest is the caller contract (the `enter_native` contract).
    let r = unsafe {
        crate::native::enter_native(cx, frame_top(fp), fp, next, callee, fi, this, argc, arg)
    };
    // After the call: a nested dispatch under native code (`call_any`) sets them too.
    cx.out_fp = fp;
    cx.out_pc = next;
    r
}

/// Spike B (feature `spike-native`): a call (no construct, no spread) of a FUNCTION whose slot
/// holds native code (`native::native_callee`), on the decode path: since M2-P3-D21-ENTRY
/// only a `Wide`/`ExtraWide` call gets here (`native_entry` takes the others first). The
/// callee frame goes right above the caller's registers with the return pc of the next
/// instruction, as on the T0 path, and the arguments are read straight from the caller's
/// registers (no `Vec`). Native code runs to its return; then the dispatch continues at the
/// next instruction with its result, or `slow::call` unwinds from this call instruction. No
/// feedback count and no native stack check here (`native.rs` module doc).
#[cfg(feature = "spike-native")]
fn call_native(cx: &mut Cx, fp: *mut u64, s: &CallSite, fi: &FunctionInfo) -> Res<u64> {
    let Ok(argc) = u32::try_from(s.args.len()) else {
        return stack_overflow(cx);
    };
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (bytecode runs
    // with no scratch area, call.rs), the offsets in `s.args` come from register operands of
    // verified code in the frame `fp`, and `native_callee` checked the native slot.
    let r = unsafe {
        crate::native::enter_native(
            cx,
            frame_top(fp),
            fp,
            s.next_pc,
            s.callee,
            fi,
            s.this,
            argc,
            |i| *fp.offset(s.args[i as usize]),
        )
    };
    // After the call: a nested dispatch under native code (`call_any`) sets them too.
    cx.out_fp = fp;
    cx.out_pc = s.next_pc;
    r
}

/// Walks the chain of the bound callee `f` (`bound::unwrap`; `new_target` undefined for a
/// call) and puts the chain's bound arguments in front of `args`. RangeError if the count
/// does not fit; polls for interrupts. Not a GC point unless it throws; `f` must stay rooted
/// (it is in the caller's callee register) while `args` holds the bound arguments.
fn prepend_bound_args(
    cx: &mut Cx,
    f: u64,
    new_target: u64,
    args: &mut Vec<u64>,
) -> Res<bound::Unwrapped> {
    let u = bound::unwrap(cx, f, new_target)?;
    let n = u.bound as usize;
    if n > 0 {
        args.splice(0..0, core::iter::repeat_n(UNDEFINED, n));
        bound::copy_chain_args(f, &mut args[..n]);
    }
    Ok(u)
}

#[inline]
fn runs_bytecode(fi: &FunctionInfo) -> bool {
    fi.is_bytecode() && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
}
