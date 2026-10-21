//! Number builtins (§21.1): module `number`, block 4. Targets `%Number%` (made here:
//! `IntrinsicDef`) and `%Number.prototype%` (`IntrinsicId::Number`,
//! `IntrinsicId::NumberPrototype`).
//!
//! Week 4 (P3-BLTB): the `Number` constructor (with and without `new`), the constants,
//! `Number.prototype.toString`/`valueOf` (`Cx::this_primitive_value`), the global binding.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-number.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 4;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "number",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
