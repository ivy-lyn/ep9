#![allow(dead_code)] // m7-stub: INTL W2 DISPLAYNAMES fills this module.
//! Intl.DisplayNames (ECMA-402 §12; block 37): `%Intl.DisplayNames%` and `%Intl.DisplayNames.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.DisplayNames {w, c}     %Intl.DisplayNames%  prototype {}
//! Intl.DisplayNames.prototype  %Intl.DisplayNames.prototype%  constructor {w, c},
//!                                [@@toStringTag] "Intl.DisplayNames" {c}
//! slots (ObjectClass::IntlDisplayNames): 0 [[Locale]] (a String); DISPLAYNAMES names the others
//! ICU4X: icu_experimental::displaynames through crate::cache at the first `of`
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W2 NF-CORE skeleton): the constructor throws a TypeError. DISPLAYNAMES fills
//! the constructor (§12.1.1), `supportedLocalesOf` (12.2.2), `of` (12.3.3),
//! `resolvedOptions` (12.3.4) (ids append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-display_names.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 37;

/// The class of an Intl.DisplayNames object.
pub const CLASS: ObjectClass = ObjectClass::IntlDisplayNames;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DisplayNames", 2, constructor), // 0 (12.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDisplayNames,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDisplayNamesPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDisplayNames,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlDisplayNamesPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlDisplayNamesPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDisplayNames,
                PropAttrs::METHOD,
            ),
            // Intl.DisplayNames.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DisplayNames"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DisplayNames"),
            IntrinsicId::IntlDisplayNames,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "display_names",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.DisplayNames ( ... ) (12.1.1): a TypeError until DISPLAYNAMES fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.DisplayNames is not implemented"))
}
