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
//! Realms (H22-Q3): a bytecode callee of another realm does not get a frame here. It runs
//! nested through `call::call_function_object` (or `call::construct_function_object`), which
//! switches to its realm and back (`realm::enter_realm_of`, `realm::leave`): one Rust level per
//! cross-realm call, and the dispatch continues at the next instruction with its result, as
//! for a host callee. A host callee switches in `call::call_host`. So a frame pushed here
//! always runs in the caller's realm, and no return path needs a realm check. Spike B: a
//! native-slotted or (b) callee of another realm runs nested in its realm too
//! (`native::call_in_its_realm`).
//!
//! Native code (`native.rs`): a call of a FUNCTION with a Baseline or AotBaseline slot (no
//! construct, not host, not a class ctor) is entered natively: in every build late in
//! `do_call` (`call_native_cold`; MC-w30-2), and with `spike-native` first at two places:
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
use crate::call::{self, ArgBuf, CallSite, ForwardList};
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

/// The call or construct instruction at `pc` (all `Call*` and `Construct*` opcodes); sets
/// `cx.out_fp`/`cx.out_pc`. `acc` is `new.target` for `Construct`. A bytecode callee gets a new
/// frame, no Rust recursion (a callee of another realm runs nested); a host callee runs here.
#[cold]
#[inline(never)]
pub(crate) fn call(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> u64 {
    enter(cx, fp, pc);
    if cx.budget <= 0 {
        if interrupt(cx, fp) == 0 {
            return error::unwind(cx, fp, pc);
        }
        // The caller gets its vector at a call poll too (PERF3 step-2 item 4), not only at a
        // JumpLoop poll: a loop whose polls always land on a call would get none. GC point,
        // as `count_call` below: the callee and arguments are in the caller's registers
        // (the frame is recorded) and `acc` is a Rust local.
        feedback::ensure(cx.rt(), cx.feedback_threshold, super::info(fp));
    }
    #[cfg(feature = "spike-native")]
    // SAFETY: `pc` is a call (or its prefix) of verified code in the live top frame `fp` (call.rs).
    if let Some(r) = unsafe { native_entry(cx, fp, pc) } {
        return match r {
            Ok(v) => v,
            Err(Throw) => error::unwind(cx, fp, pc),
        };
    }
    // SAFETY: as above (bytecode runs with no scratch area); M7-P0 CALLHOST: a host callee.
    let r = unsafe { host_entry(cx, fp, pc, acc) };
    match r.unwrap_or_else(|| do_call(cx, fp, &call::decode_call(fp, pc), acc)) {
        Ok(v) => v,
        Err(Throw) => error::unwind(cx, fp, pc),
    }
}

fn do_call(cx: &mut Cx, fp: *mut u64, s: &CallSite, acc: u64) -> Res<u64> {
    #[cfg(feature = "spike-native")]
    if !s.construct
        && !s.spread
        && let Some(fi) = crate::native::native_callee(s.callee)
        && object::function_realm(s.callee) == cx.realm
    {
        return call_native(cx, fp, s, fi);
    }
    let rt = cx.rt();
    // The argument values stay rooted in the caller's registers while this copy exists. Up
    // to `call::INLINE_ARGS` of them need no heap allocation (`call::ArgBuf`).
    // SAFETY: the offsets come from register operands of verified code in the frame `fp`.
    let mut args: ArgBuf<u64> = s.args.iter().map(|&o| unsafe { *fp.offset(o) }).collect();
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
        // A FUNCTION pays one kind test; a bound function or a callable proxy goes out of line.
        if !object::is_function(callee) {
            match construct_other(cx, fp, s, callee, new_target, &mut args)? {
                Ok(u) => (callee, new_target) = u, // a bound function (§10.4.1.2)
                Err(r) => return Ok(r),            // a callable proxy (§10.5.13)
            }
        }
        let Ok(argc) = u32::try_from(args.len()) else {
            return stack_overflow(cx);
        };
        let fi = object::function_info(callee);
        if fi.host.is_some() {
            let r = call::call_host_from_bytecode(
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
        if object::function_realm(callee) != cx.realm {
            // A constructor of another realm: [[Construct]] from Rust (one nested run).
            let r = call::construct_function_object(
                cx,
                callee,
                argc,
                |i| args[i as usize],
                new_target,
            )?;
            cx.out_fp = fp;
            cx.out_pc = s.next_pc;
            return Ok(r);
        }
        if !runs_bytecode(fi) {
            return construct_native_cold(cx, fp, s, callee, new_target, argc, &args); // D25
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
        // The construct-site hook (M4-P0; filled by M4-P1 OBJ-ALLOC, `construct_ic.rs`): it
        // records the site's state (MCV-w36-1). GC point allowed: `this` is a Rust local, the
        // rest is in caller registers.
        crate::construct_ic::miss(cx, fp, s, callee, new_target, this);
        // SAFETY: `fp` is the live top frame; `this` is valid Value bits.
        let Some(stub) = (unsafe { call::push_construct_stub(cx, fp, this, callee, s.next_pc) })
        else {
            // The call takes its unit also when its frame does not fit (R3-2).
            cx.budget -= 1;
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
        // One unit per call, also when the frame does not fit (R3-2): a caught RangeError
        // must not keep the budget from running out, or the poll never comes.
        cx.budget -= 1;
        let Some(nfp) = nfp else {
            return stack_overflow(cx);
        };
        cx.out_fp = nfp;
        cx.out_pc = fi.code;
        return Ok(UNDEFINED);
    }
    if !is_callable(callee) {
        let msg = format!("{} is not a function", describe(callee));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let mut this = s.this;
    if !object::is_function(callee) {
        match call_other(cx, fp, s, callee, this, &mut args)? {
            Ok(u) => (callee, this) = u, // a bound function (§10.4.1.1)
            Err(r) => return Ok(r),      // a callable proxy (§10.5.12)
        }
    }
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let fi = object::function_info(callee);
    if fi.host.is_some() {
        let r = call::call_host_from_bytecode(
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
    if object::function_realm(callee) != cx.realm {
        // A function of another realm: a call from Rust in its realm (one nested run; its
        // class-constructor TypeError is that realm's, §10.2.1 step 2). The arguments stay
        // rooted in the caller's registers (and `args` for a spread or bound call, as below).
        // Spike B: a native-slotted or (b) callee is entered natively, in its realm.
        #[cfg(feature = "spike-native")]
        // SAFETY: `fp` is the live top frame (bytecode runs with no scratch area), and the
        // reader reads `args`, which the push copies before any GC point.
        if let Some(r) = unsafe {
            crate::native::call_in_its_realm(cx, fp, s.next_pc, callee, this, argc, |i| {
                args[i as usize]
            })
        } {
            let r = r?;
            cx.out_fp = fp;
            cx.out_pc = s.next_pc;
            return Ok(r);
        }
        let r =
            call::call_function_object(cx, callee, this, argc, |i| args[i as usize], UNDEFINED)?;
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
    #[cfg(feature = "spike-native")]
    if crate::native::b_callee(callee).is_some() {
        return enter_b_cold(cx, fp, s.next_pc, fi, callee, this, argc, |i| {
            args[i as usize]
        });
    }
    if !runs_bytecode(fi) {
        return call_native_cold(cx, fp, s, callee, this, argc, &args); // MC-w30-2
    }
    // GC point: the callee, the receiver and the arguments are in caller registers (for a
    // bound callee: its target, bound `this` and bound arguments are reachable from it).
    feedback::count_call(rt, cx.feedback_threshold, fi);
    // GC point (the wrap of a primitive receiver of a sloppy callee, H14-Q2): as above; the
    // wrapper stays in a Rust local until the frame push stores it.
    let this = match call::try_bind_this(fi, this, rt.global()) {
        Some(t) => t,
        None => call::wrap_this(cx, this)?,
    };
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
    // One unit per call, also when the frame does not fit (R3-2): `call_fast` gives its unit
    // back at that exit, and a caught RangeError must not keep the budget from running out.
    cx.budget -= 1;
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
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
///   interrupt poll; `enter_native` takes one budget unit, also on a full VM stack (its
///   RangeError). No feedback count and no native stack check (`native.rs` module doc);
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
    // A callee of another realm takes `do_call` (its realm switch).
    if object::function_realm(callee) != cx.realm {
        return None;
    }
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

/// Spike B placement (b) (feature `spike-native`, P4-PLACEMENT-B task 4): the cold arm of a
/// call of a (b) callee (`native::b_callee`), the call that T0's `call_fast` did not take: the
/// budget poll, a prefixed (`Wide`/`ExtraWide`) call, a spread call, a bound callee, a full VM
/// stack, a primitive receiver of a sloppy callee (wrapped here, H14-Q2). It pushes the
/// canonical frame (return pc = the caller's next instruction), takes one budget unit and
/// sets `cx.out_fp`/`cx.out_pc` so that the dispatch continues at the `OP_ENTER_B` stub byte:
/// the (b) code under a threaded expansion, the bytecode under the match loop (D4). No Rust
/// recursion, no native call, no feedback count (the (b) code records none). A full VM stack
/// is the VM stack's RangeError, unwound from the call instruction.
/// `[[Construct]]` of a (b) function does not get here: it throws "not implemented", as for
/// placement (a).
#[cfg(feature = "spike-native")]
#[allow(clippy::too_many_arguments)]
fn enter_b_cold(
    cx: &mut Cx,
    fp: *mut u64,
    next_pc: *const u8,
    fi: &FunctionInfo,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    #[cfg(test)]
    crate::native::B_COLD_ENTRIES.with(|h| h.set(h.get() + 1));
    // GC point (the wrap of a primitive receiver of a sloppy callee, H14-Q2): the callee and
    // the arguments are in caller registers; the wrapper stays in a Rust local until the
    // frame push stores it.
    let this = match call::try_bind_this(fi, this, cx.rt().global()) {
        Some(t) => t,
        None => call::wrap_this(cx, this)?,
    };
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (bytecode runs
    // with no scratch area), and `arg` reads Values the caller keeps rooted.
    let nfp = unsafe {
        call::push_frame(
            frame_top(fp),
            cx.stack_limit,
            fi,
            callee,
            this,
            argc,
            arg,
            fp,
            next_pc,
            object::function_context(callee),
            UNDEFINED,
        )
    };
    // One unit, also when the frame does not fit (R3-2), as in `do_call`.
    cx.budget -= 1;
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    cx.out_fp = nfp;
    cx.out_pc = crate::native::ENTER_B_STUB.as_ptr();
    Ok(UNDEFINED)
}

/// Walks the chain of the bound callee `f` (`bound::unwrap`; `new_target` undefined for a
/// call) and puts the chain's bound arguments in front of `args`. RangeError if the count
/// does not fit; polls for interrupts. Not a GC point unless it throws; `f` must stay rooted
/// (it is in the caller's callee register) while `args` holds the bound arguments.
fn prepend_bound_args(
    cx: &mut Cx,
    f: u64,
    new_target: u64,
    args: &mut ArgBuf<u64>,
) -> Res<bound::Unwrapped> {
    let u = bound::unwrap(cx, f, new_target)?;
    let n = u.bound as usize;
    if n > 0 {
        args.prepend(n, UNDEFINED);
        bound::copy_chain_args(f, &mut args[..n]);
    }
    Ok(u)
}

/// The `Call*` callee that is not a FUNCTION (M5-P3 PROXY2), out of line so that a FUNCTION
/// callee pays one kind test. A bound function unwraps to its innermost target, with its
/// bound arguments put in front of `args` (§10.4.1.1: Call(target, boundThis, boundArgs ++
/// args)); `Ok` gives that target and its `this` when the target is a FUNCTION. A callable
/// proxy, or one at the end of the bound chain, is called here (§10.5.12): its `[[Call]]`
/// from Rust (`call::call_with`, nested, as for a host callee), then the dispatch goes on at
/// the next instruction, and `Err` gives the result. `args` stay rooted in the caller's
/// registers (and in the bound callee there).
#[inline(never)]
fn call_other(
    cx: &mut Cx,
    fp: *mut u64,
    s: &CallSite,
    callee: u64,
    this: u64,
    args: &mut ArgBuf<u64>,
) -> Res<Result<(u64, u64), u64>> {
    let (mut callee, mut this) = (callee, this);
    if is_bound(callee) {
        let u = prepend_bound_args(cx, callee, UNDEFINED, args)?;
        (callee, this) = (u.target, u.this);
    }
    if object::is_function(callee) {
        return Ok(Ok((callee, this)));
    }
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let r = call::call_with(cx, callee, this, argc, |i| args[i as usize], UNDEFINED)?;
    (cx.out_fp, cx.out_pc) = (fp, s.next_pc);
    Ok(Err(r))
}

/// The `Construct*` callee that is not a FUNCTION, as [`call_other`]: a bound function
/// unwraps (§10.4.1.2: Construct(target, boundArgs ++ args, newTarget == F ? target :
/// newTarget)), and `Ok` gives the target and the new.target; a callable proxy is
/// constructed here (§10.5.13, `call::construct_with`), and `Err` gives the result.
#[inline(never)]
fn construct_other(
    cx: &mut Cx,
    fp: *mut u64,
    s: &CallSite,
    callee: u64,
    new_target: u64,
    args: &mut ArgBuf<u64>,
) -> Res<Result<(u64, u64), u64>> {
    let (mut callee, mut new_target) = (callee, new_target);
    if is_bound(callee) {
        let u = prepend_bound_args(cx, callee, new_target, args)?;
        (callee, new_target) = (u.target, u.new_target);
    }
    if object::is_function(callee) {
        return Ok(Ok((callee, new_target)));
    }
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let r = call::construct_with(cx, callee, argc, |i| args[i as usize], new_target)?;
    (cx.out_fp, cx.out_pc) = (fp, s.next_pc);
    Ok(Err(r))
}

#[inline]
fn runs_bytecode(fi: &FunctionInfo) -> bool {
    fi.is_bytecode() && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
}

// ---- host callees: the decode-free entry and the forwarded calls (M7-P0 CALLHOST) ----------
//
// ```text
// slow::call ──> host_entry: unprefixed CallAnyReceiver, CallProperty, CallProperty0-2,
//   CallUndefinedReceiver, CallUndefinedReceiver0-2 of a host callee; operands read at their
//   fixed places, no decode_call, no Vec (as native_entry, M2-P3-D21-ENTRY)
//     ──> forward: the callee is the realm's %Function.prototype.apply% or
//         %Function.prototype.call%, its `this` a bytecode function of this realm, and the
//         list (apply) undefined, null, an unmodified arguments object or a packed Array:
//         push the bytecode frame from the source, in this dispatch run (H20-Q3 rest)
//     ──> anything else: call::call_host_fn_from_bytecode (the call_host form), as do_call
//         does
//   ──> host_construct: unprefixed Construct of a host constructor (M8-P2 TS-HOSTCALL),
//       the same fixed places ──> call::call_host_fn_from_bytecode with new.target
//   ──> anything else: decode_call ──> do_call (a prefixed, spread or bound call of apply or
//       call runs the builtin)
// ```
//
// All three are human fast paths (FASTDIFF): `Cx::set_human_fast_paths(false)` turns them off,
// and every call then takes `decode_call`, `do_call` and the host function.
//
// M7-P4 LEVER-3: the T0 call fast path takes most forwarded calls before they come here
// (`call::forward_frame`: the same tests, with no call, no GC point and no throw; on x86-64
// from the cold call body, `interp::ops::forward_or_call`, M8-P0); `forward` takes the rest: a mapped arguments object, an Array with a hole or a double list, the call
// that makes the callee's feedback vector, a call whose sampled call record is due (on aarch64;
// x86-64 forwards it after the record) (`profile::record_call`), a budget at 0 (after the poll), a full VM stack (the RangeError),
// and every call while some isolate has its human fast paths off.

/// The 9 opcodes of the decode-free entries: the bytes CallAnyReceiver ..=
/// CallUndefinedReceiver2, in this order. At single scale every operand of each is one byte,
/// so operand `i` is at `pc + 1 + i` (checked at compile time below).
const FIXED_FIRST: u8 = ep9_bytecode::Opcode::CallAnyReceiver as u8;
const FIXED_LAST: u8 = ep9_bytecode::Opcode::CallUndefinedReceiver2 as u8;
/// `Construct`, the one construct opcode of [`host_entry`]: at single scale each of its 4
/// operands is one byte too (callee, list, count, feedback slot; checked below).
const CONSTRUCT: u8 = ep9_bytecode::Opcode::Construct as u8;
const _: () = {
    use ep9_bytecode::{Opcode as O, OperandScale};
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
    assert!(FIXED_LAST - FIXED_FIRST + 1 == ops.len() as u8);
    let mut i = 0;
    while i < ops.len() {
        let info = ops[i].info();
        assert!(ops[i] as u8 == FIXED_FIRST + i as u8);
        assert!(info.size(OperandScale::Single) == 1 + info.operands.len());
        i += 1;
    }
    // CallAnyReceiver and CallProperty have the same operands, so the same length.
    assert!(
        O::CallAnyReceiver.info().size(OperandScale::Single)
            == O::CallProperty.info().size(OperandScale::Single)
    );
    let construct = O::Construct.info();
    assert!(construct.operands.len() == 4);
    assert!(construct.size(OperandScale::Single) == 1 + construct.operands.len());
    // Outside the 9, so the range test of `host_entry` keeps it apart.
    assert!(CONSTRUCT < FIXED_FIRST || CONSTRUCT > FIXED_LAST);
};

/// A call of one of the 9 fixed-operand opcodes, read at the fixed places: the receiver, the
/// argument count and where the arguments are. `Copy` and read by value (M8-P2 TS-HOSTCALL),
/// so its fields stay in registers, not in a stack copy that each argument read loads.
#[derive(Clone, Copy)]
struct Fixed {
    this: u64,
    argc: u32,
    /// Argument `i` is in slot `list + i` (a register list); `None`: in slot `a` (`i` 0) or
    /// `b` (`i` 1).
    list: Option<isize>,
    a: isize,
    b: isize,
    /// The next instruction.
    next: *const u8,
}

impl Fixed {
    /// Argument `i` (`i < argc`), read from the frame `fp`.
    ///
    /// # Safety
    /// `fp` is the live frame of the instruction this was read from.
    #[inline(always)]
    unsafe fn arg(self, fp: *mut u64, i: u32) -> u64 {
        let s = match self.list {
            Some(first) => first + i as isize,
            None if i == 0 => self.a,
            None => self.b,
        };
        // SAFETY: a slot named by a register operand (or the list with its count) of the
        // verified instruction, in the live frame `fp` (caller contract).
        unsafe { *fp.offset(s) }
    }
}

/// The decode-free entry of a host callee (M7-P0 CALLHOST; the spike-native `native_entry`
/// form for builtins): an unprefixed `CallAnyReceiver`, `CallProperty`, `CallProperty0`-`2`,
/// `CallUndefinedReceiver` or `CallUndefinedReceiver0`-`2` whose callee is a host FUNCTION,
/// and (M8-P2 TS-HOSTCALL, [`host_construct`]) an unprefixed `Construct` of a host
/// constructor, with `acc` its new.target operand.
/// It reads the operands at their fixed single-scale places and the arguments straight from
/// the caller's registers: no `decode_call`, no `CallSite`, no `ArgBuf`, for any argument
/// count. Then [`forward`] or `call::call_host_fn_from_bytecode` (the `call_host` form for a
/// bytecode caller), with the receiver, arguments, return pc and result that `do_call` gives
/// them, and the realm switch of `call_host`.
///
/// Everything else returns `None` with no state changed, and takes `decode_call` and
/// `do_call`: a `Wide`/`ExtraWide` call (`pc` is at the prefix byte), `CallWithSpread`,
/// `ConstructWithSpread`, any callee that is not a host FUNCTION (bytecode, bound, proxy, not
/// callable), a `Construct` of a host function that is not a constructor (`do_call` throws
/// its TypeError), and every call while the human fast paths are off.
///
/// # Safety
/// `pc` is the call instruction (or its prefix) of verified code in the live top frame `fp`,
/// and no scratch Values lie above that frame.
#[inline(always)]
unsafe fn host_entry(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> Option<Res<u64>> {
    use crate::interp::dsl::R;
    use ep9_bytecode::{Opcode as O, OperandScale};

    // SAFETY: caller contract: `pc` is a byte of verified code.
    let op = unsafe { *pc };
    if !(FIXED_FIRST..=FIXED_LAST).contains(&op) {
        // Off the path of the 9: the `Construct` arm reads its own operands.
        return if op == CONSTRUCT {
            // SAFETY: caller contract, and `pc` is a `Construct`.
            unsafe { host_construct(cx, fp, pc, acc) }
        } else {
            None
        };
    }
    // SAFETY: operand `i` of one of the 9 call opcodes (checked above); the verifier checked
    // that the whole instruction lies in the code.
    let byte = |i: usize| unsafe { *pc.add(1 + i) };
    let reg = |i: usize| R::from_operand(i32::from(byte(i) as i8)).0;
    // SAFETY: slots named by register operands of this verified instruction (and each list
    // with its count), which lie in the live frame `fp`.
    let rd = |s: isize| unsafe { *fp.offset(s) };
    // Operand 0 is the callee in all 9.
    let callee = rd(reg(0));
    if !object::is_function(callee) {
        return None;
    }
    let fi = object::function_info(callee);
    let h = fi.host?;
    if !call::human_fast_paths(cx.rt()) {
        return None;
    }
    #[cfg(test)]
    call::HOST_ENTRY_HITS.with(|h| h.set(h.get() + 1));
    call::fast_hit!(0);
    let len = |o: O| pc.wrapping_add(o.info().size(OperandScale::Single));
    // Register lists hold locals only, so the list operand is the slot offset itself.
    let first = isize::from(byte(1) as i8);
    let mut f = Fixed {
        this: UNDEFINED,
        argc: 0,
        list: None,
        a: 0,
        b: 0,
        next: len(O::CallProperty),
    };
    const CALL_ANY_RECEIVER: u8 = O::CallAnyReceiver as u8;
    const CALL_PROPERTY: u8 = O::CallProperty as u8;
    const CALL_PROPERTY0: u8 = O::CallProperty0 as u8;
    const CALL_PROPERTY1: u8 = O::CallProperty1 as u8;
    const CALL_PROPERTY2: u8 = O::CallProperty2 as u8;
    const CALL_UNDEFINED: u8 = O::CallUndefinedReceiver as u8;
    const CALL_UNDEFINED0: u8 = O::CallUndefinedReceiver0 as u8;
    const CALL_UNDEFINED1: u8 = O::CallUndefinedReceiver1 as u8;
    match op {
        // The list holds the receiver, then the arguments; count 0: no receiver.
        CALL_ANY_RECEIVER | CALL_PROPERTY => {
            let n = u32::from(byte(2));
            if n > 0 {
                (f.this, f.argc, f.list) = (rd(first), n - 1, Some(first + 1));
            }
        }
        CALL_PROPERTY0 => (f.this, f.next) = (rd(reg(1)), len(O::CallProperty0)),
        CALL_PROPERTY1 => {
            (f.this, f.argc, f.next) = (rd(reg(1)), 1, len(O::CallProperty1));
            f.a = reg(2);
        }
        CALL_PROPERTY2 => {
            (f.this, f.argc, f.next) = (rd(reg(1)), 2, len(O::CallProperty2));
            (f.a, f.b) = (reg(2), reg(3));
        }
        CALL_UNDEFINED => {
            (f.argc, f.list) = (u32::from(byte(2)), Some(first));
            f.next = len(O::CallUndefinedReceiver);
        }
        CALL_UNDEFINED0 => f.next = len(O::CallUndefinedReceiver0),
        CALL_UNDEFINED1 => {
            (f.argc, f.next) = (1, len(O::CallUndefinedReceiver1));
            f.a = reg(1);
        }
        // CallUndefinedReceiver2 (the range check above).
        _ => {
            (f.argc, f.next) = (2, len(O::CallUndefinedReceiver2));
            (f.a, f.b) = (reg(1), reg(2));
        }
    }
    // SAFETY: `f` was read from this verified instruction of the live frame `fp`.
    let arg = move |i: u32| unsafe { f.arg(fp, i) };
    if let Some(r) = forward(cx, fp, f.next, callee, f.this, f.argc, arg) {
        return Some(r);
    }
    let r = call::call_host_fn_from_bytecode(
        cx, callee, fi, h, f.this, f.argc, arg, UNDEFINED, fp, f.next,
    );
    if r.is_ok() {
        cx.out_fp = fp;
        cx.out_pc = f.next;
    }
    Some(r)
}

/// The forwarded call (M7-P0 CALLHOST; H20-Q3 rest, MC-P1-APPLY): a call of the current
/// realm's `%Function.prototype.apply%` or `%Function.prototype.call%` (`callee`; each builtin
/// records itself there when it first runs, `Cx::forward_apply`, `Cx::forward_call`) whose
/// `this` is a bytecode function of this realm pushes that function's frame right here, as
/// `do_call` pushes a bytecode callee: return pc `next`, the dispatch goes on at its first
/// instruction in this run, and the host function never runs. No host frame, no list, no
/// nested dispatch run. The steps it skips have no observable effect in the cases it takes:
///
/// | Builtin step (§20.2.3.1, §20.2.3.3) | Here |
/// |---|---|
/// | IsCallable(func) | `func` is a bytecode FUNCTION, not a class constructor (`call::forward_target`) |
/// | `apply`: argArray undefined or null: Call(func, thisArg) | 0 arguments |
/// | `apply`: CreateListFromArrayLike(argArray) | an arguments object with its realm's initial shape (`arguments::unmodified_length`: `length` is its own int32 data property in field 0) and an element at each index below it, or an Array with an element at each index below its `length` (`call::forward_list`): each Get reads an own data property, so no getter, proxy trap or conversion runs; a mapped index reads its parameter |
/// | `call`: the arguments after thisArg | the call's own registers |
/// | Call(func, thisArg, args), OrdinaryCallBindThis | `call::try_bind_this`; a sloppy callee with a primitive thisArg takes the builtin (its wrap) |
///
/// Any other case returns `None` with no state changed, and the caller calls the host
/// function, which takes its spec steps (CreateListFromArrayLike in its order). As V8's
/// `CallOrConstructWithArrayLike` reads an unmodified arguments object in place
/// (`builtins-call-gen.cc:235-241`) and QuickJS's `build_arg_list` a fast array
/// (`quickjs.c:42338`), the arguments go straight from the source into the callee frame.
///
/// What it keeps from the builtin path: the feedback count of the callee (a GC point: `func`,
/// thisArg and the list stay rooted in the caller's registers; the heap does not move cells,
/// so the checks above still hold), one budget unit, also when the frame does not fit (R3-2),
/// and the RangeError of a full VM stack. A deep recursion through `apply` or `call` fills the
/// VM stack instead of the native one; both throw the same RangeError. The error stack is the
/// same: host frames are not in it (`error::capture`).
#[inline(always)]
fn forward(
    cx: &mut Cx,
    fp: *mut u64,
    next: *const u8,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Option<Res<u64>> {
    use crate::builtins::{IntrinsicId, intrinsic};
    // `this` is the function to forward to: a host call whose receiver is not a FUNCTION
    // (`s.charCodeAt(i)`) is refused with one kind test, before the two intrinsic loads
    // (M8-P2 TS-HOSTCALL; `call::forward_target` refuses it too).
    if !object::is_function(this) {
        return None;
    }
    let rt = cx.rt();
    let apply = callee == intrinsic(rt, IntrinsicId::FunctionPrototypeApply);
    if !apply && callee != intrinsic(rt, IntrinsicId::FunctionPrototypeCall) {
        return None;
    }
    if !call::human_fast_paths(rt) {
        return None;
    }
    let func = this;
    let fi = call::forward_target(cx, func)?;
    let this_arg = if argc > 0 { arg(0) } else { UNDEFINED };
    let this_arg = call::try_bind_this(fi, this_arg, rt.global())?;
    let r = if apply {
        let list = if argc > 1 { arg(1) } else { UNDEFINED };
        match call::forward_list(cx, list)? {
            ForwardList::Arguments(src, n) => {
                push_forwarded(cx, fp, next, func, fi, this_arg, n, |i| src.get(i))
            }
            ForwardList::Array(n) => push_forwarded(cx, fp, next, func, fi, this_arg, n, |i| {
                object::array_get(list, i).unwrap_or(UNDEFINED)
            }),
        }
    } else {
        push_forwarded(
            cx,
            fp,
            next,
            func,
            fi,
            this_arg,
            argc.saturating_sub(1),
            |i| arg(i + 1),
        )
    };
    #[cfg(test)]
    call::FORWARDED_CALLS.with(|h| h.set(h.get() + 1));
    call::fast_hit!(if apply { 1 } else { 2 });
    Some(r)
}

/// The frame push of [`forward`]: Call(`func`, `this`, the `n` arguments that `arg` reads)
/// as `do_call` makes it for a bytecode callee. GC point (the feedback count) before the push;
/// `arg` reads after it.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn push_forwarded(
    cx: &mut Cx,
    fp: *mut u64,
    next: *const u8,
    func: u64,
    fi: &FunctionInfo,
    this: u64,
    n: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    // GC point: `func`, thisArg and the list are in caller registers; `this` is one of them
    // or the global object.
    feedback::count_call(cx.rt(), cx.feedback_threshold, fi);
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (bytecode runs
    // with no scratch area), and `arg` reads Values the caller keeps rooted.
    let nfp = unsafe {
        call::push_frame(
            frame_top(fp),
            cx.stack_limit,
            fi,
            func,
            this,
            n,
            arg,
            fp,
            next,
            object::function_context(func),
            UNDEFINED,
        )
    };
    // One unit per call, also when the frame does not fit (R3-2), as in `do_call`.
    cx.budget -= 1;
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    cx.out_fp = nfp;
    cx.out_pc = fi.code;
    Ok(UNDEFINED)
}

/// The `Construct` arm of [`host_entry`] (M8-P2 TS-HOSTCALL): an unprefixed `Construct`
/// (`pc`; `acc` its new.target operand) whose callee is a host FUNCTION that is a
/// constructor. `new F(...args)` as the host arm of `do_call`'s construct path makes it, with
/// no `decode_call` and no `ArgBuf`: the operands at their fixed single-scale places (callee,
/// list, count; the feedback slot is not read), new.target = `acc` if it is a constructor
/// (BytecodeGen sets it) else the callee, then `call::call_host_fn_from_bytecode` with an
/// undefined receiver, the list registers as the arguments and the return pc of the next
/// instruction. No feedback count, no construct-site record and no budget unit, as in
/// `do_call` (a host callee has none).
///
/// Anything else returns `None` with no state changed: a callee that is not a host FUNCTION,
/// a host function that is not a constructor (`do_call` throws its TypeError), and every call
/// while the human fast paths are off (the switch of [`host_entry`]).
///
/// # Safety
/// As [`host_entry`], and `pc` is a `Construct` instruction.
#[inline(always)]
unsafe fn host_construct(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> Option<Res<u64>> {
    use crate::interp::dsl::R;
    use ep9_bytecode::{Opcode as O, OperandScale};

    // SAFETY: operand `i` of the `Construct` at `pc` (caller contract; every operand is one
    // byte at single scale, checked at compile time above); the verifier checked that the
    // whole instruction lies in the code.
    let byte = |i: usize| unsafe { *pc.add(1 + i) };
    // SAFETY: the callee register of this verified instruction, in the live frame `fp`.
    let callee = unsafe { *fp.offset(R::from_operand(i32::from(byte(0) as i8)).0) };
    if !object::is_function(callee) {
        return None;
    }
    let fi = object::function_info(callee);
    let h = fi.host?;
    if !fi.is_constructor() || !call::human_fast_paths(cx.rt()) {
        return None;
    }
    // The list holds the arguments (no receiver); a register list holds locals only, so the
    // list operand is the slot offset itself.
    let first = isize::from(byte(1) as i8);
    let argc = u32::from(byte(2));
    let next = pc.wrapping_add(O::Construct.info().size(OperandScale::Single));
    // SAFETY: the list registers of this verified instruction, with its count, in the live
    // frame `fp`.
    let arg = move |i: u32| unsafe { *fp.offset(first + i as isize) };
    #[cfg(test)]
    call::HOST_ENTRY_HITS.with(|h| h.set(h.get() + 1));
    call::fast_hit!(0);
    let new_target = if object::is_constructor(acc) {
        acc
    } else {
        callee
    };
    let r = call::call_host_fn_from_bytecode(
        cx, callee, fi, h, UNDEFINED, argc, arg, new_target, fp, next,
    );
    if r.is_ok() {
        cx.out_fp = fp;
        cx.out_pc = next;
    }
    Some(r)
}

/// The `!runs_bytecode` arm of [`do_call`]'s call path (M7-P5 A2-RW, route MC-w30-2): native
/// code (an AotBaseline unit function or a Baseline slot) that T0's `call_fast` sent cold is
/// entered here, nested, with its frame above the caller's registers and the return pc of the
/// next instruction (the `native.rs` T0 entry). The caller counts (M8-PLAN-D26): `count_call`
/// before the entry. Any other kind throws "not implemented". (At the end of the file.)
fn call_native_cold(
    cx: &mut Cx,
    fp: *mut u64,
    s: &CallSite,
    callee: u64,
    this: u64,
    argc: u32,
    args: &[u64],
) -> Res<u64> {
    let Some(nfi) = crate::native::native_callee(callee) else {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    };
    // The caller counts (M8-PLAN-D26, MC-w29-1), as `do_call` counts a T0 callee: GC point,
    // the callee, the receiver and the arguments are in caller registers (`args` copies them;
    // `this` is in a Rust local too).
    feedback::count_call(cx.rt(), cx.feedback_threshold, nfi);
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (bytecode runs
    // with no scratch area), `args` holds valid Value bits that the push copies before any GC
    // point, and `native_callee` checked the native slot.
    let r = unsafe {
        crate::native::enter_native(
            cx,
            frame_top(fp),
            fp,
            s.next_pc,
            callee,
            nfi,
            this,
            argc,
            |i| args[i as usize],
        )
    };
    // After the call: a nested dispatch under native code (`call_any`) sets them too.
    cx.out_fp = fp;
    cx.out_pc = s.next_pc;
    r
}

/// The `!runs_bytecode` arm of [`do_call`]'s construct path (M8-P1 A2-CONSTRUCT; M8-PLAN-D25,
/// MC-w29-8 step 1, MC-w36-3): `[[Construct]]` of a FUNCTION of the caller's realm with
/// native code (an AotBaseline unit function or a Baseline slot; `native::native_callee`, so
/// never a class constructor). The steps of the bytecode arm, in its order, then the native
/// entry instead of the stub and callee frames:
///
/// ```text
/// kind check (native_callee, else "not implemented") -> count_call (M8-PLAN-D26)
///   -> the receiver (OrdinaryCreateFromConstructor(new.target), §10.2.2 step 5)
///   -> construct_ic::miss (the site's record, as for the bytecode constructor: M-R9)
///   -> native::enter_native_with (new.target in fp - 3, the construct bit; return pc = the
///      next instruction; nested) -> call::construct_result (§10.2.2 steps 10-12)
/// ```
///
/// So a unit constructor counts and records what its bytecode does at this site. The frame
/// returns to this function, not to a construct stub frame, so the result check runs here,
/// through the helper of the Rust construct path. (At the end of the file, so the H23 pin
/// at line 600 and the other cited lines do not move.)
fn construct_native_cold(
    cx: &mut Cx,
    fp: *mut u64,
    s: &CallSite,
    callee: u64,
    new_target: u64,
    argc: u32,
    args: &[u64],
) -> Res<u64> {
    let Some(nfi) = crate::native::native_callee(callee) else {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    };
    // GC point, before `this` exists: the callee and the arguments are in caller registers
    // (`args` copies them), new.target is a Rust local (as in `do_call`).
    feedback::count_call(cx.rt(), cx.feedback_threshold, nfi);
    let this = if nfi.is_derived() {
        HOLE
    } else {
        // GC point; `this` then lives in a Rust local until the frame holds it, and after the
        // return until the result check.
        call::ordinary_create_from_constructor(cx, new_target)?
    };
    // The construct-site hook, as in `do_call` (it records the site's state). GC point.
    crate::construct_ic::miss(cx, fp, s, callee, new_target, this);
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame (bytecode runs
    // with no scratch area), `args` holds valid Value bits that the push copies before any GC
    // point, `native_callee` checked the native slot, and `this` is a new object in a Rust
    // local.
    let r = unsafe {
        crate::native::enter_native_with(
            cx,
            frame_top(fp),
            fp,
            s.next_pc,
            callee,
            nfi,
            this,
            argc,
            |i| args[i as usize],
            new_target,
            false,
        )
    };
    // After the call: a nested dispatch under native code (`call_any`) sets them too.
    cx.out_fp = fp;
    cx.out_pc = s.next_pc;
    call::construct_result(cx, r?, this)
}
