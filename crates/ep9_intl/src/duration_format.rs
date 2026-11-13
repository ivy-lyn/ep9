#![allow(dead_code)] // m7-stub: INTL W4 DURATION fills this module.
//! Intl.DurationFormat (ECMA-402 §13; block 40): `%Intl.DurationFormat%` and
//! `%Intl.DurationFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.DurationFormat {w, c}     %Intl.DurationFormat%  prototype {}
//! Intl.DurationFormat.prototype  %Intl.DurationFormat.prototype%  constructor {w, c},
//!                    [@@toStringTag] "Intl.DurationFormat" {c}
//! slots (ObjectClass::IntlDurationFormat): 0 [[Locale]] (a String); DURATION names the others
//! ```
//!
//! Stub (INTL W4 DTF-OPT skeleton): the constructor throws a TypeError. DURATION fills the
//! constructor (§13.1.1) and appends the other ids after 0.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-duration_format.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 40;

/// The class of an Intl.DurationFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlDurationFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DurationFormat", 0, constructor), // 0 (13.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDurationFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDurationFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDurationFormat,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlDurationFormatPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlDurationFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDurationFormat,
                PropAttrs::METHOD,
            ),
            // Intl.DurationFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DurationFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DurationFormat"),
            IntrinsicId::IntlDurationFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "duration_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.DurationFormat ( [ locales [ , options ] ] ) (13.1.1): a TypeError until DURATION fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.DurationFormat is not implemented"))
}
