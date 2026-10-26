//! RegExp (§22.2): module `regexp`, block 16. Owners: M4-P2 RX0A (the `%RegExp.prototype%`
//! intrinsic, an ordinary object first; V9-A9), then M4-P3 REGEXP1 (the constructor, the
//! prototype methods, `%RegExpStringIteratorPrototype%`; `IntrinsicId` 65-67). It reaches
//! `ep9_regexp` only through `Cx` part-2 methods (V9-D15).
//!
//! | Intrinsic | State |
//! |---|---|
//! | `%RegExp.prototype%` | Made (M4-P2 RX0A): an ordinary object whose prototype is `%Object.prototype%` (§22.2.6; not a RegExp instance), with no properties yet. RegExp literals take it as their prototype (`ep9_vm` regexp.rs) |
//! | `%RegExp%`, the prototype methods, `%RegExpStringIteratorPrototype%` | Not made (M4-P3 REGEXP1) |
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-regexp.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, IntrinsicId, IntrinsicValue, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 16;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    // §22.2.6: %RegExp.prototype% is an ordinary object (not a RegExp instance).
    IntrinsicDef {
        id: IntrinsicId::RegExpPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "regexp",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
