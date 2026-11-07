//! The Intl namespace (ECMA-402 §8; block 29): `%Intl%`, an ordinary object, the key module of
//! the lazy group "Intl" (INTL-I1-D8): its only `%GlobalObject%` property is `Intl`, and each
//! service module adds its constructor to `%Intl%`.
//!
//! ```text
//! globalThis.Intl {w, c}  %Intl%  [@@toStringTag] "Intl" {c},
//!                                 getCanonicalLocales (8.3.1), supportedValuesOf (8.3.2)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-intl.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 29;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("getCanonicalLocales", 1, get_canonical_locales), // 0 (8.3.1)
    BuiltinDef::method("supportedValuesOf", 1, supported_values_of),     // 1 (8.3.2)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Intl,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[
            // Intl [ @@toStringTag ] ({c}, 8.1.1).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::method(PropKey::Str("getCanonicalLocales"), 0),
            PropDef::method(PropKey::Str("supportedValuesOf"), 1),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[PropDef::intrinsic(
            PropKey::Str("Intl"),
            IntrinsicId::Intl,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "intl",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.getCanonicalLocales ( locales ) (8.3.1).
fn get_canonical_locales<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.getCanonicalLocales is not implemented"))
}

/// Intl.supportedValuesOf ( key ) (8.3.2).
fn supported_values_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.supportedValuesOf is not implemented"))
}
