//! The AArch64 code generator of placement (b): the 11 Spike B opcodes, entered and left with
//! the T0 handler ABI (`(cx, pc, fp, acc)`, the Rust ABI on aarch64: x0-x3).
//!
//! ```text
//!  0  stp x29, x30, [sp, #-32]!     CFA = sp + 32; x29 at CFA-32, x30 at CFA-24
//!  4  mov x29, sp                    CFA = x29 + 32
//!  8  stp x26, x28, [sp, #16]        x26 at CFA-16, x28 at CFA-8
//! 12  mov x28, x0 ; mov x26, x2      isolate register (x0 = cx), VM frame pointer (x2 = fp)
//! 20  mov x0, x3 (acc) ; ldr x16, [x28, native_limit] ; mov x17, sp ; cmp x17, x16 ; b.ls overflow
//!     body: one label per reachable instruction, accumulator in x0 (as placement (a))
//! ret:   mov x3, x0 ; ldur x1, [x26, #-56] ; ldur x2, [x26, #-64]   return pc, caller fp
//!        eor x1, x1, #2^49 ; eor x2, x2, #2^49                       decode the native words
//! leave: mov x0, x28 ; ldrb w9, [x1] ; ldr x16, [x0, x9, lsl #3]     table[*pc] (Cx offset 0)
//!        ldp x26, x28, [sp, #16] ; ldp x29, x30, [sp], #32 ; br x16   (cx, pc, fp, acc)
//!     overflow: stack_overflow(cx, fp, 0) ; then as a throw at offset 0
//!     slow paths: table call ; cbz x0, throw<off> ; b <next instruction>
//!     throw<off>: mov w2, #off ; b unwind
//!     unwind: RuntimeTable::unwind(cx, fp, off) ; x3 = x0 ; x1 = cx.out_pc ; x2 = cx.out_fp
//!             ; b leave
//! ```
//!
//! The native frame record lives only while the body runs: every exit restores x26, x28, x29,
//! x30 and sp to their entry values and leaves by a tail jump, so no (b) frame stays on the
//! native stack across a JS call that T0 makes. A call instruction runs through
//! `RuntimeTable::call_any` (one nested native level, as in placement (a)).
//!
//! Scratch registers: x9, x10, x16 (table address), x17 (the int32 tag `0xFFFE << 48`).
//! No absolute address and no relocation: branches are pc-relative inside the function; the
//! dispatch table is at `Cx` offset 0, the runtime functions are behind `Cx::runtime_table`.

// dynasm expands a dynamic register `X(n)` to a `u8::from(n)` conversion, which clippy flags
// at the call site when `n` is already a u8 (the helpers below take register numbers as u8).
#![allow(clippy::useless_conversion)]

use std::collections::BTreeMap;

use dynasmrt::aarch64::Aarch64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE};
use ep9_vm::{cmp, layout};

use crate::emit::{Refusal, Unwind};
use crate::jit::{Op, Plan, Slow};

type Ops = VecAssembler<Aarch64Relocation>;

/// DWARF register numbers.
const X19: u16 = 19;
const X20: u16 = 20;
const X26: u16 = 26;
const X28: u16 = 28;
const FP: u16 = 29;
const LR: u16 = 30;
const SP: u16 = 31;

/// `ldr`/`str` with an unsigned scaled 12-bit offset: 0..=32760, a multiple of 8.
fn scaled(off: usize) -> Result<u32, Refusal> {
    if off.is_multiple_of(8) && off <= 32760 {
        Ok(off as u32)
    } else {
        Err(Refusal::Limit(format!("Cx or table offset {off}")))
    }
}

/// x`rd` = `v` (movz, then movk for each other non-zero halfword). `v` is a constant (a Value
/// tag or an offset), never an address.
fn mov64(ops: &mut Ops, rd: u8, v: u64) {
    let h = |i: u32| ((v >> (16 * i)) & 0xFFFF) as u32;
    let first = (0..4).find(|&i| h(i) != 0).unwrap_or(0);
    let (f, fs) = (h(first), first * 16);
    dynasm!(ops ; .arch aarch64 ; movz X(rd), f, lsl fs);
    for i in first + 1..4 {
        if h(i) != 0 {
            let (p, s) = (h(i), i * 16);
            dynasm!(ops ; .arch aarch64 ; movk X(rd), p, lsl s);
        }
    }
}

/// x`rt` = the frame slot at byte offset `off` from the VM frame pointer x26.
fn load(ops: &mut Ops, rt: u8, off: i32) {
    if off >= 0 && off % 8 == 0 && off <= 32760 {
        let o = off as u32;
        dynasm!(ops ; .arch aarch64 ; ldr X(rt), [x26, o]);
    } else if (-256..256).contains(&off) {
        dynasm!(ops ; .arch aarch64 ; ldur X(rt), [x26, off]);
    } else {
        mov64(ops, 16, off as i64 as u64);
        dynasm!(ops ; .arch aarch64 ; ldr X(rt), [x26, x16]);
    }
}

/// The frame slot at byte offset `off` = x`rt`.
fn store(ops: &mut Ops, rt: u8, off: i32) {
    if off >= 0 && off % 8 == 0 && off <= 32760 {
        let o = off as u32;
        dynasm!(ops ; .arch aarch64 ; str X(rt), [x26, o]);
    } else if (-256..256).contains(&off) {
        dynasm!(ops ; .arch aarch64 ; stur X(rt), [x26, off]);
    } else {
        mov64(ops, 16, off as i64 as u64);
        dynasm!(ops ; .arch aarch64 ; str X(rt), [x26, x16]);
    }
}

/// Calls the table function at `entry` with x0 = cx, x1 = fp, w2 = `off` (the value operands
/// are already in x3..). Result in x0.
fn call_table(ops: &mut Ops, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = scaled(layout::CX_RUNTIME_TABLE)?;
    let entry = scaled(entry)?;
    dynasm!(ops ; .arch aarch64 ; mov x0, x28 ; mov x1, x26);
    mov64(ops, 2, u64::from(off));
    dynasm!(ops
        ; .arch aarch64
        ; ldr x16, [x28, table]
        ; ldr x16, [x16, entry]
        ; blr x16
    );
    Ok(())
}

/// The exits every (b) function shares: the throw stubs (one per bytecode offset that can
/// throw) and the unwind call. `leave` is the common tail jump.
struct Exits {
    throws: BTreeMap<u32, DynamicLabel>,
    unwind: DynamicLabel,
    leave: DynamicLabel,
}

impl Exits {
    fn new(ops: &mut Ops) -> Exits {
        Exits {
            throws: BTreeMap::new(),
            unwind: ops.new_dynamic_label(),
            leave: ops.new_dynamic_label(),
        }
    }

    /// The label of "an exception is pending at bytecode offset `off`".
    fn throw(&mut self, ops: &mut Ops, off: u32) -> DynamicLabel {
        *self
            .throws
            .entry(off)
            .or_insert_with(|| ops.new_dynamic_label())
    }

    /// `ret`: x1 = the frame's return pc, x2 = its caller fp, x3 = the accumulator (x0); then
    /// `leave`: restore and tail-jump to `cx.table[*x1]`. Returns the offset of the first
    /// restoring instruction (`ldp x26, x28`), where the CFI epilogue rows start.
    fn emit_ret_and_leave(&self, ops: &mut Ops) -> Result<u32, Refusal> {
        let ret_pc = i32::try_from(layout::FP_RETURN_PC).map_err(|_| Refusal::Arch)?;
        let caller = i32::try_from(layout::FP_CALLER_FP).map_err(|_| Refusal::Arch)?;
        dynasm!(ops ; .arch aarch64 ; mov x3, x0);
        load(ops, 1, ret_pc);
        load(ops, 2, caller);
        decode(ops, 1)?;
        decode(ops, 2)?;
        let table = layout::CX_TABLE;
        if table != 0 {
            return Err(Refusal::Limit(format!("Cx::table at {table}, not 0")));
        }
        let leave = self.leave;
        dynasm!(ops
            ; .arch aarch64
            ; =>leave
            ; mov x0, x28
            ; ldrb w9, [x1]
            ; ldr x16, [x0, x9, lsl #3]
        );
        let epilogue = ops.offset().0 as u32;
        Ok(epilogue)
    }

    /// The out-of-line throw stubs and the unwind call (after the epilogue).
    fn emit_unwind(&self, ops: &mut Ops) -> Result<(), Refusal> {
        let unwind = self.unwind;
        for (&off, &l) in &self.throws {
            dynasm!(ops ; .arch aarch64 ; =>l);
            mov64(ops, 2, u64::from(off));
            dynasm!(ops ; .arch aarch64 ; b =>unwind);
        }
        let table = scaled(layout::CX_RUNTIME_TABLE)?;
        let entry = scaled(layout::RT_UNWIND)?;
        let out_pc = scaled(layout::CX_OUT_PC)?;
        let out_fp = scaled(layout::CX_OUT_FP)?;
        let leave = self.leave;
        dynasm!(ops
            ; .arch aarch64
            ; =>unwind
            ; mov x0, x28
            ; mov x1, x26
            ; ldr x16, [x28, table]
            ; ldr x16, [x16, entry]
            ; blr x16
            ; mov x3, x0
            ; ldr x1, [x28, out_pc]
            ; ldr x2, [x28, out_fp]
            ; b =>leave
        );
        Ok(())
    }
}

/// `eor x<r>, x<r>, #NATIVE_WORD_OFFSET`: decodes a frame's native word (one bit, 2^49).
fn decode(ops: &mut Ops, r: u8) -> Result<(), Refusal> {
    if layout::NATIVE_WORD_OFFSET != 1 << 49 {
        return Err(Refusal::Limit(format!(
            "native word offset {:#x}",
            layout::NATIVE_WORD_OFFSET
        )));
    }
    dynasm!(ops ; .arch aarch64 ; eor XSP(r), X(r), #0x0002_0000_0000_0000_u64);
    Ok(())
}

/// Compiles `plan`: the code, its unwind data and the native offset of each step.
pub(crate) fn emit(plan: &Plan) -> Result<(Vec<u8>, Unwind, Vec<u32>), Refusal> {
    let mut ops = Ops::new(0);
    let labels: BTreeMap<u32, DynamicLabel> = plan
        .steps
        .iter()
        .map(|s| (s.offset, ops.new_dynamic_label()))
        .collect();
    let ret = ops.new_dynamic_label();
    let overflow = ops.new_dynamic_label();
    let mut exits = Exits::new(&mut ops);
    let native_limit = scaled(layout::CX_NATIVE_LIMIT)?;
    let budget = scaled(layout::CX_BUDGET)?;
    let label = |t: u32| labels[&t];

    // ---- prologue (offsets 0, 4, 8 are in the CFI below) ----
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-32]!
        ; mov x29, sp
        ; stp x26, x28, [sp, #16]
        ; mov x28, x0
        ; mov x26, x2
        ; mov x0, x3
        ; ldr x16, [x28, native_limit]
        ; mov x17, sp
        ; cmp x17, x16
        ; b.ls =>overflow
    );

    // ---- body ----
    let mut slows: Vec<(DynamicLabel, Slow, u32, DynamicLabel)> = Vec::new();
    let mut native_offsets = Vec::with_capacity(plan.steps.len());
    for (i, s) in plan.steps.iter().enumerate() {
        dynasm!(ops ; .arch aarch64 ; =>label(s.offset));
        native_offsets.push(ops.offset().0 as u32);
        let next_emitted = plan.steps.get(i + 1).map(|n| n.offset);
        match s.op {
            Op::LdaZero => mov64(&mut ops, 0, NUMBER_TAG),
            Op::Ldar(o) => load(&mut ops, 0, o),
            Op::Star(o) => store(&mut ops, 0, o),
            Op::Add(o) => {
                let slow = ops.new_dynamic_label();
                load(&mut ops, 9, o);
                mov64(&mut ops, 17, NUMBER_TAG);
                dynasm!(ops
                    ; .arch aarch64
                    ; and x16, x9, x0
                    ; cmp x16, x17
                    ; b.lo =>slow
                    ; adds w16, w9, w0
                    ; b.vs =>slow
                    ; orr x0, x16, x17
                );
                slows.push((
                    slow,
                    Slow::Add {
                        lhs: Some(o),
                        imm: 0,
                    },
                    s.offset,
                    label(s.next),
                ));
            }
            Op::AddSmi(imm) => {
                let slow = ops.new_dynamic_label();
                mov64(&mut ops, 17, NUMBER_TAG);
                dynasm!(ops
                    ; .arch aarch64
                    ; cmp x0, x17
                    ; b.lo =>slow
                );
                mov64(&mut ops, 10, u64::from(imm as u32));
                dynasm!(ops
                    ; .arch aarch64
                    ; adds w16, w0, w10
                    ; b.vs =>slow
                    ; orr x0, x16, x17
                );
                slows.push((slow, Slow::Add { lhs: None, imm }, s.offset, label(s.next)));
            }
            Op::Inc => {
                let slow = ops.new_dynamic_label();
                mov64(&mut ops, 17, NUMBER_TAG);
                dynasm!(ops
                    ; .arch aarch64
                    ; cmp x0, x17
                    ; b.lo =>slow
                    ; adds w16, w0, 1
                    ; b.vs =>slow
                    ; orr x0, x16, x17
                );
                slows.push((slow, Slow::Inc, s.offset, label(s.next)));
            }
            Op::TestLessThan(o) => {
                let slow = ops.new_dynamic_label();
                load(&mut ops, 9, o);
                mov64(&mut ops, 17, NUMBER_TAG);
                let f = VALUE_FALSE as u32;
                dynasm!(ops
                    ; .arch aarch64
                    ; and x16, x9, x0
                    ; cmp x16, x17
                    ; b.lo =>slow
                    ; cmp w9, w0
                    ; cset x0, lt
                    ; add x0, x0, f
                );
                slows.push((slow, Slow::Less { lhs: o }, s.offset, label(s.next)));
            }
            Op::JumpIfFalse(t) => {
                let f = VALUE_FALSE as u32;
                dynasm!(ops
                    ; .arch aarch64
                    ; cmp x0, f
                    ; b.eq =>label(t)
                );
            }
            Op::JumpLoop(t) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch aarch64
                    ; ldr x16, [x28, budget]
                    ; subs x16, x16, 1
                    ; str x16, [x28, budget]
                    ; b.le =>slow
                    ; b =>label(t)
                );
                slows.push((slow, Slow::Poll, s.offset, label(t)));
            }
            Op::CallUndefinedReceiver1 => {
                let throw = exits.throw(&mut ops, s.offset);
                dynasm!(ops ; .arch aarch64 ; mov x3, x0);
                call_table(&mut ops, layout::RT_CALL_ANY, s.offset)?;
                dynasm!(ops ; .arch aarch64 ; cbz x0, =>throw);
            }
            Op::Return => {
                if next_emitted.is_some() {
                    dynasm!(ops ; .arch aarch64 ; b =>ret);
                }
            }
        }
        if s.falls_through() && next_emitted != Some(s.next) {
            dynasm!(ops ; .arch aarch64 ; b =>label(s.next));
        }
    }

    // ---- return and the common exit ----
    dynasm!(ops ; .arch aarch64 ; =>ret);
    let epilogue = exits.emit_ret_and_leave(&mut ops)?;
    dynasm!(ops
        ; .arch aarch64
        ; ldp x26, x28, [sp, #16]
        ; ldp x29, x30, [sp], #32
        ; br x16
    );

    // ---- out of line ----
    let throw0 = exits.throw(&mut ops, 0);
    dynasm!(ops ; .arch aarch64 ; =>overflow);
    call_table(&mut ops, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch aarch64 ; b =>throw0);
    for (slow, kind, off, cont) in slows {
        let throw = exits.throw(&mut ops, off);
        dynasm!(ops ; .arch aarch64 ; =>slow);
        let entry = match kind {
            Slow::Add { lhs: Some(o), .. } => {
                dynasm!(ops ; .arch aarch64 ; mov x4, x0);
                load(&mut ops, 3, o);
                layout::RT_ADD
            }
            Slow::Add { lhs: None, imm } => {
                dynasm!(ops ; .arch aarch64 ; mov x3, x0);
                mov64(&mut ops, 4, NUMBER_TAG | u64::from(imm as u32));
                layout::RT_ADD
            }
            Slow::Inc => {
                dynasm!(ops ; .arch aarch64 ; mov x3, x0);
                layout::RT_INC
            }
            Slow::Less { lhs } => {
                dynasm!(ops ; .arch aarch64 ; mov x5, x0);
                load(&mut ops, 4, lhs);
                mov64(&mut ops, 3, u64::from(cmp::LT));
                layout::RT_COMPARE
            }
            Slow::Poll => layout::RT_POLL,
        };
        call_table(&mut ops, entry, off)?;
        dynasm!(ops
            ; .arch aarch64
            ; cbz x0, =>throw
            ; b =>cont
        );
    }
    exits.emit_unwind(&mut ops)?;

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    Ok((
        code,
        Unwind {
            cfi: cfi(32, &[(X26, 2), (X28, 1)], epilogue),
            win64: None,
        },
        native_offsets,
    ))
}

/// The CFI of a (b) frame (code alignment 4, data alignment -8): `stp x29, x30, [sp,
/// #-size]!` at 0, `mov x29, sp` at 4, then one `stp` per pair in `saved` (register, slot in
/// 8-byte units below the CFA), one instruction each. The epilogue at `epilogue` restores the
/// pairs (one `ldp` per pair, in the reverse order of `saved`'s pairs), then `ldp x29, x30`,
/// then `br`; its rows sit between remember/restore state, so the out-of-line code after the
/// `br` has the body's rule again.
fn cfi(size: u8, saved: &[(u16, u8)], epilogue: u32) -> CfiProgram {
    let mut p = CfiProgram::new();
    let words = size / 8;
    p.advance_loc(1)
        .def_cfa_offset(u64::from(size))
        .offset(FP, u64::from(words))
        .offset(LR, u64::from(words - 1))
        .advance_loc(1)
        .def_cfa_register(FP);
    let pairs: Vec<&[(u16, u8)]> = saved.chunks(2).collect();
    for pair in &pairs {
        p.advance_loc(1);
        for &(r, slot) in *pair {
            p.offset(r, u64::from(slot));
        }
    }
    let prologue_insns = 2 + pairs.len() as u32;
    p.advance_loc(epilogue / 4 - prologue_insns)
        .remember_state();
    for pair in pairs.iter().rev() {
        p.advance_loc(1);
        for &(r, _) in *pair {
            p.restore(r);
        }
    }
    p.advance_loc(1)
        .def_cfa(SP, 0)
        .restore(FP)
        .restore(LR)
        .advance_loc(1)
        .restore_state();
    p
}

/// The (b) probe of check (g): `hold(mk, scrub, gc)` calls `mk()`, keeps the result only in the
/// callee-saved x19 (XORed with `mask` when it is not 0), calls `scrub()` and `gc()` (each
/// through `call_any`), and returns x19 with the (b) exit. `calls` are, per call: its bytecode
/// offset, the frame slot of its callee register, and the frame slot of the parameter the
/// bytecode moves there first. The frame saves x19/x20 too:
///
/// ```text
/// stp x29, x30, [sp, #-48]! ; mov x29, sp ; stp x26, x28, [sp, #16] ; stp x19, x20, [sp, #32]
/// ```
pub(crate) fn emit_hold(
    calls: [(u32, i32, i32); 3],
    mask: u64,
) -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    let mut exits = Exits::new(&mut ops);
    let ret = ops.new_dynamic_label();
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-48]!
        ; mov x29, sp
        ; stp x26, x28, [sp, #16]
        ; stp x19, x20, [sp, #32]
        ; mov x28, x0
        ; mov x26, x2
    );
    for (i, &(off, callee, param)) in calls.iter().enumerate() {
        let throw = exits.throw(&mut ops, off);
        load(&mut ops, 9, param);
        store(&mut ops, 9, callee);
        mov64(&mut ops, 3, ep9_value::encoding::VALUE_UNDEFINED);
        call_table(&mut ops, layout::RT_CALL_ANY, off)?;
        dynasm!(ops ; .arch aarch64 ; cbz x0, =>throw);
        if i == 0 {
            if mask == 0 {
                dynasm!(ops ; .arch aarch64 ; mov x19, x0);
            } else {
                mov64(&mut ops, 16, mask);
                dynasm!(ops ; .arch aarch64 ; eor x19, x0, x16);
            }
        }
    }
    dynasm!(ops ; .arch aarch64 ; mov x0, x19 ; =>ret);
    let epilogue = exits.emit_ret_and_leave(&mut ops)?;
    dynasm!(ops
        ; .arch aarch64
        ; ldp x19, x20, [sp, #32]
        ; ldp x26, x28, [sp, #16]
        ; ldp x29, x30, [sp], #48
        ; br x16
    );
    exits.emit_unwind(&mut ops)?;
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    Ok((
        code,
        Unwind {
            cfi: cfi(48, &[(X26, 4), (X28, 3), (X19, 2), (X20, 1)], epilogue),
            win64: None,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cfi_of_the_32_byte_frame_equals_placement_a_shape() {
        // Same rows as jit::a64's frame, with the epilogue at offset 100.
        let mut want = CfiProgram::new();
        want.advance_loc(1)
            .def_cfa_offset(32)
            .offset(FP, 4)
            .offset(LR, 3)
            .advance_loc(1)
            .def_cfa_register(FP)
            .advance_loc(1)
            .offset(X26, 2)
            .offset(X28, 1)
            .advance_loc(100 / 4 - 3)
            .remember_state()
            .advance_loc(1)
            .restore(X26)
            .restore(X28)
            .advance_loc(1)
            .def_cfa(SP, 0)
            .restore(FP)
            .restore(LR)
            .advance_loc(1)
            .restore_state();
        assert_eq!(cfi(32, &[(X26, 2), (X28, 1)], 100), want);
    }
}
