//! `finalize_jit` (feature `jit`): JIT code, where absolute addresses are allowed (H34).
//!
//! | Kind | Forms | Value |
//! |---|---|---|
//! | Local | every pc-relative form | the distance to the label (resolved at emit) |
//! | Stub | B/BL, rel32 | a direct call inside the code range: the distance to [`JitEnv::stub`] |
//! | Runtime | B/BL, rel32; Abs64; a register-relative form | a direct call; an indirect call through an immediate; or through the isolate table ([`JitEnv::runtime_slot`]) |
//! | LinkSlot | Abs64 | resolved at finalize: an immediate ([`JitEnv::link`]) |
//! | IcCell | Abs64 | the address of the FeedbackVector slot ([`JitEnv::ic_cell`]) |
//! | RootIndex | a register-relative form | a load from the isolate root table ([`JitEnv::root_slot`]) |
//! | DeoptExit | B/BL, rel32; Abs64 | a call of the shared entry ([`JitEnv::deopt_entry`]); the exit goes into [`JitCode::exits`] |
//! | HeapConst | Abs64 | the raw pointer ([`JitEnv::heap_const`]), embedded; it goes into [`JitCode::embedded`] |
//!
//! A call beyond the reach of its form is refused (`FieldError::Range`): the veneer is the open
//! JIT code-range part of H34-Q3 (milestone M5). The output is bytes: the W^X code space, the
//! dispatch-slot write and jitdump come with T1 (M5). These forms are the gated H34 part "JIT
//! finalize forms", frozen at M7 exit.

use crate::fin::{self, Exit, FinalizeError, Site, not_allowed, unresolved};
use crate::kind::RelocKind;
use crate::stream::{CodeDesc, FormClass};

/// What `finalize_jit` reads from the isolate and the code range.
pub trait JitEnv {
    /// The address where text byte 0 will be placed (inside the code range, H07).
    fn code_address(&self) -> u64;
    /// The entry address of a stub.
    fn stub(&self, id: u32) -> Option<u64>;
    /// The entry address of a runtime function (H20).
    fn runtime(&self, id: u32) -> Option<u64>;
    /// The offset of the RuntimeTable entry of a runtime function from the isolate register.
    fn runtime_slot(&self, id: u32) -> Option<i64>;
    /// The value of a link cell, resolved now.
    fn link(&self, index: u32) -> Option<u64>;
    /// The address of the FeedbackVector slot of an IC cell.
    fn ic_cell(&self, index: u32) -> Option<u64>;
    /// The offset of a root slot (H13) from the isolate register.
    fn root_slot(&self, index: u32) -> Option<i64>;
    /// The address of the shared deopt entry (H29).
    fn deopt_entry(&self) -> u64;
    /// A raw heap pointer. The code holds it weakly, with a dependency.
    fn heap_const(&self, index: u32) -> Option<u64>;
}

/// The output of [`finalize_jit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JitCode {
    /// The text with every field written, for [`JitEnv::code_address`].
    pub text: Vec<u8>,
    /// The deopt exits, in record order.
    pub exits: Vec<Exit>,
    /// The embedded HeapConst fields, in record order: (field offset, constant index). The
    /// installer must register each one as a weak source with a dependency before the code
    /// can run or a GC can see it (plan rule 8; H34 "held weakly"). Month 7 installs nothing.
    pub embedded: Vec<(u32, u32)>,
}

/// Finalizes `desc` as JIT code placed at `env.code_address()`.
///
/// Refuses the whole stream on the first problem, before or while it writes fields: an unknown
/// kind code, a form that the kind has not here, a field out of the text, an operand that `env`
/// cannot resolve, or a value that does not fit its field.
pub fn finalize_jit(desc: &CodeDesc, env: &dyn JitEnv) -> Result<JitCode, FinalizeError> {
    let sites = fin::sites(desc)?;
    let mut text = desc.text.clone();
    let mut exits = Vec::new();
    let mut embedded = Vec::new();
    let base = env.code_address();
    // The distance from the field's pc base to the absolute address `to`.
    let call = |s: &Site, to: u64| -> i64 {
        to.wrapping_sub(base.wrapping_add((s.at + s.form.pc_base()) as u64)) as i64
    };
    for s in &sites {
        let class = s.form.class();
        // Every kind has an arm, with no wildcard: a new kind does not build until this
        // finalizer handles or rejects it (H34 Freeze table).
        let value = match s.kind {
            RelocKind::Local => fin::local_value(desc, s)?,
            RelocKind::Stub => {
                if !s.form.is_call() {
                    return Err(not_allowed(s));
                }
                call(s, env.stub(s.index).ok_or_else(|| unresolved(s))?)
            }
            RelocKind::Runtime => match class {
                FormClass::PcRelative if s.form.is_call() => {
                    call(s, env.runtime(s.index).ok_or_else(|| unresolved(s))?)
                }
                FormClass::Absolute => env.runtime(s.index).ok_or_else(|| unresolved(s))? as i64,
                FormClass::RegisterRelative => {
                    env.runtime_slot(s.index).ok_or_else(|| unresolved(s))?
                }
                FormClass::PcRelative => return Err(not_allowed(s)),
            },
            RelocKind::LinkSlot => match class {
                FormClass::Absolute => env.link(s.index).ok_or_else(|| unresolved(s))? as i64,
                _ => return Err(not_allowed(s)),
            },
            RelocKind::IcCell => match class {
                FormClass::Absolute => env.ic_cell(s.index).ok_or_else(|| unresolved(s))? as i64,
                _ => return Err(not_allowed(s)),
            },
            RelocKind::RootIndex => match class {
                FormClass::RegisterRelative => {
                    env.root_slot(s.index).ok_or_else(|| unresolved(s))?
                }
                _ => return Err(not_allowed(s)),
            },
            RelocKind::DeoptExit => {
                exits.push(Exit {
                    at: s.at,
                    id: s.index,
                });
                match class {
                    FormClass::PcRelative if s.form.is_call() => call(s, env.deopt_entry()),
                    FormClass::Absolute => env.deopt_entry() as i64,
                    _ => return Err(not_allowed(s)),
                }
            }
            RelocKind::HeapConst => match class {
                FormClass::Absolute => {
                    embedded.push((s.at, s.index));
                    env.heap_const(s.index).ok_or_else(|| unresolved(s))? as i64
                }
                _ => return Err(not_allowed(s)),
            },
        };
        fin::write(&mut text, s, value)?;
    }
    Ok(JitCode {
        text,
        exits,
        embedded,
    })
}
