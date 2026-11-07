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
/// value that does not fit its field, or a finding of [`check_text`] on the output.
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
    let findings = check_text(desc.arch, &text, &fields);
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
        }
    }
}

/// The zero-text-relocation check of unit text (H34 Rules "Unit text has 0 relocations").
///
/// | Rule | AArch64 | x86-64 |
/// |---|---|---|
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
        match f.form.class() {
            FormClass::Absolute => out.push(Finding::AbsoluteField { at: f.at }),
            FormClass::PcRelative => {
                let target = (f.at + f.form.pc_base()) as i64 + f.form.read(text, f.at as usize);
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

/// [`check_text`] of a finished unit: empty for every [`UnitCode`] (`finalize_unit` checks
/// its output), so a test can state the rule on each unit it emits.
pub fn zero_text_relocations(code: &UnitCode) -> Result<(), Vec<Finding>> {
    let v = check_text(code.arch, &code.text, &code.fields);
    if v.is_empty() { Ok(()) } else { Err(v) }
}
