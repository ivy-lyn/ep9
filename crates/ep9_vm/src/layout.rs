//! The layout facts native code bakes in (C-18, doc 13 C-18; M5-P0 P0-VM5): byte offsets of
//! the interpreter `Cx`, FunctionInfo, VM frames, cells and IC data, kind ids, and the
//! RuntimeTable that native code calls. Not a stable API (`#[doc(hidden)]`): JIT code, AOT
//! unit text and Spike C read these values, so each value is part of one contract version
//! (`versions.rs`).
//!
//! Until M5-P0 this was `native::layout`, behind the `spike-native` feature. C-18 (doc 13:672)
//! moved it out of the gate, with the three native `Cx` fields (`native_limit`,
//! `runtime_table`, `unit_data`) and the [`RuntimeTable`] type, so every build has one `Cx`
//! layout. The process table itself (`native::RUNTIME_TABLE`) and every function it calls stay
//! behind `spike-native`: a default build has `Cx::runtime_table` = null.
//!
//! Each export names the one version that owns it (MC-w15-5; doc 13 C-18 and §7.3): VM_LAYOUT
//! (`Cx`, FunctionInfo and RuntimeTable offsets, the table length), FRAME (frame offsets from
//! fp, MC-w16-18), OBJECT_LAYOUT (cells, fields and kind ids), IC_LAYOUT (feedback and IC
//! data) or the Value encoding. The current numbers are in `versions.rs`; a change of any
//! value moves its owner's version.
//!
//! No item here is read by T0 code; the module adds no instruction to a handler.

use core::mem::offset_of;

use crate::dispatch::DispatchSlot;
use crate::feedback::fv;
use crate::frame::{HEADER, slot};
use crate::ic::smi;
use crate::interp::Cx;
use crate::elements::kind as ek;
use crate::object::{arr, ctxf, delems, elems, fun, obj};
use crate::unit::FunctionInfo;

/// The `Cx` that native code gets and passes back to the table functions. Opaque: native code
/// reads its fields at the offsets of this module only.
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
    /// (`op` from `native::cmp`, feature `spike-native`).
    RtCompareFn = fn(*mut NativeCx, *mut u64, u32, u32, u64, u64);
    /// A table function with three value operands: `(cx, fp, bytecode offset, a, b, c) -> acc
    /// or 0` (M4-P3 KEYED: [`RuntimeTable::set_keyed`]).
    RtFn3 = fn(*mut NativeCx, *mut u64, u32, u64, u64, u64);
}

/// The runtime functions native code calls, at fixed offsets (`#[repr(C)]`, the `RT_*`
/// constants below), reached through `Cx::runtime_table` (null in a build without
/// `spike-native`). Every function takes `(cx, fp, off, ..)`:
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
    /// `l OP r` for `op` in `native::cmp` (the `Test*` cold paths, `conv::compare`).
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
    /// `o[k]` (M4-P3 KEYED, P2-ORCH-6): the generic keyed load of the `GetKeyedProperty` at
    /// `off` (`slow::prop`, `property::get_keyed`; a private name takes the private path).
    /// Operands: the receiver and the key. The loaded value, or 0. It records no feedback
    /// (native code records none), so a keyed site of native code has a real generic path.
    pub get_keyed: RtFn2,
    /// `o[k] = v` (M4-P3 KEYED, P2-ORCH-6): the generic keyed store of the `SetKeyedProperty`
    /// at `off`, strict when the frame's function is strict (a failed store throws a
    /// TypeError there). Operands: the receiver, the key and the value. `v`, or 0. No
    /// feedback.
    pub set_keyed: RtFn3,
    /// The eager deopt entry (M5-P0, V10-D39; H29, SC WI-SC0): native code's shared exit calls
    /// it for a failed guard. Operands: the address of the register dump and the native frame
    /// pointer (x29) of the exit. It decodes the exit's record (`deopt.rs`), rebuilds the VM
    /// frame, sets `Cx::out_fp`/`Cx::out_pc` as [`RuntimeTable::unwind`] does and returns the
    /// accumulator there. It calls no recording function (MC-w15-6). M5-P0: a stub that
    /// throws "deopt not built" and returns 0; M5-P1 SPIKEC-5 builds it.
    pub deopt: RtFn2,
}

// ---- Cx (the isolate register points at it): VM_LAYOUT ----
/// VM_LAYOUT. `Cx::stack_limit`: VM frames must end at or below this slot (a `*mut u64`).
pub const CX_STACK_LIMIT: usize = offset_of!(Cx, stack_limit);
/// VM_LAYOUT. `Cx::budget` (i64): one unit per loop back edge and per function entry; call
/// [`RuntimeTable::poll`] when it reaches 0.
pub const CX_BUDGET: usize = offset_of!(Cx, budget);
/// VM_LAYOUT. `Cx::global_this` (Value bits): the receiver a sloppy callee gets for
/// undefined/null.
pub const CX_GLOBAL_THIS: usize = offset_of!(Cx, global_this);
/// VM_LAYOUT. `Cx::interrupt` (`*const AtomicU32`): the interrupt word.
pub const CX_INTERRUPT: usize = offset_of!(Cx, interrupt);
/// VM_LAYOUT. `Cx::native_limit` (usize): the lowest native stack address native code may
/// reach; the prologue's one compare.
pub const CX_NATIVE_LIMIT: usize = offset_of!(Cx, native_limit);
/// VM_LAYOUT. `Cx::runtime_table` (`*const RuntimeTable`; null without `spike-native`).
pub const CX_RUNTIME_TABLE: usize = offset_of!(Cx, runtime_table);
/// VM_LAYOUT. `Cx::unit_data` (usize; M3-P1 SP-UNIT, H35-Q9): the isolate's unit-data
/// directory, one word per unit slot, each the base of the isolate's RW copy of the unit in
/// that slot (0: none). Unit code loads its unit-data register (x27 / r14) from it on entry.
/// 0 = no directory.
pub const CX_UNIT_DATA: usize = offset_of!(Cx, unit_data);
/// VM_LAYOUT. `Cx::table`: the 256 handler addresses of the dispatch (H15: at offset 0).
/// Placement (b) code (P4-PLACEMENT-B) leaves with a tail jump to `table[*pc]`.
pub const CX_TABLE: usize = offset_of!(Cx, table);
/// VM_LAYOUT. `Cx::out_fp` (`*mut u64`): the frame [`RuntimeTable::unwind`] continues in.
pub const CX_OUT_FP: usize = offset_of!(Cx, out_fp);
/// VM_LAYOUT. `Cx::out_pc` (`*const u8`): the pc [`RuntimeTable::unwind`] continues at.
pub const CX_OUT_PC: usize = offset_of!(Cx, out_pc);

// ---- RuntimeTable: VM_LAYOUT (an H13 index list, append-only) ----
/// VM_LAYOUT. Entry 0.
pub const RT_POLL: usize = offset_of!(RuntimeTable, poll);
/// VM_LAYOUT. Entry 1.
pub const RT_CALL_ANY: usize = offset_of!(RuntimeTable, call_any);
/// VM_LAYOUT. Entry 2.
pub const RT_ADD: usize = offset_of!(RuntimeTable, add);
/// VM_LAYOUT. Entry 3.
pub const RT_INC: usize = offset_of!(RuntimeTable, inc);
/// VM_LAYOUT. Entry 4.
pub const RT_COMPARE: usize = offset_of!(RuntimeTable, compare);
/// VM_LAYOUT. Entry 5.
pub const RT_STACK_OVERFLOW: usize = offset_of!(RuntimeTable, stack_overflow);
/// VM_LAYOUT. Entry 6.
pub const RT_THROW: usize = offset_of!(RuntimeTable, throw);
/// VM_LAYOUT. Entry 7 (P4-PLACEMENT-B).
pub const RT_UNWIND: usize = offset_of!(RuntimeTable, unwind);
/// VM_LAYOUT. Entry 8 (M4-P3 KEYED, P2-ORCH-6): the generic keyed load.
pub const RT_GET_KEYED: usize = offset_of!(RuntimeTable, get_keyed);
/// VM_LAYOUT. Entry 9 (M4-P3 KEYED): the generic keyed store.
pub const RT_SET_KEYED: usize = offset_of!(RuntimeTable, set_keyed);
/// VM_LAYOUT. Entry 10 (M5-P0, V10-D39): the eager deopt entry.
pub const RT_DEOPT: usize = offset_of!(RuntimeTable, deopt);

// ---- FunctionInfo: VM_LAYOUT ----
/// VM_LAYOUT. FunctionInfo: the start of its bytecode (`*const u8`).
pub const FI_CODE: usize = offset_of!(FunctionInfo, code);
/// VM_LAYOUT. FunctionInfo: the dispatch slot kind byte (`DispatchKind`).
pub const FI_SLOT_KIND: usize = offset_of!(FunctionInfo, slot) + DispatchSlot::KIND_OFFSET;
/// VM_LAYOUT. FunctionInfo: the dispatch slot entry (usize).
pub const FI_SLOT_ENTRY: usize = offset_of!(FunctionInfo, slot) + DispatchSlot::ENTRY_OFFSET;
/// VM_LAYOUT. FunctionInfo: register count (u32).
pub const FI_REGISTER_COUNT: usize = offset_of!(FunctionInfo, register_count);
/// VM_LAYOUT. FunctionInfo: parameter count (u32).
pub const FI_PARAMETER_COUNT: usize = offset_of!(FunctionInfo, parameter_count);
/// VM_LAYOUT (MC-w13-14). FunctionInfo: `feedback`, a pointer to the unit slot that holds the
/// function's FEEDBACK cell (null: no vector yet).
pub const FI_FEEDBACK: usize = offset_of!(FunctionInfo, feedback);

// ---- VM frames (frame.rs, H14): byte offsets from fp; FRAME (MC-w16-18) ----
/// FRAME. Header words between the receiver and `r0`.
pub const FRAME_HEADER_WORDS: usize = HEADER;
/// FRAME. Caller fp (native word).
pub const FP_CALLER_FP: isize = slot::CALLER_FP * 8;
/// FRAME. Return pc (native word): the bytecode pc after the call instruction.
pub const FP_RETURN_PC: isize = slot::RETURN_PC * 8;
/// FRAME. The function object (Value).
pub const FP_CALLEE: isize = slot::CALLEE * 8;
/// FRAME. Argument count (int32), plus the construct bit.
pub const FP_ARGC: isize = slot::ARGC * 8;
/// FRAME. FunctionInfo (native word).
pub const FP_FUNCTION_INFO: isize = slot::FUNCTION_INFO * 8;
/// FRAME. new.target (Value; undefined for a native call), or the generator object in a
/// resumed frame.
pub const FP_NEW_TARGET: isize = slot::NEW_TARGET * 8;
/// FRAME. Current context (Value).
pub const FP_CONTEXT: isize = slot::CONTEXT * 8;
/// FRAME. Bytecode offset of the current instruction (int32); the table functions write it.
pub const FP_BYTECODE_OFFSET: isize = slot::BYTECODE_OFFSET * 8;
/// FRAME. The receiver.
pub const FP_RECEIVER: isize = slot::RECEIVER * 8;

/// FRAME. Byte offset from fp of parameter `a<i>` (`i < max(argc, parameter_count)`).
pub const fn fp_param(i: u32) -> isize {
    (slot::PARAM0 - i as isize) * 8
}

/// FRAME. Byte offset from fp of register `r<i>`.
pub const fn fp_register(i: u32) -> isize {
    i as isize * 8
}

// ---- Value encoding ----
/// Value encoding (`ep9_value::encoding::VERSION`). A frame's caller fp and return pc words,
/// and every native word in a cell, are `address + NATIVE_WORD_OFFSET` (frame.rs `native`;
/// one bit, 2^49, so an `eor` decodes them).
pub const NATIVE_WORD_OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;
/// Value encoding. The hole (`frame::HOLE`, `Value::EMPTY`): an empty element; in a feedback
/// key half, the polymorphic or megamorphic marker (an empty key half is undefined).
pub const HOLE: u64 = crate::frame::HOLE;

// ---- cells and kind ids: OBJECT_LAYOUT ----
//
// The constants below are what `spikes/spike-c/src/vm_layout.rs` kept by hand until M4-P4
// (same names there), taken from the VM's own definitions, so a layout change moves them. A
// change of any value is a layout change for native code and unit text.

/// OBJECT_LAYOUT. Word0 bytes 0-3: the ShapeId that an IC guard compares (`ep9_value`
/// `Word0::SHAPE_ID_OFFSET`; an `ldr w` at +0).
pub const SHAPE_ID_BYTE: u32 = ep9_value::Word0::SHAPE_ID_OFFSET as u32;
/// OBJECT_LAYOUT. The kind byte of a cell: byte 5 of word0 (`ep9_value` `Word0::KIND_OFFSET`;
/// an `ldrb` at +5).
pub const KIND_BYTE: u32 = ep9_value::Word0::KIND_OFFSET as u32;
/// OBJECT_LAYOUT. Kind id of an ordinary object (`kinds::OBJECT`).
pub const KIND_OBJECT: u8 = crate::kinds::OBJECT.get();
/// OBJECT_LAYOUT. Kind id of an Array (`kinds::ARRAY`).
pub const KIND_ARRAY: u8 = crate::kinds::ARRAY.get();
/// OBJECT_LAYOUT. The first in-object field of an OBJECT (no fixed fields): field `f` is at
/// `OBJECT_FIELDS + 8f` (`object::obj::FIXED`).
pub const OBJECT_FIELDS: u32 = obj::FIXED as u32;
/// OBJECT_LAYOUT. A JS object's elements store (every JS kind from M5-P1, H18-Q3; arrays
/// before): an elements cell, or undefined (`object::obj::ELEMENTS`).
pub const ARRAY_ELEMENTS: u32 = obj::ELEMENTS as u32;
/// OBJECT_LAYOUT. ARRAY: the length, an int32 Value (`object::arr::LENGTH`).
pub const ARRAY_LENGTH: u32 = arr::LENGTH as u32;
/// OBJECT_LAYOUT. ELEMENTS: the capacity, an int32 Value (`object::elems::CAPACITY`).
pub const ELEMENTS_CAPACITY: u32 = elems::CAPACITY as u32;
/// OBJECT_LAYOUT. ELEMENTS: the first value; value `i` is at `ELEMENTS_DATA + 8i`
/// (`object::elems::DATA`).
pub const ELEMENTS_DATA: u32 = elems::DATA as u32;
/// OBJECT_LAYOUT. FUNCTION cell field: its FunctionInfo (native word; `object::fun::INFO`).
pub const FUNCTION_INFO: usize = fun::INFO;
/// OBJECT_LAYOUT. FUNCTION cell field: its context (Value; `object::fun::CONTEXT`).
pub const FUNCTION_CONTEXT: usize = fun::CONTEXT;
/// OBJECT_LAYOUT (MC-w16-16). CONTEXT cell: the enclosing context (Value; `object::ctxf`).
pub const CONTEXT_PREVIOUS: u32 = ctxf::PREVIOUS as u32;
/// OBJECT_LAYOUT. CONTEXT cell: the scope word (a native word to the context's
/// `ContextScope`).
pub const CONTEXT_SCOPE: u32 = ctxf::SCOPE as u32;
/// OBJECT_LAYOUT. CONTEXT cell: the slot count (an int32 Value).
pub const CONTEXT_LENGTH: u32 = ctxf::LENGTH as u32;
/// OBJECT_LAYOUT. CONTEXT cell: slot 0; slot `i` is at `CONTEXT_SLOTS + 8i`.
pub const CONTEXT_SLOTS: u32 = ctxf::SLOTS as u32;
/// OBJECT_LAYOUT. Kind id of an ELEMENTS cell (65): Values (`kinds::ELEMENTS`). The only
/// elements cell native code may read as Values (V10-D9b).
pub const KIND_ELEMENTS: u8 = crate::kinds::ELEMENTS.get();
/// OBJECT_LAYOUT (5). Kind id of an ELEMENTS_DOUBLE cell (81): raw f64 data
/// (`kinds::ELEMENTS_DOUBLE`).
pub const KIND_ELEMENTS_DOUBLE: u8 = crate::kinds::ELEMENTS_DOUBLE.get();
/// OBJECT_LAYOUT (5). Kind id of an ELEMENTS_DICT cell (82): dictionary elements
/// (`kinds::ELEMENTS_DICT`).
pub const KIND_ELEMENTS_DICT: u8 = crate::kinds::ELEMENTS_DICT.get();
/// OBJECT_LAYOUT (5). ELEMENTS_DOUBLE: the capacity, an int32 Value (`object::delems`).
pub const ELEMENTS_DOUBLE_CAPACITY: u32 = delems::CAPACITY as u32;
/// OBJECT_LAYOUT (5). ELEMENTS_DOUBLE: raw f64 `i` is at `ELEMENTS_DOUBLE_DATA + 8i`.
pub const ELEMENTS_DOUBLE_DATA: u32 = delems::DATA as u32;
/// OBJECT_LAYOUT (5). ELEMENTS_DOUBLE: the hole, a NaN no store writes (`object::delems`).
pub const ELEMENTS_DOUBLE_HOLE: u64 = delems::HOLE;
/// OBJECT_LAYOUT (5). The bits of a shape's `counts` word (+48) that hold its elements kind
/// (`shape::ELEMENTS_KIND_SHIFT`, 4 bits).
pub const SHAPE_ELEMENTS_KIND_SHIFT: u32 = crate::shape::ELEMENTS_KIND_SHIFT;
/// OBJECT_LAYOUT (5). Elements kind values (`elements::kind`; 7-15 reserved, bit 0 = holey).
pub const ELEMENTS_KIND_PACKED_INT32: u8 = ek::PACKED_INT32;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_HOLEY_INT32: u8 = ek::HOLEY_INT32;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_PACKED_DOUBLE: u8 = ek::PACKED_DOUBLE;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_HOLEY_DOUBLE: u8 = ek::HOLEY_DOUBLE;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_PACKED_ELEMENTS: u8 = ek::PACKED_ELEMENTS;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_HOLEY_ELEMENTS: u8 = ek::HOLEY_ELEMENTS;
/// OBJECT_LAYOUT (5).
pub const ELEMENTS_KIND_DICTIONARY: u8 = ek::DICTIONARY;

// ---- feedback and IC data: IC_LAYOUT ----
/// IC_LAYOUT. FEEDBACK cell: the FunctionInfo of its function, a native word
/// (`feedback::fv::INFO`).
pub const FEEDBACK_INFO: u32 = fv::INFO as u32;
/// IC_LAYOUT. FEEDBACK cell: the first slot (`feedback::fv::SLOTS`).
pub const FEEDBACK_SLOTS: u32 = fv::SLOTS as u32;
/// IC_LAYOUT. FEEDBACK cell: bytes per slot, the key (weak) then the value
/// (`feedback::fv::SLOT_SIZE`, `KEY`, `VALUE`).
pub const FEEDBACK_SLOT_SIZE: u32 = fv::SLOT_SIZE as u32;
/// IC_LAYOUT. A feedback slot: the key half (weak).
pub const FEEDBACK_KEY: u32 = fv::KEY as u32;
/// IC_LAYOUT. A feedback slot: the value half (strong).
pub const FEEDBACK_VALUE: u32 = fv::VALUE as u32;
/// IC_LAYOUT. POLY_IC cell: the first case, a key (weak) then a handler (`ic::pc::CASES`;
/// also proved on a live POLY_IC cell by `tests::native` and `tests::layout5`).
pub const POLY_CASES: u32 = crate::ic::pc::CASES as u32;
/// IC_LAYOUT. POLY_IC cell: bytes per case (`ic::pc::CASE`).
pub const POLY_CASE_SIZE: u32 = crate::ic::pc::CASE as u32;
/// IC_LAYOUT (MC-w16-4). POLY_IC case: the handler half, after the key; case `i`'s handler is
/// at `POLY_CASES + POLY_CASE_SIZE * i + POLY_HANDLER` (`ic::pc::HANDLER`).
pub const POLY_HANDLER: u32 = crate::ic::pc::HANDLER as u32;
/// IC_LAYOUT. POLY_IC cell: cases per cell (`ic::POLY_CASES`).
pub const POLY_COUNT: u32 = crate::ic::POLY_CASES as u32;
/// IC_LAYOUT. Smi handler: the kind is in bits 0-3 (`ic::smi::KIND_MASK`).
pub const SMI_KIND_MASK: i32 = smi::KIND_MASK;
/// IC_LAYOUT. Smi handler: the byte offset starts at this bit (`ic::smi::SHIFT`).
pub const SMI_SHIFT: u32 = smi::SHIFT;
/// IC_LAYOUT. Smi handler kind: the field at the byte offset of the object cell
/// (`ic::smi::FIELD_IN_OBJECT`).
pub const SMI_FIELD_IN_OBJECT: i32 = smi::FIELD_IN_OBJECT;
/// IC_LAYOUT. Smi handler kind: an Array's `length`, the int32 at [`ARRAY_LENGTH`]
/// (`ic::smi::ARRAY_LENGTH`).
pub const SMI_ARRAY_LENGTH: i32 = smi::ARRAY_LENGTH;
/// IC_LAYOUT (3). A keyed slot's value half: bit `KEYED_KINDS_SET_SHIFT + k` is set once the
/// site saw elements kind `k` (0-6; `elements::KINDS_SET_SHIFT`).
pub const KEYED_KINDS_SET_SHIFT: u32 = crate::elements::KINDS_SET_SHIFT;
