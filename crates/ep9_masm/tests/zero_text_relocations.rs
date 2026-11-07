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
    Arch, Assembler, CodeDesc, Field, FinalizeError, Finding, Form, RelocKind, RwUse, Target,
    check_text, check_unit_data_loads, finalize_unit, zero_text_relocations,
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
    // `mov rax, [rip + 0x50]`. It is not a load through r14 either (J1).
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
            Finding::NotRegisterRelative { at: 3 },
            Finding::NotUnitDataLoad { at: 3 }
        ]))
    );
}

/// LinkSlot, IcCell and the stub's link cell are loads through the unit-data register x27/r14
/// (H34-Q1; Claude JIT reviewer J1). `finalize_unit` refuses each such field in another
/// instruction: a load through the isolate register, a store, a `lea`, a 32-bit load, or a
/// load with an index register. Each of these 8 inputs gave a unit (`Ok`) on the old code.
#[test]
fn a_unit_data_field_needs_a_load_through_the_unit_data_register() {
    let a64_one = |w: u32, t: Target| {
        let mut a = Assembler::new(Arch::A64);
        a.a64(w, Form::A64Ldst64Imm12, t).unwrap();
        finalize_unit(&a.finish().unwrap(), &common::TestUnit).map(|u| u.rw().len())
    };
    let x64_one = |b: &[u8], t: Target| {
        let mut a = Assembler::new(Arch::X64);
        let mut v = b.to_vec();
        v.extend_from_slice(&[0; 4]);
        a.x64(&v, b.len() as u32, Form::X64Disp32, t).unwrap();
        a.put(&x64::NOP);
        finalize_unit(&a.finish().unwrap(), &common::TestUnit).map(|u| u.rw().len())
    };
    let refused = |at| {
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotUnitDataLoad { at },
        ]))
    };
    // AArch64: ldr x0, [x28]; str x0, [x27]; ldr x16, [x28].
    assert_eq!(a64_one(a64::LDR_X0_X28, Target::LinkSlot(2)), refused(0));
    assert_eq!(a64_one(STR_X0_X27, Target::IcCell(3)), refused(0));
    assert_eq!(a64_one(a64::LDR_X16_X28, Target::Stub(1)), refused(0));
    // x86-64: mov rax, [r15 + d32]; mov [r14 + d32], rax; lea rax, [r14 + d32];
    // mov eax, [r14 + d32]; mov rax, [r14 + r12 + d32].
    assert_eq!(
        x64_one(&[0x49, 0x8b, 0x87], Target::LinkSlot(2)),
        refused(3)
    );
    assert_eq!(x64_one(&[0x49, 0x89, 0x86], Target::IcCell(1)), refused(3));
    assert_eq!(x64_one(&[0x49, 0x8d, 0x86], Target::Stub(1)), refused(3));
    assert_eq!(
        x64_one(&[0x41, 0x8b, 0x86], Target::LinkSlot(1)),
        refused(3)
    );
    assert_eq!(
        x64_one(&[0x4b, 0x8b, 0x84, 0x26], Target::IcCell(1)),
        refused(4)
    );
    // The loads of the form: ldr x16, [x27]; ldr x0, [x27]; mov rax, [r14 + d32];
    // mov r8, [r14 + d32] (REX.R: any destination).
    assert_eq!(a64_one(a64::LDR_X16_X27, Target::LinkSlot(2)), Ok(1));
    assert_eq!(a64_one(0xf940_0360, Target::IcCell(3)), Ok(1));
    assert_eq!(x64_one(&[0x49, 0x8b, 0x86], Target::Stub(1)), Ok(1));
    assert_eq!(x64_one(&[0x4d, 0x8b, 0x86], Target::IcCell(1)), Ok(1));
    // The isolate-register rows are not checked here: Runtime through x28 is a unit.
    assert_eq!(a64_one(a64::LDR_X16_X28, Target::Runtime(4)), Ok(0));

    // The check alone, on records read back: a field past the end, and an x86-64 field with
    // fewer than 3 bytes before it.
    let rw = |at| RwUse {
        at,
        kind: RelocKind::LinkSlot,
        index: 0,
        word: 0,
    };
    assert_eq!(
        check_unit_data_loads(Arch::A64, &a64::LDR_X16_X27.to_le_bytes(), &[rw(4)]),
        [Finding::FieldOutsideText { at: 4 }]
    );
    assert_eq!(
        check_unit_data_loads(Arch::X64, &[0x8b, 0x86, 0, 0, 0, 0], &[rw(2)]),
        [Finding::NotUnitDataLoad { at: 2 }]
    );
    assert_eq!(
        check_unit_data_loads(Arch::X64, &[0x49, 0x8b, 0x86, 0, 0, 0], &[rw(3)]),
        [Finding::FieldOutsideText { at: 3 }]
    );
}

/// `check_text` is public, and a check of a unit read back gives it fields from outside: a
/// field past the end of the text is a finding, not a panic (Claude JIT reviewer J2). On the
/// old code the first input panicked (a slice index) and the second overflowed `at + 4`.
#[test]
fn check_text_gives_a_finding_for_a_field_outside_the_text() {
    let one =
        |arch, len: usize, at: u32, form| check_text(arch, &vec![0; len], &[Field { at, form }]);
    assert_eq!(
        one(Arch::X64, 4, 10, Form::X64Rel32),
        [Finding::FieldOutsideText { at: 10 }]
    );
    assert_eq!(
        one(Arch::X64, 4, u32::MAX, Form::X64Rel32),
        [Finding::FieldOutsideText { at: u32::MAX }]
    );
    assert_eq!(
        one(Arch::X64, 12, 5, Form::Abs64),
        [Finding::FieldOutsideText { at: 5 }]
    );
    // A field inside the text gets the other rules.
    assert_eq!(
        one(Arch::X64, 8, 2, Form::X64Disp32),
        [Finding::NotRegisterRelative { at: 2 }]
    );
    // AArch64: a word field at the last word is inside; one word later it is not.
    let b = |at| Field {
        at,
        form: Form::A64Branch26,
    };
    // nop; b +0 (a branch to itself: inside).
    let mut t = a64::NOP.to_le_bytes().repeat(2);
    t[4..].copy_from_slice(&a64::B.to_le_bytes());
    assert_eq!(check_text(Arch::A64, &t, &[b(4)]), []);
    assert_eq!(
        check_text(Arch::A64, &t, &[b(8)]),
        [Finding::FieldOutsideText { at: 8 }]
    );
}

/// `str x0, [x27, #0]`.
const STR_X0_X27: u32 = 0xf900_0360;
