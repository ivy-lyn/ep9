//! The instruction stream: text bytes plus abstract relocations (H34), and the field forms
//! that a finalizer writes.
//!
//! An emitter writes each instruction with an empty field and records a [`Reloc`] at the
//! field: the field form ([`Form`]), the kind (a wire code, [`crate::RelocKind`]) and its
//! operand. [`Assembler::finish`] gives a [`CodeDesc`]; both finalizers take one (doc 13 S-1).
//! In month 7 the tests write the instructions by hand (plan v12 rule 71); an encoder comes
//! with T1 (04-execution section 5).

use crate::kind::{Label, RelocKind, Target};

/// The target architecture of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    /// AArch64: 4-byte little-endian instruction words.
    A64,
    /// x86-64.
    X64,
}

/// How a relocation field is stored in the text.
///
/// | Form | Field | Value | Class |
/// |---|---|---|---|
/// | [`Form::A64Branch26`] | B, BL: bits 0-25 | byte distance / 4, +-128 MiB | pc-relative |
/// | [`Form::A64Branch19`] | B.cond, CBZ, CBNZ, LDR (literal): bits 5-23 | byte distance / 4, +-1 MiB | pc-relative |
/// | [`Form::A64Branch14`] | TBZ, TBNZ: bits 5-18 | byte distance / 4, +-32 KiB | pc-relative |
/// | [`Form::A64Ldst64Imm12`] | LDR, STR Xt, \[Xn, #imm\] (64-bit, unsigned offset): bits 10-21 | byte offset / 8, 0 to 32,760 | register-relative |
/// | [`Form::X64Rel32`] | 4 bytes (call, jmp, jcc rel32) | byte distance from the end of the field | pc-relative |
/// | [`Form::X64Disp32`] | 4 bytes, the disp32 of \[reg + disp32\] | byte offset | register-relative |
/// | [`Form::Abs64`] | 8 bytes (a literal word or a movabs imm64) | an absolute value | absolute: JIT only |
///
/// A pc-relative distance counts from the instruction on AArch64 and from the end of the
/// field on x86-64. The AArch64 forms check the opcode of the word they patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
    /// AArch64 B or BL.
    A64Branch26,
    /// AArch64 B.cond, CBZ, CBNZ or LDR (literal).
    A64Branch19,
    /// AArch64 TBZ or TBNZ.
    A64Branch14,
    /// AArch64 LDR or STR of a 64-bit register, unsigned scaled offset.
    A64Ldst64Imm12,
    /// x86-64 rel32.
    X64Rel32,
    /// x86-64 disp32 of a register-relative operand.
    X64Disp32,
    /// 8 bytes of absolute value. JIT only: unit text holds no absolute value.
    Abs64,
}

/// The class of a [`Form`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormClass {
    /// A distance from the field's instruction.
    PcRelative,
    /// An offset from a base register (the isolate register or the unit-data register).
    RegisterRelative,
    /// An absolute value.
    Absolute,
}

/// Why a field value does not fit its form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldError {
    /// The value is out of the form's range.
    Range,
    /// The value is not a multiple of the form's scale (4 or 8).
    Alignment,
    /// The AArch64 word at the field is not an instruction of the form.
    Opcode(u32),
}

impl Form {
    /// The architecture of the form; `None` for [`Form::Abs64`] (both).
    pub const fn arch(self) -> Option<Arch> {
        match self {
            Form::A64Branch26 | Form::A64Branch19 | Form::A64Branch14 | Form::A64Ldst64Imm12 => {
                Some(Arch::A64)
            }
            Form::X64Rel32 | Form::X64Disp32 => Some(Arch::X64),
            Form::Abs64 => None,
        }
    }

    /// The class of the form.
    pub const fn class(self) -> FormClass {
        match self {
            Form::A64Branch26 | Form::A64Branch19 | Form::A64Branch14 | Form::X64Rel32 => {
                FormClass::PcRelative
            }
            Form::A64Ldst64Imm12 | Form::X64Disp32 => FormClass::RegisterRelative,
            Form::Abs64 => FormClass::Absolute,
        }
    }

    /// The field size in bytes. An AArch64 field is the whole instruction word.
    pub const fn size(self) -> usize {
        match self {
            Form::Abs64 => 8,
            _ => 4,
        }
    }

    /// The offset from the field start that a pc-relative distance counts from.
    pub const fn pc_base(self) -> u32 {
        match self {
            Form::X64Rel32 => 4,
            _ => 0,
        }
    }

    /// True for the forms of a branch or call: B or BL on AArch64, rel32 on x86-64. The other
    /// pc-relative forms are conditional branches and loads inside one function.
    pub const fn is_call(self) -> bool {
        matches!(self, Form::A64Branch26 | Form::X64Rel32)
    }

    /// Writes `value` into the field at `at` (`at + size()` is inside `text`).
    pub fn write(self, text: &mut [u8], at: usize, value: i64) -> Result<(), FieldError> {
        match self {
            Form::A64Branch26 => a64_field(text, at, value, 4, 26, 0, is_b_or_bl),
            Form::A64Branch19 => a64_field(text, at, value, 4, 19, 5, is_imm19),
            Form::A64Branch14 => a64_field(text, at, value, 4, 14, 5, is_tbz),
            Form::A64Ldst64Imm12 => {
                if value < 0 {
                    return Err(FieldError::Range);
                }
                a64_field(text, at, value, 8, 12, 10, is_ldst64_uimm)
            }
            Form::X64Rel32 | Form::X64Disp32 => {
                let v = i32::try_from(value).map_err(|_| FieldError::Range)?;
                text[at..at + 4].copy_from_slice(&v.to_le_bytes());
                Ok(())
            }
            Form::Abs64 => {
                text[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
                Ok(())
            }
        }
    }

    /// Reads the value of the field at `at`: the inverse of [`Form::write`] (an
    /// [`Form::Abs64`] value comes back as its bits).
    pub fn read(self, text: &[u8], at: usize) -> i64 {
        match self {
            Form::A64Branch26 => sext(word(text, at) & 0x03ff_ffff, 26) * 4,
            Form::A64Branch19 => sext((word(text, at) >> 5) & 0x7_ffff, 19) * 4,
            Form::A64Branch14 => sext((word(text, at) >> 5) & 0x3fff, 14) * 4,
            Form::A64Ldst64Imm12 => (((word(text, at) >> 10) & 0xfff) as i64) * 8,
            Form::X64Rel32 | Form::X64Disp32 => {
                i32::from_le_bytes(text[at..at + 4].try_into().expect("4 bytes")) as i64
            }
            Form::Abs64 => u64::from_le_bytes(text[at..at + 8].try_into().expect("8 bytes")) as i64,
        }
    }
}

/// The little-endian word at `at`.
pub(crate) fn word(text: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(text[at..at + 4].try_into().expect("4 bytes"))
}

/// Sign-extends the low `bits` bits of `v`.
pub(crate) const fn sext(v: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((v as i64) << shift) >> shift
}

/// AArch64 B (`0x14000000`) or BL (`0x94000000`).
pub(crate) const fn is_b_or_bl(w: u32) -> bool {
    w & 0x7c00_0000 == 0x1400_0000
}

/// AArch64 B.cond.
pub(crate) const fn is_b_cond(w: u32) -> bool {
    w & 0xff00_0010 == 0x5400_0000
}

/// AArch64 CBZ or CBNZ.
pub(crate) const fn is_cbz(w: u32) -> bool {
    w & 0x7e00_0000 == 0x3400_0000
}

/// AArch64 LDR (literal), any register class, or PRFM (literal).
pub(crate) const fn is_ldr_literal(w: u32) -> bool {
    w & 0x3b00_0000 == 0x1800_0000
}

/// The instructions of [`Form::A64Branch19`].
const fn is_imm19(w: u32) -> bool {
    is_b_cond(w) || is_cbz(w) || is_ldr_literal(w)
}

/// AArch64 TBZ or TBNZ.
pub(crate) const fn is_tbz(w: u32) -> bool {
    w & 0x7e00_0000 == 0x3600_0000
}

/// AArch64 LDR or STR of an X register, unsigned offset.
const fn is_ldst64_uimm(w: u32) -> bool {
    w & 0xffc0_0000 == 0xf940_0000 || w & 0xffc0_0000 == 0xf900_0000
}

/// Writes `value / scale` into `bits` bits at bit `lsb` of the word at `at`, signed for the
/// branch forms (scale 4) and unsigned for the load form (scale 8).
fn a64_field(
    text: &mut [u8],
    at: usize,
    value: i64,
    scale: i64,
    bits: u32,
    lsb: u32,
    opcode_ok: fn(u32) -> bool,
) -> Result<(), FieldError> {
    let w = word(text, at);
    if !opcode_ok(w) {
        return Err(FieldError::Opcode(w));
    }
    if value % scale != 0 {
        return Err(FieldError::Alignment);
    }
    let v = value / scale;
    let fits = if scale == 8 {
        (0..1i64 << bits).contains(&v)
    } else {
        (-(1i64 << (bits - 1))..1i64 << (bits - 1)).contains(&v)
    };
    if !fits {
        return Err(FieldError::Range);
    }
    let mask = ((1u32 << bits) - 1) << lsb;
    let w = (w & !mask) | (((v as u32) << lsb) & mask);
    text[at..at + 4].copy_from_slice(&w.to_le_bytes());
    Ok(())
}

/// One relocation of a [`CodeDesc`]: a field, its form, the kind as a wire code and the
/// kind's operand.
///
/// The kind is a code, not a [`RelocKind`], so that a record from a newer writer (a stream
/// read back from bytes; a JIT request finalized again as a unit, doc 13 F21) reaches the
/// finalizer as it is: a finalizer refuses a code it does not know
/// (`FinalizeError::UnknownKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reloc {
    /// The text offset of the field.
    pub at: u32,
    /// The field form.
    pub form: Form,
    /// The kind code ([`RelocKind::code`]).
    pub code: u8,
    /// The operand: a label, stub, function, cell, root, exit or constant index.
    pub index: u32,
}

impl Reloc {
    /// A relocation of a typed target.
    pub const fn new(at: u32, form: Form, target: Target) -> Reloc {
        Reloc {
            at,
            form,
            code: target.kind().code(),
            index: target.index(),
        }
    }

    /// The kind, or `None` for an unknown code.
    pub const fn kind(&self) -> Option<RelocKind> {
        RelocKind::from_code(self.code)
    }

    /// The typed target, or `None` for an unknown code.
    pub const fn target(&self) -> Option<Target> {
        match self.kind() {
            Some(k) => Some(Target::new(k, self.index)),
            None => None,
        }
    }
}

/// The output of an emitter and the input of both finalizers (doc 13 S-1): the text, its
/// relocations and the label offsets. The other CodeDesc parts of doc 13 S-1 (exits, unwind,
/// positions, the RW layout, deps) come with the tiers that make them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeDesc {
    /// The architecture of the text.
    pub arch: Arch,
    /// The text, with the relocation fields still to be written.
    pub text: Vec<u8>,
    /// The relocations, in the order the emitter made them.
    pub relocs: Vec<Reloc>,
    /// The text offset of each label; `None` for a label that was never bound.
    pub labels: Vec<Option<u32>>,
}

/// An emit error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitError {
    /// A label was bound twice.
    BoundTwice(Label),
    /// A relocation names a label that this assembler did not make.
    NoSuchLabel(Label),
    /// A relocation names a label that is never bound.
    Unbound(Label),
    /// The field of a relocation is not inside the text written so far.
    FieldOutOfText {
        /// The field offset.
        at: u32,
    },
    /// The form is of the other architecture.
    WrongArch {
        /// The field offset.
        at: u32,
    },
    /// The text is longer than `u32::MAX` bytes.
    TooLong,
}

/// The instruction stream of one function: text plus relocations.
#[derive(Clone, Debug)]
pub struct Assembler {
    arch: Arch,
    text: Vec<u8>,
    relocs: Vec<Reloc>,
    labels: Vec<Option<u32>>,
}

impl Assembler {
    /// An empty stream.
    pub fn new(arch: Arch) -> Assembler {
        Assembler {
            arch,
            text: Vec::new(),
            relocs: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// The architecture.
    pub fn arch(&self) -> Arch {
        self.arch
    }

    /// The offset of the next byte.
    pub fn offset(&self) -> u32 {
        self.text.len() as u32
    }

    /// Writes bytes.
    pub fn put(&mut self, bytes: &[u8]) {
        self.text.extend_from_slice(bytes);
    }

    /// Writes one 4-byte little-endian word (an AArch64 instruction).
    pub fn put32(&mut self, w: u32) {
        self.put(&w.to_le_bytes());
    }

    /// A new label, not bound yet.
    pub fn new_label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() as u32 - 1)
    }

    /// Binds `label` to the current offset.
    pub fn bind(&mut self, label: Label) -> Result<(), EmitError> {
        let at = self.offset();
        match self.labels.get_mut(label.0 as usize) {
            None => Err(EmitError::NoSuchLabel(label)),
            Some(Some(_)) => Err(EmitError::BoundTwice(label)),
            Some(slot) => {
                *slot = Some(at);
                Ok(())
            }
        }
    }

    /// Records a relocation at the field `at`, which is already written.
    pub fn reloc(&mut self, at: u32, form: Form, target: Target) -> Result<(), EmitError> {
        if form.arch().is_some_and(|a| a != self.arch) {
            return Err(EmitError::WrongArch { at });
        }
        if at as usize + form.size() > self.text.len() {
            return Err(EmitError::FieldOutOfText { at });
        }
        if let Target::Local(l) = target
            && l.0 as usize >= self.labels.len()
        {
            return Err(EmitError::NoSuchLabel(l));
        }
        self.relocs.push(Reloc::new(at, form, target));
        Ok(())
    }

    /// Writes one AArch64 word with an empty field and records its relocation.
    pub fn a64(&mut self, w: u32, form: Form, target: Target) -> Result<(), EmitError> {
        let at = self.offset();
        self.put32(w);
        self.reloc(at, form, target)
    }

    /// Writes an x86-64 instruction whose field starts `field` bytes into `bytes`, and records
    /// its relocation.
    pub fn x64(
        &mut self,
        bytes: &[u8],
        field: u32,
        form: Form,
        target: Target,
    ) -> Result<(), EmitError> {
        let at = self.offset() + field;
        self.put(bytes);
        self.reloc(at, form, target)
    }

    /// The finished stream. Every label that a relocation names must be bound.
    pub fn finish(self) -> Result<CodeDesc, EmitError> {
        if u32::try_from(self.text.len()).is_err() {
            return Err(EmitError::TooLong);
        }
        for r in &self.relocs {
            if let Some(Target::Local(l)) = r.target()
                && self.labels[l.0 as usize].is_none()
            {
                return Err(EmitError::Unbound(l));
            }
        }
        Ok(CodeDesc {
            arch: self.arch,
            text: self.text,
            relocs: self.relocs,
            labels: self.labels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a64_fields_round_trip_and_check_range_alignment_and_opcode() {
        let mut t = Vec::new();
        for w in [0x1400_0000u32, 0x5400_0000, 0x3600_0000, 0xf940_0370] {
            t.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(Form::A64Branch26.write(&mut t, 0, -8), Ok(()));
        assert_eq!(Form::A64Branch26.read(&t, 0), -8);
        assert_eq!(word(&t, 0), 0x17ff_fffe);
        assert_eq!(
            Form::A64Branch19.write(&mut t, 4, 1 << 20),
            Err(FieldError::Range)
        );
        assert_eq!(Form::A64Branch19.write(&mut t, 4, (1 << 20) - 4), Ok(()));
        assert_eq!(Form::A64Branch19.read(&t, 4), (1 << 20) - 4);
        assert_eq!(
            Form::A64Branch14.write(&mut t, 8, 6),
            Err(FieldError::Alignment)
        );
        assert_eq!(Form::A64Branch14.write(&mut t, 8, -(1 << 15)), Ok(()));
        assert_eq!(Form::A64Branch14.read(&t, 8), -(1 << 15));
        assert_eq!(Form::A64Ldst64Imm12.write(&mut t, 12, 8 * 4095), Ok(()));
        assert_eq!(Form::A64Ldst64Imm12.read(&t, 12), 8 * 4095);
        assert_eq!(word(&t, 12) & !(0xfff << 10), 0xf940_0370);
        assert_eq!(
            Form::A64Ldst64Imm12.write(&mut t, 12, 8 * 4096),
            Err(FieldError::Range)
        );
        assert_eq!(
            Form::A64Ldst64Imm12.write(&mut t, 12, -8),
            Err(FieldError::Range)
        );
        // A B.cond word is not a B or BL.
        assert_eq!(
            Form::A64Branch26.write(&mut t, 4, 0),
            Err(FieldError::Opcode(word(&t, 4)))
        );
    }

    #[test]
    fn x64_and_abs_fields_round_trip() {
        let mut t = vec![0u8; 12];
        assert_eq!(Form::X64Rel32.write(&mut t, 0, -5), Ok(()));
        assert_eq!(Form::X64Rel32.read(&t, 0), -5);
        assert_eq!(
            Form::X64Disp32.write(&mut t, 0, 1 << 31),
            Err(FieldError::Range)
        );
        assert_eq!(Form::Abs64.write(&mut t, 4, 0x1234_5678_9abc_def0), Ok(()));
        assert_eq!(Form::Abs64.read(&t, 4), 0x1234_5678_9abc_def0);
    }

    #[test]
    fn the_assembler_checks_labels_fields_and_arch() {
        let mut a = Assembler::new(Arch::A64);
        let l = a.new_label();
        assert_eq!(
            a.reloc(0, Form::A64Branch26, Target::Local(l)),
            Err(EmitError::FieldOutOfText { at: 0 })
        );
        a.a64(0x1400_0000, Form::A64Branch26, Target::Local(l))
            .unwrap();
        assert_eq!(
            a.reloc(0, Form::X64Rel32, Target::Local(l)),
            Err(EmitError::WrongArch { at: 0 })
        );
        assert_eq!(
            a.reloc(0, Form::A64Branch26, Target::Local(Label(9))),
            Err(EmitError::NoSuchLabel(Label(9)))
        );
        let unbound = a.clone().finish();
        assert_eq!(unbound, Err(EmitError::Unbound(l)));
        a.bind(l).unwrap();
        assert_eq!(a.bind(l), Err(EmitError::BoundTwice(l)));
        let d = a.finish().unwrap();
        assert_eq!(d.labels, [Some(4)]);
        assert_eq!(d.relocs[0].kind(), Some(RelocKind::Local));
    }
}
