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
    Arch, Assembler, CodeDesc, Field, FinalizeError, Finding, Form, Reloc, RelocKind, Target,
    check_register_loads, check_text, finalize_unit, zero_text_relocations,
};

/// A field record for the checks alone (a unit read back).
fn field(at: u32, insn: u32, form: Form, kind: RelocKind) -> Field {
    Field {
        at,
        insn,
        form,
        kind,
    }
}

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
    let mut t = vec![0x90u8; 28];
    t[0] = 0xe9;
    t[1..5].copy_from_slice(&100i32.to_le_bytes());
    // mov rax, [r14 + disp32] at 17, its disp32 at 20.
    t[17..20].copy_from_slice(&[0x49, 0x8b, 0x86]);
    let fields = [
        field(1, 0, Form::X64Rel32, RelocKind::Local),
        field(8, 8, Form::Abs64, RelocKind::HeapConst),
        field(20, 17, Form::X64Disp32, RelocKind::LinkSlot),
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
/// check decodes the instruction forward from its start (fixer r2c). Negative controls:
/// `[rip + disp32]` (mod = 00, rm = 101: a pc-relative field that the form label would
/// hide), `[disp32 + index]` with no base (mod = 00, SIB base = 101: an absolute address)
/// and a disp8 operand (mod = 01). `finalize_unit` refuses the RIP-relative one.
#[test]
fn an_x64_disp32_field_needs_a_reg_plus_disp32_operand() {
    let one = |bytes: &[u8]| {
        let mut t = bytes.to_vec();
        let at = t.len() as u32;
        t.extend_from_slice(&0x50i32.to_le_bytes());
        t.push(0x90);
        let f = [field(at, 0, Form::X64Disp32, RelocKind::LinkSlot)];
        check_text(Arch::X64, &t, &f)
    };
    // mov rax, [r14 + disp32]; mov rax, [rsp + disp32] (ModRM rm = 100, a SIB byte).
    assert_eq!(one(&[0x49, 0x8b, 0x86]), []);
    assert_eq!(one(&[0x48, 0x8b, 0x84, 0x24]), []);
    // mov rax, [rip + disp32]; mov rax, [disp32] (SIB, no base); mov rax, [rsi + disp8].
    assert_eq!(
        one(&[0x48, 0x8b, 0x05]),
        [Finding::NotRegisterRelative { at: 3 }]
    );
    assert_eq!(
        one(&[0x48, 0x8b, 0x04, 0x25]),
        [Finding::NotRegisterRelative { at: 4 }]
    );
    assert_eq!(
        one(&[0x48, 0x8b, 0x46]),
        [Finding::NotRegisterRelative { at: 3 }]
    );
    // A field at offset 0 has no ModRM byte.
    assert_eq!(one(&[]), [Finding::NotRegisterRelative { at: 0 }]);

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
/// Since fixer r2c the store and the `lea` also fail [`check_text`] (opcode not `8B`).
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
    let not_mov = |at| {
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotRegisterRelative { at },
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
    assert_eq!(x64_one(&[0x49, 0x89, 0x86], Target::IcCell(1)), not_mov(3));
    assert_eq!(x64_one(&[0x49, 0x8d, 0x86], Target::Stub(1)), not_mov(3));
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
    // no REX byte (mov eax, [rsi + disp32]: 32 bits, base rsi).
    let rw = |at, insn, form| field(at, insn, form, RelocKind::LinkSlot);
    assert_eq!(
        check_register_loads(
            Arch::A64,
            &a64::LDR_X16_X27.to_le_bytes(),
            &[rw(4, 4, Form::A64Ldst64Imm12)]
        ),
        [Finding::FieldOutsideText { at: 4 }]
    );
    assert_eq!(
        check_register_loads(
            Arch::X64,
            &[0x8b, 0x86, 0, 0, 0, 0],
            &[rw(2, 0, Form::X64Disp32)]
        ),
        [Finding::NotUnitDataLoad { at: 2 }]
    );
    assert_eq!(
        check_register_loads(
            Arch::X64,
            &[0x49, 0x8b, 0x86, 0, 0, 0],
            &[rw(3, 0, Form::X64Disp32)]
        ),
        [Finding::FieldOutsideText { at: 3 }]
    );
}

/// x86-64 (Codex r2, fixer r2c): the operand rules decode the field's instruction forward
/// from its start ([`Reloc::insn`], which [`Assembler::x64`] sets), not backward from the
/// field. Before r2c, `check_text` took the byte before the field as ModRM (or the one before
/// a SIB byte), and the J1 check took the 3 bytes before it as REX, `8B`, ModRM: a SIB byte,
/// an opcode byte, a prefix, or bytes inside a longer instruction could look like them.
/// Each input is one instruction (the field at its end, then a NOP) through `finalize_unit`.
/// The disassembly of each form (llvm-mc) is in the comment.
#[test]
fn an_x64_field_is_decoded_from_its_instruction_start() {
    let unit = |b: &[u8], t: Target| {
        let mut a = Assembler::new(Arch::X64);
        let mut v = b.to_vec();
        v.extend_from_slice(&[0; 4]);
        a.x64(&v, b.len() as u32, Form::X64Disp32, t).unwrap();
        a.put(&x64::NOP);
        finalize_unit(&a.finish().unwrap(), &common::TestUnit).map(|_| ())
    };
    let nrr = |at| Finding::NotRegisterRelative { at };
    let nud = |at| Finding::NotUnitDataLoad { at };
    let nil = |at| Finding::NotIsolateLoad { at };
    let no = |v: Vec<Finding>| Err(FinalizeError::TextRelocations(v));
    let lk = Target::LinkSlot(0);
    let rt = Target::Runtime(0);
    type Case<'a> = (&'a str, &'a [u8], Target, Result<(), FinalizeError>);
    let cases: Vec<Case> = vec![
        // The 5 forms of the r2 verifier (Ok at 88c6eb2a), as a LinkSlot and as a Runtime.
        // mov rax, qword ptr [4*riz + d32]: the SIB byte a5 is not a ModRM byte.
        (
            "sib-a5",
            &[0x48, 0x8b, 0x04, 0xa5],
            lk,
            no(vec![nrr(4), nud(4)]),
        ),
        (
            "sib-a5",
            &[0x48, 0x8b, 0x04, 0xa5],
            rt,
            no(vec![nrr(4), nil(4)]),
        ),
        // jne rel32: the opcode byte 85 is not a ModRM byte.
        ("jne", &[0x0f, 0x85], lk, no(vec![nrr(2), nud(2)])),
        ("jne", &[0x0f, 0x85], rt, no(vec![nrr(2), nil(2)])),
        // test byte ptr [rip + d32], al.
        ("test-rip", &[0x84, 0x05], lk, no(vec![nrr(2), nud(2)])),
        ("test-rip", &[0x84, 0x05], rt, no(vec![nrr(2), nil(2)])),
        // mov eax, imm32.
        ("mov-imm32", &[0xb8], lk, no(vec![nrr(1), nud(1)])),
        ("mov-imm32", &[0xb8], rt, no(vec![nrr(1), nil(1)])),
        // jbe rel32 with a REX.B byte (41 0f 86 looks like REX, opcode, ModRM rm = r14).
        ("jbe-rex", &[0x41, 0x0f, 0x86], lk, no(vec![nrr(3), nud(3)])),
        ("jbe-rex", &[0x41, 0x0f, 0x86], rt, no(vec![nrr(3), nil(3)])),
        // movabs rax, imm64: the field is the top half of the immediate, after 49 8b 86.
        (
            "movabs",
            &[0x48, 0xb8, 0x00, 0x49, 0x8b, 0x86],
            lk,
            no(vec![nrr(6), nud(6)]),
        ),
        // mov rax, qword ptr fs:[r14 + d32]: a segment prefix before the REX byte.
        (
            "fs",
            &[0x64, 0x49, 0x8b, 0x86],
            lk,
            no(vec![nrr(4), nud(4)]),
        ),
        // A load of the right form through the wrong register for the kind.
        // mov rax, qword ptr [r14 + d32] as a Runtime, RootIndex and DeoptExit.
        ("r14-runtime", &[0x49, 0x8b, 0x86], rt, no(vec![nil(3)])),
        (
            "r14-root",
            &[0x49, 0x8b, 0x86],
            Target::RootIndex(1),
            no(vec![nil(3)]),
        ),
        // mov eax, dword ptr [r15 + d32]: 32 bits.
        (
            "r15-32bit-deopt",
            &[0x41, 0x8b, 0x87],
            Target::DeoptExit(2),
            no(vec![nil(3)]),
        ),
        // mov rax, qword ptr [rsp + d32]: register-relative (check_text passes it), base rsp.
        ("rsp-sib", &[0x48, 0x8b, 0x84, 0x24], lk, no(vec![nud(4)])),
        // The forms that stay units.
        // mov rax, qword ptr [r14 + d32]; the same with a SIB byte (base r14, no index);
        // mov r8, qword ptr [r14 + d32].
        ("r14", &[0x49, 0x8b, 0x86], lk, Ok(())),
        ("r14-sib", &[0x49, 0x8b, 0x84, 0x26], lk, Ok(())),
        ("r14-r8", &[0x4d, 0x8b, 0x86], Target::IcCell(1), Ok(())),
        // mov rax, qword ptr [r15 + d32]; the same with a SIB byte (base r15, no index).
        ("r15", &[0x49, 0x8b, 0x87], rt, Ok(())),
        (
            "r15-sib",
            &[0x49, 0x8b, 0x84, 0x27],
            Target::RootIndex(1),
            Ok(()),
        ),
        (
            "r15-deopt",
            &[0x49, 0x8b, 0x87],
            Target::DeoptExit(2),
            Ok(()),
        ),
    ];
    let mut bad = Vec::new();
    for (name, b, t, want) in cases {
        let got = unit(b, t);
        if got != want {
            bad.push(format!("{name} {t:?}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    // check_text alone takes mov rax, [rsp + d32] (SIB base rsp): any base register is
    // register-relative; only the kind rules want r14 or r15.
    let mut t = vec![0x48, 0x8b, 0x84, 0x24, 0, 0, 0, 0];
    assert_eq!(
        check_text(
            Arch::X64,
            &t,
            &[field(4, 0, Form::X64Disp32, RelocKind::LinkSlot)]
        ),
        []
    );
    // mov rax, qword ptr [r14 + r12 + d32] (REX.X: SIB index r12) is refused by the kind rule.
    t[0] = 0x4b;
    t[3] = 0x26;
    assert_eq!(
        check_register_loads(
            Arch::X64,
            &t,
            &[field(4, 0, Form::X64Disp32, RelocKind::Stub)]
        ),
        [Finding::NotUnitDataLoad { at: 4 }]
    );
}

/// A record whose instruction start is after its field (`insn > at`) is refused before any
/// field is written (fixer r2c), and the checks alone give a finding for such a field. A
/// record from [`Reloc::new`] (`insn = at`) is decoded from the field itself, so an x86-64
/// disp32 field recorded that way is refused.
#[test]
fn a_record_whose_instruction_starts_after_its_field_is_refused() {
    let mut a = Assembler::new(Arch::X64);
    let (m14, f14) = x64::MOV_RAX_R14;
    a.x64(&m14, f14, Form::X64Disp32, Target::LinkSlot(0))
        .unwrap();
    a.put(&x64::NOP);
    let mut d = a.finish().unwrap();
    assert_eq!((d.relocs[0].insn, d.relocs[0].at), (0, 3));
    assert!(finalize_unit(&d, &common::TestUnit).is_ok());
    d.relocs[0].insn = 4;
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::InsnAfterField { at: 3, insn: 4 })
    );
    d.relocs[0] = Reloc::new(3, Form::X64Disp32, Target::LinkSlot(0));
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotRegisterRelative { at: 3 },
            Finding::NotUnitDataLoad { at: 3 }
        ]))
    );
    let after = [field(3, 4, Form::X64Disp32, RelocKind::LinkSlot)];
    assert_eq!(
        check_text(Arch::X64, &d.text, &after),
        [Finding::NotRegisterRelative { at: 3 }]
    );
    assert_eq!(
        check_register_loads(Arch::X64, &d.text, &after),
        [Finding::NotUnitDataLoad { at: 3 }]
    );
}

/// `check_text` is public, and a check of a unit read back gives it fields from outside: a
/// field past the end of the text is a finding, not a panic (Claude JIT reviewer J2). On the
/// old code the first input panicked (a slice index) and the second overflowed `at + 4`.
#[test]
fn check_text_gives_a_finding_for_a_field_outside_the_text() {
    let one = |arch, len: usize, at: u32, form| {
        check_text(
            arch,
            &vec![0; len],
            &[field(at, at, form, RelocKind::LinkSlot)],
        )
    };
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
    let b = |at| field(at, at, Form::A64Branch26, RelocKind::Local);
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
