//! Proxy (§28.2): module `proxy`, block 15. Owner: M4-P4 PROXY1 (the `Proxy` constructor,
//! `Proxy.revocable`; `%Proxy%` is `IntrinsicId` 64).
//! Empty until the owner adds its definitions; M4-P0 declares the module and its block,
//! because `MODULES` is frozen after the scaffold.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-proxy.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 15;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "proxy",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
