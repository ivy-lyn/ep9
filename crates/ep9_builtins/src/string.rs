//! String builtins (§22.1): module `string`, block 6. Targets `%String%` (made here) and
//! `%String.prototype%` (`IntrinsicId::String`, `IntrinsicId::StringPrototype`).
//!
//! Week 4 (P3-BLTB): the `String` constructor (`String(sym)` through `Cx::descriptive_string`),
//! `String.fromCharCode`, `String.prototype.toString`/`valueOf`/`charCodeAt`, the global binding.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-string.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 6;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "string",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
