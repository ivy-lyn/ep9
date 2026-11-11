#![allow(dead_code)] // m7-stub: INTL W2 NF-CORE fills this module.
//! Intl.NumberFormat (ECMA-402 §16; block 34): `%Intl.NumberFormat%` and `%Intl.NumberFormat.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.NumberFormat {w, c}     %Intl.NumberFormat%  prototype {}
//! Intl.NumberFormat.prototype  %Intl.NumberFormat.prototype%  constructor {w, c},
//!                                [@@toStringTag] "Intl.NumberFormat" {c}
//! slots (ObjectClass::IntlNumberFormat): 0 [[Locale]] (a String); NF-CORE names the others
//! ICU4X: icu_decimal formatters through crate::cache at the first format
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W2 NF-CORE skeleton): the constructor throws a TypeError. NF-CORE fills
//! the constructor (§16.1.1, InitializeNumberFormat), `supportedLocalesOf` (16.2.2),
//! `format` (16.3.3), `formatToParts` (16.3.4), `resolvedOptions` (16.3.5); W3 (NF-CURRENCY,
//! NF-UNIT, NF-SCIRANGE) fills the styles and `formatRange` (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-number_format.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 34;

/// The class of an Intl.NumberFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlNumberFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("NumberFormat", 0, constructor), // 0 (16.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlNumberFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlNumberFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlNumberFormat,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlNumberFormatPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlNumberFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlNumberFormat,
                PropAttrs::METHOD,
            ),
            // Intl.NumberFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.NumberFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("NumberFormat"),
            IntrinsicId::IntlNumberFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "number_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.NumberFormat ( ... ) (16.1.1): a TypeError until NF-CORE fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.NumberFormat is not implemented"))
}
