//! The AArch64 unit code generator: the placement (b) code of `crate::jit_b::a64` (same
//! body, same exits), with the unit-data register x27 (H35-Q9).
//!
//! ```text
//!  0  stp x29, x30, [sp, #-48]!     CFA = sp + 48; x29 at CFA-48, x30 at CFA-40
//!  4  mov x29, sp                    CFA = x29 + 48
//!  8  stp x26, x28, [sp, #16]        x26 at CFA-32, x28 at CFA-24
//! 12  str x27, [sp, #32]             x27 at CFA-16
//! 16  mov x28, x0 ; mov x26, x2      isolate register, VM frame pointer
//! 24  ldr x27, [x28, #unit_data] ; ldr x27, [x27, #8*slot]       this isolate's RW copy
//!     mov x0, x3 ; ldr x16, [x28, native_limit] ; mov x17, sp ; cmp x17, x16 ; b.ls overflow
//!     ldr x16, [x27, #counter] ; add x16, x16, #1 ; str x16, [x27, #counter]
//!     body (placement (b)); a table call loads the table from [x27 + ext 0] (a link cell);
//!     a call site first fills its IC cell: [x27 + ic] = callee if it is still undefined
//! leave: mov x0, x28 ; ldrb w9, [x1] ; ldr x16, [x0, x9, lsl #3]
//!        ldr x27, [sp, #32] ; ldp x26, x28, [sp, #16] ; ldp x29, x30, [sp], #48 ; br x16
//! ```
//!
//! Scratch registers: x9, x10, x16, x17 (as `jit_b::a64`). No absolute address and no
//! relocation: branches are pc-relative inside the function; every other reference goes
//! through x28 (Cx) or x27 (the RW copy).

// dynasm expands a dynamic register `X(n)` to a `u8::from(n)` conversion, which clippy flags
// at the call site when `n` is already a u8.
#![allow(clippy::useless_conversion)]

use std::collections::BTreeMap;

use dynasmrt::aarch64::Aarch64Relocation;
use dynasmrt::{DynamicLabel, DynasmApi, DynasmLabelApi, VecAssembler, dynasm};
use ep9_platform::unwind::cfi::CfiProgram;
use ep9_value::encoding::{NUMBER_TAG, VALUE_FALSE, VALUE_UNDEFINED};
use ep9_vm::{cmp, layout};

use super::emit::{EXT_RUNTIME_TABLE, FnCtx, RwLayout};
use crate::emit::{Refusal, Unwind};
use crate::jit::{Op, Plan, Slow};

type Ops = VecAssembler<Aarch64Relocation>;

/// DWARF register numbers.
const X19: u16 = 19;
const X20: u16 = 20;
const X26: u16 = 26;
const X27: u16 = 27;
const X28: u16 = 28;
const FP: u16 = 29;
const LR: u16 = 30;
const SP: u16 = 31;

/// `ldr`/`str` with an unsigned scaled 12-bit offset: 0..=32760, a multiple of 8.
fn scaled(off: usize) -> Result<u32, Refusal> {
    if off.is_multiple_of(8) && off <= 32760 {
        Ok(off as u32)
    } else {
        Err(Refusal::Limit(format!("offset {off}")))
    }
}

/// x`rd` = `v` (movz, then movk per other non-zero halfword). `v` is a constant, never an
/// address.
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

/// x`rt` = the frame slot at byte offset `off` from x26.
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

/// x27 = this isolate's RW copy of the unit: `cx.unit_data[slot]` (`cx` in `cx_reg`).
fn load_unit_data(ops: &mut Ops, cx_reg: u8, l: &RwLayout) -> Result<(), Refusal> {
    let ud = scaled(layout::CX_UNIT_DATA)?;
    let slot = scaled(l.slot_off())?;
    dynasm!(ops
        ; .arch aarch64
        ; ldr x27, [X(cx_reg), ud]
        ; ldr x27, [x27, slot]
    );
    Ok(())
}

/// Calls the table function at `entry` with x0 = cx, x1 = fp, w2 = `off`. The table comes
/// from the RW copy (external reference [`EXT_RUNTIME_TABLE`]), not from `Cx`.
fn call_table(ops: &mut Ops, l: &RwLayout, entry: usize, off: u32) -> Result<(), Refusal> {
    let table = scaled(l.ext_off(EXT_RUNTIME_TABLE))?;
    let entry = scaled(entry)?;
    dynasm!(ops ; .arch aarch64 ; mov x0, x28 ; mov x1, x26);
    mov64(ops, 2, u64::from(off));
    dynasm!(ops
        ; .arch aarch64
        ; ldr x16, [x27, table]
        ; ldr x16, [x16, entry]
        ; blr x16
    );
    Ok(())
}

/// `eor x<r>, x<r>, #NATIVE_WORD_OFFSET`.
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

/// Compiles `plan` for the function `f`: the code, its unwind data and the native offset of
/// each step.
pub(crate) fn emit(plan: &Plan, f: &FnCtx) -> Result<(Vec<u8>, Unwind, Vec<u32>), Refusal> {
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
    let native_limit = scaled(layout::CX_NATIVE_LIMIT)?;
    let budget = scaled(layout::CX_BUDGET)?;
    let counter = scaled(l.counter_off(f.counter))?;
    let label = |t: u32| labels[&t];

    // ---- prologue (offsets 0, 4, 8, 12 are in the CFI below) ----
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-48]!
        ; mov x29, sp
        ; stp x26, x28, [sp, #16]
        ; str x27, [sp, #32]
        ; mov x28, x0
        ; mov x26, x2
    );
    load_unit_data(&mut ops, 28, l)?;
    dynasm!(ops
        ; .arch aarch64
        ; mov x0, x3
        ; ldr x16, [x28, native_limit]
        ; mov x17, sp
        ; cmp x17, x16
        ; b.ls =>overflow
        ; ldr x16, [x27, counter]
        ; add x16, x16, 1
        ; str x16, [x27, counter]
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
                let fv = VALUE_FALSE as u32;
                dynasm!(ops
                    ; .arch aarch64
                    ; and x16, x9, x0
                    ; cmp x16, x17
                    ; b.lo =>slow
                    ; cmp w9, w0
                    ; cset x0, lt
                    ; add x0, x0, fv
                );
                slows.push((slow, Slow::Less { lhs: o }, s.offset, label(s.next)));
            }
            Op::JumpIfFalse(t) => {
                let fv = VALUE_FALSE as u32;
                dynasm!(ops
                    ; .arch aarch64
                    ; cmp x0, fv
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
                let site = f.site(s.offset)?;
                let ic = scaled(l.ic_off(site.ic))?;
                let filled = ops.new_dynamic_label();
                let throw = *throws
                    .entry(s.offset)
                    .or_insert_with(|| ops.new_dynamic_label());
                // IC cell: the first callee (a weak root; GC clears it to undefined).
                load(&mut ops, 9, site.callee);
                mov64(&mut ops, 17, VALUE_UNDEFINED);
                dynasm!(ops
                    ; .arch aarch64
                    ; ldr x10, [x27, ic]
                    ; cmp x10, x17
                    ; b.ne =>filled
                    ; str x9, [x27, ic]
                    ; =>filled
                    ; mov x3, x0
                );
                call_table(&mut ops, l, layout::RT_CALL_ANY, s.offset)?;
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
    let ret_pc = i32::try_from(layout::FP_RETURN_PC).map_err(|_| Refusal::Arch)?;
    let caller = i32::try_from(layout::FP_CALLER_FP).map_err(|_| Refusal::Arch)?;
    if layout::CX_TABLE != 0 {
        return Err(Refusal::Limit(format!(
            "Cx::table at {}, not 0",
            layout::CX_TABLE
        )));
    }
    dynasm!(ops ; .arch aarch64 ; =>ret ; mov x3, x0);
    load(&mut ops, 1, ret_pc);
    load(&mut ops, 2, caller);
    decode(&mut ops, 1)?;
    decode(&mut ops, 2)?;
    dynasm!(ops
        ; .arch aarch64
        ; =>leave
        ; mov x0, x28
        ; ldrb w9, [x1]
        ; ldr x16, [x0, x9, lsl #3]
    );
    let epilogue = ops.offset().0 as u32;
    dynasm!(ops
        ; .arch aarch64
        ; ldr x27, [sp, #32]
        ; ldp x26, x28, [sp, #16]
        ; ldp x29, x30, [sp], #48
        ; br x16
    );

    // ---- out of line ----
    let throw0 = *throws.entry(0).or_insert_with(|| ops.new_dynamic_label());
    dynasm!(ops ; .arch aarch64 ; =>overflow);
    call_table(&mut ops, l, layout::RT_STACK_OVERFLOW, 0)?;
    dynasm!(ops ; .arch aarch64 ; b =>throw0);
    for (slow, kind, off, cont) in slows {
        let throw = *throws.entry(off).or_insert_with(|| ops.new_dynamic_label());
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
        call_table(&mut ops, l, entry, off)?;
        dynasm!(ops
            ; .arch aarch64
            ; cbz x0, =>throw
            ; b =>cont
        );
    }
    for (&off, &t) in &throws {
        dynasm!(ops ; .arch aarch64 ; =>t);
        mov64(&mut ops, 2, u64::from(off));
        dynasm!(ops ; .arch aarch64 ; b =>unwind);
    }
    let table = scaled(l.ext_off(EXT_RUNTIME_TABLE))?;
    let rt_unwind = scaled(layout::RT_UNWIND)?;
    let out_pc = scaled(layout::CX_OUT_PC)?;
    let out_fp = scaled(layout::CX_OUT_FP)?;
    dynasm!(ops
        ; .arch aarch64
        ; =>unwind
        ; mov x0, x28
        ; mov x1, x26
        ; ldr x16, [x27, table]
        ; ldr x16, [x16, rt_unwind]
        ; blr x16
        ; mov x3, x0
        ; ldr x1, [x28, out_pc]
        ; ldr x2, [x28, out_fp]
        ; b =>leave
    );

    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    Ok((
        code,
        Unwind {
            cfi: frame_cfi(epilogue),
            win64: None,
        },
        native_offsets,
    ))
}

/// The CFI of the 48-byte unit frame (code alignment 4, data alignment -8): the prologue rows
/// at 4, 8, 12, 16, then the epilogue at `epilogue` (`ldr x27`, `ldp x26, x28`, `ldp x29, x30`,
/// `br`) between remember/restore state, so the out-of-line code after the `br` has the body's
/// rule again.
fn frame_cfi(epilogue: u32) -> CfiProgram {
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
        .offset(X27, 2)
        .advance_loc(epilogue / 4 - 4)
        .remember_state()
        .advance_loc(1)
        .restore(X27)
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

/// The probe of the unwind self-test (`ep9_platform::unwind::ProbeFn`, AAPCS64): the same
/// instructions as the platform's built-in probe.
///
/// ```text
///  0 stp x29, x30, [sp, #-32]! ;  4 mov x29, sp ;  8 stp x19, x20, [sp, #16]
/// 12 mov x19, x0 ; 16 add x0, x0, #1 ; 20 blr x1 ; 24 add x0, x0, x19
/// 28 ldp x19, x20, [sp, #16] ; 32 ldp x29, x30, [sp], #32 ; 36 ret
/// ```
pub(crate) fn probe() -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-32]!
        ; mov x29, sp
        ; stp x19, x20, [sp, #16]
        ; mov x19, x0
        ; add x0, x0, 1
        ; blr x1
        ; add x0, x0, x19
        ; ldp x19, x20, [sp, #16]
        ; ldp x29, x30, [sp], #32
        ; ret
    );
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(32)
        .offset(FP, 4)
        .offset(LR, 3)
        .advance_loc(1)
        .def_cfa_register(FP)
        .advance_loc(1)
        .offset(X19, 2)
        .offset(X20, 1)
        .advance_loc(4)
        .remember_state()
        .advance_loc(1)
        .restore(X19)
        .restore(X20)
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

/// The cell reader: `cell(cx, i)` returns word `i` of the RW copy that the unit-data register
/// reaches from `cx` (AAPCS64).
///
/// ```text
///  0 stp x29, x30, [sp, #-32]! ;  4 mov x29, sp ;  8 str x27, [sp, #16]
/// 12 ldr x27, [x0, #unit_data] ; 16 ldr x27, [x27, #8*slot] ; 20 ldr x0, [x27, x1, lsl #3]
/// 24 ldr x27, [sp, #16] ; 28 ldp x29, x30, [sp], #32 ; 32 ret
/// ```
pub(crate) fn cell_reader(l: &RwLayout) -> Result<(Vec<u8>, Unwind), Refusal> {
    let mut ops = Ops::new(0);
    dynasm!(ops
        ; .arch aarch64
        ; stp x29, x30, [sp, #-32]!
        ; mov x29, sp
        ; str x27, [sp, #16]
    );
    load_unit_data(&mut ops, 0, l)?;
    dynasm!(ops
        ; .arch aarch64
        ; ldr x0, [x27, x1, lsl #3]
        ; ldr x27, [sp, #16]
        ; ldp x29, x30, [sp], #32
        ; ret
    );
    let code = ops
        .finalize()
        .map_err(|e| Refusal::Limit(format!("dynasm: {e}")))?;
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(32)
        .offset(FP, 4)
        .offset(LR, 3)
        .advance_loc(1)
        .def_cfa_register(FP)
        .advance_loc(1)
        .offset(X27, 2)
        .advance_loc(3)
        .remember_state()
        .advance_loc(1)
        .restore(X27)
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
