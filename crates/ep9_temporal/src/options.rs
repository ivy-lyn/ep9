//! Option bags (§13.1-§13.17 of the Temporal spec). Each read is observable (a getter, a
//! Proxy), so each method calls these in its spec order; a `None` bag (the argument was
//! undefined) reads nothing and gives the default.
//!
//! | Function | Spec |
//! |---|---|
//! | [`get_options_object`] | GetOptionsObject: undefined -> `None`; an object -> it; else TypeError |
//! | [`get_string_option`] | GetOption(options, key, string, ..): Get, then ToString |
//! | [`get_parsed_option`] | GetOption with a value list: the string parsed by a temporal_rs `FromStr`, else RangeError |
//! | [`get_rounding_increment`] | GetRoundingIncrementOption |
//! | [`get_unit`] | GetTemporalUnitValuedOption (the group check is [`validate_unit`]) |
//! | [`get_fractional_second_digits`] | GetTemporalFractionalSecondDigitsOption |

// T1: the helpers get their first callers in T2 (Duration).
#![allow(dead_code)]

use core::str::FromStr;

use ep9_vm::{Cx, JsObject, JsResult, Local};
use temporal_rs::options::{RoundingIncrement, Unit, UnitGroup};
use temporal_rs::parsers::Precision;

use crate::error::OrThrow;

/// An option bag: `None` when the argument was undefined.
pub(crate) type Options<'s> = Option<Local<'s, JsObject>>;

/// GetOptionsObject (§13.2).
pub(crate) fn get_options_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Options<'s>> {
    // 1. If options is undefined, return OrdinaryObjectCreate(null).
    if v.is_undefined() {
        return Ok(None);
    }
    // 2. If options is an Object, return options.
    if let Some(o) = v.as_object() {
        return Ok(Some(o));
    }
    // 3. Throw a TypeError exception.
    Err(cx.throw_type_error("options must be an object or undefined"))
}

/// GetOption(options, key, string, ...) (§13.3) up to the value check: `None` when the
/// property is undefined.
pub(crate) fn get_string_option<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
    key: &str,
) -> JsResult<Option<String>> {
    let Some(o) = options else {
        return Ok(None);
    };
    // 1. Let value be ? Get(options, property).
    let k = cx.key(key);
    let v = cx.get(o, k)?;
    // 2. If value is undefined, return default.
    if v.is_undefined() {
        return Ok(None);
    }
    // 5. Else, set value to ? ToString(value).
    Ok(Some(cx.to_rust_string(v)?))
}

/// GetOption with a list of values: the string option parsed by `T::from_str` (the list),
/// else a RangeError.
pub(crate) fn get_parsed_option<'s, T: FromStr>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
    key: &str,
) -> JsResult<Option<T>> {
    match get_string_option(cx, options, key)? {
        None => Ok(None),
        Some(s) => match T::from_str(&s) {
            Ok(t) => Ok(Some(t)),
            Err(_) => Err(cx.throw_range_error(&format!("{s} is not a valid value for {key}"))),
        },
    }
}

/// GetRoundingIncrementOption (§13.15).
pub(crate) fn get_rounding_increment<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
) -> JsResult<Option<RoundingIncrement>> {
    let Some(o) = options else {
        return Ok(None);
    };
    // 1. Let value be ? Get(options, "roundingIncrement").
    let k = cx.key("roundingIncrement");
    let v = cx.get(o, k)?;
    // 2. If value is undefined, return 1𝔽.
    if v.is_undefined() {
        return Ok(None);
    }
    // 3. Let integerIncrement be ? ToIntegerWithTruncation(value).
    let n = cx.to_number(v)?;
    // 4. If integerIncrement < 1 or integerIncrement > 10**9, throw a RangeError exception.
    RoundingIncrement::try_from(n).or_throw(cx).map(Some)
}

/// GetTemporalUnitValuedOption (§13.17) without the group check: `None` when undefined.
pub(crate) fn get_unit<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
    key: &str,
) -> JsResult<Option<Unit>> {
    get_parsed_option::<Unit>(cx, options, key)
}

/// ValidateTemporalUnitValue (§13.18): `unit` is in `group` or is `extra`.
pub(crate) fn validate_unit(
    cx: &mut Cx<'_>,
    unit: Option<Unit>,
    group: UnitGroup,
    extra: Option<Unit>,
) -> JsResult<()> {
    group.validate_unit(unit, extra).or_throw(cx)
}

/// GetTemporalFractionalSecondDigitsOption (§13.16).
pub(crate) fn get_fractional_second_digits<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
) -> JsResult<Precision> {
    let Some(o) = options else {
        return Ok(Precision::Auto);
    };
    // 1. Let digitsValue be ? Get(options, "fractionalSecondDigits").
    let k = cx.key("fractionalSecondDigits");
    let v = cx.get(o, k)?;
    // 2. If digitsValue is undefined, return auto.
    if v.is_undefined() {
        return Ok(Precision::Auto);
    }
    // 3. If digitsValue is not a Number, then
    let Some(n) = v.as_number() else {
        // a. If ? ToString(digitsValue) is not "auto", throw a RangeError exception.
        let s = cx.to_rust_string(v)?;
        if s != "auto" {
            return Err(cx.throw_range_error("fractionalSecondDigits must be 0-9 or 'auto'"));
        }
        // b. Return auto.
        return Ok(Precision::Auto);
    };
    // 4. If digitsValue is NaN, +∞𝔽, or -∞𝔽, throw a RangeError exception.
    if !n.is_finite() {
        return Err(cx.throw_range_error("fractionalSecondDigits must be finite"));
    }
    // 5. Let digitCount be floor(ℝ(digitsValue)).
    let d = n.floor();
    // 6. If digitCount < 0 or digitCount > 9, throw a RangeError exception.
    if !(0.0..=9.0).contains(&d) {
        return Err(cx.throw_range_error("fractionalSecondDigits must be 0-9 or 'auto'"));
    }
    // 7. Return digitCount.
    Ok(Precision::Digit(d as u8))
}
