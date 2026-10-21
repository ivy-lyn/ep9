//! Array builtins (§23.1): module `array`, block 3. Targets `%Array%` and `%Array.prototype%`
//! (`IntrinsicId::Array`, `IntrinsicId::ArrayPrototype`).
//!
//! The `Array` constructor and `Array.prototype.join`/`toString`/`push` are VM builtins (block 0).
//! Week 4 (P3-BLTA): `Array.isArray`, `Array.prototype.pop`, `indexOf`.
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-array.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 3;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "array",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
