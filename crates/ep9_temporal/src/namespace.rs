//! The Temporal namespace (block 19): `%Temporal%` and `%Temporal.Now%`, both ordinary
//! objects (Temporal §1, §2). Each class module adds its constructor to `%Temporal%`; the
//! `Temporal.Now` functions come with WP T9.
//!
//! ```text
//! globalThis.Temporal {w, c}  %Temporal%      [@@toStringTag] "Temporal" {c}, Now {w, c}
//! Temporal.Now               %Temporal.Now%  [@@toStringTag] "Temporal.Now" {c}
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-temporal.txt`.

use ep9_vm::{
    BuiltinDef, BuiltinModule, ConstValue, IntrinsicDef, IntrinsicId, IntrinsicValue, ObjectProps,
    PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 19;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::Temporal,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalNow,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[
            // Temporal [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal"),
                PropAttrs::CONFIGURABLE,
            ),
            // Temporal.Now ({w, c}).
            PropDef::intrinsic(
                PropKey::Str("Now"),
                IntrinsicId::TemporalNow,
                PropAttrs::METHOD,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalNow,
        props: &[PropDef::constant(
            PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
            ConstValue::Str("Temporal.Now"),
            PropAttrs::CONFIGURABLE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[PropDef::intrinsic(
            PropKey::Str("Temporal"),
            IntrinsicId::Temporal,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "temporal",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
