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
