//! One instruction stream with abstract relocations, and its two finalizers (H34,
//! `docs/adr/H34-relocation-kinds.md`; 04-execution section 5).
//!
//! ```text
//!  emitter (month 7: hand-written test code; T1/T2 later) --Assembler--> CodeDesc
//!     text + Reloc { at, form, code, index }      code = the RelocKind wire code (H34 order)
//! ```
//!
//! | H34 item | Here |
//! |---|---|
//! | the 8 kinds, in the H34 order and with their meaning; append-only | [`RelocKind`], [`KIND_COUNT`], `tests/golden/reloc-kinds.txt` |
//!
//! No tier code uses this crate in month 7 (plan v12 rule 71): the tests write their stub
//! code by hand. The crate has no dependencies (dep-guards check 21) and no `unsafe` code; it
//! holds no `Value` (it cannot name one). The jitless `ep9` never links it (dep-guards check 5).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod kind;
mod stream;

pub use kind::{KIND_COUNT, Label, RelocKind, Target};
pub use stream::{Arch, Assembler, CodeDesc, EmitError, FieldError, Form, FormClass, Reloc};
