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
    // Since M8-P2 the decode of the whole text (UNIT-X64-DECODE) also finds the RIP target
    // 7 + 0x50 = 87 past the 8-byte text, the absolute operand, and, after the disp8 load
    // (48 8b 46 50) and `add [rax], al` (00 00), the bytes 00 90 that need a disp32 past the
    // end.
    assert_eq!(
        one(&[0x48, 0x8b, 0x05]),
        [
            Finding::NotRegisterRelative { at: 3 },
            Finding::OutsideFunction { at: 3, target: 87 }
        ]
    );
    assert_eq!(
        one(&[0x48, 0x8b, 0x04, 0x25]),
        [
            Finding::NotRegisterRelative { at: 4 },
            Finding::AbsoluteAddress { at: 0 }
        ]
    );
    assert_eq!(
        one(&[0x48, 0x8b, 0x46]),
        [
            Finding::NotRegisterRelative { at: 3 },
            Finding::Undecodable { at: 6 }
        ]
    );
    // A field at offset 0 has no ModRM byte. The text is `push rax` (50), `add [rax], al`
    // (00 00), then 00 90 with no room for its disp32.
    assert_eq!(
        one(&[]),
        [
            Finding::NotRegisterRelative { at: 0 },
            Finding::Undecodable { at: 3 }
        ]
    );

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
            Finding::OutsideFunction { at: 3, target: 87 },
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
    // Runtime through x28 is a unit (the isolate rows: `an_a64_isolate_field_needs_a_load_through_x28`).
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
    // Since M8-P2 the decode of the whole text (UNIT-X64-DECODE) adds the text findings: an
    // absolute operand, and a pc-relative target past the end (the field holds the LinkSlot
    // word 8 * 10 = 0x50 or the Runtime slot 0x800).
    let abs = |at| Finding::AbsoluteAddress { at };
    let out = |at, target| Finding::OutsideFunction { at, target };
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
            no(vec![nrr(4), abs(0), nud(4)]),
        ),
        (
            "sib-a5",
            &[0x48, 0x8b, 0x04, 0xa5],
            rt,
            no(vec![nrr(4), abs(0), nil(4)]),
        ),
        // jne rel32: the opcode byte 85 is not a ModRM byte. The target is 6 + the field.
        (
            "jne",
            &[0x0f, 0x85],
            lk,
            no(vec![nrr(2), out(2, 86), nud(2)]),
        ),
        (
            "jne",
            &[0x0f, 0x85],
            rt,
            no(vec![nrr(2), out(2, 2054), nil(2)]),
        ),
        // test byte ptr [rip + d32], al.
        (
            "test-rip",
            &[0x84, 0x05],
            lk,
            no(vec![nrr(2), out(2, 86), nud(2)]),
        ),
        (
            "test-rip",
            &[0x84, 0x05],
            rt,
            no(vec![nrr(2), out(2, 2054), nil(2)]),
        ),
        // mov eax, imm32.
        ("mov-imm32", &[0xb8], lk, no(vec![nrr(1), nud(1)])),
        ("mov-imm32", &[0xb8], rt, no(vec![nrr(1), nil(1)])),
        // jbe rel32 with a REX.B byte (41 0f 86 looks like REX, opcode, ModRM rm = r14).
        (
            "jbe-rex",
            &[0x41, 0x0f, 0x86],
            lk,
            no(vec![nrr(3), out(3, 87), nud(3)]),
        ),
        (
            "jbe-rex",
            &[0x41, 0x0f, 0x86],
            rt,
            no(vec![nrr(3), out(3, 2055), nil(3)]),
        ),
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
    // Since M8-P2 the decode of the whole text also says that offsets 3 and 4 (inside the
    // 7-byte load) are not instruction starts.
    d.relocs[0] = Reloc::new(3, Form::X64Disp32, Target::LinkSlot(0));
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotRegisterRelative { at: 3 },
            Finding::NotInstructionStart { at: 3 },
            Finding::NotUnitDataLoad { at: 3 }
        ]))
    );
    let after = [field(3, 4, Form::X64Disp32, RelocKind::LinkSlot)];
    assert_eq!(
        check_text(Arch::X64, &d.text, &after),
        [
            Finding::NotRegisterRelative { at: 3 },
            Finding::NotInstructionStart { at: 3 }
        ]
    );
    assert_eq!(
        check_register_loads(Arch::X64, &d.text, &after),
        [Finding::NotUnitDataLoad { at: 3 }]
    );
}

/// x86-64 (Codex r3 finding 1, fixer r2d): [`Assembler::x64`] records its chunk start as the
/// instruction start only at a known instruction boundary: the text start, or the end of a
/// chunk that is one whole known instruction. Otherwise the record has `insn = at`, and the
/// unit check refuses the field. On the old code `x64` always took its chunk start, so bytes
/// from `put` (a prefix, a REX byte, the start of another instruction) or the tail of an
/// earlier `x64` chunk became part of the load, and each refused input here gave a unit.
/// The disassembly of each output (llvm-mc) is in the comment.
#[test]
fn an_x64_unit_load_needs_a_known_instruction_boundary() {
    let (m14, f14) = x64::MOV_RAX_R14;
    let (m15, f15) = x64::MOV_RAX_R15;
    // `pre` with `put`, then the load with `x64`, then a NOP. Gives the record's insn and the
    // result; a unit must also pass `zero_text_relocations`.
    let after = |pre: &[u8], load: &[u8], field: u32, t: Target| {
        let mut a = Assembler::new(Arch::X64);
        a.put(pre);
        a.x64(load, field, Form::X64Disp32, t).unwrap();
        a.put(&x64::NOP);
        let d = a.finish().unwrap();
        let got = finalize_unit(&d, &common::TestUnit).map(|u| {
            assert_eq!(zero_text_relocations(&u), Ok(()));
        });
        (d.relocs[0].insn, got)
    };
    let no = |v: Vec<Finding>| Err(FinalizeError::TextRelocations(v));
    let nrr = |at| Finding::NotRegisterRelative { at };
    let nud = |at| Finding::NotUnitDataLoad { at };
    let nil = |at| Finding::NotIsolateLoad { at };
    // The 12 `put` chunks of the Codex r3 verifier. With the load after them, llvm-mc gives:
    // 64 / 65: mov rax, qword ptr fs:/gs:[r14 + 80]; 67: mov rax, qword ptr [r14d + 80];
    // 66, f3, 48: the prefix is ignored; f0: lock (#UD); 0f: cmovns ecx, ... (no load);
    // 48 b8 00 / 48 b8: movabs rax, imm64 (the field is in the immediate); b8: mov eax, imm32;
    // 80: or byte ptr [rcx - 117], -122.
    // Since M8-P2 the decode of the whole final text (UNIT-X64-DECODE) adds its findings
    // between the field rules and the register rule: the field's start `at` is not an
    // instruction start (nis), or the decode stops (und) at a REX REX pair (48 49, offset 0)
    // or at 00 90 (offset 7), which needs a disp32 past the end. After 80 the record's start
    // is an instruction start (80 49 8b 86 is `or byte ptr [rcx - 117], -122`, then
    // `push rax`), so only the field rules and the decode stop find it; as a Runtime the
    // field bytes 00 08 00 00 decode as two `add`s and the decode does not stop.
    let nis = |at| Finding::NotInstructionStart { at };
    let und = |at| Finding::Undecodable { at };
    let pres: [(&[u8], &[Finding], &[Finding]); 12] = [
        (&[0x64], &[nis(4)], &[nis(4)]),
        (&[0x65], &[nis(4)], &[nis(4)]),
        (&[0x67], &[nis(4)], &[nis(4)]),
        (&[0x66], &[nis(4)], &[nis(4)]),
        (&[0xf0], &[nis(4)], &[nis(4)]),
        (&[0xf3], &[nis(4)], &[nis(4)]),
        (&[0x48], &[und(0)], &[und(0)]),
        (&[0x0f], &[nis(4), und(7)], &[nis(4), und(7)]),
        (&[0x48, 0xb8, 0x00], &[nis(6)], &[nis(6)]),
        (&[0x48, 0xb8], &[nis(5)], &[nis(5)]),
        (&[0xb8], &[nis(4), und(7)], &[nis(4), und(7)]),
        (&[0x80], &[und(7)], &[]),
    ];
    for (pre, ls, rt) in pres {
        let at = pre.len() as u32 + 3;
        let want = |scan: &[Finding], reg: Finding| {
            let mut v = vec![nrr(at)];
            v.extend_from_slice(scan);
            v.push(reg);
            (at, no(v))
        };
        assert_eq!(
            after(pre, &m14, f14, Target::LinkSlot(0)),
            want(ls, nud(at)),
            "{pre:02x?} LinkSlot"
        );
        // Through r15 as a Runtime: the old code passed it too (mov rax, fs:[r15 + 2048]).
        assert_eq!(
            after(pre, &m15, f15, Target::Runtime(0)),
            want(rt, nil(at)),
            "{pre:02x?} Runtime"
        );
    }
    // A `put` of two NOPs: before M8-P2 the list knew only one-instruction chunks, so the
    // boundary was not known and the load was refused ((5, [nrr(5), nud(5)])). The length
    // decoder reads the chunk as two whole instructions (MC-w32-2), so the load keeps its
    // start and the unit passes.
    assert_eq!(
        after(&[0x90, 0x90], &m14, f14, Target::LinkSlot(0)),
        (2, Ok(()))
    );

    // An `x64` chunk with a byte after its field (64), then a load: the second load is
    // mov rax, qword ptr fs:[r14 + 80]. The first record keeps its start (0). The decode
    // (since M8-P2) reads 64 49 8b 86 .. as one instruction at 7, so 11 is not a start.
    let mut a = Assembler::new(Arch::X64);
    let mut fs_tail = m14.to_vec();
    fs_tail.push(0x64);
    a.x64(&fs_tail, f14, Form::X64Disp32, Target::LinkSlot(0))
        .unwrap();
    a.x64(&m14, f14, Form::X64Disp32, Target::LinkSlot(1))
        .unwrap();
    a.put(&x64::NOP);
    let d = a.finish().unwrap();
    assert_eq!((d.relocs[0].insn, d.relocs[1].insn), (0, 11));
    assert_eq!(
        finalize_unit(&d, &common::TestUnit).map(|_| ()),
        no(vec![nrr(11), nis(11), nud(11)])
    );

    // An `x64` rel32 chunk that is not a call, jmp or jcc (b8 e8 rel32), then a load:
    // mov eax, imm32 takes the bytes of the load (mov eax, 4294965992; dec dword ptr ...).
    // Since M8-P2 the decode also finds that the rel32 field at 2 is inside the immediate of
    // the `mov eax` at 0 (FieldNotOperand), and that 9 is not an instruction start (the
    // starts are 0, 5, 8, 11 and 13).
    let mut a = Assembler::new(Arch::X64);
    let top = a.new_label();
    a.bind(top).unwrap();
    a.x64(
        &[0xb8, 0xe8, 0, 0, 0, 0],
        2,
        Form::X64Rel32,
        Target::Local(top),
    )
    .unwrap();
    a.x64(&m14, f14, Form::X64Disp32, Target::LinkSlot(0))
        .unwrap();
    a.put(&x64::NOP);
    let d = a.finish().unwrap();
    assert_eq!(d.relocs[1].insn, 9);
    assert_eq!(
        finalize_unit(&d, &common::TestUnit).map(|_| ()),
        no(vec![
            nrr(9),
            Finding::FieldNotOperand { at: 2 },
            nis(9),
            nud(9)
        ])
    );

    // The forms that stay units: the load after each `put` of one whole instruction (nop,
    // ret, call reg, jmp reg, with and without REX.B) keeps its start.
    let plain: [&[u8]; 10] = [
        &[0x90],
        &[0xc3],
        &x64::CALL_RAX,
        &[0xff, 0xd7],
        &x64::JMP_RAX,
        &[0xff, 0xe7],
        &[0x41, 0xff, 0xd0],
        &[0x41, 0xff, 0xd7],
        &[0x41, 0xff, 0xe0],
        &[0x41, 0xff, 0xe7],
    ];
    for pre in plain {
        let insn = pre.len() as u32;
        assert_eq!(
            after(pre, &m14, f14, Target::LinkSlot(0)),
            (insn, Ok(())),
            "{pre:02x?}"
        );
        assert_eq!(
            after(pre, &m15, f15, Target::Runtime(0)),
            (insn, Ok(())),
            "{pre:02x?}"
        );
    }
    // je, call and jmp rel32, then a load; and two loads in a row.
    for (b, f) in [
        (&x64::JE_REL32.0[..], x64::JE_REL32.1),
        (&x64::CALL_REL32.0[..], x64::CALL_REL32.1),
        (&x64::JMP_REL32.0[..], x64::JMP_REL32.1),
        (&m14[..], f14),
    ] {
        let mut a = Assembler::new(Arch::X64);
        let top = a.new_label();
        a.bind(top).unwrap();
        if b == m14 {
            a.x64(b, f, Form::X64Disp32, Target::LinkSlot(1)).unwrap();
        } else {
            a.x64(b, f, Form::X64Rel32, Target::Local(top)).unwrap();
        }
        a.x64(&m15, f15, Form::X64Disp32, Target::Runtime(0))
            .unwrap();
        a.put(&x64::NOP);
        let d = a.finish().unwrap();
        assert_eq!(d.relocs[1].insn, b.len() as u32, "{b:02x?}");
        common::emit_unit(&d);
    }
    // The corpus keeps the start of each load (the unit check passes it: the first test).
    let d = common::x64_unit_function();
    for r in d.relocs.iter().filter(|r| r.form == Form::X64Disp32) {
        assert_eq!(r.insn + 3, r.at);
    }
}

/// AArch64 (Codex r3 finding 2, fixer r2d): a field is one instruction word, at a 4-byte
/// aligned offset with `insn = at`. Else [`check_text`] gives [`Finding::MisalignedField`]
/// (and no other rule reads the field), and the register-load check refuses it. On the old
/// code both read the word at any byte offset: the exact `ldr Xt, [x27, #imm]` test passed on
/// bytes that are parts of two other words.
#[test]
fn an_a64_field_is_one_aligned_instruction_word() {
    // eor v0.16b, v1.16b, v2.16b; mov x3, #0xca00000000000000. Bytes 3-6 (6e 03 40 f9) are
    // the word of ldr x14, [x27].
    const RAW: [u8; 8] = [0x20, 0x1c, 0x22, 0x6e, 0x03, 0x40, 0xf9, 0xd2];
    const LDR_X14_X27: u32 = 0xf940_036e;
    let rw = |at, insn| field(at, insn, Form::A64Ldst64Imm12, RelocKind::LinkSlot);
    let no = |v: Vec<Finding>| Err(FinalizeError::TextRelocations(v));
    let mis = |at| Finding::MisalignedField { at };
    let nud = |at| Finding::NotUnitDataLoad { at };
    // The checks alone, at offsets 1, 2 and 3 of the raw text.
    for at in 1..4 {
        assert_eq!(
            check_text(Arch::A64, &RAW, &[rw(at, at)]),
            [mis(at)],
            "{at}"
        );
        assert_eq!(
            check_register_loads(Arch::A64, &RAW, &[rw(at, at)]),
            [nud(at)],
            "{at}"
        );
    }
    // finalize_unit: a record made by hand at 3.
    let hand = CodeDesc {
        arch: Arch::A64,
        text: RAW.to_vec(),
        relocs: vec![Reloc::new(3, Form::A64Ldst64Imm12, Target::LinkSlot(0))],
        labels: vec![],
    };
    assert_eq!(
        finalize_unit(&hand, &common::TestUnit),
        no(vec![mis(3), nud(3)])
    );
    // The public API: put(3 bytes), a64(ldr x14, [x27]), put(1 byte).
    let mut a = Assembler::new(Arch::A64);
    a.put(&RAW[..3]);
    a.a64(LDR_X14_X27, Form::A64Ldst64Imm12, Target::LinkSlot(0))
        .unwrap();
    a.put(&RAW[7..]);
    let d = a.finish().unwrap();
    assert_eq!(d, hand);
    // Assembler::reloc at 3.
    let mut a = Assembler::new(Arch::A64);
    a.put(&RAW);
    a.reloc(3, Form::A64Ldst64Imm12, Target::LinkSlot(0))
        .unwrap();
    assert_eq!(a.finish().unwrap(), hand);
    // An isolate row: since M7-P4 (A64-ISOLATE-LOAD) also a register-load finding on AArch64.
    let mut a = Assembler::new(Arch::A64);
    a.put(&RAW);
    a.reloc(3, Form::A64Ldst64Imm12, Target::Runtime(0))
        .unwrap();
    assert_eq!(
        finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        no(vec![mis(3), Finding::NotIsolateLoad { at: 3 }])
    );
    // An aligned field whose record has insn != at: nop; ldr x14, [x27] with insn = 0.
    let mut t = a64::NOP.to_le_bytes().to_vec();
    t.extend_from_slice(&LDR_X14_X27.to_le_bytes());
    assert_eq!(check_text(Arch::A64, &t, &[rw(4, 0)]), [mis(4)]);
    assert_eq!(check_register_loads(Arch::A64, &t, &[rw(4, 0)]), [nud(4)]);
    let d = CodeDesc {
        arch: Arch::A64,
        text: t.clone(),
        relocs: vec![Reloc {
            insn: 0,
            ..Reloc::new(4, Form::A64Ldst64Imm12, Target::LinkSlot(0))
        }],
        labels: vec![],
    };
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        no(vec![mis(4), nud(4)])
    );
    // The same word with insn = at is a unit; so is every field of the corpus.
    assert_eq!(check_text(Arch::A64, &t, &[rw(4, 4)]), []);
    assert_eq!(check_register_loads(Arch::A64, &t, &[rw(4, 4)]), []);
    let mut a = Assembler::new(Arch::A64);
    a.put32(a64::NOP);
    a.a64(LDR_X14_X27, Form::A64Ldst64Imm12, Target::LinkSlot(0))
        .unwrap();
    common::emit_unit(&a.finish().unwrap());
    for d in [common::a64_unit_function(), common::a64_loop()] {
        assert!(d.relocs.iter().all(|r| r.at % 4 == 0 && r.insn == r.at));
    }
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

/// Route MASM-A64-NEWBR (M7-P2 JIT reviewer J4; M7-P4): the word scan knows the newer AArch64
/// pc-relative branches. BC.cond (FEAT_HBC) is B.cond with bit 4 set; `CB<cc>`, `CBB<cc>` and
/// `CBH<cc>` (FEAT_CMPBR) have imm9 at bits 5-13 in words of 4 bytes. The encodings are those of
/// the Arm ARM ("Conditional branch (immediate)", "Compare and branch"). Before M7-P4 the scan
/// read none of these words, so a branch out of the function was not a finding.
#[test]
fn the_a64_scan_finds_bc_cond_and_cb_cc() {
    // The same 32-byte text as `the_a64_scan_finds_planted_words`: the word at offset 16.
    let text = |w: u32| {
        let mut t = Vec::new();
        for i in 0..8 {
            t.extend_from_slice(&if i == 4 { w } else { a64::NOP }.to_le_bytes());
        }
        t
    };
    // imm9 of a CB<cc> word.
    let imm9 = |words: i32| ((words as u32) & 0x1ff) << 5;
    let cases: [(&str, u32); 6] = [
        // bc.eq: o0 = bit 4.
        ("bc.eq", BC_EQ),
        // cbgt x0, x1: sf = 1, cc = 000, Rm = 1, bits 15-14 = 00.
        ("cbgt x0, x1", 0xf400_0000 | (1 << 16)),
        // cbgt w0, w1: sf = 0.
        ("cbgt w0, w1", 0x7400_0000 | (1 << 16)),
        // cbbgt w0, w1: bit 15 = 1, H (bit 14) = 0.
        ("cbbgt w0, w1", 0x7400_8000 | (1 << 16)),
        // cbhgt w0, w1: H = 1.
        ("cbhgt w0, w1", 0x7400_c000 | (1 << 16)),
        // cbgt x0, #3: bit 24 = 1, imm6 = 3 at bits 15-20, bit 14 = 0.
        ("cbgt x0, #3", 0xf500_0000 | (3 << 15)),
    ];
    for (name, w) in cases {
        let (out, back, inside) = if w == BC_EQ {
            (w | (4 << 5), w | (0x7_fffb << 5), w | (2 << 5))
        } else {
            (w | imm9(4), w | imm9(-5), w | imm9(2))
        };
        // +16: one past the end; -20: before the start; +8: inside.
        assert_eq!(
            check_text(Arch::A64, &text(out), &[]),
            [Finding::OutsideFunction { at: 16, target: 32 }],
            "{name}"
        );
        assert_eq!(
            check_text(Arch::A64, &text(back), &[]),
            [Finding::OutsideFunction { at: 16, target: -4 }],
            "{name}"
        );
        assert_eq!(check_text(Arch::A64, &text(inside), &[]), [], "{name}");
    }
    // The imm9 range is +-1 KiB: the largest forward word offset (255) from offset 16.
    assert_eq!(
        check_text(Arch::A64, &text(0xf400_0000 | imm9(255)), &[]),
        [Finding::OutsideFunction {
            at: 16,
            target: 16 + 1020
        }]
    );
}

/// Route A64-ISOLATE-LOAD (M7-P2 fixer r2c open item 1; M7-P4): on AArch64 a Runtime, RootIndex
/// or DeoptExit field must be in `ldr Xt, [x28, #imm]` (x28 is the isolate register,
/// M2-JIT-ABI), as on x86-64 it must be in `mov r64, [r15 + disp32]`. Before M7-P4 these
/// fields were not checked on AArch64, and a load through x27 or a store became a unit.
#[test]
fn an_a64_isolate_field_needs_a_load_through_x28() {
    let a64_one = |w: u32, t: Target| {
        let mut a = Assembler::new(Arch::A64);
        a.a64(w, Form::A64Ldst64Imm12, t).unwrap();
        finalize_unit(&a.finish().unwrap(), &common::TestUnit).map(|u| u.exits().len())
    };
    let refused = |at| {
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotIsolateLoad { at },
        ]))
    };
    // ldr x16, [x27]; str x0, [x28]; ldr x0, [x27].
    assert_eq!(a64_one(a64::LDR_X16_X27, Target::Runtime(4)), refused(0));
    assert_eq!(a64_one(STR_X0_X28, Target::RootIndex(2)), refused(0));
    assert_eq!(a64_one(LDR_X0_X27, Target::DeoptExit(1)), refused(0));
    // The loads through x28 are units (a DeoptExit gives an exit).
    assert_eq!(a64_one(a64::LDR_X16_X28, Target::Runtime(4)), Ok(0));
    assert_eq!(a64_one(a64::LDR_X0_X28, Target::RootIndex(2)), Ok(0));
    assert_eq!(a64_one(a64::LDR_X16_X28, Target::DeoptExit(1)), Ok(1));
    // The check alone, on records read back: the kind picks the register.
    let one = |w: u32, kind| {
        check_register_loads(
            Arch::A64,
            &w.to_le_bytes(),
            &[field(0, 0, Form::A64Ldst64Imm12, kind)],
        )
    };
    assert_eq!(one(a64::LDR_X0_X28, RelocKind::RootIndex), []);
    assert_eq!(
        one(a64::LDR_X0_X28, RelocKind::LinkSlot),
        [Finding::NotUnitDataLoad { at: 0 }]
    );
    assert_eq!(
        one(a64::LDR_X16_X27, RelocKind::DeoptExit),
        [Finding::NotIsolateLoad { at: 0 }]
    );
    assert_eq!(one(a64::LDR_X16_X27, RelocKind::IcCell), []);
}

/// `bc.eq #0` (FEAT_HBC).
const BC_EQ: u32 = 0x5400_0010;

/// `str x0, [x28, #0]`.
const STR_X0_X28: u32 = 0xf900_0380;

/// `ldr x0, [x27, #0]`.
const LDR_X0_X27: u32 = 0xf940_0360;

/// Route UNIT-X64-DECODE (M8-P2 A2-CUT): `check_text` decodes the whole final x86-64 text from
/// offset 0 with a length decoder, so it no longer trusts the record's instruction start and
/// it checks bytes that have no record. On the old code (one instruction decoded per disp32
/// field, from the record's start) every input here gave no finding and a unit. The
/// disassembly of each text (llvm-mc) is in the comment.
#[test]
fn the_x64_decode_of_the_whole_text_finds_what_no_record_names() {
    let text = |b: &[u8]| check_text(Arch::X64, b, &[]);
    // jmp rel32 to 0x105 and jmp rel8 to -1: out of the function.
    assert_eq!(
        text(&[0xe9, 0x00, 0x01, 0x00, 0x00]),
        [Finding::OutsideFunction {
            at: 1,
            target: 0x105
        }]
    );
    assert_eq!(
        text(&[0xeb, 0xfd]),
        [Finding::OutsideFunction { at: 1, target: -1 }]
    );
    // lea rax, [rip + 0x100]; nop: a RIP-relative address past the end.
    assert_eq!(
        text(&[0x48, 0x8d, 0x05, 0x00, 0x01, 0x00, 0x00, 0x90]),
        [Finding::OutsideFunction {
            at: 3,
            target: 0x107
        }]
    );
    // jmp +1 into the middle of `mov eax, 0x90909090`: the CPU would run bytes the check
    // did not decode (90 90 90 90 ...).
    assert_eq!(
        text(&[0xeb, 0x01, 0xb8, 0x90, 0x90, 0x90, 0x90]),
        [Finding::BranchIntoInstruction { at: 1, target: 3 }]
    );
    // movabs rax, [0x1122334455667788] (moffs) and mov rax, [0x1000] (SIB, no base).
    assert_eq!(
        text(&[0x48, 0xa1, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]),
        [Finding::AbsoluteAddress { at: 0 }]
    );
    assert_eq!(
        text(&[0x90, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x10, 0x00, 0x00]),
        [Finding::AbsoluteAddress { at: 1 }]
    );
    // vzeroupper (VEX), and a load cut short: the decode stops.
    assert_eq!(
        text(&[0x90, 0xc5, 0xf8, 0x77]),
        [Finding::Undecodable { at: 1 }]
    );
    assert_eq!(
        text(&[0x90, 0x49, 0x8b, 0x86, 0x00]),
        [Finding::Undecodable { at: 1 }]
    );

    // A record made by hand with a false start: movabs rax, imm64 whose immediate holds
    // 49 8b 86 and the field. The old check decoded `mov rax, [r14 + d32]` from offset 2.
    let t = [0x48, 0xb8, 0x49, 0x8b, 0x86, 0x50, 0, 0, 0, 0x00, 0x90];
    let f = [field(5, 2, Form::X64Disp32, RelocKind::LinkSlot)];
    assert_eq!(
        check_text(Arch::X64, &t, &f),
        [Finding::NotInstructionStart { at: 5 }]
    );
    // The same load after two NOPs, with its true start: no finding.
    let t = [0x90, 0x90, 0x49, 0x8b, 0x86, 0x50, 0, 0, 0, 0x90];
    assert_eq!(check_text(Arch::X64, &t, &f), []);

    // A field write after the boundary was set: 5 NOPs, one `put` each (a known boundary on
    // the old code too), the load at 5
    // with its start, then a rel32 field over bytes 1-4 to a label at 189. The value
    // 189 - 5 = 0xb8 makes bytes 1-5 `mov eax, 0x49000000` (b8 00 00 00 49): it takes the
    // REX byte of the load, and 8b 86 .. is `mov eax, [rsi + d32]`.
    let mut a = Assembler::new(Arch::X64);
    let far = a.new_label();
    for _ in 0..5 {
        a.put(&x64::NOP);
    }
    a.x64(
        &x64::MOV_RAX_R14.0,
        x64::MOV_RAX_R14.1,
        Form::X64Disp32,
        Target::LinkSlot(0),
    )
    .unwrap();
    a.reloc(1, Form::X64Rel32, Target::Local(far)).unwrap();
    while a.offset() < 189 {
        a.put(&x64::NOP);
    }
    a.bind(far).unwrap();
    a.put(&x64::NOP);
    let d = a.finish().unwrap();
    assert_eq!(d.relocs[0].insn, 5);
    assert_eq!(
        finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::TextRelocations(vec![
            Finding::NotInstructionStart { at: 8 },
            Finding::FieldNotOperand { at: 1 },
        ]))
    );
}

/// Codex M8-P2 r1 H3: `mov eax, [eip + 0]; ret` (`67 8b 05 00 00 00 00 c3`). With the 67
/// prefix the CPU cuts the RIP-relative address to 32 bits, so the load does not read the
/// text byte at offset 7 that the old check reported as its target. The old decoder took it
/// as RIP-relative and the check gave no finding; now the decode stops there. Codex r1 H1
/// (`66 48 68 00 00 eb 7f; ret`) is a `push imm32` (REX.W wins over 66, see the decoder
/// test), so the `eb 7f` is immediate bytes on the CPU too and the text passes.
#[test]
fn the_x64_check_refuses_an_eip_relative_load() {
    let text = |b: &[u8]| check_text(Arch::X64, b, &[]);
    assert_eq!(
        text(&[0x67, 0x8b, 0x05, 0, 0, 0, 0, 0xc3]),
        [Finding::Undecodable { at: 0 }]
    );
    assert_eq!(
        text(&[0x90, 0x67, 0x48, 0x8d, 0x05, 0xf8, 0xff, 0xff, 0xff, 0xc3]),
        [Finding::Undecodable { at: 1 }]
    );
    assert_eq!(text(&[0x8b, 0x05, 0, 0, 0, 0, 0xc3]), []);
    assert_eq!(text(&[0x66, 0x48, 0x68, 0x00, 0x00, 0xeb, 0x7f, 0xc3]), []);
}
