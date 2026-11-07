#![allow(dead_code)] // m7-stub: INTL W1 COLLATOR fills this module.
//! Intl.Collator (ECMA-402 §10; block 31): `%Intl.Collator%` and `%Intl.Collator.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.Collator {w, c}            %Intl.Collator%            prototype {}
//! Intl.Collator.prototype         %Intl.Collator.prototype%  constructor {w, c},
//!                                                            [@@toStringTag] "Intl.Collator" {c}
//! slots (ObjectClass::IntlCollator): 0 [[Locale]] (a String); COLLATOR names the others
//!                                    (packed options, [[Collation]], [[BoundCompare]])
//! ICU4X: icu_collator::Collator through crate::cache at the first compare (no ICU4X object
//!        in a slot, INTL-I1-D7)
//! ```
//!
//! Stub (INTL W1 CORE): the constructor throws a TypeError. COLLATOR fills the constructor
//! (§10.1.1, InitializeCollator §10.1.2), appends `supportedLocalesOf` (10.2.2), `compare`
//! (10.3.3), `resolvedOptions` (10.3.4) and the `localeCompare` body path (ids append-only,
//! after 0).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-collator.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 31;

/// The class of an Intl.Collator object.
pub const CLASS: ObjectClass = ObjectClass::IntlCollator;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Collator", 0, constructor), // 0 (10.1.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlCollator,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlCollatorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlCollator,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlCollatorPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlCollatorPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlCollator,
                PropAttrs::METHOD,
            ),
            // Intl.Collator.prototype [ @@toStringTag ] ({c}, 10.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Collator"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Collator"),
            IntrinsicId::IntlCollator,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "collator",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Intl.Collator ( [ locales [ , options ] ] ) (10.1.1). NewTarget undefined is allowed (step
/// 1 uses the active function object).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("Intl.Collator is not implemented"))
}
