#![allow(dead_code)] // m7-stub: INTL W4 DTF-OPT fills this module.
//! Intl.DateTimeFormat (ECMA-402 §11; block 39): `%Intl.DateTimeFormat%` and
//! `%Intl.DateTimeFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.DateTimeFormat {w, c}     %Intl.DateTimeFormat%  prototype {}
//! Intl.DateTimeFormat.prototype  %Intl.DateTimeFormat.prototype%  constructor {w, c},
//!                    [@@toStringTag] "Intl.DateTimeFormat" {c}
//! slots (ObjectClass::IntlDateTimeFormat): 0 [[Locale]] (a String); DTF-OPT names the others
//! ```
//!
//! Stub (INTL W4 DTF-OPT skeleton): the constructor throws a TypeError. DTF-OPT fills the
//! constructor (§11.1.1) and appends the other ids after 0.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-date_time_format.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 39;

/// The class of an Intl.DateTimeFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlDateTimeFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DateTimeFormat", 0, constructor), // 0 (11.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDateTimeFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDateTimeFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDateTimeFormat,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlDateTimeFormatPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlDateTimeFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDateTimeFormat,
                PropAttrs::METHOD,
            ),
            // Intl.DateTimeFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DateTimeFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DateTimeFormat"),
            IntrinsicId::IntlDateTimeFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "date_time_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.DateTimeFormat ( [ locales [ , options ] ] ) (11.1.1): a TypeError until DTF-OPT fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.DateTimeFormat is not implemented"))
}
