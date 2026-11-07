#![allow(dead_code)] // m7-stub: INTL W1 LOCALE fills this module.
//! Intl.Locale (ECMA-402 §14; block 30): `%Intl.Locale%` and `%Intl.Locale.prototype%`, a
//! module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and its
//! own intrinsics.
//!
//! ```text
//! Intl.Locale {w, c}              %Intl.Locale%            prototype {}
//! Intl.Locale.prototype           %Intl.Locale.prototype%  constructor {w, c},
//!                                                          [@@toStringTag] "Intl.Locale" {c}
//! slots (ObjectClass::IntlLocale): 0 [[Locale]] (the canonical tag, a String); LOCALE names
//!                                  the others ([[Calendar]] .. [[Numeric]])
//! ```
//!
//! Stub (INTL W1 CORE): the constructor checks NewTarget and then throws a TypeError. LOCALE
//! fills the constructor (§14.1.1 steps 2-36), appends the getters and methods of §14.3 and
//! the Locale info of `locale_info.rs` (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-locale.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 30;

/// The class of an Intl.Locale object.
pub const CLASS: ObjectClass = ObjectClass::IntlLocale;

/// Slot 0: `[[Locale]]`, the canonical language tag (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Locale", 1, constructor), // 0 (14.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlLocale,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlLocalePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlLocale,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlLocalePrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlLocalePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlLocale,
                PropAttrs::METHOD,
            ),
            // Intl.Locale.prototype [ @@toStringTag ] ({c}, 14.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Locale"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Locale"),
            IntrinsicId::IntlLocale,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "locale",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.Locale ( tag [ , options ] ) (14.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    if new_target.as_object().is_none() {
        return Err(cx.throw_type_error("Constructor Intl.Locale requires 'new'"));
    }
    Err(cx.throw_type_error("Intl.Locale is not implemented"))
}
