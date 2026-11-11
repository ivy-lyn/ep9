#![allow(dead_code)] // m7-stub: INTL W2 PLURAL fills this module.
//! Intl.PluralRules (ECMA-402 §17; block 33): `%Intl.PluralRules%` and `%Intl.PluralRules.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.PluralRules {w, c}      %Intl.PluralRules%  prototype {}
//! Intl.PluralRules.prototype   %Intl.PluralRules.prototype%  constructor {w, c},
//!                                [@@toStringTag] "Intl.PluralRules" {c}
//! slots (ObjectClass::IntlPluralRules): 0 [[Locale]] (a String); PLURAL names the others
//! ICU4X: icu_plurals::PluralRules (and the `unstable` ranges) through crate::cache at the first select
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W2 NF-CORE skeleton): the constructor throws a TypeError. PLURAL fills
//! the constructor (§17.1.1, InitializePluralRules), `supportedLocalesOf` (17.2.2),
//! `select` (17.3.4), `selectRange` (17.3.5), `resolvedOptions` (17.3.3) (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plural_rules.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 33;

/// The class of an Intl.PluralRules object.
pub const CLASS: ObjectClass = ObjectClass::IntlPluralRules;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PluralRules", 0, constructor), // 0 (17.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlPluralRules,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlPluralRulesPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlPluralRules,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlPluralRulesPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlPluralRulesPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlPluralRules,
                PropAttrs::METHOD,
            ),
            // Intl.PluralRules.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.PluralRules"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("PluralRules"),
            IntrinsicId::IntlPluralRules,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plural_rules",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.PluralRules ( ... ) (17.1.1): a TypeError until PLURAL fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.PluralRules is not implemented"))
}
