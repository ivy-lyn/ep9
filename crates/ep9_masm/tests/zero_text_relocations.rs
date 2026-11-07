//! The zero-text-relocation check (H34 Rules "Unit text has 0 relocations"; 10-quality section 5
//! "Zero text relocations", from A2). The CI step "zero text relocations (ep9_masm)" runs this
//! file without `jit`.
//!
//! Positive: every unit the tests emit (both architectures, every kind but HeapConst) passes.
//! Negative controls: each rule finds the defect it names, in text with and without a record,
//! and `finalize_unit` refuses such text.
#![cfg(feature = "aot")]

mod common;

use common::{a64, x64};
use ep9_masm::{
    Arch, Assembler, CodeDesc, Field, FinalizeError, Finding, Form, Target, check_text,
    finalize_unit, zero_text_relocations,
};

/// Every unit that the ep9_masm tests emit: the corpus of `tests/common`.
fn corpus() -> Vec<CodeDesc> {
    vec![
        common::a64_unit_function(),
        common::x64_unit_function(),
        common::a64_loop(),
    ]
}

#[test]
fn every_unit_the_tests_emit_has_zero_text_relocations() {
    for d in corpus() {
        let u = common::emit_unit(&d);
        assert_eq!(zero_text_relocations(&u), Ok(()), "{:?}", d.arch);
        assert!(!u.fields().is_empty());
        assert!(
            u.fields()
                .iter()
                .all(|f| f.form.class() != ep9_masm::FormClass::Absolute)
        );
    }
}

/// AArch64 words planted in otherwise clean text, with no relocation record: the word scan
/// finds each one.
#[test]
fn the_a64_scan_finds_planted_words() {
    // 4 NOPs, then the planted word at offset 16, then 3 NOPs: text of 32 bytes.
    let text = |w: u32| {
        let mut t = Vec::new();
        for i in 0..8 {
            t.extend_from_slice(&if i == 4 { w } else { a64::NOP }.to_le_bytes());
        }
        t
    };
    let cases: [(u32, Finding); 9] = [
        (a64::ADRP_X0, Finding::Adrp { at: 16 }),
        (a64::BL, Finding::DirectCall { at: 16 }),
        // b +16: 16 + 16 = 32, one past the end.
        (a64::B | 4, Finding::OutsideFunction { at: 16, target: 32 }),
        // b -20: before the start.
        (
            a64::B | (0x03ff_ffff & (-5i32 as u32)),
            Finding::OutsideFunction { at: 16, target: -4 },
        ),
        (
            a64::B_EQ | (8 << 5),
            Finding::OutsideFunction { at: 16, target: 48 },
        ),
        (
            a64::CBZ_X0 | (4 << 5),
            Finding::OutsideFunction { at: 16, target: 32 },
        ),
        (
            a64::TBZ_X0_0 | (4 << 5),
            Finding::OutsideFunction { at: 16, target: 32 },
        ),
        (
            a64::LDR_X0_LIT | (100 << 5),
            Finding::OutsideFunction {
                at: 16,
                target: 416,
            },
        ),
        // adr x0, +17: immlo = 1, immhi = 4.
        (
            a64::ADR_X0 | (1 << 29) | (4 << 5),
            Finding::OutsideFunction { at: 16, target: 33 },
        ),
    ];
    for (w, want) in cases {
        assert_eq!(check_text(Arch::A64, &text(w), &[]), [want], "{w:#010x}");
    }
    // The same forms inside the function are not findings.
    for w in [
        a64::B | 2,
        a64::B_EQ | (2 << 5),
        a64::CBZ_X0 | (0x7_ffff << 5),
        a64::TBZ_X0_0 | (3 << 5),
        a64::LDR_X0_LIT | (1 << 5),
        a64::ADR_X0 | (3 << 29) | (3 << 5),
    ] {
        assert_eq!(check_text(Arch::A64, &text(w), &[]), [], "{w:#010x}");
    }
    let mut odd = text(a64::NOP);
    odd.push(0);
    assert_eq!(
        check_text(Arch::A64, &odd, &[]),
        [Finding::Misaligned { len: 33 }]
    );
}

/// The field rules on both architectures: an absolute field, and a pc-relative field that
/// leaves the function.
#[test]
fn the_field_rules_find_absolute_and_outside_fields() {
    let mut t = vec![0x90u8; 24];
    t[0] = 0xe9;
    t[1..5].copy_from_slice(&100i32.to_le_bytes());
    let fields = [
        Field {
            at: 1,
            form: Form::X64Rel32,
        },
        Field {
            at: 8,
            form: Form::Abs64,
        },
        Field {
            at: 16,
            form: Form::X64Disp32,
        },
    ];
    assert_eq!(
        check_text(Arch::X64, &t, &fields),
        [
            Finding::OutsideFunction { at: 1, target: 105 },
            Finding::AbsoluteField { at: 8 }
        ]
    );
    t[1..5].copy_from_slice(&(-5i32).to_le_bytes());
    // rel32 -5 from the field end (5) is offset 0: inside.
    assert_eq!(check_text(Arch::X64, &t, &fields[..1]), []);
    t[1..5].copy_from_slice(&(-6i32).to_le_bytes());
    assert_eq!(
        check_text(Arch::X64, &t, &fields[..1]),
        [Finding::OutsideFunction { at: 1, target: -1 }]
    );
}

/// `finalize_unit` runs the check: text with a planted `bl` (no record) or a Local label at
/// the end of the text is refused, and no unit comes out.
#[test]
fn finalize_unit_refuses_text_that_fails_the_check() {
    let mut d = common::a64_loop();
    d.text.extend_from_slice(&a64::BL.to_le_bytes());
    let at = d.text.len() as u32 - 4;
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![Finding::DirectCall {
            at
        }]))
    );

    // jmp to a label bound at the end: the target is one past the last byte.
    let mut a = Assembler::new(Arch::X64);
    let end = a.new_label();
    a.x64(
        &x64::JMP_REL32.0,
        x64::JMP_REL32.1,
        Form::X64Rel32,
        Target::Local(end),
    )
    .unwrap();
    a.bind(end).unwrap();
    assert_eq!(
        finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![
            Finding::OutsideFunction { at: 1, target: 5 }
        ]))
    );
}

/// x86-64: a disp32 field ([`Form::X64Disp32`], register-relative) must be the disp32 of a
/// `[reg + disp32]` operand, whose ModRM byte has mod = 10 (Codex r1 A2-MASM 1 (ii)). The
/// check reads the ModRM byte before the field, or the one before a SIB byte. Negative
/// controls: `[rip + disp32]` (mod = 00, rm = 101: a pc-relative field that the form label
/// would hide), `[disp32 + index]` with no base (mod = 00, SIB base = 101: an absolute
/// address) and a disp8 operand (mod = 01). `finalize_unit` refuses the RIP-relative one.
#[test]
fn an_x64_disp32_field_needs_a_reg_plus_disp32_operand() {
    let field = |bytes: &[u8]| {
        let mut t = bytes.to_vec();
        let at = t.len() as u32;
        t.extend_from_slice(&0x50i32.to_le_bytes());
        t.push(0x90);
        let f = [Field {
            at,
            form: Form::X64Disp32,
        }];
        check_text(Arch::X64, &t, &f)
    };
    // mov rax, [r14 + disp32]; mov rax, [rsp + disp32] (ModRM rm = 100, a SIB byte).
    assert_eq!(field(&[0x49, 0x8b, 0x86]), []);
    assert_eq!(field(&[0x48, 0x8b, 0x84, 0x24]), []);
    // mov rax, [rip + disp32]; mov rax, [disp32] (SIB, no base); mov rax, [rsi + disp8].
    assert_eq!(
        field(&[0x48, 0x8b, 0x05]),
        [Finding::NotRegisterRelative { at: 3 }]
    );
    assert_eq!(
        field(&[0x48, 0x8b, 0x04, 0x25]),
        [Finding::NotRegisterRelative { at: 4 }]
    );
    assert_eq!(
        field(&[0x48, 0x8b, 0x46]),
        [Finding::NotRegisterRelative { at: 3 }]
    );
    // A field at offset 0 has no ModRM byte.
    assert_eq!(field(&[]), [Finding::NotRegisterRelative { at: 0 }]);

    // The Codex probe: 48 8b 05 <disp32> as X64Disp32 / LinkSlot(0) gave unit text
    // `mov rax, [rip + 0x50]`.
    let mut a = Assembler::new(Arch::X64);
    a.x64(
        &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
        3,
        Form::X64Disp32,
        Target::LinkSlot(0),
    )
    .unwrap();
    a.put(&x64::NOP);
    assert_eq!(
        finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotRegisterRelative { at: 3 }
        ]))
    );
}
