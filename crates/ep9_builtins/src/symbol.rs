//! Symbol builtins (§20.4): module `symbol`, block 7. Targets `%Symbol%` (made here) and
//! `%Symbol.prototype%` (`IntrinsicId::Symbol`, `IntrinsicId::SymbolPrototype`).
//!
//! Week 4 (P3-BLTB): the `Symbol` function (`new Symbol()` throws), `Symbol.for`/`keyFor`
//! (`Cx::symbol_for`, `Cx::key_for`), the well-known symbol properties (`PropValue::Symbol`),
//! `Symbol.prototype.toString`/`valueOf`, the `description` getter (`PropValue::Accessor`).
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-symbol.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 7;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "symbol",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
