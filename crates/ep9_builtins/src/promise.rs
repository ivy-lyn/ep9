#![allow(dead_code)] // m2-stub
//! Promise (§27.2): module `promise`, block 10. Owner: P4-PROMISE. Empty until the owner adds its
//! definitions; M2-P0 declares the module and its block, because `MODULES` is frozen after
//! the scaffold.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-promise.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 10;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "promise",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
