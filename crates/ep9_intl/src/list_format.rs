#![allow(dead_code)] // m7-stub: INTL W2 LIST+CASEMAP fills this module.
//! Intl.ListFormat (ECMA-402 §13; block 35): `%Intl.ListFormat%` and `%Intl.ListFormat.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.ListFormat {w, c}       %Intl.ListFormat%  prototype {}
//! Intl.ListFormat.prototype    %Intl.ListFormat.prototype%  constructor {w, c},
//!                                [@@toStringTag] "Intl.ListFormat" {c}
//! slots (ObjectClass::IntlListFormat): 0 [[Locale]] (a String); LIST+CASEMAP names the others
//! ICU4X: icu_list::ListFormatter through crate::cache at the first format
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W2 NF-CORE skeleton): the constructor throws a TypeError. LIST+CASEMAP fills
//! the constructor (§13.1.1), `supportedLocalesOf` (13.2.2), `format` (13.3.3),
//! `formatToParts` (13.3.4), `resolvedOptions` (13.3.5) (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-list_format.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 35;

/// The class of an Intl.ListFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlListFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("ListFormat", 0, constructor), // 0 (13.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlListFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlListFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlListFormat,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlListFormatPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlListFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlListFormat,
                PropAttrs::METHOD,
            ),
            // Intl.ListFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.ListFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("ListFormat"),
            IntrinsicId::IntlListFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "list_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.ListFormat ( ... ) (13.1.1): a TypeError until LIST+CASEMAP fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.ListFormat is not implemented"))
}
