//! Object builtins (§20.1): module `object`, block 1. Targets `%Object%` and `%Object.prototype%`
//! (`IntrinsicId::Object`, `IntrinsicId::ObjectPrototype`).
//!
//! The `Object` constructor and `Object.prototype.toString`/`valueOf` are VM builtins (block 0).
//! Week 4 (P3-BLTA): `Object.prototype.hasOwnProperty`, `propertyIsEnumerable`,
//! `Object.defineProperty`, `getOwnPropertyDescriptor`, `getOwnPropertyNames`, `keys`.
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-object.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 1;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "object",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
