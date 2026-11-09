//! Hand-written stub code and test environments (plan v12 rule 71: no tier code; the tests
//! write the instructions). Every unit that a test emits goes through [`emit_unit`], which
//! checks it for zero text relocations.
#![allow(dead_code)]

use ep9_masm::{Arch, Assembler, CodeDesc, Form, Target};

/// AArch64 words.
pub mod a64 {
    pub const B: u32 = 0x1400_0000;
    pub const BL: u32 = 0x9400_0000;
    pub const B_EQ: u32 = 0x5400_0000;
    pub const CBZ_X0: u32 = 0xb400_0000;
    pub const TBZ_X0_0: u32 = 0x3600_0000;
    pub const ADR_X0: u32 = 0x1000_0000;
    pub const ADRP_X0: u32 = 0x9000_0000;
    pub const LDR_X0_LIT: u32 = 0x5800_0000;
    pub const LDR_X16_LIT: u32 = 0x5800_0010;
    /// `ldr x16, [x27, #0]`: x27 is the unit-data register (H34-Q1, H35-Q9).
    pub const LDR_X16_X27: u32 = 0xf940_0370;
    /// `ldr x16, [x28, #0]`: x28 is the isolate register of these tests.
    pub const LDR_X16_X28: u32 = 0xf940_0390;
    /// `ldr x0, [x28, #0]`.
    pub const LDR_X0_X28: u32 = 0xf940_0380;
    pub const BLR_X16: u32 = 0xd63f_0200;
    pub const BR_X16: u32 = 0xd61f_0200;
    pub const MOVZ_X0_1: u32 = 0xd280_0020;
    pub const NOP: u32 = 0xd503_201f;
}

/// x86-64 instructions and the offset of their field.
pub mod x64 {
    /// `mov rax, [r14 + disp32]`: r14 is the unit-data register.
    pub const MOV_RAX_R14: ([u8; 7], u32) = ([0x49, 0x8b, 0x86, 0, 0, 0, 0], 3);
    /// `mov rax, [r15 + disp32]`: r15 is the isolate register of these tests.
    pub const MOV_RAX_R15: ([u8; 7], u32) = ([0x49, 0x8b, 0x87, 0, 0, 0, 0], 3);
    pub const CALL_RAX: [u8; 2] = [0xff, 0xd0];
    pub const JMP_RAX: [u8; 2] = [0xff, 0xe0];
    pub const CALL_REL32: ([u8; 5], u32) = ([0xe8, 0, 0, 0, 0], 1);
    pub const JMP_REL32: ([u8; 5], u32) = ([0xe9, 0, 0, 0, 0], 1);
    pub const JE_REL32: ([u8; 6], u32) = ([0x0f, 0x84, 0, 0, 0, 0], 2);
    /// `movabs rax, imm64`.
    pub const MOVABS_RAX: ([u8; 10], u32) = ([0x48, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0], 2);
    pub const NOP: [u8; 1] = [0x90];
}

/// One AArch64 function that uses each kind except HeapConst in its unit form:
/// branches inside the function (Local), loads of RW cells through x27 (LinkSlot, IcCell,
/// the stub's link cell), loads through the isolate register x28 (Runtime, RootIndex,
/// DeoptExit), calls through x16.
pub fn a64_unit_function() -> CodeDesc {
    use a64::*;
    let mut a = Assembler::new(Arch::A64);
    let done = a.new_label();
    let slow = a.new_label();
    a.a64(CBZ_X0, Form::A64Branch19, Target::Local(slow))
        .unwrap();
    a.a64(TBZ_X0_0, Form::A64Branch14, Target::Local(slow))
        .unwrap();
    a.a64(B_EQ, Form::A64Branch19, Target::Local(done)).unwrap();
    a.a64(LDR_X16_X27, Form::A64Ldst64Imm12, Target::LinkSlot(2))
        .unwrap();
    a.a64(LDR_X16_X27, Form::A64Ldst64Imm12, Target::IcCell(1))
        .unwrap();
    a.a64(LDR_X0_X28, Form::A64Ldst64Imm12, Target::RootIndex(5))
        .unwrap();
    a.a64(LDR_X16_X27, Form::A64Ldst64Imm12, Target::Stub(3))
        .unwrap();
    a.put32(BLR_X16);
    a.a64(B, Form::A64Branch26, Target::Local(done)).unwrap();
    a.bind(slow).unwrap();
    a.a64(LDR_X16_X28, Form::A64Ldst64Imm12, Target::Runtime(4))
        .unwrap();
    a.put32(BLR_X16);
    a.a64(LDR_X16_X28, Form::A64Ldst64Imm12, Target::DeoptExit(9))
        .unwrap();
    a.put32(BLR_X16);
    a.bind(done).unwrap();
    a.put32(MOVZ_X0_1);
    a.put32(BR_X16);
    a.finish().unwrap()
}

/// The x86-64 form of [`a64_unit_function`].
pub fn x64_unit_function() -> CodeDesc {
    use x64::*;
    let mut a = Assembler::new(Arch::X64);
    let done = a.new_label();
    let slow = a.new_label();
    a.x64(&JE_REL32.0, JE_REL32.1, Form::X64Rel32, Target::Local(slow))
        .unwrap();
    let (m14, f14) = MOV_RAX_R14;
    let (m15, f15) = MOV_RAX_R15;
    a.x64(&m14, f14, Form::X64Disp32, Target::LinkSlot(2))
        .unwrap();
    a.x64(&m14, f14, Form::X64Disp32, Target::IcCell(1))
        .unwrap();
    a.x64(&m15, f15, Form::X64Disp32, Target::RootIndex(5))
        .unwrap();
    a.x64(&m14, f14, Form::X64Disp32, Target::Stub(3)).unwrap();
    a.put(&CALL_RAX);
    a.x64(
        &JMP_REL32.0,
        JMP_REL32.1,
        Form::X64Rel32,
        Target::Local(done),
    )
    .unwrap();
    a.bind(slow).unwrap();
    a.x64(&m15, f15, Form::X64Disp32, Target::Runtime(4))
        .unwrap();
    a.put(&CALL_RAX);
    a.x64(&m15, f15, Form::X64Disp32, Target::DeoptExit(9))
        .unwrap();
    a.put(&CALL_RAX);
    a.bind(done).unwrap();
    a.put(&NOP);
    a.put(&JMP_RAX);
    a.finish().unwrap()
}

/// A function with only straight-line code and one backward Local branch (a loop).
pub fn a64_loop() -> CodeDesc {
    use a64::*;
    let mut a = Assembler::new(Arch::A64);
    let top = a.new_label();
    a.bind(top).unwrap();
    a.put32(NOP);
    a.a64(CBZ_X0, Form::A64Branch19, Target::Local(top))
        .unwrap();
    a.a64(LDR_X16_X27, Form::A64Ldst64Imm12, Target::LinkSlot(0))
        .unwrap();
    a.put32(BR_X16);
    a.finish().unwrap()
}

/// The unit environment of the tests: RW words 10+i for link cells, 100+i for IC cells,
/// 200+i for stub link cells; isolate slots 0x800+8i (runtime), 0x400+8i (roots), 0x7f8
/// (the deopt entry). Operand 77 has no value.
#[cfg(feature = "aot")]
pub struct TestUnit;

#[cfg(feature = "aot")]
impl ep9_masm::UnitEnv for TestUnit {
    fn stub_link(&self, id: u32) -> Option<u32> {
        (id != 77).then_some(200 + id)
    }
    fn runtime_slot(&self, id: u32) -> Option<i64> {
        (id != 77).then_some(0x800 + 8 * id as i64)
    }
    fn link_word(&self, index: u32) -> Option<u32> {
        (index != 77).then_some(10 + index)
    }
    fn ic_word(&self, index: u32) -> Option<u32> {
        (index != 77).then_some(100 + index)
    }
    fn root_slot(&self, index: u32) -> Option<i64> {
        (index != 77).then_some(0x400 + 8 * index as i64)
    }
    fn deopt_slot(&self) -> i64 {
        0x7f8
    }
}

/// Finalizes `desc` as unit text and states the zero-text-relocation rule on it. Every unit
/// that the tests emit goes through here.
#[cfg(feature = "aot")]
pub fn emit_unit(desc: &CodeDesc) -> ep9_masm::UnitCode {
    let u = ep9_masm::finalize_unit(desc, &TestUnit).expect("finalize_unit");
    assert_eq!(ep9_masm::zero_text_relocations(&u), Ok(()));
    u
}

/// The JIT environment of the tests: code at 0x1_0000_0000; stubs at 0x1_0010_0000 + 0x100*id;
/// runtime functions at 0x1_0020_0000 + 0x100*id, table slots 0x800+8i; link values
/// 0xaaaa_0000+i; IC cells at 0x2_0000_0000+8i; roots 0x400+8i; deopt entry 0x1_0030_0000;
/// heap constants 0x3_0000_0000+16i. Operand 77 has no value.
#[cfg(feature = "jit")]
pub struct TestJit;

#[cfg(feature = "jit")]
pub const CODE: u64 = 0x1_0000_0000;

#[cfg(feature = "jit")]
impl ep9_masm::JitEnv for TestJit {
    fn code_address(&self) -> u64 {
        CODE
    }
    fn stub(&self, id: u32) -> Option<u64> {
        (id != 77).then_some(0x1_0010_0000 + 0x100 * id as u64)
    }
    fn runtime(&self, id: u32) -> Option<u64> {
        (id != 77).then_some(0x1_0020_0000 + 0x100 * id as u64)
    }
    fn runtime_slot(&self, id: u32) -> Option<i64> {
        (id != 77).then_some(0x800 + 8 * id as i64)
    }
    fn link(&self, index: u32) -> Option<u64> {
        (index != 77).then_some(0xaaaa_0000 + index as u64)
    }
    fn ic_cell(&self, index: u32) -> Option<u64> {
        (index != 77).then_some(0x2_0000_0000 + 8 * index as u64)
    }
    fn root_slot(&self, index: u32) -> Option<i64> {
        (index != 77).then_some(0x400 + 8 * index as i64)
    }
    fn deopt_entry(&self) -> u64 {
        0x1_0030_0000
    }
    fn heap_const(&self, index: u32) -> Option<u64> {
        (index != 77).then_some(0x3_0000_0000 + 16 * index as u64)
    }
}
