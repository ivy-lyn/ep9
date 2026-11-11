#![allow(dead_code)] // m7-stub: INTL W2 SEGMENTER fills this module.
//! Intl.Segmenter (ECMA-402 §19; block 36): `%Intl.Segmenter%` and `%Intl.Segmenter.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.Segmenter {w, c}        %Intl.Segmenter%  prototype {}
//! Intl.Segmenter.prototype     %Intl.Segmenter.prototype%  constructor {w, c},
//!                                [@@toStringTag] "Intl.Segmenter" {c}
//! (no global path)            %IntlSegmentsPrototype%  (SEGMENTER: containing, @@iterator)
//! (no global path)            %IntlSegmentIteratorPrototype%  [[Prototype]] %IteratorPrototype%,
//!                                [@@toStringTag] "Segmenter String Iterator" {c}
//! slots: ObjectClass::IntlSegmenter 0 [[Locale]] (a String); ObjectClass::IntlSegments and
//!        ObjectClass::IntlSegmentIterator: SEGMENTER names them
//! ICU4X: icu_segmenter (dictionary, no LSTM) through crate::cache at the first segment
//!        (no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W2 NF-CORE skeleton): the constructor throws a TypeError. SEGMENTER fills
//! the constructor (§19.1.1), `supportedLocalesOf` (19.2.2), `segment` (19.3.3),
//! `resolvedOptions` (19.3.4), the %IntlSegmentsPrototype% methods `containing` and
//! `[@@iterator]` (19.5.2) and %IntlSegmentIteratorPrototype%.next (19.6.2.1) (ids
//! append-only, after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-segmenter.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 36;

/// The class of an Intl.Segmenter object.
pub const CLASS: ObjectClass = ObjectClass::IntlSegmenter;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Segmenter", 0, constructor), // 0 (19.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlSegmenter,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmenterPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmentsPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmentIteratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlSegmenter,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlSegmenterPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlSegmenterPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlSegmenter,
                PropAttrs::METHOD,
            ),
            // Intl.Segmenter.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Segmenter"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlSegmentIteratorPrototype,
        props: &[
            // %IntlSegmentIteratorPrototype% [ @@toStringTag ] ({c}, 19.6.2.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Segmenter String Iterator"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Segmenter"),
            IntrinsicId::IntlSegmenter,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "segmenter",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.Segmenter ( ... ) (19.1.1): a TypeError until SEGMENTER fills it.
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.Segmenter is not implemented"))
}
