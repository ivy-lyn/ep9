//! Conversions of the Temporal spec (§13 abstract operations) on `Cx` values.
//!
//! | Function | Spec |
//! |---|---|
//! | [`to_integer_with_truncation`] | ToIntegerWithTruncation: ToNumber; NaN or ±∞ is a RangeError; truncate |
//! | [`to_positive_integer_with_truncation`] | ToPositiveIntegerWithTruncation: the same, and ≤ 0 is a RangeError |
//! | [`to_integer_if_integral`] | ToIntegerIfIntegral: ToNumber; not an integral Number is a RangeError |
//! | [`temporal_text`] | The text of a Temporal string argument: ASCII, or U+2212 (the grammar has no other char), else a RangeError |
//! | [`required_string`] | "If x is not a String, throw a TypeError" |

use ep9_vm::{Cx, JsResult, JsString, Local, ToPrimitiveHint};

/// ToIntegerWithTruncation (§13.38): an integral `f64` (never -0).
pub(crate) fn to_integer_with_truncation<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<f64> {
    // 1. Let number be ? ToNumber(argument).
    let n = cx.to_number(v)?;
    // 2. If number is NaN, +∞𝔽 or -∞𝔽, throw a RangeError exception.
    if !n.is_finite() {
        return Err(cx.throw_range_error("value must be a finite number"));
    }
    // 3. Return truncate(ℝ(number)).
    Ok(n.trunc() + 0.0)
}

/// ToPositiveIntegerWithTruncation (§13.39).
pub(crate) fn to_positive_integer_with_truncation<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<f64> {
    // 1. Let integer be ? ToIntegerWithTruncation(argument).
    let i = to_integer_with_truncation(cx, v)?;
    // 2. If integer ≤ 0, throw a RangeError exception.
    if i <= 0.0 {
        return Err(cx.throw_range_error("value must be a positive integer"));
    }
    Ok(i)
}

/// ToIntegerIfIntegral (§13.40): an integral `f64` (never -0).
pub(crate) fn to_integer_if_integral<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<f64> {
    // 1. Let number be ? ToNumber(argument).
    let n = cx.to_number(v)?;
    // 2. If number is not an integral Number, throw a RangeError exception.
    if !n.is_finite() || n.trunc() != n {
        return Err(cx.throw_range_error("value must be an integral number"));
    }
    // 3. Return ℝ(number).
    Ok(n + 0.0)
}

/// The text of the String `s` for a Temporal parser (temporal_rs parses UTF-8). A Temporal
/// string holds only ASCII and U+2212 (MINUS SIGN); any other char, a lone surrogate too,
/// cannot match the grammar, so it is a RangeError here.
pub(crate) fn temporal_text<'s>(cx: &mut Cx<'s>, s: Local<'s, JsString>) -> JsResult<String> {
    let t = cx.to_rust_string(s)?;
    if t.chars().all(|c| c.is_ascii() || c == '\u{2212}') {
        Ok(t)
    } else {
        Err(cx.throw_range_error(&format!("invalid Temporal string: {t}")))
    }
}

/// `v` as a String, or a TypeError "`what` must be a string".
pub(crate) fn required_string<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
    what: &str,
) -> JsResult<Local<'s, JsString>> {
    match v.as_string() {
        Some(s) => Ok(s),
        None => Err(cx.throw_type_error(&format!("{what} must be a string"))),
    }
}

/// ToPrimitive(v, string), then the result must be a String (TypeError otherwise): the
/// `monthCode` and `offset` fields of a property bag.
pub(crate) fn to_primitive_string<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
    what: &str,
) -> JsResult<String> {
    let p = cx.to_primitive(v, ToPrimitiveHint::String)?;
    let s = required_string(cx, p, what)?;
    cx.to_rust_string(s)
}
