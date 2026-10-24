//! The x86-64 unit code generator: the placement (b) code of `crate::jit_b::x64` (same body,
//! same exits, `extern "sysv64"` on every OS), with the unit-data register r14 (H35-Q9).
//!
//! ```text
//!  0  push rbp ; push r15 ; push r13 ; push r14 ; push rbx      rbx: 16-byte alignment only
//!  8  lea rbp, [rsp + 32]                 CFA = rbp + 16          UWOP_SET_FPREG rbp, 32
//! 13  mov r15, rdi ; mov r13, rdx         isolate register, VM frame pointer
//!     mov r14, [r15 + unit_data] ; mov r14, [r14 + 8*slot]        this isolate's RW copy
//!     mov rax, rcx ; cmp rsp, [r15 + native_limit] ; jbe overflow
//!     add qword [r14 + counter], 1
//!     body (placement (b)); a table call loads the table from [r14 + ext 0] (a link cell);
//!     a call site first fills its IC cell: [r14 + ic] = callee if it is still undefined
//! leave: mov rdi, r15 ; movzx eax, byte [rsi]
//!        lea rsp, [rbp - 32] ; pop rbx ; pop r14 ; pop r13 ; pop r15 ; pop rbp
//!        rex.w jmp qword ptr [rdi + rax*8]
//! ```
//!
//! The entry rsp is 8 mod 16; after the 5 pushes every table call is 16-byte aligned. There
//! is no `ret`. Scratch registers: as `jit_b::x64` (rax, rcx, rdx, rsi, rdi, r8-r11). No
//! absolute address and no relocation: branches are rip-relative inside the function; every
//! other reference goes through r15 (Cx) or r14 (the RW copy).

use std::collections::BTreeMap;

use dynasmrt::x64::X64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg as wreg};
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE, VALUE_UNDEFINED};
use ep9_vm::{cmp, layout};

use super::emit::{EXT_RUNTIME_TABLE, FnCtx, RwLayout};
use crate::emit::{Refusal, Unwind};
use crate::jit::{Op, Plan, Slow};

type Ops = VecAssembler<X64Relocation>;

/// The exit jump: `rex.w jmp qword ptr [rdi + rax*8]` (as `jit_b::x64::EXIT_JMP`).
pub const EXIT_JMP: [u8; 4] = [0x48, 0xFF, 0x24, 0xC7];

/// DWARF register numbers (System V psABI figure 3.36).
const RBX: u16 = 3;
const RBP: u16 = 6;
const RSP: u16 = 7;
const R13: u16 = 13;
const R14: u16 = 14;
const R15: u16 = 15;

/// The pushes of a unit frame (x86 register numbers, also the Win64 numbers), in order.
const FRAME: [u8; 5] = [5, 15, 13, 14, 3];

fn disp(off: usize) -> Result<i32, Refusal> {
    i32::try_from(off).map_err(|_| Refusal::Limit(format!("offset {off}")))
}

fn here(ops: &Ops) -> u32 {
    ops.offset().0 as u32
}

/// r14 = this isolate's RW copy of the unit: `cx.unit_data[slot]` (`cx` in rdi or r15).
fn load_unit_data(ops: &mut Ops, cx_in_rdi: bool, l: &RwLayout) -> Result<(), Refusal> {
    let ud = disp(layout::CX_UNIT_DATA)?;
    let slot = disp(l.slot_off())?;
    if cx_in_rdi {
        dynasm!(ops ; .arch x64 ; mov r14, QWORD [rdi + ud]);
    } else {
        dynasm!(ops ; .arch x64 ; mov r14, QWORD [r15 + ud]);
    }
    dynasm!(ops ; .arch x64 ; mov r14, QWORD [r14 + slot]);
    Ok(())
}

/// Calls the table function at `entry` with rdi = cx, rsi = fp, edx = `off` (value operands
/// already in rcx, r8, r9). The table comes from the RW copy (external reference
/// [`EXT_RUNTIME_TABLE`]).
fn call_table(ops: &mut Ops, l: &RwLayout, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = disp(l.ext_off(EXT_RUNTIME_TABLE))?;
    let entry = disp(entry)?;
    let off = off as i32;
    dynasm!(ops
        ; .arch x64
        ; mov rdi, r15
        ; mov rsi, r13
        ; mov edx, DWORD off
        ; mov rax, QWORD [r14 + table]
        ; call QWORD [rax + entry]
    );
    Ok(())
}

/// What [`emit`] makes besides the code: the frame offsets the unwind data and the tests read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameOffsets {
    /// The offset after each push, then after the `lea rbp`.
    pub prologue: Vec<u32>,
    /// The `lea rsp`, each `pop`, then the exit jump.
    pub epilogue: Vec<u32>,
}

/// Compiles `plan` for the function `f`: the code, its unwind data and the native offset of
/// each step.
pub(crate) fn emit(plan: &Plan, f: &FnCtx) -> Result<(Vec<u8>, Unwind, Vec<u32>), Refusal> {
    let (code, unwind, offs, _) = emit_with_frame(plan, f)?;
    Ok((code, unwind, offs))
}

pub(crate) fn emit_with_frame(
    plan: &Plan,
    f: &FnCtx,
) -> Result<(Vec<u8>, Unwind, Vec<u32>, FrameOffsets), Refusal> {
    let l = &f.layout;
    let mut ops = Ops::new(0);
    let labels: BTreeMap<u32, DynamicLabel> = plan
        .steps
        .iter()
        .map(|s| (s.offset, ops.new_dynamic_label()))
        .collect();
    let ret = ops.new_dynamic_label();
    let overflow = ops.new_dynamic_label();
    let unwind = ops.new_dynamic_label();
    let leave = ops.new_dynamic_label();
    let mut throws: BTreeMap<u32, DynamicLabel> = BTreeMap::new();
    let native_limit = disp(layout::CX_NATIVE_LIMIT)?;
    let budget = disp(layout::CX_BUDGET)?;
    let counter = disp(l.counter_off(f.counter))?;
    let label = |t: u32| labels[&t];
    let tag = NUMBER_TAG as i64;
    let fv = VALUE_FALSE as i32;
    let undef = VALUE_UNDEFINED as i64;

    // ---- prologue ----
    let mut at = Vec::with_capacity(FRAME.len() + 1);
    dynasm!(ops ; .arch x64 ; push rbp);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; push r15);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; push r13);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; push r14);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; push rbx);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; lea rbp, [rsp + 32]);
    at.push(here(&ops));
    dynasm!(ops ; .arch x64 ; mov r15, rdi ; mov r13, rdx);
    load_unit_data(&mut ops, false, l)?;
    dynasm!(ops
        ; .arch x64
        ; mov rax, rcx
        ; cmp rsp, QWORD [r15 + native_limit]
        ; jbe =>overflow
        ; add QWORD [r14 + counter], 1
    );

    // ---- body ----
    let mut slows: Vec<(DynamicLabel, Slow, u32, DynamicLabel)> = Vec::new();
    let mut native_offsets = Vec::with_capacity(plan.steps.len());
    for (i, s) in plan.steps.iter().enumerate() {
        dynasm!(ops ; .arch x64 ; =>label(s.offset));
        native_offsets.push(here(&ops));
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
                    ; add eax, fv
                );
                slows.push((slow, Slow::Less { lhs: o }, s.offset, label(s.next)));
            }
            Op::JumpIfFalse(t) => {
                dynasm!(ops
                    ; .arch x64
                    ; cmp rax, fv
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
                let site = f.site(s.offset)?;
                let ic = disp(l.ic_off(site.ic))?;
                let callee = site.callee;
                let filled = ops.new_dynamic_label();
                let throw = *throws
                    .entry(s.offset)
                    .or_insert_with(|| ops.new_dynamic_label());
                // IC cell: the first callee (a weak root; GC clears it to undefined).
                dynasm!(ops
                    ; .arch x64
                    ; mov r10, QWORD [r13 + callee]
                    ; mov r11, QWORD undef
                    ; cmp QWORD [r14 + ic], r11
                    ; jne =>filled
                    ; mov QWORD [r14 + ic], r10
                    ; =>filled
                    ; mov rcx, rax
                );
                call_table(&mut ops, l, layout::RT_CALL_ANY, s.offset)?;
                dynasm!(ops
                    ; .arch x64
                    ; test rax, rax
                    ; jz =>throw
                );
            }
            Op::Return => {
                if next_emitted.is_some() {
                    dynasm!(ops ; .arch x64 ; jmp =>ret);
                }
            }
        }
        if s.falls_through() && next_emitted != Some(s.next) {
            dynasm!(ops ; .arch x64 ; jmp =>label(s.next));
        }
    }

    // ---- return and the common exit ----
    let ret_pc = i32::try_from(layout::FP_RETURN_PC).map_err(|_| Refusal::Arch)?;
    let caller = i32::try_from(layout::FP_CALLER_FP).map_err(|_| Refusal::Arch)?;
    if layout::NATIVE_WORD_OFFSET != 1 << 49 {
        return Err(Refusal::Limit(format!(
            "native word offset {:#x}",
            layout::NATIVE_WORD_OFFSET
        )));
    }
    if layout::CX_TABLE != 0 {
        return Err(Refusal::Limit(format!(
            "Cx::table at {}, not 0",
            layout::CX_TABLE
        )));
    }
    dynasm!(ops
        ; .arch x64
        ; =>ret
        ; mov rcx, rax
        ; mov rsi, QWORD [r13 + ret_pc]
        ; mov rdx, QWORD [r13 + caller]
        ; btc rsi, 49
        ; btc rdx, 49
        ; =>leave
        ; mov rdi, r15
        ; movzx eax, BYTE [rsi]
    );
    let mut ep = vec![here(&ops)];
    dynasm!(ops ; .arch x64 ; lea rsp, [rbp - 32]);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; pop rbx);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; pop r14);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; pop r13);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; pop r15);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; pop rbp);
    ep.push(here(&ops));
    dynasm!(ops ; .arch x64 ; .bytes EXIT_JMP.iter());

    // ---- out of line ----
    let throw0 = *throws.entry(0).or_insert_with(|| ops.new_dynamic_label());
    dynasm!(ops ; .arch x64 ; =>overflow);
    call_table(&mut ops, l, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch x64 ; jmp =>throw0);
    for (slow, kind, off, cont) in slows {
        let throw = *throws.entry(off).or_insert_with(|| ops.new_dynamic_label());
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
        call_table(&mut ops, l, entry, off)?;
        dynasm!(ops
            ; .arch x64
            ; test rax, rax
            ; jz =>throw
            ; jmp =>cont
        );
    }
    for (&off, &t) in &throws {
        dynasm!(ops
            ; .arch x64
            ; =>t
            ; mov edx, DWORD off as i32
            ; jmp =>unwind
        );
    }
    let table = disp(l.ext_off(EXT_RUNTIME_TABLE))?;
    let rt_unwind = disp(layout::RT_UNWIND)?;
    let out_pc = disp(layout::CX_OUT_PC)?;
    let out_fp = disp(layout::CX_OUT_FP)?;
    dynasm!(ops
        ; .arch x64
        ; =>unwind
        ; mov rdi, r15
        ; mov rsi, r13
        ; mov rax, QWORD [r14 + table]
        ; call QWORD [rax + rt_unwind]
        ; mov rcx, rax
        ; mov rsi, QWORD [r15 + out_pc]
        ; mov rdx, QWORD [r15 + out_fp]
        ; jmp =>leave
    );

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let unwind = frame_unwind(&at, &ep);
    Ok((
        code,
        unwind,
        native_offsets,
        FrameOffsets {
            prologue: at,
            epilogue: ep,
        },
    ))
}

/// The DWARF number of a pushed register.
fn dwarf(r: u8) -> u16 {
    match r {
        3 => RBX,
        5 => RBP,
        13 => R13,
        14 => R14,
        15 => R15,
        _ => unreachable!("the unit frame pushes rbp, r15, r13, r14, rbx only"),
    }
}

/// CFI (code alignment 1, data alignment -8) and the Win64 UNWIND_INFO of the unit frame.
/// `at`: the offsets after each push and after the `lea rbp`; `ep`: the `lea rsp`, each `pop`
/// and the exit jump. The epilogue rows sit between remember/restore state, so the
/// out-of-line code after the jump has the body's rule again (as `jit_b::x64`).
fn frame_unwind(at: &[u32], ep: &[u32]) -> Unwind {
    let n = FRAME.len();
    debug_assert!(at.len() == n + 1 && ep.len() == n + 2);
    let mut p = CfiProgram::new();
    let mut prev = 0;
    for (i, &r) in FRAME.iter().enumerate() {
        p.advance_loc(at[i] - prev)
            .def_cfa_offset(8 * (i as u64 + 2))
            .offset(dwarf(r), i as u64 + 2);
        prev = at[i];
    }
    // lea rbp, [rsp + 32]: rbp points at the saved rbp, so CFA = rbp + 16.
    p.advance_loc(at[n] - prev).def_cfa(RBP, 16);
    p.advance_loc(ep[1] - at[n]).remember_state();
    for (j, &r) in FRAME.iter().rev().enumerate() {
        p.advance_loc(ep[j + 2] - ep[j + 1]);
        if r == 5 {
            p.def_cfa(RSP, 8).restore(RBP);
        } else {
            p.restore(dwarf(r));
        }
    }
    p.advance_loc(EXIT_JMP.len() as u32).restore_state();
    let mut w = UnwindInfoBuilder::new();
    for (i, &r) in FRAME.iter().enumerate() {
        w.push_nonvol(at[i] as u8, r);
    }
    w.set_fpreg(at[n] as u8, wreg::RBP, 8 * (n as u8 - 1));
    Unwind {
        cfi: p,
        win64: Some(w.encode()),
    }
}

/// The probe of the unwind self-test (`ep9_platform::unwind::ProbeFn` on unix; System V):
/// the platform's built-in probe bytes.
///
/// ```text
///  0 push rbp ;  1 mov rbp, rsp ;  4 push rbx ;  5 sub rsp, 8 ;  9 mov rbx, rdi
/// 12 lea rdi, [rdi + 1] ; 16 call rsi ; 18 add rax, rbx ; 21 add rsp, 8 ; 25 pop rbx
/// 26 pop rbp ; 27 ret
/// ```
pub(crate) fn probe() -> Result<(Vec<u8>, Unwind), Refusal> {
    #[rustfmt::skip]
    let code = vec![
        0x55,
        0x48, 0x89, 0xe5,
        0x53,
        0x48, 0x83, 0xec, 0x08,
        0x48, 0x89, 0xfb,
        0x48, 0x8d, 0x7f, 0x01,
        0xff, 0xd6,
        0x48, 0x01, 0xd8,
        0x48, 0x83, 0xc4, 0x08,
        0x5b,
        0x5d,
        0xc3,
    ];
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(16)
        .offset(RBP, 2)
        .advance_loc(3)
        .def_cfa_register(RBP)
        .advance_loc(1)
        .offset(RBX, 3)
        .advance_loc(20)
        .remember_state()
        .advance_loc(1)
        .restore(RBX)
        .advance_loc(1)
        .def_cfa(RSP, 8)
        .restore(RBP)
        .advance_loc(1)
        .restore_state();
    let mut w = UnwindInfoBuilder::new();
    w.push_nonvol(1, wreg::RBP)
        .set_fpreg(4, wreg::RBP, 0)
        .push_nonvol(5, wreg::RBX)
        .alloc(9, 8);
    Ok((
        code,
        Unwind {
            cfi: p,
            win64: Some(w.encode()),
        },
    ))
}

/// The cell reader: `cell(cx, i)` returns word `i` of the RW copy that the unit-data register
/// reaches from `cx` (System V: rdi, rsi).
///
/// ```text
///  0 push r14 ; mov r14, [rdi + unit_data] ; mov r14, [r14 + 8*slot]
///    mov rax, [r14 + rsi*8] ; pop r14 ; ret
/// ```
pub(crate) fn cell_reader(l: &RwLayout) -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    dynasm!(ops ; .arch x64 ; push r14);
    let pushed = here(&ops);
    load_unit_data(&mut ops, true, l)?;
    dynasm!(ops ; .arch x64 ; mov rax, QWORD [r14 + rsi * 8]);
    let pop = here(&ops);
    dynasm!(ops ; .arch x64 ; pop r14 ; ret);
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let mut p = CfiProgram::new();
    p.advance_loc(pushed)
        .def_cfa_offset(16)
        .offset(R14, 2)
        .advance_loc(pop - pushed)
        .remember_state()
        .advance_loc(2)
        .def_cfa_offset(8)
        .restore(R14)
        .advance_loc(1)
        .restore_state();
    let mut w = UnwindInfoBuilder::new();
    w.push_nonvol(pushed as u8, wreg::R14);
    Ok((
        code,
        Unwind {
            cfi: p,
            win64: Some(w.encode()),
        },
    ))
}
