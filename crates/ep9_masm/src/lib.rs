//! One instruction stream with abstract relocations, and its two finalizers (H34,
//! `docs/adr/H34-relocation-kinds.md`; 04-execution section 5).
//!
//! ```text
//!  emitter (month 7: hand-written test code; T1/T2 later) --Assembler--> CodeDesc
//!     text + Reloc { at, insn, form, code, index }   insn = instruction start; code = the RelocKind wire code (H34 order)
//!         |
//!         +-- finalize_jit  (`jit`) --> JitCode: absolute addresses allowed; exits; embedded HeapConst
//!         +-- finalize_unit (`aot`) --> UnitCode: zero text relocations (check_text); RW words; exits
//!                                       HeapConst -> FinalizeError::HeapConstInUnit
//! ```
//!
//! | H34 item | Here |
//! |---|---|
//! | the 8 kinds, in the H34 order and with their meaning; append-only | [`RelocKind`], [`KIND_COUNT`], `tests/golden/reloc-kinds.txt` |
//! | a new kind is handled or rejected by both finalizers | each finalizer matches every kind with no wildcard arm; an unknown code is `FinalizeError::UnknownKind` |
//! | the unit writer rejects HeapConst | `finalize_unit` gives `FinalizeError::HeapConstInUnit`; a `UnitCode` has no public constructor, so a writer that takes one gets no HeapConst (the ep9_unit writer is still empty) |
//! | zero text relocations | `finalize_unit` runs `check_text` on every unit it makes |
//! | LinkSlot, IcCell and the stub's link cell through the unit-data register (H34-Q1); on x86-64 also Runtime, RootIndex and DeoptExit through the isolate register | `finalize_unit` runs `check_register_loads` on every unit it makes |
//!
//! No tier code uses this crate in month 7 (plan v12 rule 71): the tests write their stub
//! code by hand. The crate has no dependencies (dep-guards check 21) and no `unsafe` code; it
//! holds no `Value` (it cannot name one). The jitless `ep9` never links it (dep-guards check 5).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(any(feature = "jit", feature = "aot"))]
mod fin;
#[cfg(feature = "jit")]
mod jit;
mod kind;
mod stream;
#[cfg(feature = "aot")]
mod unit;

#[cfg(any(feature = "jit", feature = "aot"))]
pub use fin::{Exit, FinalizeError};
#[cfg(feature = "jit")]
pub use jit::{JitCode, JitEnv, finalize_jit};
pub use kind::{KIND_COUNT, Label, RelocKind, Target};
pub use stream::{Arch, Assembler, CodeDesc, EmitError, FieldError, Form, FormClass, Reloc};
#[cfg(feature = "aot")]
pub use unit::{
    Field, Finding, RwUse, UnitCode, UnitEnv, check_register_loads, check_text, finalize_unit,
    zero_text_relocations,
};
