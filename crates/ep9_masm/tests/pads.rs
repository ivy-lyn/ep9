//! Landing pads (MC-w23-15): `Assembler::pad` writes `bti c` at an AArch64 function entry, and
//! a unit with landing pads (the container compat flag `has_pads`, ep9_unit) passes
//! `check_pads`: the entry is a landing pad, and every indirect jump goes through x16 or x17,
//! so the loader can map its text with BTI.
#![cfg(feature = "aot")]

mod common;

use common::{a64, x64};
use ep9_masm::{
    A64_BTI_C, Arch, Assembler, FinalizeError, Finding, Form, Target, UnitEnv, check_pads,
    finalize_unit, zero_text_relocations,
};

/// The test environment of `common`, for a unit with landing pads.
struct Padded;

impl UnitEnv for Padded {
    fn stub_link(&self, id: u32) -> Option<u32> {
        common::TestUnit.stub_link(id)
    }
    fn runtime_slot(&self, id: u32) -> Option<i64> {
        common::TestUnit.runtime_slot(id)
    }
    fn link_word(&self, index: u32) -> Option<u32> {
        common::TestUnit.link_word(index)
    }
    fn ic_word(&self, index: u32) -> Option<u32> {
        common::TestUnit.ic_word(index)
    }
    fn root_slot(&self, index: u32) -> Option<i64> {
        common::TestUnit.root_slot(index)
    }
    fn deopt_slot(&self) -> i64 {
        common::TestUnit.deopt_slot()
    }
    fn pads(&self) -> bool {
        true
    }
}

/// `bti c` at the entry, a call through a register, a tail call through x16.
fn padded_a64(tail: u32) -> ep9_masm::CodeDesc {
    let mut a = Assembler::new(Arch::A64);
    a.pad();
    a.a64(a64::LDR_X16_X27, Form::A64Ldst64Imm12, Target::Stub(3))
        .unwrap();
    a.put32(a64::BLR_X16);
    a.a64(a64::LDR_X16_X27, Form::A64Ldst64Imm12, Target::LinkSlot(1))
        .unwrap();
    a.put32(tail);
    a.finish().unwrap()
}

#[test]
fn a_padded_a64_unit_passes_the_pad_check() {
    let d = padded_a64(a64::BR_X16);
    assert_eq!(&d.text[..4], &A64_BTI_C.to_le_bytes());
    let u = finalize_unit(&d, &Padded).unwrap();
    assert!(u.has_pads());
    assert_eq!(zero_text_relocations(&u), Ok(()));
    // br x17 too, after a load of x17; ret through any register.
    let mut t = d.text.clone();
    t.truncate(t.len() - 4);
    t.extend_from_slice(&0xf940_0371u32.to_le_bytes()); // ldr x17, [x27]
    t.extend_from_slice(&0xd61f_0220u32.to_le_bytes()); // br x17
    t.extend_from_slice(&0xd65f_0020u32.to_le_bytes()); // ret x1
    assert_eq!(check_pads(Arch::A64, &t), []);
    // A blr through a register that nothing in its run set is refused (PADS-PROVENANCE).
    let mut t = d.text.clone();
    t.extend_from_slice(&0xd63f_0000u32.to_le_bytes()); // blr x0
    assert_eq!(
        check_pads(Arch::A64, &t),
        [Finding::IndirectTargetNotProven {
            at: t.len() as u32 - 4
        }]
    );
    // The same unit without the flag: no pad check, and no pad needed.
    let u = finalize_unit(&d, &common::TestUnit).unwrap();
    assert!(!u.has_pads());
}

#[test]
fn the_pad_check_refuses_a_missing_pad_and_other_jump_registers() {
    // No pad: the entry is the load.
    let mut d = padded_a64(a64::BR_X16);
    d.text.drain(..4);
    for r in &mut d.relocs {
        r.at -= 4;
        r.insn -= 4;
    }
    assert_eq!(
        finalize_unit(&d, &Padded),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NoLandingPad { at: 0 }
        ]))
    );
    assert!(finalize_unit(&d, &common::TestUnit).is_ok());
    // br x0, braa x1, x2, braaz x3, brabz x9: the target would need `bti j`.
    for (w, name) in [
        (0xd61f_0000u32, "br x0"),
        (0xd71f_0822, "braa x1, x2"),
        (0xd61f_087f, "braaz x3"),
        (0xd61f_0d3f, "brabz x9"),
    ] {
        let d = padded_a64(w);
        let at = d.text.len() as u32 - 4;
        assert_eq!(
            finalize_unit(&d, &Padded),
            Err(FinalizeError::TextRelocations(vec![
                Finding::IndirectJumpRegister { at },
                Finding::IndirectTargetNotProven { at }
            ])),
            "{name}"
        );
    }
    // Text shorter than one word has no pad.
    assert_eq!(
        check_pads(Arch::A64, &[0x5f, 0x24]),
        [Finding::NoLandingPad { at: 0 }]
    );
    // `bti jc` is a landing pad for calls too; `bti j` is not.
    assert_eq!(check_pads(Arch::A64, &0xd503_24dfu32.to_le_bytes()), []);
    assert_eq!(
        check_pads(Arch::A64, &0xd503_249fu32.to_le_bytes()),
        [Finding::NoLandingPad { at: 0 }]
    );
}

#[test]
fn x64_has_no_pad_form_yet() {
    let mut a = Assembler::new(Arch::X64);
    a.pad();
    assert_eq!(a.offset(), 0);
    a.put(&x64::JMP_RAX);
    let d = a.finish().unwrap();
    assert_eq!(check_pads(Arch::X64, &d.text), []);
    assert!(finalize_unit(&d, &Padded).unwrap().has_pads());
}

/// JIT review M8-P2 B2: `adr x16, L; br x16` enters L by an indirect jump, so under BTI the
/// word at L must be a landing pad. The old check read only the entry and the `br` register,
/// so `bti c; adr x16, +8; br x16; mov x0, #1` passed `finalize_unit` and would fault at
/// offset 12 under PROT_BTI.
#[test]
fn the_pad_check_refuses_an_adr_jump_target_with_no_pad() {
    const ADR_X16_8: u32 = 0x1000_0050; // adr x16, #8
    const ADR_X17_8: u32 = 0x1000_0051; // adr x17, #8
    const ADR_X0_8: u32 = 0x1000_0040; // adr x0, #8
    const RET: u32 = 0xd65f_03c0;
    let unit = |words: &[u32]| {
        let mut a = Assembler::new(Arch::A64);
        a.pad();
        for &w in words {
            a.put32(w);
        }
        a.finish().unwrap()
    };
    let d = unit(&[ADR_X16_8, a64::BR_X16, a64::MOVZ_X0_1, RET]);
    assert_eq!(
        finalize_unit(&d, &Padded),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NoLandingPad { at: 12 }
        ]))
    );
    assert!(finalize_unit(&d, &common::TestUnit).is_ok());
    let d = unit(&[ADR_X17_8, 0xd61f_0220, a64::MOVZ_X0_1, RET]);
    assert_eq!(
        check_pads(Arch::A64, &d.text),
        [Finding::NoLandingPad { at: 12 }]
    );
    // With `bti c` (or `bti jc`) at the target: no finding.
    for pad in [A64_BTI_C, 0xd503_24df] {
        let d = unit(&[ADR_X16_8, a64::BR_X16, pad, RET]);
        assert!(finalize_unit(&d, &Padded).unwrap().has_pads());
    }
    // An `adr` into another register is a target too (Codex M8-P2 r2, below).
    assert_eq!(
        check_pads(Arch::A64, &unit(&[ADR_X0_8, RET, a64::MOVZ_X0_1]).text),
        [Finding::NoLandingPad { at: 12 }]
    );
}

/// Codex M8-P2 r2 R2-2: `bti c; adr x0, +8; blr x0; mov x0, #1; ret`. `blr` may use any
/// register, so the call reaches offset 12, which is not a landing pad, and the text would
/// fault under PROT_BTI. The old check read only an `adr` into x16 or x17, so this unit passed
/// `finalize_unit`. Now every `adr` target in the text must be `bti c` or `bti jc`; a jump
/// through a copy of the register (`adr x0; mov x16, x0; br x16`) is also not a proven form.
#[test]
fn the_pad_check_refuses_an_adr_call_target_with_no_pad_in_any_register() {
    const ADR_X0_8: u32 = 0x1000_0040; // adr x0, #8
    const ADR_X0_12: u32 = 0x1000_0060; // adr x0, #12
    const BLR_X0: u32 = 0xd63f_0000;
    const MOV_X16_X0: u32 = 0xaa00_03f0; // mov x16, x0
    const RET: u32 = 0xd65f_03c0;
    let unit = |words: &[u32]| {
        let mut a = Assembler::new(Arch::A64);
        a.pad();
        for &w in words {
            a.put32(w);
        }
        a.finish().unwrap()
    };
    let d = unit(&[ADR_X0_8, BLR_X0, a64::MOVZ_X0_1, RET]);
    assert_eq!(
        finalize_unit(&d, &Padded),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NoLandingPad { at: 12 }
        ]))
    );
    assert!(finalize_unit(&d, &common::TestUnit).is_ok());
    // Through a copy: the target has no pad, and the copy is not a proven form either
    // (PADS-PROVENANCE).
    let d = unit(&[ADR_X0_12, MOV_X16_X0, a64::BR_X16, a64::MOVZ_X0_1, RET]);
    assert_eq!(
        check_pads(Arch::A64, &d.text),
        [
            Finding::NoLandingPad { at: 16 },
            Finding::IndirectTargetNotProven { at: 12 }
        ]
    );
    // With `bti c` (or `bti jc`) at the target: no finding.
    for pad in [A64_BTI_C, 0xd503_24df] {
        let d = unit(&[ADR_X0_8, BLR_X0, pad, RET]);
        assert!(finalize_unit(&d, &Padded).unwrap().has_pads());
    }
}

/// PADS-PROVENANCE (Codex M8-P2 r3; route PADS-PROVENANCE): under BTI the target of each
/// `br` and `blr` must be a landing pad or code outside this text. The old check read only
/// the target of each `adr`, so a text address that comes another way passed: arithmetic on
/// an `adr` result (`adr x16, +12; add x16, x16, #4; br x16` lands on an unpadded `ret`), x30
/// after a `blr` (a return address: `mov x16, x30; br x16`), a literal load from the text, a
/// load from the stack (a saved x30), and a value that a branch can bypass. Now the register
/// of each `br` and `blr` must be set in the same straight-line run by an allowed form: a
/// 64-bit load through the unit-data register x27 or the isolate register x28 (a link cell,
/// a stub cell or a RuntimeTable entry: code outside the text), a load through a register
/// that such a load set (one step: a table read through the isolate), or an `adr` (its
/// target must be a pad). Else `IndirectTargetNotProven`.
#[test]
fn the_pad_check_proves_each_indirect_branch_target() {
    const BTI_C: u32 = A64_BTI_C;
    const ADR_X16_12: u32 = 0x1000_0070; // adr x16, #12
    const ADD_X16_X16_4: u32 = 0x9100_1210; // add x16, x16, #4
    const MOV_X16_X30: u32 = 0xaa1e_03f0; // mov x16, x30
    const LDR_X16_LIT_8: u32 = 0x5800_0050; // ldr x16, #8
    const LDR_X16_SP_8: u32 = 0xf940_07f0; // ldr x16, [sp, #8]
    const CBZ_X0_8: u32 = 0xb400_0040; // cbz x0, #8
    const LDP_FP_LR_POST: u32 = 0xa8c1_7bfd; // ldp x29, x30, [sp], #16
    const LDR_X16_X16_24: u32 = 0xf940_0e10; // ldr x16, [x16, #24]
    const LDR_X17_X27_8: u32 = 0xf940_0771; // ldr x17, [x27, #8]
    const LDR_X16_X28_8: u32 = 0xf940_0790; // ldr x16, [x28, #8]
    const MOVZ_W2_0: u32 = 0x5280_0002; // mov w2, #0
    const MOV_X3_XZR: u32 = 0xaa1f_03e3; // mov x3, xzr
    const BR_X17: u32 = 0xd61f_0220;
    const RET: u32 = 0xd65f_03c0;
    let text = |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|w| w.to_le_bytes()).collect() };
    let not_proven = |at: u32| Finding::IndirectTargetNotProven { at };

    // Refused: each form that the old check let through.
    // The Codex r3 case: the adr target (16) is a pad, the jump lands at 20, a `ret`.
    let codex = text(&[BTI_C, ADR_X16_12, ADD_X16_X16_4, a64::BR_X16, BTI_C, RET]);
    assert_eq!(check_pads(Arch::A64, &codex), [not_proven(12)]);
    // x30 after `blr`: the return address is a text address with no pad.
    let lr = text(&[
        BTI_C,
        a64::LDR_X16_X27,
        a64::BLR_X16,
        MOV_X16_X30,
        a64::BR_X16,
    ]);
    assert_eq!(check_pads(Arch::A64, &lr), [not_proven(16)]);
    // A literal load reads text bytes.
    let lit = text(&[BTI_C, LDR_X16_LIT_8, a64::BR_X16, 0, 0]);
    assert_eq!(check_pads(Arch::A64, &lit), [not_proven(8)]);
    // A load from the stack may read a saved x30.
    let stack = text(&[BTI_C, LDR_X16_SP_8, a64::BR_X16]);
    assert_eq!(check_pads(Arch::A64, &stack), [not_proven(8)]);
    // A branch target between the load and the jump: the jump is also reached without it.
    let bypass = text(&[BTI_C, CBZ_X0_8, a64::LDR_X16_X27, a64::BR_X16]);
    assert_eq!(check_pads(Arch::A64, &bypass), [not_proven(12)]);
    // A landing pad between the load and the call: an indirect entry bypasses the load too.
    let pad = text(&[BTI_C, a64::LDR_X16_X27, BTI_C, a64::BLR_X16]);
    assert_eq!(check_pads(Arch::A64, &pad), [not_proven(12)]);
    // `br x17` after a load of x16: x17 holds what it held before.
    let other = text(&[BTI_C, a64::LDR_X16_X27, BR_X17]);
    assert_eq!(check_pads(Arch::A64, &other), [not_proven(8)]);
    // No definition at all (the entry).
    assert_eq!(
        check_pads(Arch::A64, &text(&[BTI_C, a64::BLR_X16])),
        [not_proven(4)]
    );
    // A call in between: the callee may change x16 (IP0).
    let call = text(&[BTI_C, a64::LDR_X16_X27, a64::BLR_X16, a64::BLR_X16]);
    assert_eq!(check_pads(Arch::A64, &call), [not_proven(12)]);
    // A load through an `adr` base reads text bytes (JIT review M8-P3 F2, Codex M8-P3 r1 #1):
    // `adr x1, #12; ldr x16, [x1]; br x16` jumps to the 8 bytes at 16 (here `bti c` and 0).
    // An `adr` is a proven form only as the branch register itself, not as a load base.
    const ADR_X1_12: u32 = 0x1000_0061; // adr x1, #12
    const LDR_X16_X1: u32 = 0xf940_0030; // ldr x16, [x1]
    let adr_base = text(&[BTI_C, ADR_X1_12, LDR_X16_X1, a64::BR_X16, BTI_C, 0]);
    assert_eq!(check_pads(Arch::A64, &adr_base), [not_proven(12)]);

    // Allowed: the emitter forms.
    for ok in [
        // A tail call through a link cell after the frame pop (the route's first form).
        text(&[BTI_C, LDR_X17_X27_8, LDP_FP_LR_POST, BR_X17]),
        // A RuntimeTable call: the argument moves do not touch x16.
        text(&[
            BTI_C,
            LDR_X16_X28_8,
            MOVZ_W2_0,
            MOV_X3_XZR,
            a64::BLR_X16,
            RET,
        ]),
        // A table read through the isolate register, then its entry (one step).
        text(&[BTI_C, LDR_X16_X28_8, LDR_X16_X16_24, a64::BLR_X16, RET]),
        // `adr` of a pad, then a call through it (and through x16).
        text(&[BTI_C, 0x1000_0040, 0xd63f_0000, BTI_C, RET]), // adr x0, #8; blr x0
        text(&[BTI_C, 0x1000_0050, a64::BR_X16, BTI_C, RET]), // adr x16, #8; br x16
        // `ret` is not a BTI branch: any register.
        text(&[BTI_C, 0xd65f_0020]), // ret x1
    ] {
        assert_eq!(check_pads(Arch::A64, &ok), [], "{ok:02x?}");
    }
    // The two-step chain through a register that no allowed load set is refused.
    let chain = text(&[BTI_C, LDR_X16_SP_8, LDR_X16_X16_24, a64::BLR_X16]);
    assert_eq!(check_pads(Arch::A64, &chain), [not_proven(12)]);
    // finalize_unit refuses it too.
    let mut a = Assembler::new(Arch::A64);
    a.pad();
    for w in [ADR_X16_12, ADD_X16_X16_4, a64::BR_X16, BTI_C, RET] {
        a.put32(w);
    }
    assert_eq!(
        finalize_unit(&a.finish().unwrap(), &Padded),
        Err(FinalizeError::TextRelocations(vec![not_proven(12)]))
    );
}

/// JIT review M8-P3 F3, Codex M8-P3 r1 #1: a load through x27 or x28 proves a branch target
/// only because the unit-data register x27 and the isolate register x28 hold their pinned
/// values (the RW copy, the Cx). The old check did not check that premise, so
/// `bti c; ldr x27, [x0]; ldr x16, [x27]; br x16` and the same with `mov x27, x0` passed:
/// the target is any value that x0 names. Now a load through x27 (x28) proves nothing when a
/// word of the text may write x27 (x28), wherever that word is.
#[test]
fn the_pad_check_needs_the_pinned_registers_unwritten() {
    const BTI_C: u32 = A64_BTI_C;
    const LDR_X27_X0: u32 = 0xf940_001b; // ldr x27, [x0]
    const MOV_X27_X0: u32 = 0xaa00_03fb; // mov x27, x0
    const MOV_X28_X0: u32 = 0xaa00_03fc; // mov x28, x0
    const CASP_X26_X27: u32 = 0x483a_7c40; // casp x26, x27, x0, x1, [x2] (writes x26, x27)
    const LD64B_X20: u32 = 0xf83f_d014; // ld64b x20, [x0] (writes x20 to x27)
    const LDP_X27_X28_POST: u32 = 0xa8c1_73fb; // ldp x27, x28, [sp], #16
    const LDR_X0_X27_POST: u32 = 0xf840_8760; // ldr x0, [x27], #8 (writes x27 back)
    const LDR_X16_X28_8: u32 = 0xf940_0790; // ldr x16, [x28, #8]
    const STR_X27_SP_32: u32 = 0xf900_13fb; // str x27, [sp, #32]
    const LDR_D27_X0: u32 = 0xfd40_001b; // ldr d27, [x0]
    const ADD_X0_X27_1: u32 = 0x9100_0760; // add x0, x27, #1
    const RET: u32 = 0xd65f_03c0;
    let text = |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|w| w.to_le_bytes()).collect() };
    let not_proven = |at: u32| Finding::IndirectTargetNotProven { at };

    // The ldr and the mov form of the probe, and the pair and LD64B forms: refused.
    for w in [LDR_X27_X0, MOV_X27_X0, CASP_X26_X27, LD64B_X20] {
        let t = text(&[BTI_C, w, a64::LDR_X16_X27, a64::BR_X16]);
        assert_eq!(check_pads(Arch::A64, &t), [not_proven(12)], "{w:#x}");
    }
    // A write anywhere in the text counts: after the branch, or as a writeback base.
    for w in [MOV_X27_X0, LDR_X0_X27_POST, LDP_X27_X28_POST] {
        let t = text(&[BTI_C, a64::LDR_X16_X27, a64::BR_X16, w, RET]);
        assert_eq!(check_pads(Arch::A64, &t), [not_proven(8)], "{w:#x}");
    }
    // x28 the same way.
    for w in [MOV_X28_X0, LDP_X27_X28_POST] {
        let t = text(&[BTI_C, w, LDR_X16_X28_8, a64::BLR_X16, RET]);
        assert_eq!(check_pads(Arch::A64, &t), [not_proven(12)], "{w:#x}");
    }
    // A write of x27 does not touch a proof through x28.
    let t = text(&[BTI_C, MOV_X27_X0, LDR_X16_X28_8, a64::BLR_X16, RET]);
    assert_eq!(check_pads(Arch::A64, &t), []);
    // Words that only read x27 or write d27 are not writes.
    let t = text(&[
        BTI_C,
        STR_X27_SP_32,
        LDR_D27_X0,
        ADD_X0_X27_1,
        a64::LDR_X16_X27,
        a64::BR_X16,
    ]);
    assert_eq!(check_pads(Arch::A64, &t), []);
}
