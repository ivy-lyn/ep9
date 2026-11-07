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
//! `finalize_unit` checks the base register of the 3 unit-data rows: each LinkSlot, IcCell and
//! Stub field must be in a 64-bit load through x27/r14 ([`check_unit_data_loads`]). It does
//! not check the base register of the isolate-register rows (the emitter picks it).
//!
//! [`UnitCode`] has no public constructor: only `finalize_unit` makes one, and it runs
//! [`check_text`] on its output, so a writer that takes a `UnitCode` gets only checked text
//! with no HeapConst. (The ep9_unit writer, behind its `aot` feature, is still empty.) These forms are the gated H34 parts "Unit finalize forms" (baseline-AOT at the A3
//! gate, optimized-AOT at the A4 gates).

use std::fmt;

use crate::fin::{self, Exit, FinalizeError, not_allowed, unresolved};
use crate::kind::RelocKind;
use crate::stream::{
    Arch, CodeDesc, Form, FormClass, is_adr, is_adrp, is_b_cond, is_b_or_bl, is_bl, is_cbz,
    is_ldr_literal, is_tbz, sext, word,
};

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

/// One written field of unit text, for [`check_text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// The field offset.
    pub at: u32,
    /// The form.
    pub form: Form,
}

/// The output of [`finalize_unit`]: position-independent text with zero text relocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitCode {
    arch: Arch,
    text: Vec<u8>,
    fields: Vec<Field>,
    rw: Vec<RwUse>,
    exits: Vec<Exit>,
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
}

/// Finalizes `desc` as the unit text of one function.
///
/// Refuses the whole stream on the first problem: an unknown kind code, a HeapConst, an
/// absolute form, a form that the kind has not here, an operand that `env` cannot resolve, a
/// value that does not fit its field, or a finding of [`check_text`] or
/// [`check_unit_data_loads`] on the output.
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
            form: s.form,
        })
        .collect();
    let mut findings = check_text(desc.arch, &text, &fields);
    findings.extend(check_unit_data_loads(desc.arch, &text, &rw));
    if !findings.is_empty() {
        return Err(FinalizeError::TextRelocations(findings));
    }
    Ok(UnitCode {
        arch: desc.arch,
        text,
        fields,
        rw,
        exits,
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
    /// `[reg + disp32]` operand: no ModRM byte with mod = 10 before it (for example
    /// `[rip + disp32]`, a pc-relative field under a register-relative label).
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
    /// A LinkSlot, IcCell or Stub field ([`RwUse`]) that is not in a 64-bit load through the
    /// unit-data register (H34-Q1): see [`check_unit_data_loads`]. A store, or a load through
    /// another base register, reads or writes another cell.
    NotUnitDataLoad {
        /// The field offset.
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
        }
    }
}

/// The zero-text-relocation check of unit text (H34 Rules "Unit text has 0 relocations").
///
/// | Rule | AArch64 | x86-64 |
/// |---|---|---|
/// | each field is inside the text (else [`Finding::FieldOutsideText`], and no other rule reads it) | yes | yes |
/// | no field is absolute ([`Form::Abs64`]) | yes | yes |
/// | each pc-relative field reaches a byte of this function's text | yes | yes |
/// | each [`Form::X64Disp32`] field follows a ModRM byte with mod = 10 (`[reg + disp32]`) | - | yes |
/// | no `adrp`, no `bl` | yes | - |
/// | each `b`, `b.cond`, `cbz`/`cbnz`, `tbz`/`tbnz`, `adr` and `ldr` (literal) word reaches a byte of this function's text, with or without a field | yes | - |
///
/// The AArch64 rules scan every word, so hand-written code with no relocation record is
/// checked too (the spike-b `unit/scan.rs` rules). x86-64 has no decoder here, as in the
/// spike: its text gets the field rules, and [`UnitEnv`] gives no address to write.
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
                if f.form == Form::X64Disp32 && !x64_reg_disp32(text, f.at as usize) {
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
    out
}

/// True if the x86-64 disp32 field at `at` is the disp32 of a `[reg + disp32]` operand: the
/// byte before it is a ModRM byte with mod = 10 and rm != 100, or the byte before that is one
/// with rm = 100 (a SIB byte between them). Other forms with a disp32 are not register-relative:
/// mod = 00 with rm = 101 is `[rip + disp32]`, and mod = 00 with SIB base 101 has no base
/// register. No decoder: the bytes before the field are taken as its operand's ModRM (and SIB).
fn x64_reg_disp32(text: &[u8], at: usize) -> bool {
    let modrm = |i: Option<usize>| i.and_then(|i| text.get(i)).map(|&b| (b >> 6, b & 7));
    match modrm(at.checked_sub(1)) {
        Some((0b10, rm)) if rm != 0b100 => true,
        _ => modrm(at.checked_sub(2)) == Some((0b10, 0b100)),
    }
}

/// The unit-data register (H34-Q1, H35-Q9) on AArch64: x27.
const UNIT_DATA_A64: u32 = 27;

/// The unit-data load check: each [`RwUse`] field (LinkSlot, IcCell, the stub's link cell) is
/// the offset of a 64-bit load through the unit-data register (H34-Q1), or the finding
/// [`Finding::NotUnitDataLoad`] (or [`Finding::FieldOutsideText`]).
///
/// | Arch | The one form accepted |
/// |---|---|
/// | AArch64 | the field word is `ldr Xt, [x27, #imm]` (LDR X, unsigned offset, Rn = 27): exact, the word is the instruction |
/// | x86-64 | the 3 bytes before the field are REX (W = 1, B = 1), `8B`, ModRM (mod = 10, rm = 110): `mov r64, [r14 + disp32]`. No decoder: these bytes are read as the field's instruction, so a prefix before the REX byte is not seen |
pub fn check_unit_data_loads(arch: Arch, text: &[u8], rw: &[RwUse]) -> Vec<Finding> {
    let mut out = Vec::new();
    for u in rw {
        let at = u.at as usize;
        let ok = match arch {
            Arch::A64 => match at.checked_add(4).and_then(|end| text.get(at..end)) {
                Some(b) => {
                    let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    // LDR X, unsigned offset (any imm12, any Rt), with Rn = x27.
                    w & 0xffc0_03e0 == 0xf940_0000 | (UNIT_DATA_A64 << 5)
                }
                None => {
                    out.push(Finding::FieldOutsideText { at: u.at });
                    continue;
                }
            },
            Arch::X64 => {
                if at.checked_add(4).is_none_or(|end| end > text.len()) {
                    out.push(Finding::FieldOutsideText { at: u.at });
                    continue;
                }
                match at.checked_sub(3).map(|i| &text[i..at]) {
                    // REX.W + REX.B (REX.R free: any destination), MOV r64, r/m64, then
                    // mod = 10 and rm = 110 (with REX.B: r14), any reg.
                    Some(&[rex, op, modrm]) => {
                        rex & 0xf9 == 0x49 && op == 0x8b && modrm & 0xc7 == 0x86
                    }
                    _ => false,
                }
            }
        };
        if !ok {
            out.push(Finding::NotUnitDataLoad { at: u.at });
        }
    }
    out
}

/// [`check_text`] and [`check_unit_data_loads`] of a finished unit: empty for every
/// [`UnitCode`] (`finalize_unit` checks its output), so a test can state the rules on each
/// unit it emits.
pub fn zero_text_relocations(code: &UnitCode) -> Result<(), Vec<Finding>> {
    let mut v = check_text(code.arch, &code.text, &code.fields);
    v.extend(check_unit_data_loads(code.arch, &code.text, &code.rw));
    if v.is_empty() { Ok(()) } else { Err(v) }
}
