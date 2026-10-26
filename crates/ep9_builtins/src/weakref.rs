//! WeakRef and FinalizationRegistry (§26.1, §26.2): module `weakref`, block 17. Owner: M4-P4
//! WEAKREF (`IntrinsicId` 68-71; `ObjectClass::WeakRef` 11 and `FinalizationRegistry` 12).
//! Empty until the owner adds its definitions; M4-P0 declares the module and its block,
//! because `MODULES` is frozen after the scaffold.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-weakref.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 17;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "weakref",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
