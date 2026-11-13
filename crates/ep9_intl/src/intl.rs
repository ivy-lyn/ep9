//! The Intl namespace (ECMA-402 §8; block 30): `%Intl%`, an ordinary object, the key module of
//! the lazy group "Intl" (INTL-I1-D8): its only `%GlobalObject%` property is `Intl`, and each
//! service module adds its constructor to `%Intl%`.
//!
//! ```text
//! globalThis.Intl {w, c}  %Intl%  [@@toStringTag] "Intl" {c},
//!                                 getCanonicalLocales (8.3.1), supportedValuesOf (8.3.2)
//! %IntlFallbackSymbolHolder%      ep9-internal: a null-prototype object that keeps
//!                                 %Intl%.[[FallbackSymbol]] (8, normative optional) as a
//!                                 data property; one per realm; no property names it
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-intl.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsObject, JsResult, JsSymbol, Local, ObjectProps, PropAttrs, PropDef, PropKey,
    PropertyDescriptor, WellKnownSymbol,
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
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::Intl,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    // The holder of %Intl%.[[FallbackSymbol]] (INTL W4 DTF-OPT, the O-B decision): a group
    // intrinsic of this module, so each realm has its own.
    IntrinsicDef {
        id: IntrinsicId::IntlFallbackSymbolHolder,
        value: IntrinsicValue::Object { proto: None },
    },
];

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

// ---- %Intl%.[[FallbackSymbol]] and the legacy constructor path (normative optional) ----------

/// The key of the Symbol in `%IntlFallbackSymbolHolder%` (the holder is not reachable from
/// script, so the key is not observable).
const FALLBACK_SYMBOL_KEY: &str = "fallbackSymbol";

/// The description of `%Intl%.[[FallbackSymbol]]` (ECMA-402 §8).
pub const FALLBACK_SYMBOL_DESCRIPTION: &str = "IntlLegacyConstructedSymbol";

/// `%Intl%.[[FallbackSymbol]]` of the current realm (ECMA-402 §8: "a new %Symbol% in the current
/// realm with the `[[Description]]` "IntlLegacyConstructedSymbol""). The Symbol lives in a data
/// property of the realm's `%IntlFallbackSymbolHolder%` (the INTL W4 O-B decision): made at
/// the first use in the realm, then read back, so each realm has one Symbol and the heap keeps
/// it (plan rule 8). A GC point (the group install, the first allocation).
pub fn fallback_symbol<'s>(cx: &mut Cx<'s>) -> JsResult<Local<'s, JsSymbol>> {
    let holder = cx.intrinsic(IntrinsicId::IntlFallbackSymbolHolder);
    if let Some(s) = cx.get(holder, FALLBACK_SYMBOL_KEY)?.as_symbol() {
        return Ok(s);
    }
    let description = cx.new_string(FALLBACK_SYMBOL_DESCRIPTION)?;
    let s = cx.new_symbol(description)?;
    cx.create_data_property_or_throw(holder, FALLBACK_SYMBOL_KEY, s)?;
    Ok(s)
}

/// OrdinaryHasInstance ( C, O ) (ECMA-262 §7.3.21) for the intrinsic constructor `c` (a
/// table-made constructor: callable, not bound, its own `prototype` is an object that cannot
/// change): is `%c.prototype%` on the prototype chain of `o`? Each step is
/// O.`[[GetPrototypeOf]]`(), so a Proxy's trap runs.
pub fn ordinary_has_instance<'s>(cx: &mut Cx<'s>, c: IntrinsicId, o: Local<'s>) -> JsResult<bool> {
    // 3. If O is not an Object, return false.
    let Some(mut o) = o.as_object() else {
        return Ok(false);
    };
    // 4. Let P be ? Get(C, "prototype").
    let c = cx.intrinsic(c);
    let p = cx.get(c, "prototype")?;
    // 5. If P is not an Object, throw a TypeError exception.
    if !p.is_object() {
        return Err(cx.throw_type_error("Function has non-object prototype in instanceof check"));
    }
    // 6. Repeat: set O to ? O.[[GetPrototypeOf]](); null: false; P: true.
    let mut hops = 0;
    loop {
        let next = cx.prototype_walk_step(o, &mut hops)?;
        let Some(next) = next.as_object() else {
            return Ok(false);
        };
        if cx.same_value(next, p) {
            return Ok(true);
        }
        o = next;
    }
}

/// The legacy constructor path of a service constructor (ChainDateTimeFormat (11.1.3),
/// ChainNumberFormat (16.1.2); normative optional, ECMA-402 §4.3 Note 1): when NewTarget is
/// undefined and `this` is an instance of `%C%` (`constructor`, OrdinaryHasInstance), define
/// `this[%Intl%.[[FallbackSymbol]]]` = `made` ({}: not writable, enumerable or configurable)
/// and return `this`; else return `made`.
pub fn chain_legacy_constructed<'s>(
    cx: &mut Cx<'s>,
    constructor: IntrinsicId,
    made: Local<'s, JsObject>,
    new_target: Local<'s>,
    this: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If newTarget is undefined and ? OrdinaryHasInstance(%C%, this) is true, then
    if new_target.is_undefined() && ordinary_has_instance(cx, constructor, this)? {
        let Some(this_object) = this.as_object() else {
            return Ok(made.into());
        };
        // a. Perform ? DefinePropertyOrThrow(this, %Intl%.[[FallbackSymbol]],
        //    PropertyDescriptor{ [[Value]]: made, [[Writable]]: false, [[Enumerable]]: false,
        //    [[Configurable]]: false }).
        let symbol = fallback_symbol(cx)?;
        let desc = PropertyDescriptor::data(made.into(), false, false, false);
        cx.define_property_or_throw(this_object, symbol, desc)?;
        // b. Return this.
        return Ok(this);
    }
    // 2. Return the made object.
    Ok(made.into())
}

/// The legacy unwrap of a service object (UnwrapDateTimeFormat (11.5.2), UnwrapNumberFormat
/// (16.5.1); normative optional): `o` itself if it is not an Object of the class (checked by
/// `has_slots`) or not an instance of `%C%`; else ? Get(o, `%Intl%.[[FallbackSymbol]]`). The
/// caller then performs RequireInternalSlot on the result.
pub fn unwrap_legacy_constructed<'s>(
    cx: &mut Cx<'s>,
    constructor: IntrinsicId,
    o: Local<'s>,
    has_slots: bool,
) -> JsResult<Local<'s>> {
    // 1. If o is not an Object, throw a TypeError exception.
    let Some(object) = o.as_object() else {
        return Err(cx.throw_type_error("Receiver is not an object"));
    };
    // 2. If o does not have the internal slot and ? OrdinaryHasInstance(%C%, o) is true, then
    //    return ? Get(o, %Intl%.[[FallbackSymbol]]).
    if !has_slots && ordinary_has_instance(cx, constructor, o)? {
        let symbol = fallback_symbol(cx)?;
        return cx.get(object, symbol);
    }
    // 3. Return o.
    Ok(o)
}
