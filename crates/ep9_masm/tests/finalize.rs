//! Both finalizers: each kind as the H34 table says, the unknown kind (the probe at the top
//! value, H34 Freeze table), the HeapConst rejection of the unit writer, and the forms that
//! a kind has not.
#![cfg(any(feature = "jit", feature = "aot"))]

mod common;

use common::{a64, x64};
use ep9_masm::{
    Arch, Assembler, CodeDesc, FieldError, FinalizeError, Form, KIND_COUNT, Reloc, RelocKind,
    Target,
};

/// A stream of one valid Local branch, then a record with the kind code `code` on a NOP.
fn with_code(arch: Arch, code: u8) -> CodeDesc {
    let mut a = Assembler::new(arch);
    let l = a.new_label();
    match arch {
        Arch::A64 => {
            a.a64(a64::B, Form::A64Branch26, Target::Local(l)).unwrap();
            a.bind(l).unwrap();
            a.put32(a64::NOP);
            a.put32(a64::NOP);
        }
        Arch::X64 => {
            a.x64(
                &x64::JMP_REL32.0,
                x64::JMP_REL32.1,
                Form::X64Rel32,
                Target::Local(l),
            )
            .unwrap();
            a.bind(l).unwrap();
            a.put(&[0x90; 8]);
        }
    }
    let mut d = a.finish().unwrap();
    let (at, form) = match arch {
        Arch::A64 => (4, Form::A64Branch26),
        Arch::X64 => (5, Form::Abs64),
    };
    d.relocs.push(Reloc {
        at,
        insn: at,
        form,
        code,
        index: 0,
    });
    d
}

/// The unknown codes the tests feed: the top value, the one after it, and the last code.
const UNKNOWN: [u8; 3] = [KIND_COUNT as u8, KIND_COUNT as u8 + 1, u8::MAX];

#[cfg(feature = "jit")]
#[test]
fn finalize_jit_refuses_an_unknown_kind() {
    for arch in [Arch::A64, Arch::X64] {
        for code in UNKNOWN {
            let d = with_code(arch, code);
            let at = d.relocs[1].at;
            assert_eq!(
                ep9_masm::finalize_jit(&d, &common::TestJit),
                Err(FinalizeError::UnknownKind { at, code }),
                "{arch:?} code {code}"
            );
        }
    }
}

#[cfg(feature = "aot")]
#[test]
fn finalize_unit_refuses_an_unknown_kind() {
    for arch in [Arch::A64, Arch::X64] {
        for code in UNKNOWN {
            let d = with_code(arch, code);
            let at = d.relocs[1].at;
            assert_eq!(
                ep9_masm::finalize_unit(&d, &common::TestUnit),
                Err(FinalizeError::UnknownKind { at, code }),
                "{arch:?} code {code}"
            );
        }
    }
}

/// Each known code goes past the kind check in both finalizers: the error, if any, is not
/// UnknownKind. So the refusal above is the top value's, not every value's.
#[test]
fn every_known_code_passes_the_kind_check() {
    for arch in [Arch::A64, Arch::X64] {
        for k in RelocKind::ALL {
            let d = with_code(arch, k.code());
            #[cfg(feature = "jit")]
            assert!(
                !matches!(
                    ep9_masm::finalize_jit(&d, &common::TestJit),
                    Err(FinalizeError::UnknownKind { .. })
                ),
                "{arch:?} {k:?}"
            );
            #[cfg(feature = "aot")]
            assert!(
                !matches!(
                    ep9_masm::finalize_unit(&d, &common::TestUnit),
                    Err(FinalizeError::UnknownKind { .. })
                ),
                "{arch:?} {k:?}"
            );
        }
    }
}

#[cfg(feature = "aot")]
#[test]
fn the_unit_writer_rejects_heap_const_in_every_form() {
    let cases: [(Arch, u32, &[u8], u32, Form); 4] = [
        (
            Arch::A64,
            0,
            &a64::LDR_X0_LIT.to_le_bytes(),
            0,
            Form::A64Branch19,
        ),
        (
            Arch::A64,
            0,
            &a64::LDR_X16_X27.to_le_bytes(),
            0,
            Form::A64Ldst64Imm12,
        ),
        (
            Arch::X64,
            0,
            &x64::MOVABS_RAX.0,
            x64::MOVABS_RAX.1,
            Form::Abs64,
        ),
        (
            Arch::X64,
            0,
            &x64::MOV_RAX_R14.0,
            x64::MOV_RAX_R14.1,
            Form::X64Disp32,
        ),
    ];
    for (arch, _, bytes, field, form) in cases {
        let mut a = Assembler::new(arch);
        a.put(&[0; 8]);
        let at = a.offset() + field;
        a.put(bytes);
        a.reloc(at, form, Target::HeapConst(4)).unwrap();
        let d = a.finish().unwrap();
        assert_eq!(
            ep9_masm::finalize_unit(&d, &common::TestUnit),
            Err(FinalizeError::HeapConstInUnit { at, index: 4 }),
            "{arch:?} {form:?}"
        );
    }
}

#[cfg(feature = "aot")]
#[test]
fn unit_kinds_finalize_as_h34_says() {
    use ep9_masm::RwUse;
    for d in [common::a64_unit_function(), common::x64_unit_function()] {
        let u = common::emit_unit(&d);
        let read = |i: usize| {
            let r = d.relocs[i];
            r.form.read(u.text(), r.at as usize)
        };
        let target = |i: usize| {
            let r = d.relocs[i];
            (r.at + r.form.pc_base()) as i64 + read(i)
        };
        let label = |l: u32| d.labels[l as usize].unwrap() as i64;
        let by_kind = |k: RelocKind| {
            (0..d.relocs.len())
                .filter(|&i| d.relocs[i].kind() == Some(k))
                .collect::<Vec<_>>()
        };
        for i in by_kind(RelocKind::Local) {
            assert_eq!(
                target(i),
                label(d.relocs[i].index),
                "{:?} Local {i}",
                d.arch
            );
        }
        let one = |k: RelocKind| by_kind(k)[0];
        assert_eq!(read(one(RelocKind::LinkSlot)), 8 * (10 + 2));
        assert_eq!(read(one(RelocKind::IcCell)), 8 * (100 + 1));
        assert_eq!(read(one(RelocKind::Stub)), 8 * (200 + 3));
        assert_eq!(read(one(RelocKind::Runtime)), 0x800 + 8 * 4);
        assert_eq!(read(one(RelocKind::RootIndex)), 0x400 + 8 * 5);
        assert_eq!(read(one(RelocKind::DeoptExit)), 0x7f8);
        let at = |k: RelocKind| d.relocs[one(k)].at;
        assert_eq!(
            u.rw(),
            [
                RwUse {
                    at: at(RelocKind::LinkSlot),
                    kind: RelocKind::LinkSlot,
                    index: 2,
                    word: 12
                },
                RwUse {
                    at: at(RelocKind::IcCell),
                    kind: RelocKind::IcCell,
                    index: 1,
                    word: 101
                },
                RwUse {
                    at: at(RelocKind::Stub),
                    kind: RelocKind::Stub,
                    index: 3,
                    word: 203
                },
            ]
        );
        assert_eq!(
            u.exits(),
            [ep9_masm::Exit {
                at: at(RelocKind::DeoptExit),
                id: 9
            }]
        );
        // Only the fields changed.
        let mut fields = vec![false; d.text.len()];
        for r in &d.relocs {
            fields[r.at as usize..r.at as usize + r.form.size()].fill(true);
        }
        for (i, (a, b)) in d.text.iter().zip(u.text()).enumerate() {
            if !fields[i] {
                assert_eq!(a, b, "{:?} byte {i}", d.arch);
            }
        }
        // The same request gives the same bytes (doc 13 P-R4).
        assert_eq!(common::emit_unit(&d), u);
    }
}

#[cfg(feature = "aot")]
#[test]
fn unit_text_has_no_absolute_form_and_no_in_unit_stub_call_yet() {
    // Every kind but HeapConst refuses Abs64 in unit text.
    for k in RelocKind::ALL {
        let mut a = Assembler::new(Arch::X64);
        let l = a.new_label();
        a.bind(l).unwrap();
        a.x64(
            &x64::MOVABS_RAX.0,
            x64::MOVABS_RAX.1,
            Form::Abs64,
            Target::new(k, 0),
        )
        .unwrap();
        let got = ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit);
        let want = match k {
            RelocKind::HeapConst => FinalizeError::HeapConstInUnit { at: 2, index: 0 },
            _ => FinalizeError::FormNotAllowed {
                at: 2,
                kind: k,
                form: Form::Abs64,
            },
        };
        assert_eq!(got, Err(want), "{k:?}");
    }
    // A pc-relative Stub call (the stub inside the unit, H34-Q4) is not built.
    let mut a = Assembler::new(Arch::A64);
    a.a64(a64::BL, Form::A64Branch26, Target::Stub(1)).unwrap();
    assert_eq!(
        ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::FormNotBuilt {
            at: 0,
            kind: RelocKind::Stub,
            form: Form::A64Branch26
        })
    );
    // A register-relative form for Local is refused.
    let mut a = Assembler::new(Arch::A64);
    let l = a.new_label();
    a.a64(a64::LDR_X16_X27, Form::A64Ldst64Imm12, Target::Local(l))
        .unwrap();
    a.bind(l).unwrap();
    a.put32(a64::NOP);
    assert_eq!(
        ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::FormNotAllowed {
            at: 0,
            kind: RelocKind::Local,
            form: Form::A64Ldst64Imm12
        })
    );
}

#[cfg(feature = "aot")]
#[test]
fn unit_refusals_name_the_field() {
    // No value for the operand.
    let mut a = Assembler::new(Arch::A64);
    a.a64(a64::LDR_X16_X27, Form::A64Ldst64Imm12, Target::LinkSlot(77))
        .unwrap();
    assert_eq!(
        ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::Unresolved {
            at: 0,
            kind: RelocKind::LinkSlot,
            index: 77
        })
    );
    // A RW word past the reach of the 12-bit load form.
    let mut a = Assembler::new(Arch::A64);
    a.a64(a64::LDR_X16_X27, Form::A64Ldst64Imm12, Target::IcCell(4000))
        .unwrap();
    assert_eq!(
        ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::Field {
            at: 0,
            kind: RelocKind::IcCell,
            error: FieldError::Range
        })
    );
    // The word at the field is not of the form.
    let mut a = Assembler::new(Arch::A64);
    a.a64(a64::NOP, Form::A64Ldst64Imm12, Target::RootIndex(1))
        .unwrap();
    assert_eq!(
        ep9_masm::finalize_unit(&a.finish().unwrap(), &common::TestUnit),
        Err(FinalizeError::Field {
            at: 0,
            kind: RelocKind::RootIndex,
            error: FieldError::Opcode(a64::NOP)
        })
    );
    // Two records on one field.
    let mut d = common::a64_loop();
    let dup = d.relocs[1];
    d.relocs.push(dup);
    assert_eq!(
        ep9_masm::finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::FieldOverlap { at: dup.at })
    );
    // A field out of the text, and a form of the other arch, in a stream read back as data.
    let mut d = common::a64_loop();
    d.relocs[0].at = d.text.len() as u32;
    assert_eq!(
        ep9_masm::finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::FieldOutOfText {
            at: d.text.len() as u32
        })
    );
    let mut d = common::a64_loop();
    d.relocs[0].form = Form::X64Rel32;
    assert_eq!(
        ep9_masm::finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::WrongArch { at: d.relocs[0].at })
    );
}

#[cfg(feature = "jit")]
#[test]
fn jit_kinds_finalize_as_h34_says() {
    use a64::*;
    use common::CODE;
    let mut a = Assembler::new(Arch::A64);
    let lit = a.new_label();
    let end = a.new_label();
    a.a64(CBZ_X0, Form::A64Branch19, Target::Local(end))
        .unwrap(); // 0
    a.a64(BL, Form::A64Branch26, Target::Stub(2)).unwrap(); // 4
    a.a64(BL, Form::A64Branch26, Target::Runtime(3)).unwrap(); // 8
    a.a64(LDR_X16_X28, Form::A64Ldst64Imm12, Target::Runtime(3))
        .unwrap(); // 12
    a.a64(LDR_X0_X28, Form::A64Ldst64Imm12, Target::RootIndex(6))
        .unwrap(); // 16
    a.a64(LDR_X16_LIT, Form::A64Branch19, Target::Local(lit))
        .unwrap(); // 20
    a.a64(BL, Form::A64Branch26, Target::DeoptExit(5)).unwrap(); // 24
    a.bind(end).unwrap();
    a.put32(BR_X16); // 28
    a.bind(lit).unwrap();
    for (i, t) in [
        Target::LinkSlot(1),
        Target::IcCell(2),
        Target::HeapConst(3),
        Target::Runtime(4),
        Target::DeoptExit(8),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 32 + 8 * i as u32;
        a.put(&[0; 8]);
        a.reloc(at, Form::Abs64, t).unwrap();
    }
    let d = a.finish().unwrap();
    let j = ep9_masm::finalize_jit(&d, &common::TestJit).unwrap();
    let v = |at: u32, f: Form| f.read(&j.text, at as usize);
    assert_eq!(v(0, Form::A64Branch19), 28);
    assert_eq!(v(4, Form::A64Branch26), (0x1_0010_0200 - (CODE + 4)) as i64);
    assert_eq!(v(8, Form::A64Branch26), (0x1_0020_0300 - (CODE + 8)) as i64);
    assert_eq!(v(12, Form::A64Ldst64Imm12), 0x800 + 8 * 3);
    assert_eq!(v(16, Form::A64Ldst64Imm12), 0x400 + 8 * 6);
    assert_eq!(v(20, Form::A64Branch19), 32 - 20);
    assert_eq!(
        v(24, Form::A64Branch26),
        (0x1_0030_0000 - (CODE + 24)) as i64
    );
    assert_eq!(v(32, Form::Abs64), 0xaaaa_0001);
    assert_eq!(v(40, Form::Abs64), 0x2_0000_0010);
    assert_eq!(v(48, Form::Abs64), 0x3_0000_0030);
    assert_eq!(v(56, Form::Abs64), 0x1_0020_0400);
    assert_eq!(v(64, Form::Abs64), 0x1_0030_0000);
    assert_eq!(
        j.exits,
        [
            ep9_masm::Exit { at: 24, id: 5 },
            ep9_masm::Exit { at: 64, id: 8 }
        ]
    );
    assert_eq!(j.embedded, [(48, 3)]);

    // x86-64: rel32 counts from the end of the field.
    let mut a = Assembler::new(Arch::X64);
    a.x64(
        &x64::CALL_REL32.0,
        x64::CALL_REL32.1,
        Form::X64Rel32,
        Target::Stub(1),
    )
    .unwrap();
    a.x64(
        &x64::MOV_RAX_R15.0,
        x64::MOV_RAX_R15.1,
        Form::X64Disp32,
        Target::RootIndex(2),
    )
    .unwrap();
    a.x64(
        &x64::MOVABS_RAX.0,
        x64::MOVABS_RAX.1,
        Form::Abs64,
        Target::HeapConst(0),
    )
    .unwrap();
    let j = ep9_masm::finalize_jit(&a.finish().unwrap(), &common::TestJit).unwrap();
    assert_eq!(
        Form::X64Rel32.read(&j.text, 1),
        (0x1_0010_0100 - (CODE + 5)) as i64
    );
    assert_eq!(Form::X64Disp32.read(&j.text, 8), 0x400 + 16);
    assert_eq!(Form::Abs64.read(&j.text, 14), 0x3_0000_0000);
    assert_eq!(j.embedded, [(14, 0)]);
}

#[cfg(feature = "jit")]
#[test]
fn jit_refuses_forms_the_table_has_not_and_calls_out_of_reach() {
    let refuse = |t: Target, form: Form, w: u32| {
        let mut a = Assembler::new(Arch::A64);
        a.put32(a64::NOP);
        a.put32(w);
        a.put32(0);
        a.reloc(4, form, t).unwrap();
        ep9_masm::finalize_jit(&a.finish().unwrap(), &common::TestJit)
    };
    let not = |t: Target, form| {
        Err(FinalizeError::FormNotAllowed {
            at: 4,
            kind: t.kind(),
            form,
        })
    };
    for (t, form, w) in [
        (Target::Stub(1), Form::A64Branch19, a64::CBZ_X0),
        (Target::Stub(1), Form::Abs64, 0),
        (Target::LinkSlot(1), Form::A64Ldst64Imm12, a64::LDR_X16_X27),
        (Target::IcCell(1), Form::A64Branch26, a64::BL),
        (Target::RootIndex(1), Form::Abs64, 0),
        (Target::DeoptExit(1), Form::A64Ldst64Imm12, a64::LDR_X16_X28),
        (Target::HeapConst(1), Form::A64Ldst64Imm12, a64::LDR_X16_X27),
        (Target::Runtime(1), Form::A64Branch14, a64::TBZ_X0_0),
    ] {
        assert_eq!(refuse(t, form, w), not(t, form), "{t:?} {form:?}");
    }
    // A stub 128 MiB away is out of the reach of BL (the veneer is H34-Q3, M5).
    struct Far;
    impl ep9_masm::JitEnv for Far {
        fn code_address(&self) -> u64 {
            0
        }
        fn stub(&self, _: u32) -> Option<u64> {
            Some(4 + (128 << 20))
        }
        fn runtime(&self, _: u32) -> Option<u64> {
            None
        }
        fn runtime_slot(&self, _: u32) -> Option<i64> {
            None
        }
        fn link(&self, _: u32) -> Option<u64> {
            None
        }
        fn ic_cell(&self, _: u32) -> Option<u64> {
            None
        }
        fn root_slot(&self, _: u32) -> Option<i64> {
            None
        }
        fn deopt_entry(&self) -> u64 {
            0
        }
        fn heap_const(&self, _: u32) -> Option<u64> {
            None
        }
    }
    let mut a = Assembler::new(Arch::A64);
    a.put32(a64::NOP);
    a.a64(a64::BL, Form::A64Branch26, Target::Stub(1)).unwrap();
    assert_eq!(
        ep9_masm::finalize_jit(&a.finish().unwrap(), &Far),
        Err(FinalizeError::Field {
            at: 4,
            kind: RelocKind::Stub,
            error: FieldError::Range
        })
    );
    // No value for the operand.
    let mut a = Assembler::new(Arch::A64);
    a.a64(a64::BL, Form::A64Branch26, Target::Runtime(77))
        .unwrap();
    assert_eq!(
        ep9_masm::finalize_jit(&a.finish().unwrap(), &common::TestJit),
        Err(FinalizeError::Unresolved {
            at: 0,
            kind: RelocKind::Runtime,
            index: 77
        })
    );
}

/// The JIT keeps HeapConst; the same stream as a unit is refused: HeapConst appears only in
/// JIT code (doc 13 P-R2).
#[cfg(all(feature = "jit", feature = "aot"))]
#[test]
fn heap_const_is_jit_only() {
    let mut a = Assembler::new(Arch::X64);
    a.x64(
        &x64::MOVABS_RAX.0,
        x64::MOVABS_RAX.1,
        Form::Abs64,
        Target::HeapConst(2),
    )
    .unwrap();
    let d = a.finish().unwrap();
    let j = ep9_masm::finalize_jit(&d, &common::TestJit).unwrap();
    assert_eq!(j.embedded, [(2, 2)]);
    assert_eq!(
        ep9_masm::finalize_unit(&d, &common::TestUnit),
        Err(FinalizeError::HeapConstInUnit { at: 2, index: 2 })
    );
}
