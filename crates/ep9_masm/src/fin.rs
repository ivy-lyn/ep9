//! The parts that both finalizers share: the error type, the record check (an unknown kind
//! code refuses the whole stream) and the Local value.

use std::fmt;

use crate::kind::{Label, RelocKind};
use crate::stream::{CodeDesc, FieldError, Form, FormClass};

/// Why a finalizer refused a [`CodeDesc`]. A refusal gives no code at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinalizeError {
    /// The kind code is not a kind this build knows (at or above [`crate::KIND_COUNT`]).
    UnknownKind {
        /// The field offset.
        at: u32,
        /// The code.
        code: u8,
    },
    /// A HeapConst in unit text: the unit writer rejects it (H34; doc 13 P-R2).
    HeapConstInUnit {
        /// The field offset.
        at: u32,
        /// The constant index.
        index: u32,
    },
    /// This finalizer has no such form for the kind.
    FormNotAllowed {
        /// The field offset.
        at: u32,
        /// The kind.
        kind: RelocKind,
        /// The form.
        form: Form,
    },
    /// A form of the H34 table that is not built yet (the in-unit Stub call, H34-Q4).
    FormNotBuilt {
        /// The field offset.
        at: u32,
        /// The kind.
        kind: RelocKind,
        /// The form.
        form: Form,
    },
    /// The form is of another architecture than the stream.
    WrongArch {
        /// The field offset.
        at: u32,
    },
    /// The field is not inside the text.
    FieldOutOfText {
        /// The field offset.
        at: u32,
    },
    /// Two fields overlap.
    FieldOverlap {
        /// The offset of the second field.
        at: u32,
    },
    /// The value does not fit the field (out of reach, a bad scale or a wrong opcode).
    Field {
        /// The field offset.
        at: u32,
        /// The kind.
        kind: RelocKind,
        /// The reason.
        error: FieldError,
    },
    /// A Local names a label that is not bound, or not in the stream.
    UnboundLabel {
        /// The field offset.
        at: u32,
        /// The label.
        label: Label,
    },
    /// The environment has no value for the operand.
    Unresolved {
        /// The field offset.
        at: u32,
        /// The kind.
        kind: RelocKind,
        /// The operand.
        index: u32,
    },
    /// The unit text failed the zero-text-relocation check.
    #[cfg(feature = "aot")]
    TextRelocations(Vec<crate::unit::Finding>),
}

impl fmt::Display for FinalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FinalizeError::UnknownKind { at, code } => {
                write!(f, "+{at:#x}: unknown relocation kind code {code}")
            }
            FinalizeError::HeapConstInUnit { at, index } => {
                write!(f, "+{at:#x}: HeapConst({index}) in unit text")
            }
            FinalizeError::FormNotAllowed { at, kind, form } => {
                write!(f, "+{at:#x}: {} has no {form:?} form here", kind.name())
            }
            FinalizeError::FormNotBuilt { at, kind, form } => {
                write!(
                    f,
                    "+{at:#x}: the {} {form:?} form is not built",
                    kind.name()
                )
            }
            FinalizeError::WrongArch { at } => write!(f, "+{at:#x}: form of another arch"),
            FinalizeError::FieldOutOfText { at } => write!(f, "+{at:#x}: field out of the text"),
            FinalizeError::FieldOverlap { at } => write!(f, "+{at:#x}: fields overlap"),
            FinalizeError::Field { at, kind, error } => {
                write!(f, "+{at:#x}: {}: {error:?}", kind.name())
            }
            FinalizeError::UnboundLabel { at, label } => {
                write!(f, "+{at:#x}: label {} is not bound", label.0)
            }
            FinalizeError::Unresolved { at, kind, index } => {
                write!(f, "+{at:#x}: no value for {}({index})", kind.name())
            }
            #[cfg(feature = "aot")]
            FinalizeError::TextRelocations(v) => {
                write!(f, "{} text relocation finding(s)", v.len())?;
                for x in v {
                    write!(f, "; {x}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FinalizeError {}

/// One checked relocation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Site {
    pub at: u32,
    pub form: Form,
    pub kind: RelocKind,
    pub index: u32,
}

/// Checks every record of `desc` before any field is written: each kind code is known, each
/// form is of the stream's arch, each field is inside the text, and no two fields overlap.
/// The first failure refuses the whole stream.
pub(crate) fn sites(desc: &CodeDesc) -> Result<Vec<Site>, FinalizeError> {
    let mut out = Vec::with_capacity(desc.relocs.len());
    for r in &desc.relocs {
        let at = r.at;
        let kind = r
            .kind()
            .ok_or(FinalizeError::UnknownKind { at, code: r.code })?;
        if r.form.arch().is_some_and(|a| a != desc.arch) {
            return Err(FinalizeError::WrongArch { at });
        }
        if at as usize + r.form.size() > desc.text.len() {
            return Err(FinalizeError::FieldOutOfText { at });
        }
        out.push(Site {
            at,
            form: r.form,
            kind,
            index: r.index,
        });
    }
    let mut by_at: Vec<&Site> = out.iter().collect();
    by_at.sort_by_key(|s| s.at);
    for w in by_at.windows(2) {
        if w[0].at as usize + w[0].form.size() > w[1].at as usize {
            return Err(FinalizeError::FieldOverlap { at: w[1].at });
        }
    }
    Ok(out)
}

/// The field value of a Local: the distance from the field's pc base to the label. A Local
/// is resolved at emit in both finalizers (H34): the value does not depend on where the text
/// is placed.
pub(crate) fn local_value(desc: &CodeDesc, s: &Site) -> Result<i64, FinalizeError> {
    if s.form.class() != FormClass::PcRelative {
        return Err(FinalizeError::FormNotAllowed {
            at: s.at,
            kind: s.kind,
            form: s.form,
        });
    }
    let label = Label(s.index);
    let to = desc
        .labels
        .get(s.index as usize)
        .copied()
        .flatten()
        .ok_or(FinalizeError::UnboundLabel { at: s.at, label })?;
    Ok(to as i64 - (s.at + s.form.pc_base()) as i64)
}

/// Writes `value` into the field of `s`.
pub(crate) fn write(text: &mut [u8], s: &Site, value: i64) -> Result<(), FinalizeError> {
    s.form
        .write(text, s.at as usize, value)
        .map_err(|error| FinalizeError::Field {
            at: s.at,
            kind: s.kind,
            error,
        })
}

/// The error for a form that the finalizer has not for this kind.
pub(crate) fn not_allowed(s: &Site) -> FinalizeError {
    FinalizeError::FormNotAllowed {
        at: s.at,
        kind: s.kind,
        form: s.form,
    }
}

/// The error for an operand that the environment cannot resolve.
pub(crate) fn unresolved(s: &Site) -> FinalizeError {
    FinalizeError::Unresolved {
        at: s.at,
        kind: s.kind,
        index: s.index,
    }
}

/// Where a deopt exit is: the field offset and the exit id (the metadata is keyed by pc
/// offset, H29; the H29 reader of A4 maps the field to its pc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit {
    /// The field offset.
    pub at: u32,
    /// The exit id.
    pub id: u32,
}
