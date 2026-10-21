//! The native entry for spike-made code (Spike B, D21; feature `spike-native` only, never in
//! the ep9 graph: dep-guards check 11). Owners: P1-NATIVE-ENTRY, then P2-SPIKE-B.
//!
//! ```text
//! T0 call (slow/call.rs do_call) --FUNCTION, not host, not class ctor, Baseline / AotBaseline
//!   slot, no construct, no spread--> enter_native: push_frame (return pc = next bytecode pc),
//!   publish fp --> slot entry (cx, fp) through extern "C" (aarch64) / extern "sysv64" (x86-64)
//!   --> acc bits, or 0 = exception pending --> T0 continues at the next pc, or unwinds
//!
//! native code --> cx.runtime_table (RUNTIME_TABLE) --> poll, call_any, add, inc, compare,
//!   stack_overflow, throw: (cx, fp, bytecode offset, values..) -> acc bits or 0
//! call_any --> native callee: enter_native (as above, nested)
//!          --> T0 or host callee, bound function, construct: call::call_with / construct_with
//! ```
//!
//! | Rule | Why |
//! |---|---|
//! | Native frames are canonical VM frames (H14) on the VM stack; the return pc is the bytecode pc after the call | stack traces, the GC root scan and the T0 code see one frame format (D21 placement (a): VM register stack + native control stack) |
//! | Entry and table ABIs: `extern "C"` (aarch64), `extern "sysv64"` (x86-64, every OS), never `-unwind` | a panic cannot unwind through native frames: it aborts at the table function's boundary (H23) |
//! | An exception is the result 0 (the pending exception is in the runtime) | native frames have no handlers in the spike; T0 unwinds from the caller's call instruction |
//! | Every table function records the state first (`Runtime::enter`): publishes `fp` and saves the bytecode offset in the frame's offset slot | GC root scan, stack traces, nested calls start above the native frame |
//! | Table functions take a bytecode offset, not a pc | the same code works for JIT code and for unit text (no relocation of bytecode addresses) |
//! | The T0 entry records no feedback and does not count the call (`feedback::count_call`) | the spike JIT records no feedback (plan-v3 §5 P2-SPIKE-B) |
//! | The T0 entry checks no native stack limit | the native prologue compares `Cx::native_limit` (H23-Q5); `call_any` checks it before every nested native or T0 entry, so T0↔native recursion ends in a catchable RangeError |
//!
//! Rust -> native: [`call_from_rust`] (P2-SPIKE-B, M2-P2-SPIKEB-RUSTENTRY) enters a
//! native-slotted function from Rust with an entry frame (return pc = the `Exit` stub), as
//! `call::call_function_object` does for T0. The general paths still refuse native code: a call
//! from Rust through `call::call_function_object` (for example `Isolate::call` or a builtin
//! callback) throws "not implemented", and so do `[[Construct]]` and a bound-function call of a
//! native-slotted function (they take the T0 paths). `call_any` takes no spread call
//! (BytecodeGen's spread array needs the iterator path of `slow::call`). Generator functions
//! never get native code ([`InstallError::Generator`]): a resume pushes a new frame and runs
//! T0 (`call::run_resumed`).

use core::ops::Range;

use ep9_bytecode::Opcode;
use ep9_value::{Throw, Value};

use crate::call;
use crate::conv::{BinOp, Res};
use crate::dispatch::{CodeEntry, CodeRegistry, DispatchKind, RegistryError};
use crate::error::{FrameRef, not_implemented, stack_overflow, throw_error};
use crate::frame::{HEADER, HOLE, UNDEFINED, slot};
use crate::interp::slow::{self, CmpOp, UnOp};
use crate::interp::{Cx, EXIT_STUB};
use crate::isolate::{Exception, Isolate};
use crate::object;
use crate::runtime::{ErrorKind, Runtime, val};
use crate::unit::FunctionInfo;

/// The `Cx` that native code gets and passes back to the table functions. Opaque: native code
/// reads its fields at the [`layout`] offsets only.
#[repr(C)]
pub struct NativeCx {
    _opaque: [u8; 0],
}

/// Declares function pointer types with the native-code ABI: `extern "sysv64"` on x86-64 (every
/// OS), `extern "C"` elsewhere (aarch64). Both are non-unwinding ABIs.
macro_rules! native_fn_types {
    ($($(#[$m:meta])* $name:ident = fn($($t:ty),*);)*) => {$(
        $(#[$m])*
        #[cfg(target_arch = "x86_64")]
        pub type $name = unsafe extern "sysv64" fn($($t),*) -> u64;
        $(#[$m])*
        #[cfg(not(target_arch = "x86_64"))]
        pub type $name = unsafe extern "C" fn($($t),*) -> u64;
    )*};
}

/// Defines functions with the native-code ABI (see `native_fn_types`).
macro_rules! native_fns {
    ($($(#[$m:meta])* unsafe fn $name:ident($($a:ident: $t:ty),* $(,)?) -> u64 $body:block)*) => {$(
        $(#[$m])*
        #[cfg(target_arch = "x86_64")]
        unsafe extern "sysv64" fn $name($($a: $t),*) -> u64 $body
        $(#[$m])*
        #[cfg(not(target_arch = "x86_64"))]
        unsafe extern "C" fn $name($($a: $t),*) -> u64 $body
    )*};
}

native_fn_types! {
    /// A native slot entry: `(cx, fp) -> acc`. `fp` is the function's own frame, pushed and
    /// published by the caller. The result is the accumulator bits, or 0 if an exception is
    /// pending.
    NativeEntry = fn(*mut NativeCx, *mut u64);
    /// A table function with no value operand: `(cx, fp, bytecode offset) -> acc or 0`.
    RtFn0 = fn(*mut NativeCx, *mut u64, u32);
    /// A table function with one value operand: `(cx, fp, bytecode offset, v) -> acc or 0`.
    RtFn1 = fn(*mut NativeCx, *mut u64, u32, u64);
    /// A table function with two value operands: `(cx, fp, bytecode offset, l, r) -> acc or 0`.
    RtFn2 = fn(*mut NativeCx, *mut u64, u32, u64, u64);
    /// The compare function: `(cx, fp, bytecode offset, op, l, r) -> true/false bits or 0`
    /// (`op` from [`cmp`]).
    RtCompareFn = fn(*mut NativeCx, *mut u64, u32, u32, u64, u64);
}

/// The runtime functions native code calls, at fixed offsets (`#[repr(C)]`, [`layout`]
/// `RT_*`), reached through `Cx::runtime_table`. Every function takes `(cx, fp, off, ..)`:
/// `cx` is the [`NativeCx`] native code got, `fp` its own frame, `off` the bytecode offset of
/// the instruction it runs. Each one first publishes `fp` and saves `off` in the frame's offset
/// slot (`Runtime::enter`), then returns the accumulator bits, or 0 if an exception is
/// pending. None of them unwinds: a panic inside one aborts the process at its non-unwinding
/// ABI boundary.
#[repr(C)]
#[derive(Debug)]
pub struct RuntimeTable {
    /// The interrupt poll (budget exhausted, for example at a `JumpLoop`): resets
    /// `Cx::budget` and handles termination (`slow::interrupt`). True bits, or 0 when the
    /// isolate terminates.
    pub poll: RtFn0,
    /// Runs the call or construct instruction at `off` (any `Call*` but `CallWithSpread`, and
    /// `Construct`) to its end and returns the result: a native callee is entered at once
    /// (nested), any other callee goes through `call::call_with` / `construct_with` (T0 code
    /// runs a nested dispatch). Operand: the accumulator (new.target for `Construct`).
    pub call_any: RtFn1,
    /// `l + r` (the `Add` cold path, `conv::binary`).
    pub add: RtFn2,
    /// `v + 1` after ToNumeric (the `Inc` cold path).
    pub inc: RtFn1,
    /// `l OP r` for `op` in [`cmp`] (the `Test*` cold paths, `conv::compare`).
    pub compare: RtCompareFn,
    /// Throws the catchable RangeError "Maximum call stack size exceeded" (a failed native
    /// stack compare in a prologue). Always 0.
    pub stack_overflow: RtFn0,
    /// `throw v` (the `Throw` cold path: records the throw site). Always 0.
    pub throw: RtFn1,
}

/// The `op` codes of [`RuntimeTable::compare`].
pub mod cmp {
    /// `l < r` (TestLessThan).
    pub const LT: u32 = 0;
    /// `l > r` (TestGreaterThan).
    pub const GT: u32 = 1;
    /// `l <= r` (TestLessThanOrEqual).
    pub const LE: u32 = 2;
    /// `l >= r` (TestGreaterThanOrEqual).
    pub const GE: u32 = 3;
    /// `l == r` (TestEqual, IsLooselyEqual).
    pub const EQ: u32 = 4;
}

/// The one runtime table of the process; every isolate's `Cx::runtime_table` points at it.
pub(crate) static RUNTIME_TABLE: RuntimeTable = RuntimeTable {
    poll: rt_poll,
    call_any: rt_call_any,
    add: rt_add,
    inc: rt_inc,
    compare: rt_compare,
    stack_overflow: rt_stack_overflow,
    throw: rt_throw,
};

/// Byte offsets native code reads and writes. All are fixed for the build (Cx and
/// FunctionInfo are `repr(C)`; FRAME 2 and OBJECT_LAYOUT 2, `versions.rs`).
pub mod layout {
    use core::mem::offset_of;

    use crate::dispatch::DispatchSlot;
    use crate::frame::{HEADER, slot};
    use crate::interp::Cx;
    use crate::object::fun;
    use crate::unit::FunctionInfo;

    use super::RuntimeTable;

    // ---- Cx (the isolate register points at it) ----
    /// `Cx::stack_limit`: VM frames must end at or below this slot (a `*mut u64`).
    pub const CX_STACK_LIMIT: usize = offset_of!(Cx, stack_limit);
    /// `Cx::budget` (i64): one unit per loop back edge and per function entry; call
    /// [`RuntimeTable::poll`] when it reaches 0.
    pub const CX_BUDGET: usize = offset_of!(Cx, budget);
    /// `Cx::global_this` (Value bits): the receiver a sloppy callee gets for undefined/null.
    pub const CX_GLOBAL_THIS: usize = offset_of!(Cx, global_this);
    /// `Cx::interrupt` (`*const AtomicU32`): the interrupt word.
    pub const CX_INTERRUPT: usize = offset_of!(Cx, interrupt);
    /// `Cx::native_limit` (usize): the lowest native stack address native code may reach;
    /// the prologue's one compare.
    pub const CX_NATIVE_LIMIT: usize = offset_of!(Cx, native_limit);
    /// `Cx::runtime_table` (`*const RuntimeTable`).
    pub const CX_RUNTIME_TABLE: usize = offset_of!(Cx, runtime_table);

    // ---- RuntimeTable ----
    pub const RT_POLL: usize = offset_of!(RuntimeTable, poll);
    pub const RT_CALL_ANY: usize = offset_of!(RuntimeTable, call_any);
    pub const RT_ADD: usize = offset_of!(RuntimeTable, add);
    pub const RT_INC: usize = offset_of!(RuntimeTable, inc);
    pub const RT_COMPARE: usize = offset_of!(RuntimeTable, compare);
    pub const RT_STACK_OVERFLOW: usize = offset_of!(RuntimeTable, stack_overflow);
    pub const RT_THROW: usize = offset_of!(RuntimeTable, throw);

    // ---- VM frames (frame.rs, H14): byte offsets from fp ----
    /// Header words between the receiver and `r0`.
    pub const FRAME_HEADER_WORDS: usize = HEADER;
    /// Caller fp (native word).
    pub const FP_CALLER_FP: isize = slot::CALLER_FP * 8;
    /// Return pc (native word): the bytecode pc after the call instruction.
    pub const FP_RETURN_PC: isize = slot::RETURN_PC * 8;
    /// The function object (Value).
    pub const FP_CALLEE: isize = slot::CALLEE * 8;
    /// Argument count (int32), plus the construct bit.
    pub const FP_ARGC: isize = slot::ARGC * 8;
    /// FunctionInfo (native word).
    pub const FP_FUNCTION_INFO: isize = slot::FUNCTION_INFO * 8;
    /// new.target (Value; undefined for a native call).
    pub const FP_NEW_TARGET: isize = slot::NEW_TARGET * 8;
    /// Current context (Value).
    pub const FP_CONTEXT: isize = slot::CONTEXT * 8;
    /// Bytecode offset of the current instruction (int32); the table functions write it.
    pub const FP_BYTECODE_OFFSET: isize = slot::BYTECODE_OFFSET * 8;
    /// The receiver.
    pub const FP_RECEIVER: isize = slot::RECEIVER * 8;

    /// Byte offset from fp of parameter `a<i>` (`i < max(argc, parameter_count)`).
    pub const fn fp_param(i: u32) -> isize {
        (slot::PARAM0 - i as isize) * 8
    }

    /// Byte offset from fp of register `r<i>`.
    pub const fn fp_register(i: u32) -> isize {
        i as isize * 8
    }

    // ---- FUNCTION cells and FunctionInfo ----
    /// FUNCTION cell field: its FunctionInfo (native word).
    pub const FUNCTION_INFO: usize = fun::INFO;
    /// FUNCTION cell field: its context (Value).
    pub const FUNCTION_CONTEXT: usize = fun::CONTEXT;
    /// FunctionInfo: the dispatch slot kind byte (`DispatchKind`).
    pub const FI_SLOT_KIND: usize = offset_of!(FunctionInfo, slot) + DispatchSlot::KIND_OFFSET;
    /// FunctionInfo: the dispatch slot entry (usize).
    pub const FI_SLOT_ENTRY: usize = offset_of!(FunctionInfo, slot) + DispatchSlot::ENTRY_OFFSET;
    /// FunctionInfo: register count (u32).
    pub const FI_REGISTER_COUNT: usize = offset_of!(FunctionInfo, register_count);
    /// FunctionInfo: parameter count (u32).
    pub const FI_PARAMETER_COUNT: usize = offset_of!(FunctionInfo, parameter_count);
}

// ---- the T0 entry --------------------------------------------------------------------------

/// The FunctionInfo of `callee` if a call (not a construct) enters it natively: a FUNCTION
/// cell (not bound), not a host function, not a class constructor, not a generator function,
/// with a Baseline or AotBaseline slot.
#[inline]
pub(crate) fn native_callee<'a>(callee: u64) -> Option<&'a FunctionInfo> {
    if !object::is_function(callee) {
        return None;
    }
    let fi = object::function_info(callee);
    let k = fi.slot.kind_byte();
    let native = k == DispatchKind::Baseline as u8 || k == DispatchKind::AotBaseline as u8;
    if fi.is_generator() {
        // `install_native` refuses generators (M2-P1-R1): a resume pushes a new frame and
        // runs T0 (`call::run_resumed`), and the runtime table has no generator entries.
        debug_assert!(!native, "generator function {:?} has native code", fi.name);
        return None;
    }
    (native && fi.host.is_none() && !fi.is_class_constructor()).then_some(fi)
}

/// Calls the native-slotted function `callee` (`fi`, from [`native_callee`]): pushes its frame
/// at `base` (receiver from `this` after OrdinaryCallBindThis, `argc` arguments that `arg`
/// reads, return pc `return_pc`), publishes it, calls the slot entry, and publishes
/// `caller_fp` again. `Err(Throw)` when native code returned 0 (the exception is pending); the
/// caller unwinds. RangeError if the frame does not fit the VM stack.
///
/// # Safety
/// `caller_fp` is the live top frame, `base` the first free slot above it (and above any
/// scratch Values), `arg(i)` returns valid Value bits for `i < argc`, and the slot entry of
/// `fi` is native code with the [`NativeEntry`] ABI (`install_native` contract).
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn enter_native(
    cx: &mut Cx,
    base: *mut u64,
    caller_fp: *mut u64,
    return_pc: *const u8,
    callee: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    let rt = cx.rt();
    let this = call::bind_this(fi, this, rt.global());
    // SAFETY: caller contract (the `push_frame` contract).
    let nfp = unsafe {
        call::push_frame(
            base,
            cx.stack_limit,
            fi,
            callee,
            this,
            argc,
            arg,
            caller_fp,
            return_pc,
            object::function_context(callee),
            UNDEFINED,
        )
    };
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    cx.budget -= 1;
    rt.stack().set_fp(nfp);
    // Native code runs with no scratch area, as bytecode does. Only an entry from Rust
    // ([`call_from_rust`]) can find one here; it puts the caller's scratch top back after the
    // call. The T0 and `call_any` paths never have one, so this is a no-op for them.
    rt.stack().clear_scratch();
    // SAFETY: the slot entry is the address of a function with the `NativeEntry` ABI (caller
    // contract); fn pointers and data pointers have the same size on every target.
    let entry: NativeEntry = unsafe {
        core::mem::transmute::<*const (), NativeEntry>(core::ptr::with_exposed_provenance(
            fi.slot.entry(),
        ))
    };
    // SAFETY: `nfp` is the frame of `fi` that was just pushed and published; native code gets
    // the Cx as a raw pointer and the table functions reborrow it one at a time.
    let r = unsafe { entry(core::ptr::from_mut(cx).cast::<NativeCx>(), nfp) };
    rt.stack().set_fp(caller_fp);
    if r == 0 {
        debug_assert!(
            rt.pending() != 0,
            "native code of {:?} returned 0 without a pending exception",
            fi.name
        );
        return Err(Throw);
    }
    Ok(r)
}

// ---- the runtime table ---------------------------------------------------------------------

/// The `&mut Cx` of a table function, and the pc of bytecode offset `off` in the frame `fp`
/// (`None` if `off` is outside the function's bytecode). Publishes `fp` and saves `off` in its
/// offset slot (`Runtime::enter`).
///
/// # Safety
/// `cx` is the pointer native code got from [`enter_native`] and `fp` its live frame; no other
/// reference to the Cx is in use.
unsafe fn enter<'c>(cx: *mut NativeCx, fp: *mut u64, off: u32) -> (&'c mut Cx, Option<*const u8>) {
    // SAFETY: caller contract: `cx` came from `&mut Cx` in `enter_native`, whose borrow is
    // not used while native code runs.
    let cx = unsafe { &mut *cx.cast::<Cx>() };
    let fi = FrameRef(fp).info();
    let pc = fi.code.wrapping_add(off as usize);
    // Publishes `fp`; saves the offset only when `pc` lies in the bytecode.
    cx.rt().enter(fp, pc);
    let pc = (fi.is_bytecode() && off < fi.code_len).then_some(pc);
    (cx, pc)
}

/// The internal error for a bytecode offset outside the frame's function. Returns 0.
fn bad_offset(cx: &mut Cx, off: u32) -> u64 {
    throw_error(
        cx,
        ErrorKind::Error,
        &format!("ep9 internal error: native code passed bytecode offset {off}"),
    );
    0
}

#[inline]
fn ret(r: Res<u64>) -> u64 {
    r.unwrap_or(0)
}

native_fns! {
    /// [`RuntimeTable::poll`].
    ///
    /// # Safety
    /// Native code calls it with its own `cx` and live frame `fp` ([`enter`]).
    unsafe fn rt_poll(cx: *mut NativeCx, fp: *mut u64, off: u32) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        if pc.is_none() {
            return bad_offset(cx, off);
        }
        slow::interrupt(cx, fp)
    }

    /// [`RuntimeTable::call_any`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_call_any(cx: *mut NativeCx, fp: *mut u64, off: u32, acc: u64) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        let Some(pc) = pc else {
            return bad_offset(cx, off);
        };
        // The poll of a call instruction (as `slow::call`).
        if cx.budget <= 0 && slow::interrupt(cx, fp) == 0 {
            return 0;
        }
        ret(call_at(cx, fp, pc, acc))
    }

    /// [`RuntimeTable::add`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_add(cx: *mut NativeCx, fp: *mut u64, off: u32, l: u64, r: u64) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        let Some(pc) = pc else {
            return bad_offset(cx, off);
        };
        slow::binary(cx, fp, pc, BinOp::Add, l, r)
    }

    /// [`RuntimeTable::inc`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_inc(cx: *mut NativeCx, fp: *mut u64, off: u32, v: u64) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        let Some(pc) = pc else {
            return bad_offset(cx, off);
        };
        slow::unary(cx, fp, pc, UnOp::Inc, v)
    }

    /// [`RuntimeTable::compare`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_compare(cx: *mut NativeCx, fp: *mut u64, off: u32, op: u32, l: u64, r: u64) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        let Some(pc) = pc else {
            return bad_offset(cx, off);
        };
        let op = match op {
            cmp::LT => CmpOp::Lt,
            cmp::GT => CmpOp::Gt,
            cmp::LE => CmpOp::Le,
            cmp::GE => CmpOp::Ge,
            cmp::EQ => CmpOp::Eq,
            _ => {
                let msg = format!("ep9 internal error: native code passed compare op {op}");
                throw_error(cx, ErrorKind::Error, &msg);
                return 0;
            }
        };
        slow::compare(cx, fp, pc, op, l, r)
    }

    /// [`RuntimeTable::stack_overflow`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_stack_overflow(cx: *mut NativeCx, fp: *mut u64, off: u32) -> u64 {
        // SAFETY: caller contract.
        let (cx, _) = unsafe { enter(cx, fp, off) };
        ret(stack_overflow(cx))
    }

    /// [`RuntimeTable::throw`].
    ///
    /// # Safety
    /// As [`rt_poll`].
    unsafe fn rt_throw(cx: *mut NativeCx, fp: *mut u64, off: u32, v: u64) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        let Some(pc) = pc else {
            return bad_offset(cx, off);
        };
        slow::throw(cx, fp, pc, v, false)
    }
}

/// The argument registers of a call instruction (frame slot offsets), without a heap copy.
#[derive(Clone, Copy)]
enum ArgSlots {
    /// A register list: `n` slots from `first` up.
    List { first: isize, n: u32 },
    /// Up to two single registers.
    Few { regs: [isize; 2], n: u32 },
}

impl ArgSlots {
    fn len(self) -> u32 {
        match self {
            ArgSlots::List { n, .. } | ArgSlots::Few { n, .. } => n,
        }
    }

    fn slot(self, i: u32) -> isize {
        match self {
            ArgSlots::List { first, .. } => first + i as isize,
            ArgSlots::Few { regs, .. } => regs[i as usize],
        }
    }
}

/// Runs the call or construct instruction at `pc` of the frame `fp` to its end (see
/// [`RuntimeTable::call_any`]). The operand decoding follows `call::decode_call`, with no
/// heap copy of the argument list.
fn call_at(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> Res<u64> {
    use Opcode as O;
    let fi = FrameRef(fp).info();
    // SAFETY: the code of a bytecode FunctionInfo is `code_len` bytes of verified bytecode
    // (`enter` checked that `pc` lies in it).
    let code = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let off = pc.addr() - fi.code.addr();
    let Ok(insn) = ep9_bytecode::decode_at(code, off as u32) else {
        return Err(not_call(cx, off));
    };
    let reg = |i: usize| -> isize {
        let o = insn.operand(i) as isize;
        if o < 0 { o - HEADER as isize } else { o }
    };
    // SAFETY: register operands of verified code name slots of the live frame `fp`.
    let rd = |s: isize| unsafe { *fp.offset(s) };
    let list = |i: usize, n: usize| ArgSlots::List {
        first: insn.operand(i) as isize,
        n: n as u32,
    };
    let few = |regs: [isize; 2], n: u32| ArgSlots::Few { regs, n };
    let (this, args, construct) = match insn.opcode {
        O::CallAnyReceiver | O::CallProperty => match insn.operand(2) as usize {
            0 => (UNDEFINED, list(1, 0), false),
            n => {
                let first = insn.operand(1) as isize;
                let args = ArgSlots::List {
                    first: first + 1,
                    n: n as u32 - 1,
                };
                (rd(first), args, false)
            }
        },
        O::CallProperty0 => (rd(reg(1)), few([0, 0], 0), false),
        O::CallProperty1 => (rd(reg(1)), few([reg(2), 0], 1), false),
        O::CallProperty2 => (rd(reg(1)), few([reg(2), reg(3)], 2), false),
        O::CallUndefinedReceiver => (UNDEFINED, list(1, insn.operand(2) as usize), false),
        O::CallUndefinedReceiver0 => (UNDEFINED, few([0, 0], 0), false),
        O::CallUndefinedReceiver1 => (UNDEFINED, few([reg(1), 0], 1), false),
        O::CallUndefinedReceiver2 => (UNDEFINED, few([reg(1), reg(2)], 2), false),
        O::Construct => (UNDEFINED, list(1, insn.operand(2) as usize), true),
        O::CallWithSpread | O::ConstructWithSpread => {
            return Err(not_implemented(cx, "spread calls from native code"));
        }
        _ => return Err(not_call(cx, off)),
    };
    let callee = rd(reg(0));
    let argc = args.len();
    let arg = |i: u32| rd(args.slot(i));
    if construct {
        // As `slow::call`: new.target is the accumulator if it is a constructor, else the
        // callee.
        let new_target = if object::is_constructor(acc) {
            acc
        } else {
            callee
        };
        return call::construct_with(cx, callee, argc, arg, new_target);
    }
    if let Some(cfi) = native_callee(callee) {
        if !cx.rt().native_stack_ok() {
            return stack_overflow(cx);
        }
        let base = call::top_of(cx, fp);
        let next = pc.wrapping_add(insn.size() as usize);
        // SAFETY: `fp` is the live top frame (published by `enter`), `base` is the first free
        // slot above it, `arg` reads registers of this verified instruction, and the callee's
        // slot holds native code (`native_callee`, `install_native` contract).
        return unsafe { enter_native(cx, base, fp, next, callee, cfi, this, argc, arg) };
    }
    call::call_with(cx, callee, this, argc, arg, UNDEFINED)
}

/// The internal error for an offset that holds no call instruction.
fn not_call(cx: &mut Cx, off: usize) -> Throw {
    let msg = format!("ep9 internal error: native call_any at offset {off}, not a call");
    throw_error(cx, ErrorKind::Error, &msg)
}

// ---- the spike API -------------------------------------------------------------------------

/// Why [`install_native`] or [`uninstall_native`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The value is not a FUNCTION with bytecode (a host or bound function, or no function).
    NotBytecode,
    /// A class constructor: T0 never enters one natively (`[[Call]]` throws, and
    /// `[[Construct]]` stays in T0).
    ClassConstructor,
    /// A generator function (`FunctionInfo::is_generator`; M2-P1-R1): its frames are entered
    /// at the `SwitchOnGeneratorState` prologue on every resume (`call::run_resumed`, T0
    /// only), and native code has no suspend or resume path.
    Generator,
    /// The kind is not [`DispatchKind::Baseline`] or [`DispatchKind::AotBaseline`].
    Kind(DispatchKind),
    /// The function already has native code ([`uninstall_native`] first).
    AlreadyNative,
    /// The function runs T0 (nothing to uninstall).
    NotNative,
    /// The code registry refused the range.
    Registry(RegistryError),
}

/// What native code for a function needs to know about it.
#[derive(Clone, Copy, Debug)]
pub struct NativeTarget<'i> {
    /// The function's name (may be empty).
    pub name: &'i str,
    /// Its verified bytecode; table functions take offsets into it.
    pub bytecode: &'i [u8],
    pub parameter_count: u32,
    pub register_count: u32,
    pub strict: bool,
    /// Entries in the function's handler table (try/catch/finally ranges). Native code of the
    /// spike has no handlers, so a JIT refuses a function that has any.
    pub handler_count: u32,
    /// The current slot kind.
    pub kind: Option<DispatchKind>,
    /// The FunctionInfo address ([`CodeEntry::function`] of its registry entries).
    pub info: usize,
}

/// The bytecode FunctionInfo of the function `f`, if `f` is a FUNCTION with bytecode.
fn bytecode_info<'i>(f: Value) -> Result<&'i FunctionInfo, InstallError> {
    let b = f.to_bits();
    if !object::is_function(b) {
        return Err(InstallError::NotBytecode);
    }
    let fi = object::function_info(b);
    if fi.host.is_some() || !fi.is_bytecode() || fi.code.is_null() {
        return Err(InstallError::NotBytecode);
    }
    Ok(fi)
}

/// The facts native code for the function `f` needs, or `None` if `f` is not a FUNCTION with
/// bytecode. The bytecode lives as long as the isolate (loaded units are never freed).
pub fn native_target(_iso: &Isolate, f: Value) -> Option<NativeTarget<'_>> {
    let fi = bytecode_info(f).ok()?;
    // SAFETY: a bytecode FunctionInfo's code is `code_len` bytes that its loaded unit owns;
    // units live until the isolate drops, and the result borrows the isolate.
    let bytecode = unsafe { core::slice::from_raw_parts(fi.code, fi.code_len as usize) };
    let handler_count = fi.loaded_unit().map_or(0, |u| {
        u.unit.functions[fi.function_id as usize].handlers.len() as u32
    });
    Some(NativeTarget {
        name: &fi.name,
        bytecode,
        parameter_count: fi.parameter_count,
        register_count: fi.register_count,
        strict: fi.is_strict(),
        handler_count,
        kind: fi.slot.kind(),
        info: core::ptr::from_ref(fi).addr(),
    })
}

/// Installs the native code `code` (its first byte is the entry) for the function `f`: one
/// dispatch slot write of `kind` (every closure of the function shares the slot, H14), plus a
/// [`CodeEntry`] in the isolate's code registry. From then on a T0 call of `f` (no construct,
/// no spread) enters the code natively. Refused before the slot changes: a function that is
/// not bytecode, a class constructor, a generator function, a function that already has
/// native code, or a range the registry refuses.
///
/// # Safety
/// `code.start` is the entry of a function with the [`NativeEntry`] ABI that runs `f`'s
/// bytecode against its VM frame and follows the [`RuntimeTable`] rules (0 only with a
/// pending exception, valid Value bits otherwise). The code stays mapped and unchanged until
/// [`uninstall_native`] or the isolate drops.
pub unsafe fn install_native(
    iso: &Isolate,
    f: Value,
    kind: DispatchKind,
    code: Range<usize>,
) -> Result<(), InstallError> {
    if !matches!(kind, DispatchKind::Baseline | DispatchKind::AotBaseline) {
        return Err(InstallError::Kind(kind));
    }
    let fi = bytecode_info(f)?;
    if fi.is_class_constructor() {
        return Err(InstallError::ClassConstructor);
    }
    if fi.is_generator() {
        return Err(InstallError::Generator);
    }
    if fi.slot.kind_byte() != DispatchKind::Interp as u8 {
        return Err(InstallError::AlreadyNative);
    }
    let rt: &Runtime = &iso.rt;
    let entry = CodeEntry {
        kind,
        start: code.start,
        len: code.end.saturating_sub(code.start),
        function: core::ptr::from_ref(fi).addr(),
    };
    rt.registry
        .borrow_mut()
        .register(entry)
        .map_err(InstallError::Registry)?;
    fi.slot.install(kind, code.start);
    Ok(())
}

/// Puts the function `f` back on T0 (slot kind Interp, entry = its bytecode) and removes the
/// registry entry [`install_native`] added. Returns that entry.
pub fn uninstall_native(iso: &Isolate, f: Value) -> Result<CodeEntry, InstallError> {
    let fi = bytecode_info(f)?;
    if fi.slot.kind_byte() == DispatchKind::Interp as u8 {
        return Err(InstallError::NotNative);
    }
    let rt: &Runtime = &iso.rt;
    let start = fi.slot.entry();
    fi.slot
        .install(DispatchKind::Interp, fi.code.expose_provenance());
    rt.registry
        .borrow_mut()
        .unregister(start)
        .ok_or(InstallError::NotNative)
}

/// The registry entry that holds `pc` in the isolate's code registry (H14: JIT, unit and
/// bytecode ranges).
pub fn lookup_code(iso: &Isolate, pc: usize) -> Option<CodeEntry> {
    iso.rt.registry.borrow().lookup(pc)
}

// ---- Rust -> native ------------------------------------------------------------------------

/// Call(`f`, `this`, `args`) from Rust into native code (M2-P2-SPIKEB-RUSTENTRY; Spike B
/// configs J and JC time `drive` through it). The types are those of `Isolate::call`.
///
/// A function with native code ([`install_native`]; a FUNCTION with a Baseline or AotBaseline
/// slot, not a class constructor or generator) is entered through the T0 entry path
/// (`enter_native`) with an entry frame, as `call::call_function_object` enters T0 code:
///
/// | Step | Here | As `call_function_object` |
/// |---|---|---|
/// | native stack check | `Runtime::native_stack_ok`, else a RangeError | same |
/// | interrupt poll | when the budget is out (`slow::interrupt`), else termination | `call::poll` |
/// | frame | pushed above the top frame and the caller's scratch Values (`call::top_of`), return pc = the `Exit` stub, `this` after OrdinaryCallBindThis | `push_frame` with `EXIT_STUB` |
/// | run | the slot entry, with the new frame published and no scratch area | `interp::run` |
///
/// Any other callee takes the T0 path of `Isolate::call` (`call::call_with`), so this function
/// also calls T0, host and bound functions.
///
/// **State on every exit** (D20). Whether the call returns, throws, is terminated or fails its
/// stack check, it puts back what it found: the published top frame (`VmStack::fp`), the
/// scratch top, and `Cx::out_fp`/`out_pc`. So the VM stack depth after the call equals the
/// depth before it, and a later `call_from_rust` or `Isolate::call` starts from the same
/// state. `call_function_object` does the same for the frame and the scratch top on its error
/// path. An exception or a termination comes back as `Err`, taken from the isolate as
/// `Isolate::call` takes it (the pending exception is cleared).
///
/// The arguments are copied into the callee frame before the first GC point; `args` must hold
/// live values (as for `Isolate::call`).
pub fn call_from_rust(
    iso: &mut Isolate,
    f: Value,
    this: Value,
    args: &[Value],
) -> Result<Value, Exception> {
    let (f, this) = (f.to_bits(), this.to_bits());
    let r = iso.with_cx(|acx| {
        let cx = acx.inner();
        let Ok(argc) = u32::try_from(args.len()) else {
            return stack_overflow(cx);
        };
        let arg = |i: u32| args[i as usize].to_bits();
        let (out_fp, out_pc) = (cx.out_fp, cx.out_pc);
        let r = match native_callee(f) {
            Some(fi) => enter_from_rust(cx, f, fi, this, argc, arg),
            // No native code: the T0 path of `Isolate::call`.
            None => call::call_with(cx, f, this, argc, arg, UNDEFINED),
        };
        // Nested T0 calls under native code set them (`slow::call`); put them back.
        cx.out_fp = out_fp;
        cx.out_pc = out_pc;
        r
    });
    // The hole never leaves the VM (as `Isolate::call`).
    r.map(|b| val(if b == HOLE { UNDEFINED } else { b }))
}

/// [`call_from_rust`] for the native-slotted function `f` (`fi` from [`native_callee`]).
fn enter_from_rust(
    cx: &mut Cx,
    f: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let stack = rt.stack();
    let caller = stack.fp();
    // The interrupt poll of a function entry from Rust (`call::poll`).
    if cx.budget <= 0 && slow::interrupt(cx, caller) == 0 {
        return Err(Throw);
    }
    let saved_scratch = stack.scratch_top();
    let base = call::top_of(cx, caller);
    // SAFETY: `caller` is the published top frame (or null at the outermost entry), `base` is
    // the first free slot above it and above the caller's scratch Values (`top_of`), `arg`
    // reads live Values for `i < argc`, and `native_callee` checked that the slot of `fi`
    // holds native code (`install_native` contract).
    let r = unsafe { enter_native(cx, base, caller, EXIT_STUB.as_ptr(), f, fi, this, argc, arg) };
    // Put back the frame state this call found, on every exit (`enter_native` already
    // published `caller` again, except when its frame push failed). `call_from_rust` puts
    // back `out_fp`/`out_pc`.
    stack.set_fp(caller);
    if saved_scratch.is_null() {
        stack.clear_scratch();
    } else {
        stack.set_scratch_top(saved_scratch);
    }
    r
}

// Keep the frame layout the offsets above describe.
const _: () = {
    assert!(HEADER == 8);
    assert!(slot::PARAM0 == -10 && slot::BYTECODE_OFFSET == -1);
};
