#![allow(dead_code)] // m7-stub: INTL W3 RTF fills this module.
//! Intl.RelativeTimeFormat (ECMA-402 §18; block 38): `%Intl.RelativeTimeFormat%` and
//! `%Intl.RelativeTimeFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.RelativeTimeFormat {w, c}     %Intl.RelativeTimeFormat%  prototype {}
//! Intl.RelativeTimeFormat.prototype  %Intl.RelativeTimeFormat.prototype%  constructor {w, c},
//!                                      [@@toStringTag] "Intl.RelativeTimeFormat" {c}
//! slots (ObjectClass::IntlRelativeTimeFormat): 0 [[Locale]] (a String); RTF names the others
//! ICU4X: icu_experimental::relativetime data through crate::cache at the first format
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W3 RTF skeleton): the constructor throws a TypeError. RTF fills the
//! constructor (§18.1.1), `supportedLocalesOf` (18.2.2), `format` (18.3.3), `formatToParts`
//! (18.3.4) and `resolvedOptions` (18.3.5) (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-relative_time_format.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 38;

/// The class of an Intl.RelativeTimeFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlRelativeTimeFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("RelativeTimeFormat", 0, constructor), // 0 (18.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlRelativeTimeFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlRelativeTimeFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlRelativeTimeFormat,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlRelativeTimeFormatPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlRelativeTimeFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlRelativeTimeFormat,
                PropAttrs::METHOD,
            ),
            // Intl.RelativeTimeFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.RelativeTimeFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("RelativeTimeFormat"),
            IntrinsicId::IntlRelativeTimeFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "relative_time_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.RelativeTimeFormat ( [ locales [ , options ] ] ) (18.1.1): a TypeError until RTF
/// fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.RelativeTimeFormat is not implemented"))
}
