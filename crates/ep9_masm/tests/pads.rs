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
    // br x17 too; blr and ret through any register.
    assert_eq!(check_pads(Arch::A64, &padded_a64(0xd61f_0220).text), []);
    let mut t = d.text.clone();
    t.extend_from_slice(&0xd63f_0000u32.to_le_bytes()); // blr x0
    t.extend_from_slice(&0xd65f_03c0u32.to_le_bytes()); // ret
    assert_eq!(check_pads(Arch::A64, &t), []);
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
                Finding::IndirectJumpRegister { at }
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
/// `finalize_unit`. Now every `adr` target in the text must be `bti c` or `bti jc`, also
/// through a copy of the register (`adr x0; mov x16, x0; br x16`).
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
    let d = unit(&[ADR_X0_12, MOV_X16_X0, a64::BR_X16, a64::MOVZ_X0_1, RET]);
    assert_eq!(
        check_pads(Arch::A64, &d.text),
        [Finding::NoLandingPad { at: 16 }]
    );
    // With `bti c` (or `bti jc`) at the target: no finding.
    for pad in [A64_BTI_C, 0xd503_24df] {
        let d = unit(&[ADR_X0_8, BLR_X0, pad, RET]);
        assert!(finalize_unit(&d, &Padded).unwrap().has_pads());
    }
}
