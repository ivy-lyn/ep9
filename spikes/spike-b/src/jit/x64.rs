//! The x86-64 code generator (System V, `extern "sysv64"` on every OS, Windows too) of the
//! spike JIT. One generator for Linux, macOS and Windows; only the unwind data differs (CFI
//! for unix, UNWIND_INFO for Windows, both made here).
//!
//! ```text
//!  0  push rbp                  CFA = rsp + 16; rbp at CFA-16        UWOP_PUSH_NONVOL rbp
//!  1  push r15                  CFA = rsp + 24; r15 at CFA-24        UWOP_PUSH_NONVOL r15
//!  3  push r13                  CFA = rsp + 32; r13 at CFA-32        UWOP_PUSH_NONVOL r13
//!  5  lea rbp, [rsp + 16]       CFA = rbp + 16 (rbp -> saved rbp)    UWOP_SET_FPREG rbp, 16
//! 10  mov r15, rdi ; mov r13, rsi          isolate register, VM frame pointer
//!     cmp rsp, [r15 + native_limit] ; jbe overflow
//!     body: one label per reachable instruction, accumulator in rax
//! exit: lea rsp, [rbp - 16] ; pop r13 ; pop r15 ; pop rbp ; ret    (rax = acc, or 0)
//!     overflow: stack_overflow(cx, fp, 0) ; jmp exit
//!     slow paths: table call ; test rax, rax ; jz exit ; jmp <next instruction>
//! ```
//!
//! rsp is 16-byte aligned after the three pushes, so every table call is aligned. Scratch
//! registers: rcx, r8, r9, r10, r11 (the int32 tag `0xFFFE << 48`).

use std::collections::BTreeMap;

use dynasmrt::x64::X64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg as wreg};
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE};
use ep9_vm::{cmp, layout};

use super::{Op, Plan, Slow};
use crate::emit::{Refusal, Unwind};

type Ops = VecAssembler<X64Relocation>;

/// DWARF register numbers (System V psABI figure 3.36).
const RBP: u16 = 6;
const RSP: u16 = 7;
const R13: u16 = 13;
const R15: u16 = 15;

fn disp(off: usize) -> Result<i32, Refusal> {
    i32::try_from(off).map_err(|_| Refusal::Limit(format!("Cx or table offset {off}")))
}

/// Calls the table function at `entry` with rdi = cx, rsi = fp, edx = `off` (the value
/// operands are already in rcx, r8, r9). Result in rax.
fn call_table(ops: &mut Ops, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = disp(layout::CX_RUNTIME_TABLE)?;
    let entry = disp(entry)?;
    let off = off as i32;
    dynasm!(ops
        ; .arch x64
        ; mov rdi, r15
        ; mov rsi, r13
        ; mov edx, DWORD off
        ; mov rax, QWORD [r15 + table]
        ; call QWORD [rax + entry]
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
    let native_limit = disp(layout::CX_NATIVE_LIMIT)?;
    let budget = disp(layout::CX_BUDGET)?;
    let label = |t: u32| labels[&t];
    let tag = NUMBER_TAG as i64;
    let f = VALUE_FALSE as i32;

    // ---- prologue ----
    let mut at = [0u32; 4];
    dynasm!(ops ; .arch x64 ; push rbp);
    at[0] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push r15);
    at[1] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push r13);
    at[2] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; lea rbp, [rsp + 16]);
    at[3] = ops.offset().0 as u32;
    dynasm!(ops
        ; .arch x64
        ; mov r15, rdi
        ; mov r13, rsi
        ; cmp rsp, QWORD [r15 + native_limit]
        ; jbe =>overflow
    );

    // ---- body ----
    let mut slows: Vec<(DynamicLabel, Slow, u32, DynamicLabel)> = Vec::new();
    let mut native_offsets = Vec::with_capacity(plan.steps.len());
    for (i, s) in plan.steps.iter().enumerate() {
        dynasm!(ops ; .arch x64 ; =>label(s.offset));
        native_offsets.push(ops.offset().0 as u32);
        let next_emitted = plan.steps.get(i + 1).map(|n| n.offset);
        match s.op {
            Op::LdaZero => {
                dynasm!(ops ; .arch x64 ; mov rax, QWORD tag);
            }
            Op::Ldar(o) => {
                dynasm!(ops ; .arch x64 ; mov rax, QWORD [r13 + o]);
            }
            Op::Star(o) => {
                dynasm!(ops ; .arch x64 ; mov QWORD [r13 + o], rax);
            }
            Op::Add(o) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, QWORD [r13 + o]
                    ; mov r11, QWORD tag
                    ; mov r10, rcx
                    ; and r10, rax
                    ; cmp r10, r11
                    ; jb =>slow
                    ; mov r10d, ecx
                    ; add r10d, eax
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
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
                dynasm!(ops
                    ; .arch x64
                    ; mov r11, QWORD tag
                    ; cmp rax, r11
                    ; jb =>slow
                    ; mov r10d, eax
                    ; add r10d, DWORD imm
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
                );
                slows.push((slow, Slow::Add { lhs: None, imm }, s.offset, label(s.next)));
            }
            Op::Inc => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov r11, QWORD tag
                    ; cmp rax, r11
                    ; jb =>slow
                    ; mov r10d, eax
                    ; add r10d, 1
                    ; jo =>slow
                    ; or r10, r11
                    ; mov rax, r10
                );
                slows.push((slow, Slow::Inc, s.offset, label(s.next)));
            }
            Op::TestLessThan(o) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, QWORD [r13 + o]
                    ; mov r11, QWORD tag
                    ; mov r10, rcx
                    ; and r10, rax
                    ; cmp r10, r11
                    ; jb =>slow
                    ; cmp ecx, eax
                    ; setl al
                    ; movzx eax, al
                    ; add eax, f
                );
                slows.push((slow, Slow::Less { lhs: o }, s.offset, label(s.next)));
            }
            Op::JumpIfFalse(t) => {
                dynasm!(ops
                    ; .arch x64
                    ; cmp rax, f
                    ; je =>label(t)
                );
            }
            Op::JumpLoop(t) => {
                let slow = ops.new_dynamic_label();
                dynasm!(ops
                    ; .arch x64
                    ; sub QWORD [r15 + budget], 1
                    ; jle =>slow
                    ; jmp =>label(t)
                );
                slows.push((slow, Slow::Poll, s.offset, label(t)));
            }
            Op::CallUndefinedReceiver1 => {
                dynasm!(ops ; .arch x64 ; mov rcx, rax);
                call_table(&mut ops, layout::RT_CALL_ANY, s.offset)?;
                dynasm!(ops
                    ; .arch x64
                    ; test rax, rax
                    ; jz =>exit
                );
            }
            Op::Return => {
                if next_emitted.is_some() {
                    dynasm!(ops ; .arch x64 ; jmp =>exit);
                }
            }
        }
        if s.falls_through() && next_emitted != Some(s.next) {
            dynasm!(ops ; .arch x64 ; jmp =>label(s.next));
        }
    }

    // ---- epilogue ----
    dynasm!(ops ; .arch x64 ; =>exit);
    let mut ep = [0u32; 5];
    ep[0] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; lea rsp, [rbp - 16]);
    ep[1] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop r13);
    ep[2] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop r15);
    ep[3] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop rbp);
    ep[4] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; ret);

    // ---- out of line ----
    dynasm!(ops ; .arch x64 ; =>overflow);
    call_table(&mut ops, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch x64 ; jmp =>exit);
    for (slow, kind, off, cont) in slows {
        dynasm!(ops ; .arch x64 ; =>slow);
        let entry = match kind {
            Slow::Add { lhs: Some(o), .. } => {
                dynasm!(ops
                    ; .arch x64
                    ; mov r8, rax
                    ; mov rcx, QWORD [r13 + o]
                );
                layout::RT_ADD
            }
            Slow::Add { lhs: None, imm } => {
                let r = (NUMBER_TAG | u64::from(imm as u32)) as i64;
                dynasm!(ops
                    ; .arch x64
                    ; mov rcx, rax
                    ; mov r8, QWORD r
                );
                layout::RT_ADD
            }
            Slow::Inc => {
                dynasm!(ops ; .arch x64 ; mov rcx, rax);
                layout::RT_INC
            }
            Slow::Less { lhs } => {
                let lt = cmp::LT as i32;
                dynasm!(ops
                    ; .arch x64
                    ; mov r9, rax
                    ; mov r8, QWORD [r13 + lhs]
                    ; mov ecx, DWORD lt
                );
                layout::RT_COMPARE
            }
            Slow::Poll => layout::RT_POLL,
        };
        call_table(&mut ops, entry, off)?;
        dynasm!(ops
            ; .arch x64
            ; test rax, rax
            ; jz =>exit
            ; jmp =>cont
        );
    }

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    Ok((code, unwind(at, ep), native_offsets))
}

/// CFI (code alignment 1, data alignment -8) and the Win64 UNWIND_INFO of the frame above.
/// `at`: the offsets after push rbp, push r15, push r13, lea rbp. `ep`: the offsets of the
/// epilogue's lea, pop r13, pop r15, pop rbp and ret.
fn unwind(at: [u32; 4], ep: [u32; 5]) -> Unwind {
    let mut p = CfiProgram::new();
    p.advance_loc(at[0])
        .def_cfa_offset(16)
        .offset(RBP, 2)
        .advance_loc(at[1] - at[0])
        .def_cfa_offset(24)
        .offset(R15, 3)
        .advance_loc(at[2] - at[1])
        .def_cfa_offset(32)
        .offset(R13, 4)
        .advance_loc(at[3] - at[2])
        .def_cfa(RBP, 16)
        // The epilogue: lea rsp keeps CFA = rbp + 16; the pops restore the registers.
        .advance_loc(ep[1] - at[3])
        .remember_state()
        .advance_loc(ep[2] - ep[1])
        .restore(R13)
        .advance_loc(ep[3] - ep[2])
        .restore(R15)
        .advance_loc(ep[4] - ep[3])
        .def_cfa(RSP, 8)
        .restore(RBP)
        .advance_loc(1)
        .restore_state();
    let mut w = UnwindInfoBuilder::new();
    w.push_nonvol(at[0] as u8, wreg::RBP)
        .push_nonvol(at[1] as u8, wreg::R15)
        .push_nonvol(at[2] as u8, wreg::R13)
        .set_fpreg(at[3] as u8, wreg::RBP, 16);
    Unwind {
        cfi: p,
        win64: Some(w.encode()),
    }
}

/// The probe of gate check (g) (see `a64::emit_hold`): the result of `mk()` lives only in
/// the callee-saved rbx (XORed with `mask` when it is not 0) across `scrub()` and `gc()`.
///
/// ```text
/// push rbp ; push r15 ; push r13 ; push rbx ; push r12 ; lea rbp, [rsp + 32]
/// ```
pub(crate) fn emit_hold(
    calls: [(u32, i32, i32); 3],
    mask: u64,
) -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    let exit = ops.new_dynamic_label();
    let mut at = [0u32; 6];
    dynasm!(ops ; .arch x64 ; push rbp);
    at[0] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push r15);
    at[1] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push r13);
    at[2] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push rbx);
    at[3] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; push r12);
    at[4] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; lea rbp, [rsp + 32]);
    at[5] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; mov r15, rdi ; mov r13, rsi);
    let undef = ep9_value::encoding::VALUE_UNDEFINED as i64;
    for (i, &(off, callee, param)) in calls.iter().enumerate() {
        dynasm!(ops
            ; .arch x64
            ; mov rcx, QWORD [r13 + param]
            ; mov QWORD [r13 + callee], rcx
            ; mov rcx, QWORD undef
        );
        call_table(&mut ops, layout::RT_CALL_ANY, off)?;
        dynasm!(ops ; .arch x64 ; test rax, rax ; jz =>exit);
        if i == 0 {
            if mask == 0 {
                dynasm!(ops ; .arch x64 ; mov rbx, rax);
            } else {
                let m = mask as i64;
                dynasm!(ops ; .arch x64 ; mov rbx, QWORD m ; xor rbx, rax);
            }
        }
    }
    dynasm!(ops ; .arch x64 ; mov rax, rbx ; =>exit);
    let mut ep = [0u32; 7];
    ep[0] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; lea rsp, [rbp - 32]);
    ep[1] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop r12);
    ep[2] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop rbx);
    ep[3] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop r13);
    ep[4] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop r15);
    ep[5] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; pop rbp);
    ep[6] = ops.offset().0 as u32;
    dynasm!(ops ; .arch x64 ; ret);
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    const RBX: u16 = 3;
    const R12: u16 = 12;
    let mut p = CfiProgram::new();
    p.advance_loc(at[0])
        .def_cfa_offset(16)
        .offset(RBP, 2)
        .advance_loc(at[1] - at[0])
        .def_cfa_offset(24)
        .offset(R15, 3)
        .advance_loc(at[2] - at[1])
        .def_cfa_offset(32)
        .offset(R13, 4)
        .advance_loc(at[3] - at[2])
        .def_cfa_offset(40)
        .offset(RBX, 5)
        .advance_loc(at[4] - at[3])
        .def_cfa_offset(48)
        .offset(R12, 6)
        .advance_loc(at[5] - at[4])
        .def_cfa(RBP, 16)
        .advance_loc(ep[1] - at[5])
        .remember_state()
        .advance_loc(ep[2] - ep[1])
        .restore(R12)
        .advance_loc(ep[3] - ep[2])
        .restore(RBX)
        .advance_loc(ep[4] - ep[3])
        .restore(R13)
        .advance_loc(ep[5] - ep[4])
        .restore(R15)
        .advance_loc(ep[6] - ep[5])
        .def_cfa(RSP, 8)
        .restore(RBP)
        .advance_loc(1)
        .restore_state();
    let mut w = UnwindInfoBuilder::new();
    w.push_nonvol(at[0] as u8, wreg::RBP)
        .push_nonvol(at[1] as u8, wreg::R15)
        .push_nonvol(at[2] as u8, wreg::R13)
        .push_nonvol(at[3] as u8, wreg::RBX)
        .push_nonvol(at[4] as u8, wreg::R12)
        .set_fpreg(at[5] as u8, wreg::RBP, 32);
    Ok((
        code,
        Unwind {
            cfi: p,
            win64: Some(w.encode()),
        },
    ))
}
