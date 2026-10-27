//! The AArch64 code generator (AAPCS64, `extern "C"`) of the spike JIT.
//!
//! ```text
//!  0  stp x29, x30, [sp, #-32]!     CFA = sp + 32; x29 at CFA-32, x30 at CFA-24
//!  4  mov x29, sp                    CFA = x29 + 32
//!  8  stp x26, x28, [sp, #16]        x26 at CFA-16, x28 at CFA-8
//! 12  mov x28, x0 ; mov x26, x1      isolate register, VM frame pointer
//!     ldr x16, [x28, #native_limit] ; mov x17, sp ; cmp x17, x16 ; b.ls overflow
//!     body: one label per reachable instruction, accumulator in x0
//! exit: ldp x26, x28, [sp, #16] ; ldp x29, x30, [sp], #32 ; ret      (x0 = acc, or 0)
//!     overflow: stack_overflow(cx, fp, 0) ; b exit
//!     slow paths: table call ; cbz x0, exit ; b <next instruction>
//! ```
//!
//! Scratch registers: x9, x10, x16 (table address), x17 (the int32 tag `0xFFFE << 48`).

// dynasm expands a dynamic register `X(n)` to a `u8::from(n)` conversion, which clippy flags
// at the call site when `n` is already a u8 (the helpers below take register numbers as u8).
#![allow(clippy::useless_conversion)]

use std::collections::BTreeMap;

use dynasmrt::aarch64::Aarch64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE};
use ep9_vm::{cmp, layout};

use super::{Op, Plan, Slow};
use crate::emit::{Refusal, Unwind};

type Ops = VecAssembler<Aarch64Relocation>;

/// DWARF register numbers.
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

/// x`rd` = `v` (movz, then movk for each other non-zero halfword).
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

/// Compiles `plan`: the code, its unwind data and the native offset of each step.
pub(crate) fn emit(plan: &Plan) -> Result<(Vec<u8>, Unwind, Vec<u32>), Refusal> {
    let mut ops = Ops::new(0);
    let labels: BTreeMap<u32, DynamicLabel> = plan
        .steps
        .iter()
        .map(|s| (s.offset, ops.new_dynamic_label()))
        .collect();
    let exit = ops.new_dynamic_label();
    let overflow = ops.new_dynamic_label();
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
        ; mov x26, x1
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
                dynasm!(ops ; .arch aarch64 ; mov x3, x0);
                call_table(&mut ops, layout::RT_CALL_ANY, s.offset)?;
                dynasm!(ops ; .arch aarch64 ; cbz x0, =>exit);
            }
            Op::Return => {
                if next_emitted.is_some() {
                    dynasm!(ops ; .arch aarch64 ; b =>exit);
                }
            }
        }
        if s.falls_through() && next_emitted != Some(s.next) {
            dynasm!(ops ; .arch aarch64 ; b =>label(s.next));
        }
    }

    // ---- epilogue ----
    dynasm!(ops ; .arch aarch64 ; =>exit);
    let epilogue = ops.offset().0 as u32;
    dynasm!(ops
        ; .arch aarch64
        ; ldp x26, x28, [sp, #16]
        ; ldp x29, x30, [sp], #32
        ; ret
    );

    // ---- out of line ----
    dynasm!(ops ; .arch aarch64 ; =>overflow);
    call_table(&mut ops, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch aarch64 ; b =>exit);
    for (slow, kind, off, cont) in slows {
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
            ; cbz x0, =>exit
            ; b =>cont
        );
    }

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    Ok((
        code,
        Unwind {
            cfi: cfi(epilogue),
            win64: None,
        },
        native_offsets,
    ))
}

/// The CFI of the frame above (code alignment 4, data alignment -8): the prologue rows, then
/// the epilogue rows between remember/restore state, so the out-of-line code after `ret` has
/// the body's rule again.
fn cfi(epilogue: u32) -> CfiProgram {
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(32)
        .offset(FP, 4)
        .offset(LR, 3)
        .advance_loc(1)
        .def_cfa_register(FP)
        .advance_loc(1)
        .offset(X26, 2)
        .offset(X28, 1)
        .advance_loc(epilogue / 4 - 3)
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
    p
}

/// The probe of gate check (g) (not bytecode-driven): `hold(mk, scrub, gc)` calls `mk()`,
/// keeps the result only in the callee-saved x19 (XORed with `mask` when it is not 0), calls
/// `scrub()` and `gc()`, and returns x19. `calls` are, per call: its bytecode offset
/// (`call_any` decodes it), the frame slot of its callee register, and the frame slot of the
/// parameter the bytecode moves there first. The frame also saves x19/x20:
///
/// ```text
/// stp x29, x30, [sp, #-48]! ; mov x29, sp ; stp x26, x28, [sp, #16] ; stp x19, x20, [sp, #32]
/// ```
pub(crate) fn emit_hold(
    calls: [(u32, i32, i32); 3],
    mask: u64,
) -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    let exit = ops.new_dynamic_label();
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-48]!
        ; mov x29, sp
        ; stp x26, x28, [sp, #16]
        ; stp x19, x20, [sp, #32]
        ; mov x28, x0
        ; mov x26, x1
    );
    for (i, &(off, callee, param)) in calls.iter().enumerate() {
        load(&mut ops, 9, param);
        store(&mut ops, 9, callee);
        mov64(&mut ops, 3, ep9_value::encoding::VALUE_UNDEFINED);
        call_table(&mut ops, layout::RT_CALL_ANY, off)?;
        dynasm!(ops ; .arch aarch64 ; cbz x0, =>exit);
        if i == 0 {
            if mask == 0 {
                dynasm!(ops ; .arch aarch64 ; mov x19, x0);
            } else {
                mov64(&mut ops, 16, mask);
                dynasm!(ops ; .arch aarch64 ; eor x19, x0, x16);
            }
        }
    }
    dynasm!(ops ; .arch aarch64 ; mov x0, x19 ; =>exit);
    let epilogue = ops.offset().0 as u32;
    dynasm!(ops
        ; .arch aarch64
        ; ldp x19, x20, [sp, #32]
        ; ldp x26, x28, [sp, #16]
        ; ldp x29, x30, [sp], #48
        ; ret
    );
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(48)
        .offset(FP, 6)
        .offset(LR, 5)
        .advance_loc(1)
        .def_cfa_register(FP)
        .advance_loc(1)
        .offset(X26, 4)
        .offset(X28, 3)
        .advance_loc(1)
        .offset(19, 2)
        .offset(20, 1)
        .advance_loc(epilogue / 4 - 4)
        .remember_state()
        .advance_loc(1)
        .restore(19)
        .restore(20)
        .advance_loc(1)
        .restore(X26)
        .restore(X28)
        .advance_loc(1)
        .def_cfa(SP, 0)
        .restore(FP)
        .restore(LR)
        .advance_loc(1)
        .restore_state();
    Ok((
        code,
        Unwind {
            cfi: p,
            win64: None,
        },
    ))
}
