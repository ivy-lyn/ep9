//! The Intl namespace (ECMA-402 §8; block 30): `%Intl%`, an ordinary object, the key module of
//! the lazy group "Intl" (INTL-I1-D8): its only `%GlobalObject%` property is `Intl`, and each
//! service module adds its constructor to `%Intl%`.
//!
//! ```text
//! globalThis.Intl {w, c}  %Intl%  [@@toStringTag] "Intl" {c},
//!                                 getCanonicalLocales (8.3.1), supportedValuesOf (8.3.2)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-intl.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

use crate::available;
use crate::tag::canonicalize_locale_list;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 30;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("getCanonicalLocales", 1, get_canonical_locales), // 0 (8.3.1)
    BuiltinDef::method("supportedValuesOf", 1, supported_values_of),     // 1 (8.3.2)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Intl,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[
            // Intl [ @@toStringTag ] ({c}, 8.1.1).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::method(PropKey::Str("getCanonicalLocales"), 0),
            PropDef::method(PropKey::Str("supportedValuesOf"), 1),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[PropDef::intrinsic(
            PropKey::Str("Intl"),
            IntrinsicId::Intl,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "intl",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// An Array of the strings `items` (CreateArrayFromList; no `Local` in a Rust container).
pub fn string_array<'s, S: AsRef<str>>(cx: &mut Cx<'s>, items: &[S]) -> JsResult<Local<'s>> {
    let array = cx.new_array();
    for (i, s) in items.iter().enumerate() {
        let v = cx.new_string(s.as_ref())?;
        cx.create_data_property_index_or_throw(array, i as u64, v)?;
    }
    Ok(array.into())
}

/// Intl.getCanonicalLocales ( locales ) (8.3.1).
fn get_canonical_locales<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let ll be ? CanonicalizeLocaleList(locales).
    let ll = canonicalize_locale_list(cx, args.get(0))?;
    // 2. Return CreateArrayFromList(ll).
    string_array(cx, &ll)
}

/// Intl.supportedValuesOf ( key ) (8.3.2).
fn supported_values_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let key be ? ToString(key).
    let key = cx.to_rust_string(args.get(0))?;
    // 2-8. The list of the key; any other key is a RangeError.
    let list: &[&str] = match key.as_str() {
        "calendar" => available::CALENDARS,
        "collation" => available::COLLATIONS,
        "currency" => available::CURRENCIES,
        "numberingSystem" => available::NUMBERING_SYSTEMS,
        "timeZone" => available::time_zones(),
        "unit" => available::UNITS,
        _ => return Err(cx.throw_range_error(&format!("Invalid key : {key}"))),
    };
    // 9. Return CreateArrayFromList(list).
    string_array(cx, list)
}
