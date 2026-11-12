//! `finalize_unit` (feature `aot`): unit text with zero text relocations, and the check of it
//! (H34, H35; 10-quality section 5 "Zero text relocations").
//!
//! Unit text is mapped once per process and may be signed, so it holds no absolute address and
//! no field that the loader must write (H34 Context). [`UnitEnv`] gives no address at all: every
//! external reference goes through a register.
//!
//! | Kind | Form | Value |
//! |---|---|---|
//! | Local | every pc-relative form | the distance to the label, inside the function (H34-Q2) |
//! | Stub | a register-relative form | a load of the stub's link cell through the unit-data register, then a call through a register; the in-unit pc-relative call is not built (H34-Q4, open) |
//! | Runtime | a register-relative form | a load of the RuntimeTable entry through the isolate register |
//! | LinkSlot | a register-relative form | a load of RW link cell i through the unit-data register x27/r14 (H34-Q1, H35-Q9) |
//! | IcCell | a register-relative form | a load of RW IC cell i through the unit-data register (H34-Q1) |
//! | RootIndex | a register-relative form | a load from the isolate root table (isolate register) |
//! | DeoptExit | a register-relative form | a load of the shared deopt entry through the isolate register; the exit goes into [`UnitCode::exits`] |
//! | HeapConst | none | **rejected by the unit writer** ([`FinalizeError::HeapConstInUnit`]) |
//!
//! `finalize_unit` checks the base register of the register-relative rows
//! ([`check_register_loads`]): each LinkSlot, IcCell and Stub field must be in a 64-bit load
//! through the unit-data register x27/r14, and each Runtime, RootIndex and DeoptExit field in a
//! 64-bit load through the isolate register x28/r15 (M2-JIT-ABI, H35; the AArch64 rows since
//! M7-P4, route A64-ISOLATE-LOAD).
//!
//! [`UnitCode`] has no public constructor: only `finalize_unit` makes one, and it runs
//! [`check_text`] on its output, so a writer that takes a `UnitCode` gets only checked text
//! with no HeapConst. (The ep9_unit writer, behind its `aot` feature, is still empty.) These forms are the gated H34 parts "Unit finalize forms" (baseline-AOT at the A3
//! gate, optimized-AOT at the A4 gates).

use std::fmt;

use crate::fin::{self, Exit, FinalizeError, not_allowed, unresolved};
use crate::kind::RelocKind;
use crate::stream::{
    Arch, CodeDesc, Form, FormClass, is_adr, is_adrp, is_b_cond, is_b_or_bl, is_bl, is_cb_cc,
    is_cbz, is_ldr_literal, is_tbz, sext, word, x64_load,
};
use crate::x64;

/// What `finalize_unit` reads from the unit layout and the isolate ABI. No method gives an
/// address.
pub trait UnitEnv {
    /// The RW word of a stub's link cell.
    fn stub_link(&self, id: u32) -> Option<u32>;
    /// The offset of the RuntimeTable entry of a runtime function from the isolate register.
    fn runtime_slot(&self, id: u32) -> Option<i64>;
    /// The RW word of link cell `index`.
    fn link_word(&self, index: u32) -> Option<u32>;
    /// The RW word of IC cell `index`.
    fn ic_word(&self, index: u32) -> Option<u32>;
    /// The offset of a root slot (H13) from the isolate register.
    fn root_slot(&self, index: u32) -> Option<i64>;
    /// The offset of the shared deopt entry from the isolate register.
    fn deopt_slot(&self) -> i64;
    /// True when the unit has landing pads (the container compat flag `has_pads`,
    /// MC-w23-15): `finalize_unit` then also runs [`check_pads`]. The default is false.
    fn pads(&self) -> bool {
        false
    }
}

/// One RW word that the unit text loads through the unit-data register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RwUse {
    /// The field offset.
    pub at: u32,
    /// LinkSlot, IcCell or Stub (the stub's link cell).
    pub kind: RelocKind,
    /// The operand.
    pub index: u32,
    /// The RW word.
    pub word: u32,
}

/// One written field of unit text, for [`check_text`] and [`check_register_loads`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// The field offset.
    pub at: u32,
    /// The start of the field's instruction ([`crate::Reloc::insn`]): the x86-64 checks
    /// decode the operand forward from here, and [`check_text`] checks that it is an
    /// instruction start of the whole-text decode. On AArch64 `insn = at` (else
    /// [`Finding::MisalignedField`]).
    pub insn: u32,
    /// The form.
    pub form: Form,
    /// The kind (the register rules of [`check_register_loads`] depend on it).
    pub kind: RelocKind,
}

/// The output of [`finalize_unit`]: position-independent text with zero text relocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitCode {
    arch: Arch,
    text: Vec<u8>,
    fields: Vec<Field>,
    rw: Vec<RwUse>,
    exits: Vec<Exit>,
    pads: bool,
}

impl UnitCode {
    /// The architecture.
    pub fn arch(&self) -> Arch {
        self.arch
    }
    /// The text: every field written, nothing left for the loader.
    pub fn text(&self) -> &[u8] {
        &self.text
    }
    /// The written fields, in record order.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }
    /// The RW words that the text loads through the unit-data register, in record order.
    pub fn rw(&self) -> &[RwUse] {
        &self.rw
    }
    /// The deopt exits, in record order.
    pub fn exits(&self) -> &[Exit] {
        &self.exits
    }
    /// True when `finalize_unit` checked the landing pads ([`UnitEnv::pads`], [`check_pads`]).
    pub fn has_pads(&self) -> bool {
        self.pads
    }
}

/// Finalizes `desc` as the unit text of one function.
///
/// Refuses the whole stream on the first problem: an unknown kind code, a HeapConst, an
/// absolute form, a form that the kind has not here, an operand that `env` cannot resolve, a
/// value that does not fit its field, or a finding of [`check_text`] or
/// [`check_register_loads`] on the output.
pub fn finalize_unit(desc: &CodeDesc, env: &dyn UnitEnv) -> Result<UnitCode, FinalizeError> {
    let sites = fin::sites(desc)?;
    let mut text = desc.text.clone();
    let mut rw = Vec::new();
    let mut exits = Vec::new();
    for s in &sites {
        let class = s.form.class();
        let reg = class == FormClass::RegisterRelative;
        let mut rw_word = |w: Option<u32>| -> Result<i64, FinalizeError> {
            let word = w.ok_or_else(|| unresolved(s))?;
            rw.push(RwUse {
                at: s.at,
                kind: s.kind,
                index: s.index,
                word,
            });
            Ok(8 * word as i64)
        };
        // Every kind has an arm, with no wildcard: a new kind does not build until this
        // finalizer handles or rejects it (H34 Freeze table).
        let value = match s.kind {
            RelocKind::Local => fin::local_value(desc, s)?,
            RelocKind::Stub if reg => rw_word(env.stub_link(s.index))?,
            RelocKind::Stub if class == FormClass::PcRelative => {
                return Err(FinalizeError::FormNotBuilt {
                    at: s.at,
                    kind: s.kind,
                    form: s.form,
                });
            }
            RelocKind::Stub => return Err(not_allowed(s)),
            RelocKind::Runtime if reg => env.runtime_slot(s.index).ok_or_else(|| unresolved(s))?,
            RelocKind::Runtime => return Err(not_allowed(s)),
            RelocKind::LinkSlot if reg => rw_word(env.link_word(s.index))?,
            RelocKind::LinkSlot => return Err(not_allowed(s)),
            RelocKind::IcCell if reg => rw_word(env.ic_word(s.index))?,
            RelocKind::IcCell => return Err(not_allowed(s)),
            RelocKind::RootIndex if reg => env.root_slot(s.index).ok_or_else(|| unresolved(s))?,
            RelocKind::RootIndex => return Err(not_allowed(s)),
            RelocKind::DeoptExit if reg => {
                exits.push(Exit {
                    at: s.at,
                    id: s.index,
                });
                env.deopt_slot()
            }
            RelocKind::DeoptExit => return Err(not_allowed(s)),
            RelocKind::HeapConst => {
                return Err(FinalizeError::HeapConstInUnit {
                    at: s.at,
                    index: s.index,
                });
            }
        };
        fin::write(&mut text, s, value)?;
    }
    let fields: Vec<Field> = sites
        .iter()
        .map(|s| Field {
            at: s.at,
            insn: s.insn,
            form: s.form,
            kind: s.kind,
        })
        .collect();
    let mut findings = check_text(desc.arch, &text, &fields);
    findings.extend(check_register_loads(desc.arch, &text, &fields));
    let pads = env.pads();
    if pads {
        findings.extend(check_pads(desc.arch, &text));
    }
    if !findings.is_empty() {
        return Err(FinalizeError::TextRelocations(findings));
    }
    Ok(UnitCode {
        arch: desc.arch,
        text,
        fields,
        rw,
        exits,
        pads,
    })
}

/// A finding of [`check_text`]: a place in unit text that would need a text relocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finding {
    /// An absolute field (Abs64): its value depends on where things are placed.
    AbsoluteField {
        /// The field offset.
        at: u32,
    },
    /// A pc-relative field or instruction whose target is outside this function's text.
    OutsideFunction {
        /// The instruction or field offset.
        at: u32,
        /// The target offset.
        target: i64,
    },
    /// An AArch64 `adrp` (a page-relative reference).
    Adrp {
        /// The instruction offset.
        at: u32,
    },
    /// An AArch64 `bl` (a direct call out of the function: H34-Q3 unit text has none).
    DirectCall {
        /// The instruction offset.
        at: u32,
    },
    /// AArch64 text whose length is not a multiple of 4.
    Misaligned {
        /// The text length.
        len: u32,
    },
    /// An x86-64 disp32 field ([`Form::X64Disp32`]) that is not the disp32 of a
    /// `mov r, [reg + disp32]` decoded from the field's instruction start (see [`check_text`]):
    /// for example `[rip + disp32]` (a pc-relative field under a register-relative label), a
    /// SIB operand with no base, or a field inside another instruction.
    NotRegisterRelative {
        /// The field offset.
        at: u32,
    },
    /// A field that is not inside the text (`at + size` is past the end). No other rule reads
    /// such a field.
    FieldOutsideText {
        /// The field offset.
        at: u32,
    },
    /// A LinkSlot, IcCell or Stub field that is not in a 64-bit load through the unit-data
    /// register (H34-Q1): see [`check_register_loads`]. A store, or a load through another
    /// base register, reads or writes another cell.
    NotUnitDataLoad {
        /// The field offset.
        at: u32,
    },
    /// A Runtime, RootIndex or DeoptExit field that is not in a 64-bit load through the
    /// isolate register x28/r15: see [`check_register_loads`].
    NotIsolateLoad {
        /// The field offset.
        at: u32,
    },
    /// An AArch64 field that is not one instruction word: its offset is not a multiple of 4,
    /// or its record has `insn != at` (see [`check_text`]). No other rule reads such a field.
    MisalignedField {
        /// The field offset.
        at: u32,
    },
    /// x86-64 bytes that the length decoder does not know, or an instruction that runs past
    /// the end of the text (see [`check_text`]). The decode of the text stops here.
    Undecodable {
        /// The offset of the instruction.
        at: u32,
    },
    /// An x86-64 field whose record names an instruction start ([`Field::insn`]) that is not
    /// an instruction start of the decode of the whole text.
    NotInstructionStart {
        /// The field offset.
        at: u32,
    },
    /// An x86-64 [`Form::X64Rel32`] field that is not the rel32 of the `call`, `jmp` or `jcc`
    /// that starts at its record's instruction start, nor the disp32 of a RIP-relative
    /// operand at the end of that instruction.
    FieldNotOperand {
        /// The field offset.
        at: u32,
    },
    /// An x86-64 instruction with an absolute memory operand: `moffs` (A0-A3), or a SIB byte
    /// with no base and mod = 00 (`[disp32]`, `[index * scale + disp32]`).
    AbsoluteAddress {
        /// The offset of the instruction.
        at: u32,
    },
    /// An x86-64 pc-relative branch whose target is inside the text but not an instruction
    /// start of the decode: the CPU would run bytes that the check did not decode.
    BranchIntoInstruction {
        /// The offset of the branch's rel field.
        at: u32,
        /// The target offset.
        target: i64,
    },
    /// A unit with landing pads whose AArch64 function entry (offset 0), or the target of an
    /// `adr` (into any register), is not `bti c` or `bti jc` (see [`check_pads`]).
    NoLandingPad {
        /// The entry or target offset.
        at: u32,
    },
    /// A unit with landing pads with an AArch64 indirect jump (`br`, `braa`, `brab`,
    /// `braaz`, `brabz`) through a register other than x16 or x17 (see [`check_pads`]).
    IndirectJumpRegister {
        /// The instruction offset.
        at: u32,
    },
    /// A unit with landing pads with an AArch64 `br` or `blr` (or a PAC form) whose register
    /// value is not proven to be a landing pad or code outside the text (see [`check_pads`]):
    /// it is not set in the same straight-line run by a load through x27 or x28 or by an
    /// `adr` (route PADS-PROVENANCE).
    IndirectTargetNotProven {
        /// The instruction offset.
        at: u32,
    },
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::AbsoluteField { at } => write!(f, "+{at:#x}: absolute field"),
            Finding::OutsideFunction { at, target } => {
                write!(
                    f,
                    "+{at:#x}: pc-relative target {target:#x} outside the function"
                )
            }
            Finding::Adrp { at } => write!(f, "+{at:#x}: adrp"),
            Finding::DirectCall { at } => write!(f, "+{at:#x}: bl"),
            Finding::Misaligned { len } => write!(f, "text length {len} is not a multiple of 4"),
            Finding::NotRegisterRelative { at } => {
                write!(f, "+{at:#x}: disp32 field not in a [reg + disp32] operand")
            }
            Finding::FieldOutsideText { at } => write!(f, "+{at:#x}: field outside the text"),
            Finding::NotUnitDataLoad { at } => {
                write!(f, "+{at:#x}: not a load through the unit-data register")
            }
            Finding::NotIsolateLoad { at } => {
                write!(f, "+{at:#x}: not a load through the isolate register")
            }
            Finding::MisalignedField { at } => {
                write!(
                    f,
                    "+{at:#x}: AArch64 field not at an aligned instruction word"
                )
            }
            Finding::Undecodable { at } => write!(f, "+{at:#x}: x86-64 bytes not decoded"),
            Finding::NotInstructionStart { at } => {
                write!(f, "+{at:#x}: field record's instruction start is not one")
            }
            Finding::FieldNotOperand { at } => {
                write!(
                    f,
                    "+{at:#x}: rel32 field not the operand of its instruction"
                )
            }
            Finding::AbsoluteAddress { at } => write!(f, "+{at:#x}: absolute memory operand"),
            Finding::BranchIntoInstruction { at, target } => {
                write!(
                    f,
                    "+{at:#x}: branch target {target:#x} inside an instruction"
                )
            }
            Finding::NoLandingPad { at } => write!(f, "+{at:#x}: entry without bti c"),
            Finding::IndirectJumpRegister { at } => {
                write!(f, "+{at:#x}: indirect jump not through x16 or x17")
            }
            Finding::IndirectTargetNotProven { at } => {
                write!(f, "+{at:#x}: indirect branch target not proven")
            }
        }
    }
}

/// The zero-text-relocation check of unit text (H34 Rules "Unit text has 0 relocations").
///
/// | Rule | AArch64 | x86-64 |
/// |---|---|---|
/// | each field is inside the text (else [`Finding::FieldOutsideText`], and no other rule reads it) | yes | yes |
/// | each field is one instruction word: a 4-byte aligned offset with `insn = at` (else [`Finding::MisalignedField`], and no other rule reads it) | yes | - |
/// | no field is absolute ([`Form::Abs64`]) | yes | yes |
/// | each pc-relative field reaches a byte of this function's text | yes | yes |
/// | each [`Form::X64Disp32`] field is the disp32 of `mov r, [reg + disp32]`, decoded forward from the field's instruction start ([`Field::insn`]): an optional REX byte, opcode `8B`, ModRM with mod = 10, a SIB byte if rm = 100, then the disp32 exactly at the field. [`crate::Assembler::x64`] records a chunk start only at a known instruction boundary (else `insn = at`, which fails this rule); the decode of the whole text (below) checks that `insn` is an instruction start | - | yes |
/// | the whole text decodes from offset 0 with the length decoder (route UNIT-X64-DECODE; else [`Finding::Undecodable`] where it stops) | - | yes |
/// | each field's `insn` is an instruction start of that decode (else [`Finding::NotInstructionStart`]); each [`Form::X64Rel32`] field is the rel32 of the `call`, `jmp` or `jcc` at `insn`, or the disp32 of a RIP-relative operand at the end of that instruction (else [`Finding::FieldNotOperand`]) | - | yes |
/// | each pc-relative branch (rel8, rel32) and RIP-relative operand of the decode reaches a byte of the text, with or without a field (else [`Finding::OutsideFunction`]); each branch target is an instruction start (else [`Finding::BranchIntoInstruction`]) | - | yes |
/// | no absolute memory operand (`moffs`; SIB with no base and mod = 00; else [`Finding::AbsoluteAddress`]) | - | yes |
/// | no `adrp`, no `bl` | yes | - |
/// | each `b`, `b.cond`, `bc.cond` (FEAT_HBC), `cbz`/`cbnz`, `cb<cc>`/`cbb<cc>`/`cbh<cc>` (FEAT_CMPBR, register and immediate forms), `tbz`/`tbnz`, `adr` and `ldr` (literal) word reaches a byte of this function's text, with or without a field | yes | - |
///
/// The AArch64 rules scan every word, so hand-written code with no relocation record is
/// checked too (the spike-b `unit/scan.rs` rules). Since M8-P2 the x86-64 rules decode the
/// whole final text (after all field writes) from offset 0 with a hand-written length
/// decoder (route UNIT-X64-DECODE; `src/x64.rs`, lengths checked against llvm-mc): text
/// with no record is checked too, and a record's start is not trusted. The decoder refuses
/// what it does not know (VEX, EVEX and XOP forms; opcodes invalid in 64-bit mode; a REX
/// byte not right before the opcode), so such text is a finding. The decode does not know
/// what an 8-byte immediate (`movabs`) holds: an absolute value there is found only as an
/// [`Form::Abs64`] field. [`UnitEnv`] gives no address to write.
pub fn check_text(arch: Arch, text: &[u8], fields: &[Field]) -> Vec<Finding> {
    let len = text.len() as i64;
    let inside = |t: i64| (0..len).contains(&t);
    let mut out = Vec::new();
    for f in fields {
        // `fields` can come from outside (a check of a unit read back): a field past the end
        // is a finding, not a panic.
        let end = (f.at as usize).checked_add(f.form.size());
        if end.is_none_or(|end| end > text.len()) {
            out.push(Finding::FieldOutsideText { at: f.at });
            continue;
        }
        // An AArch64 field is one instruction word: the rules below and the register-load
        // check read the word at the field.
        if f.form.arch() == Some(Arch::A64) && (f.at % 4 != 0 || f.insn != f.at) {
            out.push(Finding::MisalignedField { at: f.at });
            continue;
        }
        match f.form.class() {
            FormClass::Absolute => out.push(Finding::AbsoluteField { at: f.at }),
            FormClass::PcRelative => {
                // In i64: `at + pc_base` cannot overflow.
                let target = i64::from(f.at)
                    + i64::from(f.form.pc_base())
                    + f.form.read(text, f.at as usize);
                if !inside(target) {
                    out.push(Finding::OutsideFunction { at: f.at, target });
                }
            }
            FormClass::RegisterRelative => {
                if f.form == Form::X64Disp32 && x64_load(text, f.insn, f.at).is_none() {
                    out.push(Finding::NotRegisterRelative { at: f.at });
                }
            }
        }
    }
    if arch == Arch::A64 {
        if !text.len().is_multiple_of(4) {
            out.push(Finding::Misaligned {
                len: text.len() as u32,
            });
        }
        for at in (0..text.len() / 4 * 4).step_by(4) {
            let w = word(text, at);
            let at32 = at as u32;
            let distance = if is_adrp(w) {
                out.push(Finding::Adrp { at: at32 });
                None
            } else if is_bl(w) {
                out.push(Finding::DirectCall { at: at32 });
                None
            } else if is_b_or_bl(w) {
                Some(sext(w & 0x03ff_ffff, 26) * 4)
            } else if is_b_cond(w) || is_cbz(w) || is_ldr_literal(w) {
                Some(sext((w >> 5) & 0x7_ffff, 19) * 4)
            } else if is_tbz(w) {
                Some(sext((w >> 5) & 0x3fff, 14) * 4)
            } else if is_cb_cc(w) {
                Some(sext((w >> 5) & 0x1ff, 9) * 4)
            } else if is_adr(w) {
                Some(sext((((w >> 5) & 0x7_ffff) << 2) | ((w >> 29) & 3), 21))
            } else {
                None
            };
            if let Some(d) = distance
                && !inside(at as i64 + d)
            {
                let finding = Finding::OutsideFunction {
                    at: at32,
                    target: at as i64 + d,
                };
                if !out.contains(&finding) {
                    out.push(finding);
                }
            }
        }
    }
    if arch == Arch::X64 {
        for finding in x64_scan(text, fields) {
            if !out.contains(&finding) {
                out.push(finding);
            }
        }
    }
    out
}

/// The decode of the whole x86-64 text from offset 0 (route UNIT-X64-DECODE), for
/// [`check_text`]: the field findings first ([`Finding::NotInstructionStart`],
/// [`Finding::FieldNotOperand`]), then the text findings in offset order. A field whose
/// instruction start is past an [`Finding::Undecodable`] point is not read, and a branch
/// target past it is not checked for a start.
fn x64_scan(text: &[u8], fields: &[Field]) -> Vec<Finding> {
    let len = text.len();
    let mut starts = vec![false; len];
    // (field offset, target, is a branch)
    let mut refs = Vec::new();
    let mut found = Vec::new();
    let mut at = 0;
    let mut end = len;
    while at < len {
        let Some(i) = x64::decode(text, at) else {
            found.push(Finding::Undecodable { at: at as u32 });
            end = at;
            break;
        };
        starts[at] = true;
        let next = at + usize::from(i.len);
        if let Some((off, size)) = i.rel {
            let f = at + usize::from(off);
            let v = if size == 1 {
                i64::from(text[f] as i8)
            } else {
                i64::from(word(text, f) as i32)
            };
            refs.push((f, next as i64 + v, true));
        }
        if let Some(off) = i.rip {
            let f = at + usize::from(off);
            refs.push((f, next as i64 + i64::from(word(text, f) as i32), false));
        }
        if i.abs {
            found.push(Finding::AbsoluteAddress { at: at as u32 });
        }
        at = next;
    }
    for (f, target, branch) in refs {
        let at = f as u32;
        if !(0..len as i64).contains(&target) {
            found.push(Finding::OutsideFunction { at, target });
        } else if branch && (target as usize) < end && !starts[target as usize] {
            found.push(Finding::BranchIntoInstruction { at, target });
        }
    }
    found.sort_by_key(|f| match *f {
        Finding::Undecodable { at }
        | Finding::AbsoluteAddress { at }
        | Finding::OutsideFunction { at, .. }
        | Finding::BranchIntoInstruction { at, .. } => at,
        _ => u32::MAX,
    });
    let mut out = Vec::new();
    for f in fields {
        if !matches!(f.form, Form::X64Rel32 | Form::X64Disp32) {
            continue;
        }
        let (insn, at) = (f.insn as usize, f.at as usize);
        // A field outside the text is FieldOutsideText; one past the decoded part is not read.
        if at.checked_add(4).is_none_or(|e| e > len) || insn >= end {
            continue;
        }
        if !starts[insn] {
            out.push(Finding::NotInstructionStart { at: f.at });
            continue;
        }
        if f.form == Form::X64Rel32 {
            let i = x64::decode(text, insn).expect("decoded by the scan");
            let off = at.checked_sub(insn).and_then(|o| u8::try_from(o).ok());
            let ok =
                off.is_some_and(|o| i.rel == Some((o, 4)) || (i.rip == Some(o) && o + 4 == i.len));
            if !ok {
                out.push(Finding::FieldNotOperand { at: f.at });
            }
        }
    }
    out.extend(found);
    out
}

/// The unit-data register (H34-Q1, H35-Q9) on AArch64: x27.
const UNIT_DATA_A64: u32 = 27;

/// The unit-data register on x86-64: r14 (H34-Q1, H35-Q9).
const UNIT_DATA_X64: u8 = 14;

/// The isolate register on AArch64: x28 (M2-JIT-ABI, H35).
const ISOLATE_A64: u32 = 28;

/// The isolate register on x86-64: r15 (M2-JIT-ABI, H35).
const ISOLATE_X64: u8 = 15;

/// The register-load check: each field of a register kind is the offset of a 64-bit load
/// through the register of its kind, or a finding (or [`Finding::FieldOutsideText`]).
///
/// | Kind | AArch64 | x86-64 |
/// |---|---|---|
/// | LinkSlot, IcCell, Stub (the RW words, H34-Q1) | the field is a 4-byte aligned word with `insn = at`, and the word is `ldr Xt, [x27, #imm]` (LDR X, unsigned offset, Rn = 27): exact, the word is the instruction; else [`Finding::NotUnitDataLoad`] | the instruction decoded forward from [`Field::insn`] (as in [`check_text`]) is `mov r64, [r14 + disp32]`: REX.W = 1, base r14 (in ModRM, or in a SIB byte with no index), the disp32 at the field; else [`Finding::NotUnitDataLoad`] |
/// | Runtime, RootIndex, DeoptExit (the isolate rows) | the same word rule with Rn = 28: `ldr Xt, [x28, #imm]` (since M7-P4, route A64-ISOLATE-LOAD); else [`Finding::NotIsolateLoad`] | the same decode, with base r15: `mov r64, [r15 + disp32]`; else [`Finding::NotIsolateLoad`] |
/// | Local, HeapConst | - | - |
///
/// One instruction is decoded per field, from the start that the record gives; [`check_text`]
/// checks that this start is an instruction start of the decode of the whole text
/// ([`Finding::NotInstructionStart`]), and `finalize_unit` runs both checks.
pub fn check_register_loads(arch: Arch, text: &[u8], fields: &[Field]) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in fields {
        let (a64_reg, x64_reg, finding) = match f.kind {
            RelocKind::LinkSlot | RelocKind::IcCell | RelocKind::Stub => (
                UNIT_DATA_A64,
                UNIT_DATA_X64,
                Finding::NotUnitDataLoad { at: f.at },
            ),
            RelocKind::Runtime | RelocKind::RootIndex | RelocKind::DeoptExit => (
                ISOLATE_A64,
                ISOLATE_X64,
                Finding::NotIsolateLoad { at: f.at },
            ),
            RelocKind::Local | RelocKind::HeapConst => continue,
        };
        let at = f.at as usize;
        if at.checked_add(4).is_none_or(|end| end > text.len()) {
            out.push(Finding::FieldOutsideText { at: f.at });
            continue;
        }
        let ok = match arch {
            // One aligned word (as in check_text): LDR X, unsigned offset (any imm12, any Rt),
            // with Rn = x27 (the RW words) or x28 (the isolate rows).
            Arch::A64 => {
                f.at % 4 == 0
                    && f.insn == f.at
                    && word(text, at) & 0xffc0_03e0 == 0xf940_0000 | (a64_reg << 5)
            }
            // Any destination (REX.R free).
            Arch::X64 => {
                x64_load(text, f.insn, f.at).is_some_and(|l| l.w && l.base == x64_reg && !l.index)
            }
        };
        if !ok {
            out.push(finding);
        }
    }
    out
}

/// [`check_text`], [`check_register_loads`] and, for a unit with landing pads,
/// [`check_pads`] of a finished unit: empty for every
/// [`UnitCode`] (`finalize_unit` checks its output), so a test can state the rules on each
/// unit it emits. It reads the fields that `finalize_unit` took, with the same `insn`, and
/// decodes the whole x86-64 text as [`check_text`] does.
pub fn zero_text_relocations(code: &UnitCode) -> Result<(), Vec<Finding>> {
    let mut v = check_text(code.arch, &code.text, &code.fields);
    v.extend(check_register_loads(code.arch, &code.text, &code.fields));
    if code.pads {
        v.extend(check_pads(code.arch, &code.text));
    }
    if v.is_empty() { Ok(()) } else { Err(v) }
}

/// AArch64 `bti c`: the landing pad that [`crate::Assembler::pad`] writes at an indirect
/// entry (MC-w23-15).
pub const A64_BTI_C: u32 = 0xd503_245f;

/// AArch64 `bti jc`: a landing pad for calls and jumps.
const A64_BTI_JC: u32 = 0xd503_24df;

/// The landing-pad check of the text of one function of a unit with landing pads (the
/// container compat flag `has_pads`, MC-w23-15). A loader maps such text with BTI
/// (`PROT_BTI`): an indirect branch to a word that is not a landing pad of its kind faults.
///
/// | Rule | AArch64 | x86-64 |
/// |---|---|---|
/// | the function entry (offset 0) is `bti c` or `bti jc`: the target of `blr` (a call) and of `br x16`/`br x17` (a tail call) from other code (else [`Finding::NoLandingPad`]) | yes | - |
/// | each indirect jump (`br`, `braa`, `brab`, `braaz`, `brabz`) goes through x16 or x17, so its target needs only `bti c` (a jump through another register needs `bti j`; else [`Finding::IndirectJumpRegister`]) | yes | - |
/// | each `adr` with a target in the text, into any register, makes an indirect entry: the word there is `bti c` or `bti jc` (else [`Finding::NoLandingPad`] at the target; a target outside the text is a [`check_text`] finding) | yes | - |
/// | each `br` and `blr` (and their PAC forms) takes its register from an allowed form, set in the same straight-line run before it (else [`Finding::IndirectTargetNotProven`]; route PADS-PROVENANCE): a 64-bit load through x27 or x28 (a link cell, a stub cell or a RuntimeTable entry: code outside the text, whose own entry rule holds there), only when no word of the text may write that register (the pinning premise, below), a 64-bit load through a register that such a load set in its own run (one step: a table that the isolate names; not an `adr` base, which reads text bytes), or an `adr` into the branch register itself (the rule above checks its target) | yes | - |
///
/// The run of a branch goes back from it to its definition over words that write neither the
/// register nor the flow of control: loads and pairs into other registers (a pair from the
/// stack, `ldp x29, x30`, pops a frame), stores, `add`/`sub` (immediate), `mov` (wide
/// immediate or register) and `nop` into other registers. A run stops at the function entry,
/// at the target of a direct branch in the text and at a landing pad (another way in), and at
/// any other word (a call, a branch, a write of the register). So a text address made any
/// other way is refused: arithmetic on an `adr` result, x30 after a `blr` (a return address),
/// a load from the stack (a saved x30) and a literal load (text bytes). `ret` is not a BTI
/// branch and may use any register. x86-64 has no pad form here yet (CET IBT `endbr64` is not
/// built): the check gives no finding.
///
/// The pinning premise (JIT review M8-P3 F3): a load through x27 or x28 reads engine memory
/// only while x27 holds the unit's RW copy and x28 the `Cx`. The check reads every word of the
/// text, and a write of x27 (x28) anywhere, also in another run or after the branch, takes
/// the proof away from every load through x27 (x28). The H35-Q9 unit-data entry
/// (`ldr x27, [x28, #ud]`, `ldr x27, [x27, #8 * slot]`) and the restore of x27 before an exit
/// write x27, so a text with them gets no proof through x27 now: the allowed write forms come
/// with the production wiring (route UNIT-PADS-WIRING).
pub fn check_pads(arch: Arch, text: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    if arch != Arch::A64 {
        return out;
    }
    if text.len() < 4 || !matches!(word(text, 0), A64_BTI_C | A64_BTI_JC) {
        out.push(Finding::NoLandingPad { at: 0 });
    }
    let starts = run_starts(text);
    let words = (0..text.len() / 4).map(|i| word(text, i * 4));
    let pinned = [27, 28].map(|r| !words.clone().any(|w| writes_pinned(w, r)));
    for at in (0..text.len() / 4 * 4).step_by(4) {
        let w = word(text, at);
        // Unconditional branch (register), op2 = 11111: opc 0000 (BR, BRAAZ, BRABZ), 0001
        // (BLR, BLRAAZ, BLRABZ), 1000 (BRAA, BRAB), 1001 (BLRAA, BLRAB). RET (0010) is not a
        // BTI branch.
        let opc = (w >> 21) & 0xf;
        let branch = w & 0xfe1f_0000 == 0xd61f_0000 && matches!(opc, 0 | 1 | 8 | 9);
        let reg = (w >> 5) & 31;
        if branch && matches!(opc, 0 | 8) && !matches!(reg, 16 | 17) {
            out.push(Finding::IndirectJumpRegister { at: at as u32 });
        }
        if branch && !proven_target(text, &starts, pinned, at, reg, 1, true) {
            out.push(Finding::IndirectTargetNotProven { at: at as u32 });
        }
        // `adr Xn, L` then `br x16` or `blr Xn`: L is entered by an indirect branch (JIT
        // review M8-P2 B2; Codex M8-P2 r2: `adr x0, L; blr x0`).
        if is_adr(w) {
            let target = at as i64 + sext((((w >> 5) & 0x7_ffff) << 2) | ((w >> 29) & 3), 21);
            if (0..text.len() as i64).contains(&target) {
                let t = target as usize;
                let pad = t.is_multiple_of(4)
                    && t + 4 <= text.len()
                    && matches!(word(text, t), A64_BTI_C | A64_BTI_JC);
                let finding = Finding::NoLandingPad { at: t as u32 };
                if !pad && !out.contains(&finding) {
                    out.push(finding);
                }
            }
        }
    }
    out
}

/// The words of `text` (by word index) where a straight-line run starts in [`check_pads`]:
/// the entry, each target of a direct branch in the text and each BTI landing pad.
fn run_starts(text: &[u8]) -> Vec<bool> {
    let n = text.len() / 4;
    let mut starts = vec![false; n];
    if n > 0 {
        starts[0] = true;
    }
    for i in 0..n {
        let w = word(text, i * 4);
        // BTI (any target kind): HINT #32, #34, #36, #38.
        if w & 0xffff_ff3f == 0xd503_241f {
            starts[i] = true;
        }
        let distance = if is_b_or_bl(w) {
            Some(sext(w & 0x03ff_ffff, 26) * 4)
        } else if is_b_cond(w) || is_cbz(w) {
            Some(sext((w >> 5) & 0x7_ffff, 19) * 4)
        } else if is_tbz(w) {
            Some(sext((w >> 5) & 0x3fff, 14) * 4)
        } else if is_cb_cc(w) {
            Some(sext((w >> 5) & 0x1ff, 9) * 4)
        } else {
            None
        };
        if let Some(d) = distance {
            let t = (i * 4) as i64 + d;
            if (0..(n * 4) as i64).contains(&t) {
                starts[t as usize / 4] = true;
            }
        }
    }
    starts
}

/// True when the value of register `reg` at the AArch64 word `at` is proven by an allowed form
/// in the straight-line run before `at` (the table of [`check_pads`]). `chain` is how many
/// loads through a register that is itself proven may still follow (1 at a branch). `adr_ok`
/// is true only for the branch register: an `adr` base of a load is a text address, so the
/// load reads text bytes (JIT review M8-P3 F2). `pinned` is true for x27 (x28) when no word
/// of the text may write it.
fn proven_target(
    text: &[u8],
    starts: &[bool],
    pinned: [bool; 2],
    at: usize,
    reg: u32,
    chain: u32,
    adr_ok: bool,
) -> bool {
    let mut q = at;
    loop {
        if starts[q / 4] || q < 4 {
            return false;
        }
        let p = q - 4;
        let w = word(text, p);
        let rd = w & 31;
        if is_adr(w) && rd == reg {
            let target = p as i64 + sext((((w >> 5) & 0x7_ffff) << 2) | ((w >> 29) & 3), 21);
            return adr_ok && (0..text.len() as i64).contains(&target);
        }
        // LDR (immediate), 64-bit, unsigned offset.
        if w & 0xffc0_0000 == 0xf940_0000 {
            if rd != reg {
                q = p;
                continue;
            }
            let base = (w >> 5) & 31;
            return (base == 27 && pinned[0])
                || (base == 28 && pinned[1])
                || (chain > 0
                    && base != 31
                    && proven_target(text, starts, pinned, p, base, chain - 1, false));
        }
        let neutral = match w & 0xffc0_0000 {
            // LDP, 64-bit: post-index, pre-index, signed offset.
            0xa8c0_0000 | 0xa9c0_0000 | 0xa940_0000 => {
                let (t2, base) = ((w >> 10) & 31, (w >> 5) & 31);
                rd != reg && t2 != reg && base != reg
            }
            // STP, 64-bit (the same forms); STR (immediate), 64-bit, unsigned offset.
            0xa880_0000 | 0xa980_0000 => (w >> 5) & 31 != reg,
            0xa900_0000 | 0xf900_0000 => true,
            _ => {
                // ADD, ADDS, SUB, SUBS (immediate); MOVN, MOVZ, MOVK; ORR (shifted register)
                // with Rd != the register (Rd 31 is sp or xzr); NOP.
                let add_imm = w & 0x1f80_0000 == 0x1100_0000;
                let mov_wide = w & 0x1f80_0000 == 0x1280_0000 && (w >> 29) & 3 != 1;
                let orr = w & 0x7f20_0000 == 0x2a00_0000;
                ((add_imm || mov_wide || orr) && rd != reg) || w == 0xd503_201f
            }
        };
        if !neutral {
            return false;
        }
        q = p;
    }
}

/// True when the AArch64 word `w` may write the general register `r` (x27 or x28 in
/// [`check_pads`]). It is sound by excess: a form that it does not know in detail counts as a
/// write when `r` is in any register field of the word. Exact forms (Arm ARM, A64 encoding
/// index): data processing (immediate and register) writes Rd; a load writes Rt (a pair Rt
/// and Rt2) and a pre- or post-index form writes back its base; a store of `r` and a load into
/// an FP or SIMD register are not writes; MRS and SYSL write Rt; of the FP and SIMD words, only
/// the moves and conversions to a general register (`fmov`, `fcvt*`, `umov`, `smov`) write Rd.
/// A call (`bl`, `blr`) writes x30 only (x27 and x28 are callee-saved).
fn writes_pinned(w: u32, r: u32) -> bool {
    let (rt, rn, rt2, rs) = (w & 31, (w >> 5) & 31, (w >> 10) & 31, (w >> 16) & 31);
    let op0 = (w >> 25) & 0xf;
    // Data processing, immediate (100x) and register (x101).
    if op0 & 0b1110 == 0b1000 || op0 & 0b0111 == 0b0101 {
        return rt == r;
    }
    // Loads and stores (x1x0).
    if op0 & 0b0101 == 0b0100 {
        let gpr = w & (1 << 26) == 0;
        let load = (w >> 22) & 3 != 0;
        return if w & 0x3b00_0000 == 0x3900_0000 {
            // LDR/STR (immediate, unsigned offset): no writeback.
            gpr && load && rt == r
        } else if w & 0x3b20_0000 == 0x3800_0000 {
            // LDUR/STUR, LDTR/STTR, and the post-index (01) and pre-index (11) forms.
            (gpr && load && rt == r) || ((w >> 10) & 1 == 1 && rn == r)
        } else if w & 0x3b20_0c00 == 0x3820_0800 {
            // LDR/STR (register offset): no writeback.
            gpr && load && rt == r
        } else if w & 0x3a00_0000 == 0x2800_0000 {
            // LDP/STP, LDNP/STNP: L (bit 22) loads Rt and Rt2; index 01 and 11 write back.
            (gpr && (w >> 22) & 1 == 1 && (rt == r || rt2 == r)) || ((w >> 23) & 1 == 1 && rn == r)
        } else if w & 0x3b00_0000 == 0x1800_0000 {
            // LDR (literal).
            gpr && rt == r
        } else {
            // Exclusives, atomics, acquire and release forms, structure loads and the rest.
            [rt, rt2, rs, rn].contains(&r)
        };
    }
    // Branches, exception generation and system (101x): MRS and SYSL write Rt.
    if op0 & 0b1110 == 0b1010 {
        return w & 0xffe0_0000 == 0xd520_0000 && rt == r;
    }
    // FP and SIMD (x111).
    if op0 & 0b0111 == 0b0111 {
        let opcode = (w >> 16) & 7;
        return rt == r
            && if w & 0x5f20_fc00 == 0x1e20_0000 {
                // FP and integer conversion: all but SCVTF, UCVTF and FMOV to an FP register.
                !matches!(opcode, 2 | 3 | 7)
            } else if w & 0x5f20_0000 == 0x1e00_0000 {
                // FP and fixed-point conversion: FCVTZS, FCVTZU.
                !matches!(opcode, 2 | 3)
            } else {
                // SMOV, UMOV.
                w & 0xbfe0_ec00 == 0x0e00_2c00
            };
    }
    // SME, SVE and the unallocated groups (00xx): any Rd.
    rt == r
}

#[cfg(test)]
mod tests {
    use super::writes_pinned;

    /// Each word with its text, encoded by `llvm-mc -triple=aarch64 -mattr=+lse,+rcpc`, and
    /// whether it writes x27 or x28.
    #[test]
    fn writes_pinned_knows_the_a64_write_forms() {
        let writes: [(u32, &str); 45] = [
            (0xaa00_03fb, "mov x27, x0"),
            (0x9100_03fb, "mov x27, sp"),
            (0x9100_237b, "add x27, x27, #8"),
            (0xf940_001b, "ldr x27, [x0]"),
            (0xf940_0b9b, "ldr x27, [x28, #16]"),
            (0xb940_041b, "ldr w27, [x0, #4]"),
            (0xb980_001b, "ldrsw x27, [x0]"),
            (0x3940_001b, "ldrb w27, [x0]"),
            (0xf940_13fb, "ldr x27, [sp, #32]"),
            (0xf85f_801b, "ldur x27, [x0, #-8]"),
            (0xf840_8760, "ldr x0, [x27], #8"),
            (0xf840_8f60, "ldr x0, [x27, #8]!"),
            (0xf800_8f60, "str x0, [x27, #8]!"),
            (0xf801_0760, "str x0, [x27], #16"),
            (0xf861_781b, "ldr x27, [x0, x1, lsl #3]"),
            (0xa941_6ffa, "ldp x26, x27, [sp, #16]"),
            (0xa8c1_73fb, "ldp x27, x28, [sp], #16"),
            (0xa8c1_0760, "ldp x0, x1, [x27], #16"),
            (0xa9bf_0760, "stp x0, x1, [x27, #-16]!"),
            (0x5800_005b, "ldr x27, #8"),
            (0xd53b_d05b, "mrs x27, tpidr_el0"),
            (0x0e0c_3c1b, "umov w27, v0.s[1]"),
            (0x4e06_2c1b, "smov x27, v0.h[1]"),
            (0x9e66_001b, "fmov x27, d0"),
            (0x9e78_001b, "fcvtzs x27, d0"),
            (0x9e58_f41b, "fcvtzs x27, d0, #3"),
            (0x9a81_001b, "csel x27, x0, x1, eq"),
            (0x9b01_081b, "madd x27, x0, x1, x2"),
            (0xd280_003b, "mov x27, #1"),
            (0xf2a0_003b, "movk x27, #1, lsl #16"),
            (0x1000_005b, "adr x27, #8"),
            (0xb000_001b, "adrp x27, #4096"),
            (0xc85f_7c1b, "ldxr x27, [x0]"),
            (0xc81b_7c20, "stxr w27, x0, [x1]"),
            (0xc85f_fc1b, "ldaxr x27, [x0]"),
            (0xc8bb_7c20, "cas x27, x0, [x1]"),
            (0xf820_803b, "swp x0, x27, [x1]"),
            (0xf820_003b, "ldadd x0, x27, [x1]"),
            (0xc8df_fc1b, "ldar x27, [x0]"),
            (0xf8bf_c01b, "ldapr x27, [x0]"),
            (0x4cdf_7360, "ld1 {v0.16b}, [x27], #16"),
            (0xd343_181b, "ubfx x27, x0, #3, #4"),
            (0xd37d_f01b, "lsl x27, x0, #3"),
            (0xcb00_03fb, "neg x27, x0"),
            (0xda80_141b, "cneg x27, x0, eq"),
        ];
        let reads: [(u32, &str); 26] = [
            (0xf81f_0ffb, "str x27, [sp, #-16]!"),
            (0xf900_13fb, "str x27, [sp, #32]"),
            (0xa901_73fb, "stp x27, x28, [sp, #16]"),
            (0xf940_0770, "ldr x16, [x27, #8]"),
            (0xf940_0390, "ldr x16, [x28]"),
            (0xa941_0760, "ldp x0, x1, [x27, #16]"),
            (0xf861_7b60, "ldr x0, [x27, x1, lsl #3]"),
            (0xf85f_8360, "ldur x0, [x27, #-8]"),
            (0x9100_0760, "add x0, x27, #1"),
            (0x8b1b_0020, "add x0, x1, x27"),
            (0xb400_005b, "cbz x27, #8"),
            (0x3618_005b, "tbz w27, #3, #8"),
            (0xeb00_037f, "cmp x27, x0"),
            (0xfd40_001b, "ldr d27, [x0]"),
            (0x3dc0_041c, "ldr q28, [x0, #16]"),
            (0x1e61_281b, "fadd d27, d0, d1"),
            (0x9e67_001b, "fmov d27, x0"),
            (0x9e62_001b, "scvtf d27, x0"),
            (0x4e0c_1c1b, "mov v27.s[1], w0"),
            (0x4e04_0c1b, "dup v27.4s, w0"),
            (0xd63f_0360, "blr x27"),
            (0xd61f_0200, "br x16"),
            (0xd51b_d05b, "msr tpidr_el0, x27"),
            (0xfa40_036b, "ccmp x27, x0, #11, eq"),
            (0xb900_001b, "str w27, [x0]"),
            (0x3900_001b, "strb w27, [x0]"),
        ];
        for (w, what) in writes {
            assert!(writes_pinned(w, 27) || writes_pinned(w, 28), "{what}");
        }
        for (w, what) in reads {
            assert!(!writes_pinned(w, 27) && !writes_pinned(w, 28), "{what}");
        }
    }
}
