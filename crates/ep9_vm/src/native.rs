//! The native entry for spike-made code (Spike B, D21; feature `spike-native` only, never in
//! the ep9 graph: dep-guards check 11). Owners: P1-NATIVE-ENTRY, then P2-SPIKE-B.
//!
//! ```text
//! T0 call (slow/call.rs) --FUNCTION, not host, not class ctor, Baseline / AotBaseline
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
//! from Rust through `call::call_function_object` (for example `Cx::call` or a builtin
//! callback) throws "not implemented", and so do `[[Construct]]` and a bound-function call of a
//! native-slotted function (they take the T0 paths). `call_any` takes no spread call
//! (BytecodeGen's spread array needs the iterator path of `slow::call`). Generator functions
//! never get native code ([`InstallError::Generator`]): a resume pushes a new frame and runs
//! T0 (`call::run_resumed`).
//!
//! Realms (H22-Q3): native code runs in its function's realm, as T0 code does. The T0 entries
//! (`native_entry`, `call_native`, `enter_b_cold`, `call_fast`) take a callee of the caller's
//! realm only. A native-slotted or (b) callee of another realm is entered nested from
//! `slow::call` ([`call_in_its_realm`]), and `call_any`, [`call_from_rust`] and [`run_b`]
//! switch too: `realm::enter_realm_of` before OrdinaryCallBindThis, `realm::leave` after the
//! return or throw.

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
use crate::realm;
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
    /// Placement (b) (P4-PLACEMENT-B): unwinds the pending exception from the instruction at
    /// `off` of the (b) frame `fp` (`error::unwind`). It sets `Cx::out_fp`/`Cx::out_pc` to the
    /// frame and pc where the run goes on (a handler, or the EXIT stub) and returns the
    /// accumulator there: the exception, or 0 when no handler of this run catches it (unlike
    /// the other entries, 0 is a normal result here). (b) code then pops its native frame
    /// record and tail-jumps to `cx.table[*out_pc]` with `(cx, out_pc, out_fp, result)`.
    pub unwind: RtFn0,
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
    unwind: rt_unwind,
};

impl RuntimeTable {
    /// The entries in declaration order: name, byte offset, function type. The list is an
    /// H13 index list (H13-Q3, M3-P1 SP-UNIT): native code and unit text call an entry by its
    /// offset, so the list is append-only (`crates/ep9_vm/tests/golden/runtime-table.txt`).
    pub const ENTRIES: [(&'static str, usize, &'static str); 8] = [
        ("poll", layout::RT_POLL, "RtFn0"),
        ("call_any", layout::RT_CALL_ANY, "RtFn1"),
        ("add", layout::RT_ADD, "RtFn2"),
        ("inc", layout::RT_INC, "RtFn1"),
        ("compare", layout::RT_COMPARE, "RtCompareFn"),
        ("stack_overflow", layout::RT_STACK_OVERFLOW, "RtFn0"),
        ("throw", layout::RT_THROW, "RtFn1"),
        ("unwind", layout::RT_UNWIND, "RtFn0"),
    ];

    /// The one runtime table of the process (every isolate's `Cx::runtime_table` points at it).
    /// SP-UNIT's loader resolves the unit's runtime-table link cell to it (an external
    /// reference, H13), and its tests wrap its entries.
    pub fn process() -> &'static RuntimeTable {
        &RUNTIME_TABLE
    }
}

impl NativeCx {
    /// The `Cx` of `iso` as native code gets it (the isolate register). Only native code and
    /// the table functions may use the pointer, and only while `iso` lives and runs nothing
    /// else.
    pub fn of(iso: &mut Isolate) -> *mut NativeCx {
        core::ptr::from_mut::<Cx>(&mut iso.cx).cast::<NativeCx>()
    }

    /// `Cx::unit_data` of `iso` (M3-P1 SP-UNIT, H35-Q9; [`layout::CX_UNIT_DATA`]).
    pub fn unit_data(iso: &Isolate) -> usize {
        iso.cx.unit_data
    }

    /// Sets `Cx::unit_data` of `iso` and returns the old value. The value is the address of
    /// the isolate's unit-data directory (or 0): unit code reads it on every entry, so it must
    /// stay valid while code of a unit it names can run in `iso`.
    pub fn set_unit_data(iso: &mut Isolate, directory: usize) -> usize {
        core::mem::replace(&mut iso.cx.unit_data, directory)
    }

    /// The intrinsic `id` of the current realm (undefined if no table made it): the value a
    /// unit link cell of kind "intrinsic by stable index" (H13) resolves to.
    pub fn intrinsic(iso: &Isolate, id: crate::IntrinsicId) -> Value {
        val(crate::builtins::intrinsic(&iso.rt, id))
    }

    /// A new string with the text `s`: the value a unit link cell of kind "unit constant"
    /// resolves to. GC point. The result is not rooted: store it into a root (a unit link
    /// cell in a strong root range) before the next allocation.
    pub fn new_string(iso: &mut Isolate, s: &str) -> Value {
        val(crate::string::new_string(&iso.rt, s))
    }
}

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
    /// `Cx::unit_data` (usize; M3-P1 SP-UNIT, H35-Q9): the isolate's unit-data directory, one
    /// word per unit slot, each the base of the isolate's RW copy of the unit in that slot (0:
    /// none). Unit code loads its unit-data register (x27 / r14) from it on entry. 0 = no
    /// directory.
    pub const CX_UNIT_DATA: usize = offset_of!(Cx, unit_data);

    // ---- RuntimeTable ----
    pub const RT_POLL: usize = offset_of!(RuntimeTable, poll);
    pub const RT_CALL_ANY: usize = offset_of!(RuntimeTable, call_any);
    pub const RT_ADD: usize = offset_of!(RuntimeTable, add);
    pub const RT_INC: usize = offset_of!(RuntimeTable, inc);
    pub const RT_COMPARE: usize = offset_of!(RuntimeTable, compare);
    pub const RT_STACK_OVERFLOW: usize = offset_of!(RuntimeTable, stack_overflow);
    pub const RT_THROW: usize = offset_of!(RuntimeTable, throw);
    pub const RT_UNWIND: usize = offset_of!(RuntimeTable, unwind);

    // ---- placement (b) (P4-PLACEMENT-B) ----
    /// `Cx::table`: the 256 handler addresses of the dispatch (H15: at offset 0). (b) code
    /// leaves with a tail jump to `table[*pc]`.
    pub const CX_TABLE: usize = offset_of!(Cx, table);
    /// `Cx::out_fp` (`*mut u64`): the frame [`RuntimeTable::unwind`] continues in.
    pub const CX_OUT_FP: usize = offset_of!(Cx, out_fp);
    /// `Cx::out_pc` (`*const u8`): the pc [`RuntimeTable::unwind`] continues at.
    pub const CX_OUT_PC: usize = offset_of!(Cx, out_pc);
    /// FunctionInfo: the start of its bytecode (`*const u8`).
    pub const FI_CODE: usize = offset_of!(FunctionInfo, code);
    /// A frame's caller fp and return pc words are native words: `address +
    /// NATIVE_WORD_OFFSET` (frame.rs `native`; one bit, 2^49, so an `eor` decodes them).
    pub const NATIVE_WORD_OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;

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

/// Calls the native-slotted function `callee` (`fi`, from [`native_callee`]): pushes its frame at
/// `base` (receiver `this` after OrdinaryCallBindThis, `argc` arguments that `arg` reads, return
/// pc `return_pc`), publishes it, calls the slot entry, and publishes `caller_fp` again. `Err` when
/// native code returned 0 (exception pending; the caller unwinds); RangeError on a full VM stack.
///
/// # Safety
/// `caller_fp` is the live top frame, `base` the first free slot above it (and above any
/// scratch Values), `arg(i)` returns valid Value bits for `i < argc`, and the slot entry of
/// `fi` is native code with the [`NativeEntry`] ABI (`install_native` contract).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
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
    // OrdinaryCallBindThis: no GC point before the push, because `arg` may read memory that
    // the GC does not scan (the embedder slice of `call_from_rust`); a primitive receiver of a
    // sloppy callee goes in as it is and is wrapped after the push (`wrap_receiver`, H14-Q2).
    let bound = call::try_bind_this(fi, this, rt.global());
    // SAFETY: caller contract (the `push_frame` contract).
    let nfp = unsafe {
        call::push_frame(
            base,
            cx.stack_limit,
            fi,
            callee,
            bound.unwrap_or(this),
            argc,
            arg,
            caller_fp,
            return_pc,
            object::function_context(callee),
            UNDEFINED,
        )
    };
    cx.budget -= 1; // 1 unit per call, also when the frame does not fit (R3-2; D1).
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    if bound.is_none() {
        // SAFETY: `nfp` is the frame just pushed at `base`, above every live frame and scratch
        // Value, and not published yet.
        unsafe { wrap_receiver(cx, nfp, this) }?;
    }
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

/// OrdinaryCallBindThis step 6.b (§10.2.1.2) for a call that [`call::try_bind_this`] refused
/// (a primitive receiver of a sloppy callee, H14-Q2; M3-P0 P0-D10): ToObject(`this`) into the
/// receiver slot of the frame `nfp`, as `call::call_function_object` does. GC point: while the
/// wrap runs, the scratch top covers the new frame, so the GC scans its copied arguments and
/// its callee, and a stack walk does not see it (it is not published). The scratch top is put
/// back on every exit. `enter_native` and `run_b` call it after `push_frame`, because their
/// `arg` may read memory that the GC does not scan.
///
/// # Safety
/// `nfp` is the frame that `push_frame` just wrote above every live frame and every scratch
/// Value, and it is not published yet.
#[cold]
#[inline(never)]
unsafe fn wrap_receiver(cx: &mut Cx, nfp: *mut u64, this: u64) -> Res<()> {
    let stack = cx.rt().stack();
    let saved = stack.scratch_top();
    stack.set_scratch_top(crate::runtime::frame_top(nfp));
    let r = call::wrap_this(cx, this);
    if saved.is_null() {
        stack.clear_scratch();
    } else {
        stack.set_scratch_top(saved);
    }
    // SAFETY: the receiver slot of the frame that `push_frame` just wrote (caller contract).
    unsafe { *nfp.offset(slot::RECEIVER) = r? };
    Ok(())
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

    /// [`RuntimeTable::unwind`].
    ///
    /// # Safety
    /// As [`rt_poll`], with an exception pending.
    unsafe fn rt_unwind(cx: *mut NativeCx, fp: *mut u64, off: u32) -> u64 {
        // SAFETY: caller contract.
        let (cx, pc) = unsafe { enter(cx, fp, off) };
        if cx.rt().pending() == 0 {
            // A (b) code bug: unwind with nothing pending. Make it a catchable error.
            let msg = format!("ep9 internal error: native code unwinds at offset {off} with no exception");
            throw_error(cx, ErrorKind::Error, &msg);
        }
        // An offset outside the bytecode unwinds from the function's start (no handler of
        // this frame covers either: (b) code has no handlers).
        let pc = pc.unwrap_or(FrameRef(fp).info().code);
        crate::error::unwind(cx, fp, pc)
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
        // A callee of another realm runs in its realm (H22-Q3), from OrdinaryCallBindThis on.
        let saved = realm::enter_realm_of(cx, callee);
        // SAFETY: `fp` is the live top frame (published by `enter`), `base` is the first free
        // slot above it, `arg` reads registers of this verified instruction, and the callee's
        // slot holds native code (`native_callee`, `install_native` contract).
        let r = unsafe { enter_native(cx, base, fp, next, callee, cfi, this, argc, arg) };
        realm::leave(cx, saved);
        return r;
    }
    if let Some(bfi) = b_callee(callee) {
        let base = call::top_of(cx, fp);
        // SAFETY: as for `enter_native` above: `fp` is the live top frame, `base` the first
        // free slot above it, `arg` reads registers of this verified instruction, and
        // `b_callee` checked the (b) slot.
        return unsafe { run_b(cx, base, fp, callee, bfi, this, argc, arg) };
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
/// configs J and JC time `drive` through it). It takes and returns raw `Value`s by design.
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
/// Any other callee takes the T0 path of `Cx::call` (`call::call_with`), so this function
/// also calls T0, host and bound functions.
///
/// **State on every exit** (D20). Whether the call returns, throws, is terminated or fails its
/// stack check, it puts back what it found: the published top frame (`VmStack::fp`), the
/// scratch top, and `Cx::out_fp`/`out_pc`. So the VM stack depth after the call equals the
/// depth before it, and a later `call_from_rust` or `Cx::call` starts from the same
/// state. `call_function_object` does the same for the frame and the scratch top on its error
/// path. An exception or a termination comes back as `Err`, taken from the isolate as
/// `Isolate::with_cx` takes it (the pending exception is cleared).
///
/// The arguments are copied into the callee frame before the first GC point; `args` must hold
/// live values (as for `Cx::call`).
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
        let r = match (native_callee(f), b_callee(f)) {
            (Some(fi), _) => enter_from_rust(cx, f, fi, this, argc, arg),
            (None, Some(fi)) => b_from_rust(cx, f, fi, this, argc, arg),
            // No native code: the T0 path of `Cx::call`.
            (None, None) => call::call_with(cx, f, this, argc, arg, UNDEFINED),
        };
        // Nested T0 calls under native code set them (`slow::call`); put them back.
        cx.out_fp = out_fp;
        cx.out_pc = out_pc;
        r
    });
    // The hole never leaves the VM (as in `Cx::run_script`).
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
    // A function of another realm runs in its realm (H22-Q3), as `call_function_object`.
    let saved_realm = realm::enter_realm_of(cx, f);
    // SAFETY: `caller` is the published top frame (or null at the outermost entry), `base` is
    // the first free slot above it and above the caller's scratch Values (`top_of`), `arg`
    // reads live Values for `i < argc`, and `native_callee` checked that the slot of `fi`
    // holds native code (`install_native` contract).
    let r = unsafe { enter_native(cx, base, caller, EXIT_STUB.as_ptr(), f, fi, this, argc, arg) };
    realm::leave(cx, saved_realm);
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

/// T0's call (`slow::call`) of a native-slotted or (b) function `callee` of another realm
/// (H22-Q3): runs it nested, in its realm, as `call::call_function_object` runs a T0 callee
/// of another realm (one Rust level per cross-realm call). A native-slotted callee is
/// entered with return pc `next_pc` (as `call_native`), a (b) callee by a nested dispatch
/// ([`run_b`], which switches itself). `None` if `callee` has neither (the caller takes
/// `call_function_object`). The caller sets `out_fp`/`out_pc`.
///
/// # Safety
/// `fp` is the live top frame with no scratch Values above it, and `arg(i)` returns valid
/// Value bits for `i < argc` (it is read before the first GC point).
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn call_in_its_realm(
    cx: &mut Cx,
    fp: *mut u64,
    next_pc: *const u8,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Option<Res<u64>> {
    let base = crate::runtime::frame_top(fp);
    if let Some(fi) = native_callee(callee) {
        let saved = realm::enter_realm_of(cx, callee);
        // SAFETY: caller contract; `base` is the first free slot above `fp`, and
        // `native_callee` checked the native slot.
        let r = unsafe { enter_native(cx, base, fp, next_pc, callee, fi, this, argc, arg) };
        realm::leave(cx, saved);
        return Some(r);
    }
    let fi = b_callee(callee)?;
    // SAFETY: caller contract; `b_callee` checked the (b) slot.
    Some(unsafe { run_b(cx, base, fp, callee, fi, this, argc, arg) })
}

// ---- placement (b) (P4-PLACEMENT-B) ---------------------------------------------------------

/// The stub byte of the M2-P4-B-AB report: its handler runs the placement (a) code of an
/// AotBaseline slot that T0's `call_fast` entered (`a_hot_call`).
pub(crate) const OP_ENTER_A_HOT: u8 = 0xFC;
/// The pc `call_fast` and `slow::call` continue at after they pushed the frame of a (b)
/// callee: one `OP_ENTER_B` byte (never in verified bytecode).
pub(crate) static ENTER_B_STUB: [u8; 1] = [crate::interp::OP_ENTER_B];
/// The pc `call_fast` continues at after it pushed the frame of an A-hot callee.
pub(crate) static ENTER_A_HOT_STUB: [u8; 1] = [OP_ENTER_A_HOT];

const _: () = {
    use crate::interp::{OP_CONSTRUCT_RETURN, OP_ENTER_B, OP_EXIT};
    assert!(Opcode::from_u8(OP_ENTER_B).is_none());
    assert!(Opcode::from_u8(OP_ENTER_A_HOT).is_none());
    assert!(OP_ENTER_B != OP_EXIT && OP_ENTER_B != OP_CONSTRUCT_RETURN);
    assert!(OP_ENTER_A_HOT != OP_EXIT && OP_ENTER_A_HOT != OP_CONSTRUCT_RETURN);
    assert!(OP_ENTER_A_HOT != OP_ENTER_B);
};

/// The stub pc for a callee with slot kind byte `kind` that T0's `call_fast` enters on its hot
/// path: `ENTER_B_STUB` for a (b) slot (`DispatchKind::Opt`, [`install_native_b`]),
/// `ENTER_A_HOT_STUB` for an AotBaseline slot (the M2-P4-B-AB report). `None` for every other
/// kind (Baseline: placement (a), config A of the D21 bench, goes cold as before).
#[inline(always)]
pub(crate) fn hot_stub(kind: u8) -> Option<*const u8> {
    if kind == DispatchKind::Opt as u8 {
        Some(ENTER_B_STUB.as_ptr())
    } else if kind == DispatchKind::AotBaseline as u8 {
        Some(ENTER_A_HOT_STUB.as_ptr())
    } else {
        None
    }
}

/// The (b) entry of the function whose frame is `fp`, as a handler, and its bytecode start
/// (the pc the entry gets).
///
/// # Safety
/// `fp` is a live frame of a function with a (b) slot ([`install_native_b`]).
#[inline(always)]
pub(crate) unsafe fn b_target(fp: *mut u64) -> (crate::interp::Handler, *const u8) {
    let fi = FrameRef(fp).info();
    debug_assert_eq!(fi.slot.kind_byte(), DispatchKind::Opt as u8);
    #[cfg(test)]
    B_STUB_ENTRIES.with(|h| h.set(h.get() + 1));
    // SAFETY: the slot entry of a (b) slot is the address of code with the handler ABI
    // (`install_native_b` contract); fn pointers and data pointers have the same size.
    let h = unsafe {
        core::mem::transmute::<*const (), crate::interp::Handler>(
            core::ptr::with_exposed_provenance(fi.slot.entry()),
        )
    };
    (h, fi.code)
}

/// The bytecode start of the function whose frame is `fp`: the match loop runs it at a stub
/// byte (D4: the match loop never enters native code).
#[inline(always)]
pub(crate) fn bytecode_of(fp: *mut u64) -> *const u8 {
    FrameRef(fp).info().code
}

/// The FunctionInfo of `callee` if it has a (b) slot ([`install_native_b`]): a FUNCTION (not
/// bound, not host) whose slot kind is `DispatchKind::Opt`. The install refuses class
/// constructors and generator functions.
#[inline]
pub(crate) fn b_callee<'a>(callee: u64) -> Option<&'a FunctionInfo> {
    if !object::is_function(callee) {
        return None;
    }
    let fi = object::function_info(callee);
    (fi.slot.kind_byte() == DispatchKind::Opt as u8 && fi.host.is_none()).then_some(fi)
}

/// The M2-P4-B-AB report path (config A-hot of the D21 bench): T0's `call_fast` pushed the
/// frame `fp` of a placement (a) callee in an AotBaseline slot and the dispatch reached
/// `OP_ENTER_A_HOT`. This does what `enter_native` does after its frame push (publish the
/// frame, call the slot entry with the [`NativeEntry`] ABI, publish the caller again) and
/// sets the next state: the caller's next instruction with the result, or the unwind target
/// (`error::unwind` from the callee frame, which has no handlers) when native code returned
/// 0. Returns the accumulator for that state; sets `cx.out_fp`/`cx.out_pc`. Out of line, so the
/// stub handler keeps its tail jump (no indirect call in a handler, `cargo xtask disasm-check`).
///
/// # Safety
/// `fp` is the live top frame that `call_fast` pushed for a function whose slot holds native
/// code with the [`NativeEntry`] ABI (`install_native` contract).
#[inline(never)]
pub(crate) unsafe fn a_hot_call(cx: &mut Cx, fp: *mut u64) -> u64 {
    let f = FrameRef(fp);
    let fi = f.info();
    #[cfg(test)]
    A_HOT_ENTRIES.with(|h| h.set(h.get() + 1));
    let rt = cx.rt();
    rt.stack().set_fp(fp);
    // SAFETY: caller contract (the slot holds `NativeEntry` code).
    let entry: NativeEntry = unsafe {
        core::mem::transmute::<*const (), NativeEntry>(core::ptr::with_exposed_provenance(
            fi.slot.entry(),
        ))
    };
    // SAFETY: `fp` is the published frame of `fi`; native code gets the Cx as a raw pointer.
    let r = unsafe { entry(core::ptr::from_mut(cx).cast::<NativeCx>(), fp) };
    let caller = f.caller();
    rt.stack().set_fp(caller.0);
    if r == 0 {
        debug_assert!(
            rt.pending() != 0,
            "native code returned 0 with nothing pending"
        );
        return crate::error::unwind(cx, fp, fi.code);
    }
    cx.out_fp = caller.0;
    cx.out_pc = f.return_pc();
    r
}

/// Calls the (b) function `callee` (`fi` from [`b_callee`]) from Rust code that runs under a
/// frame (`call_any`, [`call_from_rust`]): pushes its frame at `base` with return pc = the
/// `Exit` stub, publishes it, runs a nested dispatch from `ENTER_B_STUB` (the (b) code under
/// `Dispatch::Sibling`/`Become`; the bytecode under `Dispatch::Match`), and publishes
/// `caller_fp` again. One nested native level, as a T0 callee of `call::call_with`; `Err`
/// when the run ends with an exception pending. The native and VM stack checks come first.
/// A callee of another realm runs in its realm (`realm::enter_realm_of`, `realm::leave`).
///
/// # Safety
/// `caller_fp` is the live top frame (or null), `base` is the first free slot above it and
/// above any scratch Values, and `arg(i)` returns valid Value bits for `i < argc`.
#[allow(clippy::too_many_arguments)]
unsafe fn run_b(
    cx: &mut Cx,
    base: *mut u64,
    caller_fp: *mut u64,
    callee: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    if !cx.rt().native_stack_ok() {
        return stack_overflow(cx);
    }
    // A callee of another realm runs in its realm (H22-Q3), from OrdinaryCallBindThis on.
    let saved = realm::enter_realm_of(cx, callee);
    // SAFETY: caller contract.
    let r = unsafe { run_b_in_realm(cx, base, caller_fp, callee, fi, this, argc, arg) };
    realm::leave(cx, saved);
    r
}

/// [`run_b`] after its native stack check and realm switch.
///
/// # Safety
/// As [`run_b`].
#[allow(clippy::too_many_arguments)]
unsafe fn run_b_in_realm(
    cx: &mut Cx,
    base: *mut u64,
    caller_fp: *mut u64,
    callee: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    let rt = cx.rt();
    // OrdinaryCallBindThis as in `enter_native`: wrap a primitive receiver after the push.
    let bound = call::try_bind_this(fi, this, rt.global());
    // SAFETY: caller contract (the `push_frame` contract).
    let nfp = unsafe {
        call::push_frame(
            base,
            cx.stack_limit,
            fi,
            callee,
            bound.unwrap_or(this),
            argc,
            arg,
            caller_fp,
            EXIT_STUB.as_ptr(),
            object::function_context(callee),
            UNDEFINED,
        )
    };
    // 1 unit per call, also when the frame does not fit, as in `enter_native` (R3-2, D1): a
    // loop that catches the RangeError of a full VM stack still reaches the next poll
    // (M3-P2-STOP).
    cx.budget -= 1;
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    if bound.is_none() {
        // SAFETY: as in `enter_native`.
        unsafe { wrap_receiver(cx, nfp, this) }?;
    }
    rt.stack().set_fp(nfp);
    rt.stack().clear_scratch();
    // SAFETY: `nfp` is the frame of `fi` just pushed and published; the stub byte enters its
    // (b) code (or its bytecode under the match loop), which ends at the `Exit` stub.
    let r = unsafe { crate::interp::run(cx, ENTER_B_STUB.as_ptr(), nfp) };
    rt.stack().set_fp(caller_fp);
    if r == 0 { Err(Throw) } else { Ok(r) }
}

/// [`call_from_rust`] for the (b) function `f` (`fi` from [`b_callee`]); the state rules of
/// [`enter_from_rust`].
fn b_from_rust(
    cx: &mut Cx,
    f: u64,
    fi: &FunctionInfo,
    this: u64,
    argc: u32,
    arg: impl Fn(u32) -> u64,
) -> Res<u64> {
    let stack = cx.rt().stack();
    let caller = stack.fp();
    if cx.budget <= 0 && slow::interrupt(cx, caller) == 0 {
        return Err(Throw);
    }
    let saved_scratch = stack.scratch_top();
    let base = call::top_of(cx, caller);
    // SAFETY: `caller` is the published top frame (or null), `base` is above it and its
    // scratch Values (`top_of`), `arg` reads live Values, `b_callee` checked the slot.
    let r = unsafe { run_b(cx, base, caller, f, fi, this, argc, arg) };
    stack.set_fp(caller);
    if saved_scratch.is_null() {
        stack.clear_scratch();
    } else {
        stack.set_scratch_top(saved_scratch);
    }
    r
}

/// Installs the placement (b) code `code` (its first byte is the entry) for the function `f`
/// (P4-PLACEMENT-B, plan-v6 D3): one dispatch slot write of kind `DispatchKind::Opt` and a
/// [`CodeEntry`] of that kind. The Opt byte is the spike's (b) marker only, so placement (a)
/// ([`install_native`], Baseline/AotBaseline) and (b) can run in one process; a production
/// (b) would use Baseline (the kind bytes are an H14/H35 contract). From then on:
///
/// | Caller | Path |
/// |---|---|
/// | T0 call, hot (`call_fast`) | frame push, then the stub byte `OP_ENTER_B`: a tail jump into the code with `(cx, pc, fp, acc)` |
/// | T0 call, cold (`slow::call`: budget poll, feedback threshold, prefixed call, spread, bound) | frame push, then `OP_ENTER_B` (no Rust recursion) |
/// | `call_any` from native code, [`call_from_rust`] | a nested dispatch from `OP_ENTER_B` (return pc = the `Exit` stub) |
/// | the match loop (`Dispatch::Match`) | the bytecode (D4) |
/// | `[[Construct]]`, a call from Rust through `call::call_with` (`Cx::call`, builtins) | "not implemented", as for placement (a) |
///
/// Refused before the slot changes, as [`install_native`]: not bytecode, a class constructor,
/// a generator function, a function that already has native code, a range the registry
/// refuses.
///
/// # Safety
/// `code.start` is the entry of code with the T0 handler ABI (`interp::Handler`: `(cx, pc, fp,
/// acc)`, the Rust ABI on aarch64, `sysv64` on x86-64) that runs `f`'s bytecode against the
/// frame `fp` it is entered with: it reads `cx` only through the [`layout`] offsets, calls
/// only [`RuntimeTable`] functions, keeps every callee-saved register, and leaves only by a
/// tail jump to `cx.table[*pc]` with `(cx, pc, fp', acc)`: after a `Return`, the frame's
/// return pc and caller fp; after a table result of 0, the state that
/// [`RuntimeTable::unwind`] set. The code stays mapped and unchanged until
/// [`uninstall_native`] or the isolate drops.
pub unsafe fn install_native_b(
    iso: &Isolate,
    f: Value,
    code: Range<usize>,
) -> Result<(), InstallError> {
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
        kind: DispatchKind::Opt,
        start: code.start,
        len: code.end.saturating_sub(code.start),
        function: core::ptr::from_ref(fi).addr(),
    };
    rt.registry
        .borrow_mut()
        .register(entry)
        .map_err(InstallError::Registry)?;
    fi.slot.install(DispatchKind::Opt, code.start);
    Ok(())
}

// Keep the frame layout the offsets above describe.
const _: () = {
    assert!(HEADER == 8);
    assert!(slot::PARAM0 == -10 && slot::BYTECODE_OFFSET == -1);
};

#[cfg(test)]
std::thread_local! {
    /// Tests only: how many calls this thread entered through the decode-free T0 entry
    /// (`slow::call` `native_entry`, M2-P3-D21-ENTRY), so a test can tell it from the decode
    /// path.
    pub(crate) static T0_ENTRY_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    /// Tests only: how many times this thread's dispatch reached `OP_ENTER_B` under a threaded
    /// expansion (`b_target`): the hot path, the cold arm and nested runs.
    pub(crate) static B_STUB_ENTRIES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    /// Tests only: how many (b) calls this thread took through the cold arm of `slow::call`.
    pub(crate) static B_COLD_ENTRIES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    /// Tests only: how many A-hot calls (`a_hot_call`) this thread ran.
    pub(crate) static A_HOT_ENTRIES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}
