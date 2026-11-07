//! The option readers of ECMA-402 §9.2.10-§9.2.15: GetOptionsObject, CoerceOptionsToObject,
//! GetOption, GetBooleanOrStringNumberFormatOption, DefaultNumberOption and GetNumberOption.
//!
//! ```text
//! options (JS value) ─ get_options_object / coerce_options_to_object ─> object (or a
//!                      null-prototype empty object for undefined)
//!   ─ get_string_option(o, "localeMatcher", &["lookup", "best fit"], Some("best fit"))
//!   ─ get_boolean_option(o, "numeric", None) ─ get_number_option(o, "minimumIntegerDigits",
//!     1, 21, Some(1)) ...                                         RangeError / TypeError
//! ```
//!
//! Each reader does one `[[Get]]` of the property, in the spec order of its caller.

use ep9_vm::{Cx, JsObject, JsResult, Local};

/// GetOptionsObject ( options ) (9.2.10): undefined gives a new empty object with a null
/// prototype, an object gives itself, any other value is a TypeError.
pub fn get_options_object<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    if options.is_undefined() {
        return Ok(cx.new_object_with_proto(Local::null()));
    }
    match options.as_object() {
        Some(o) => Ok(o),
        None => Err(cx.throw_type_error("options must be an object or undefined")),
    }
}

/// CoerceOptionsToObject ( options ) (9.2.11): undefined gives a new empty object with a
/// null prototype, else ? ToObject(options).
pub fn coerce_options_to_object<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    if options.is_undefined() {
        return Ok(cx.new_object_with_proto(Local::null()));
    }
    cx.to_object(options)
}

/// GetOption ( options, property, string, values, default ) (9.2.12) for type "string":
/// `None` if the property is undefined and `default` is `None` (the spec's undefined
/// default). `values` empty means any string; else a value outside it is a RangeError.
pub fn get_string_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
    values: &[&str],
    default: Option<&str>,
) -> JsResult<Option<String>> {
    // 1. Let value be ? Get(options, property).
    let value = cx.get(options, property)?;
    // 2. If value is undefined, return default (step 2.b: a required option has none).
    if value.is_undefined() {
        return Ok(default.map(str::to_string));
    }
    // 5. Else (type "string"), set value to ? ToString(value).
    let s = cx.to_rust_string(value)?;
    // 6. If values is not empty and does not contain value, throw a RangeError exception.
    if !values.is_empty() && !values.contains(&s.as_str()) {
        return Err(cx.throw_range_error(&format!("Value {s} out of range for {property}")));
    }
    Ok(Some(s))
}

/// GetOption ( options, property, boolean, empty, default ) (9.2.12) for type "boolean":
/// `None` if the property is undefined and `default` is `None`.
pub fn get_boolean_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
    default: Option<bool>,
) -> JsResult<Option<bool>> {
    let value = cx.get(options, property)?;
    if value.is_undefined() {
        return Ok(default);
    }
    // 4. If type is "boolean", set value to ToBoolean(value).
    Ok(Some(cx.to_boolean(value)))
}

/// The result of GetBooleanOrStringNumberFormatOption (9.2.13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoolOrString {
    /// true
    True,
    /// false (from a value that ToBoolean makes false)
    False,
    /// One of the allowed strings.
    Str(String),
}

/// GetBooleanOrStringNumberFormatOption ( options, property, stringValues, fallback ) (9.2.13).
pub fn get_boolean_or_string_number_format_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
    string_values: &[&str],
    fallback: BoolOrString,
) -> JsResult<BoolOrString> {
    // 1. Let value be ? Get(options, property).
    let value = cx.get(options, property)?;
    // 2. If value is undefined, return fallback.
    if value.is_undefined() {
        return Ok(fallback);
    }
    // 3. If value is true, return true.
    if value.as_bool() == Some(true) {
        return Ok(BoolOrString::True);
    }
    // 4. If ToBoolean(value) is false, return false.
    if !cx.to_boolean(value) {
        return Ok(BoolOrString::False);
    }
    // 5. Set value to ? ToString(value).
    let s = cx.to_rust_string(value)?;
    // 6. If stringValues does not contain value, throw a RangeError exception.
    if !string_values.contains(&s.as_str()) {
        return Err(cx.throw_range_error(&format!("Value {s} out of range for {property}")));
    }
    Ok(BoolOrString::Str(s))
}

/// DefaultNumberOption ( value, minimum, maximum, fallback ) (9.2.14).
pub fn default_number_option<'s>(
    cx: &mut Cx<'s>,
    value: Local<'s>,
    property: &str,
    minimum: i32,
    maximum: i32,
    fallback: Option<i32>,
) -> JsResult<Option<i32>> {
    // 1. If value is undefined, return fallback.
    if value.is_undefined() {
        return Ok(fallback);
    }
    // 2. Set value to ? ToNumber(value).
    let n = cx.to_number(value)?;
    // 3. If value is NaN or less than minimum or greater than maximum, throw a RangeError.
    if n.is_nan() || n < f64::from(minimum) || n > f64::from(maximum) {
        return Err(cx.throw_range_error(&format!("{property} value is out of range")));
    }
    // 4. Return floor(ℝ(value)).
    Ok(Some(n.floor() as i32))
}

/// GetNumberOption ( options, property, minimum, maximum, fallback ) (9.2.15).
pub fn get_number_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
    minimum: i32,
    maximum: i32,
    fallback: Option<i32>,
) -> JsResult<Option<i32>> {
    // 1. Let value be ? Get(options, property).
    let value = cx.get(options, property)?;
    // 2. Return ? DefaultNumberOption(value, minimum, maximum, fallback).
    default_number_option(cx, value, property, minimum, maximum, fallback)
}

/// The `localeMatcher` option: GetOption(options, "localeMatcher", string, « "lookup",
/// "best fit" », "best fit"). Both values run the LookupMatcher (INTL-I1 §5.3 row 15), so the
/// result only checks the value.
pub fn get_locale_matcher<'s>(cx: &mut Cx<'s>, options: Local<'s, JsObject>) -> JsResult<()> {
    get_string_option(
        cx,
        options,
        "localeMatcher",
        &["lookup", "best fit"],
        Some("best fit"),
    )?;
    Ok(())
}
