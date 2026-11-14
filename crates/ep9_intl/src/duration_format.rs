//! Intl.DurationFormat (ECMA-402 §13; block 40): `%Intl.DurationFormat%` and
//! `%Intl.DurationFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.DurationFormat {w, c}     %Intl.DurationFormat%  prototype {}, supportedLocalesOf
//! Intl.DurationFormat.prototype  %Intl.DurationFormat.prototype%  constructor {w, c},
//!                                  resolvedOptions, format, formatToParts {w, c},
//!                                  [@@toStringTag] "Intl.DurationFormat" {c}
//! slots (ObjectClass::IntlDurationFormat, SLOTS = 4):
//!   0 [[Locale]]           a String ("nu" is its only Unicode extension key)
//!   1 options              an int32: bits 0-1 [[Style]], bits 2-5 [[FractionalDigits]] (15:
//!                          undefined), bits 6-21 the unit options of years, months, weeks, days
//!   2 time unit options    an int32: the unit options of hours .. nanoseconds
//!   3 [[NumberingSystem]]  a String
//!   unit options: 4 bits per unit (Table 20 order): bits 0-2 the style (index in UNIT_STYLES),
//!   bit 3 the display ("always")
//! format(duration) ─ ToDurationRecord ─ PartitionDurationFormatPattern:
//!   each unit in Table 20 order, until the first numeric unit:
//!     long/short/narrow ─ number_format::partition_numeric (style "unit", the unit, its
//!       style as unitDisplay; the next units folded in as an exact fraction if they are
//!       "fractional") ─> one element
//!     numeric/2-digit ─ FormatNumericUnits: hours, minutes, seconds (with the fraction) as
//!       decimal numbers joined by the time separator of the locale ─> one element
//!   ListFormatParts: list_format::create_parts_from_list (type "unit", the style; digital:
//!     "short") ─> the parts of the elements in the place of each "element" part
//! ```
//!
//! `[[NumberFormat]]` and `[[ListFormat]]` objects are not made: the parts come from
//! `number_format` and `list_format` with the resolved options that the spec's `Construct`
//! calls give (`number_format::decimal_defaults`, `number_format::unit_defaults`). No ICU4X
//! object is in a slot (INTL-I1-D7): the time separator (`[[DigitalFormat]]`, ICU4X
//! `UnitsDurationDigitalV1`) is in `crate::cache`.
//!
//! The sums of the sub-second fields are exact (DL INTL-V2-FP4): `i128` sums in the smallest
//! unit and one `Decimal` (`fractional_value`), never an `f64` sum (H36).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-duration_format.txt`.

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use fixed_decimal::Decimal;
use icu_experimental::duration::DurationFormatterPreferences;
use icu_experimental::duration::provider::UnitsDurationDigitalV1;
use icu_experimental::provider::Baked;
use icu_provider::prelude::*;

use crate::cache::{self, Service};
use crate::digits::{Interrupted, RoundingMode, Work, polled};
use crate::list_format::{self, PartKind};
use crate::locale::is_unicode_type;
use crate::number_format::{self, decimal};
use crate::options::{
    get_locale_matcher, get_number_option, get_options_object, get_string_option,
};
use crate::resolve;
use crate::tag::{self, icu_locale};
use decimal::{Grouping, IntlMv, SignDisplay};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 40;

/// The class of an Intl.DurationFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlDurationFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: `[[Style]]`, `[[FractionalDigits]]` and the options of the date units (an int32,
/// `Resolved::pack`).
pub const SLOT_OPTIONS: usize = 1;
/// Slot 2: the options of the time units (an int32, `Resolved::pack`).
pub const SLOT_TIME_UNITS: usize = 2;
/// Slot 3: `[[NumberingSystem]]` (a String).
pub const SLOT_NUMBERING_SYSTEM: usize = 3;
/// The slot count of an Intl.DurationFormat object.
pub const SLOTS: usize = 4;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const FORMAT: u16 = 2;
    pub const FORMAT_TO_PARTS: u16 = 3;
    pub const RESOLVED_OPTIONS: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DurationFormat", 0, constructor), // 0 (13.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1
    BuiltinDef::method("format", 1, format),                   // 2
    BuiltinDef::method("formatToParts", 1, format_to_parts),   // 3
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 4
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDurationFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDurationFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDurationFormat,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlDurationFormatPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlDurationFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDurationFormat,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, format, formatToParts.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("format"), def::FORMAT),
            PropDef::method(PropKey::Str("formatToParts"), def::FORMAT_TO_PARTS),
            // Intl.DurationFormat.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DurationFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DurationFormat"),
            IntrinsicId::IntlDurationFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "duration_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Style]]` values, in their packed order.
pub const STYLES: [&str; 4] = ["long", "short", "narrow", "digital"];
/// The index of "digital" in [`STYLES`].
const DIGITAL: usize = 3;

/// The unit styles, in their packed order. "fractional" is the internal style of a numeric
/// sub-second unit (GetDurationUnitOptions step 4); resolvedOptions shows it as "numeric".
pub const UNIT_STYLES: [&str; 6] = [
    "long",
    "short",
    "narrow",
    "numeric",
    "2-digit",
    "fractional",
];
const LONG: u8 = 0;
const NUMERIC: u8 = 3;
const TWO_DIGIT: u8 = 4;
const FRACTIONAL: u8 = 5;

/// The units of Table 20 (the fields of a duration record), in table order.
pub const UNITS: [&str; 10] = [
    "years",
    "months",
    "weeks",
    "days",
    "hours",
    "minutes",
    "seconds",
    "milliseconds",
    "microseconds",
    "nanoseconds",
];
/// The NumberFormat unit of each of [`UNITS`] (Table 20, column NumberFormat Unit).
const NF_UNITS: [&str; 10] = [
    "year",
    "month",
    "week",
    "day",
    "hour",
    "minute",
    "second",
    "millisecond",
    "microsecond",
    "nanosecond",
];
/// The `<unit>Display` option names, in [`UNITS`] order.
const DISPLAY_NAMES: [&str; 10] = [
    "yearsDisplay",
    "monthsDisplay",
    "weeksDisplay",
    "daysDisplay",
    "hoursDisplay",
    "minutesDisplay",
    "secondsDisplay",
    "millisecondsDisplay",
    "microsecondsDisplay",
    "nanosecondsDisplay",
];
const HOURS: usize = 4;
const MINUTES: usize = 5;
const SECONDS: usize = 6;

/// The style and display of one unit (the `[[<Unit>Options]]` record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitOptions {
    /// `[[Style]]`: an index in [`UNIT_STYLES`].
    pub style: u8,
    /// `[[Display]]`: true "always", false "auto".
    pub always: bool,
}

impl UnitOptions {
    /// The style is "numeric" or "2-digit" (not "fractional").
    fn is_numeric(self) -> bool {
        self.style == NUMERIC || self.style == TWO_DIGIT
    }

    /// The 4-bit packed form.
    fn bits(self) -> u32 {
        u32::from(self.style & 7) | u32::from(self.always) << 3
    }

    /// The options of the 4-bit packed form `b`.
    fn from_bits(b: u32) -> UnitOptions {
        UnitOptions {
            style: ((b & 7) as u8).min(FRACTIONAL),
            always: b & 8 != 0,
        }
    }
}

/// The internal slots of an Intl.DurationFormat (ECMA-402 §13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[NumberingSystem]]`.
    pub numbering_system: String,
    /// `[[Style]]`: an index in [`STYLES`].
    pub style: u8,
    /// The `[[<Unit>Options]]` of each unit, in [`UNITS`] order.
    pub units: [UnitOptions; 10],
    /// `[[FractionalDigits]]` (`None`: undefined).
    pub fractional_digits: Option<u8>,
}

impl Resolved {
    /// The two packed int32 slots ([`SLOT_OPTIONS`], [`SLOT_TIME_UNITS`]).
    fn pack(&self) -> (i32, i32) {
        let mut a =
            u32::from(self.style & 3) | u32::from(self.fractional_digits.unwrap_or(15)) << 2;
        for (i, u) in self.units[..HOURS].iter().enumerate() {
            a |= u.bits() << (6 + 4 * i);
        }
        let mut b = 0u32;
        for (i, u) in self.units[HOURS..].iter().enumerate() {
            b |= u.bits() << (4 * i);
        }
        (a as i32, b as i32)
    }

    /// The options of the packed slots `a` and `b` (the locale and numbering system are the
    /// caller's).
    fn unpack(locale: String, numbering_system: String, a: i32, b: i32) -> Resolved {
        let (a, b) = (a as u32, b as u32);
        let mut units = [UnitOptions {
            style: LONG,
            always: false,
        }; 10];
        for (i, u) in units.iter_mut().enumerate() {
            *u = if i < HOURS {
                UnitOptions::from_bits(a >> (6 + 4 * i) & 15)
            } else {
                UnitOptions::from_bits(b >> (4 * (i - HOURS)) & 15)
            };
        }
        let fd = (a >> 2 & 15) as u8;
        Resolved {
            locale,
            numbering_system,
            style: (a & 3) as u8,
            units,
            fractional_digits: (fd <= 9).then_some(fd),
        }
    }
}

/// The GetOption values of the `unit` option of each unit (Table 20, column Values).
fn styles_list(unit: usize) -> &'static [&'static str] {
    match unit {
        0..=3 => &UNIT_STYLES[..3],
        4..=6 => &UNIT_STYLES[..5],
        _ => &UNIT_STYLES[..4],
    }
}

/// The digital default style of each unit (Table 20, column Digital Default): "short" for
/// the date units, "numeric" for the others.
fn digital_base(unit: usize) -> u8 {
    if unit < HOURS { 1 } else { NUMERIC }
}

/// IsFractionalSecondUnitName ( unit ): milliseconds, microseconds or nanoseconds.
fn is_fractional_second_unit(unit: usize) -> bool {
    unit > SECONDS
}

/// GetDurationUnitOptions ( unit, options, baseStyle, stylesList, digitalBase, prevStyle,
/// twoDigitHours ) for the unit index `unit`; `prev` is prevStyle (`None`: the
/// empty String). twoDigitHours is false for every locale, as in V8 and SpiderMonkey (their
/// output keeps "numeric" hours; `[[DigitalFormat]]` is implementation locale data), so the
/// twoDigitHours step never changes the style.
fn get_duration_unit_options<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    unit: usize,
    base_style: usize,
    prev: Option<u8>,
) -> JsResult<UnitOptions> {
    let name = UNITS[unit];
    // 1. Let style be ? GetOption(options, unit, string, stylesList, undefined).
    let given = get_string_option(cx, options, name, styles_list(unit), None)?;
    let mut style = given
        .as_deref()
        .map(|s| UNIT_STYLES.iter().position(|x| *x == s).unwrap_or(0) as u8);
    // 2. Let displayDefault be "always".
    let mut always_default = true;
    let prev_numeric = matches!(prev, Some(NUMERIC | TWO_DIGIT | FRACTIONAL));
    // 3. If style is undefined, then
    if style.is_none() {
        if base_style == DIGITAL {
            // a. baseStyle "digital": the units other than hours, minutes and seconds show
            //    only when nonzero; style is digitalBase.
            if !(HOURS..=SECONDS).contains(&unit) {
                always_default = false;
            }
            style = Some(digital_base(unit));
        } else if prev_numeric {
            // b. After a numeric unit: "numeric"; only minutes and seconds always show.
            style = Some(NUMERIC);
            if unit != MINUTES && unit != SECONDS {
                always_default = false;
            }
        } else {
            // c. Else baseStyle, shown only when nonzero.
            style = Some(base_style as u8);
            always_default = false;
        }
    }
    let mut style = style.unwrap_or(LONG);
    // 4. If style is "numeric" and IsFractionalSecondUnitName(unit) is true, then set style
    //    to "fractional" and displayDefault to "auto".
    if style == NUMERIC && is_fractional_second_unit(unit) {
        style = FRACTIONAL;
        always_default = false;
    }
    // 5-6. Let display be ? GetOption(options, displayField, string, « "auto", "always" »,
    //      displayDefault).
    let display = get_string_option(
        cx,
        options,
        DISPLAY_NAMES[unit],
        &["auto", "always"],
        Some(if always_default { "always" } else { "auto" }),
    )?;
    let always = display.as_deref() == Some("always");
    // 7. Perform ? ValidateDurationUnitStyle(unit, style, display, prevStyle).
    //    1. A fractional unit is never shown when zero.
    if always && style == FRACTIONAL {
        return Err(cx.throw_range_error(&format!(
            "{name}Display \"always\" is not allowed with a numeric {name} style"
        )));
    }
    //    2. After a fractional unit, every unit is fractional.
    if prev == Some(FRACTIONAL) && style != FRACTIONAL {
        return Err(cx.throw_range_error(&format!(
            "{name} must be \"numeric\" after a numeric sub-second unit"
        )));
    }
    //    3. After a numeric or 2-digit unit, every unit is numeric.
    if matches!(prev, Some(NUMERIC | TWO_DIGIT))
        && !matches!(style, NUMERIC | TWO_DIGIT | FRACTIONAL)
    {
        return Err(cx.throw_range_error(&format!(
            "{name} style \"{}\" is not allowed after a numeric unit",
            UNIT_STYLES[style as usize]
        )));
    }
    // 8. (twoDigitHours is false.) 9. Minutes and seconds after a numeric unit are 2-digit.
    if (unit == MINUTES || unit == SECONDS) && matches!(prev, Some(NUMERIC | TWO_DIGIT)) {
        style = TWO_DIGIT;
    }
    // 10. Return the record.
    Ok(UnitOptions { style, always })
}

/// Intl.DurationFormat steps 3-16 (13.1.1; the object creation of step 2 is the caller's): the
/// internal slots for `locales` and `options`, in the spec order of the observable steps.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 3. ResolveOptions(%Intl.DurationFormat%, ..., locales, options): CanonicalizeLocaleList,
    //    GetOptionsObject (no coerce: null is a TypeError), localeMatcher, the option of the
    //    relevant extension key "nu", ResolveLocale.
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    let options = get_options_object(cx, options)?;
    get_locale_matcher(cx, options)?;
    let nu = get_string_option(cx, options, "numberingSystem", &[], None)?;
    if let Some(n) = &nu
        && !is_unicode_type(n)
    {
        return Err(cx.throw_range_error(&format!("Invalid numberingSystem : {n}")));
    }
    let r = resolve::resolve_locale(cx, &requested, &[("nu", nu)], &["nu"], &|l, _| {
        number_format::nu_data(l)
    })?;
    // 5-11. [[Locale]], [[NumberingSystem]] = r.[[nu]]; [[HourMinuteSeparator]] and
    //       [[MinuteSecondSeparator]] come from the data locale at each format (the cache).
    let numbering_system = r
        .keywords
        .iter()
        .find(|(k, _)| k == "nu")
        .map_or_else(|| "latn".to_string(), |(_, v)| v.clone());
    // 12-13. Let style be ? GetOption(options, "style", string, « "long", "short", "narrow",
    //        "digital" », "short").
    let style = get_string_option(cx, options, "style", &STYLES, Some(STYLES[1]))?;
    let style = STYLES
        .iter()
        .position(|s| Some(*s) == style.as_deref())
        .unwrap_or(1);
    // 14-15. prevStyle is "" first; for each row of Table 20, GetDurationUnitOptions; the
    //        style of hours .. microseconds is the prevStyle of the next unit.
    let mut units = [UnitOptions {
        style: LONG,
        always: false,
    }; 10];
    let mut prev = None;
    for (unit, slot) in units.iter_mut().enumerate() {
        let u = get_duration_unit_options(cx, options, unit, style, prev)?;
        *slot = u;
        if unit >= HOURS {
            prev = Some(u.style);
        }
    }
    // 16. Set durationFormat.[[FractionalDigits]] to ? GetNumberOption(options,
    //     "fractionalDigits", 0, 9, undefined).
    let fd = get_number_option(cx, options, "fractionalDigits", 0, 9, None)?;
    Ok(Resolved {
        locale: r.locale,
        numbering_system,
        style: style as u8,
        units,
        fractional_digits: fd.map(|d| d as u8),
    })
}

// ---- the duration record ----------------------------------------------------------------------

/// A duration record: the ten fields in [`UNITS`] order, each an integral Number (never
/// -0), valid by IsValidDuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record(pub [f64; 10]);

/// 2**53, the bound of IsValidDuration on the time part, in seconds.
const MAX_SECONDS: i128 = 1 << 53;
/// The nanoseconds of each unit from days on (days .. nanoseconds).
const NANOS: [i128; 7] = [
    86_400_000_000_000,
    3_600_000_000_000,
    60_000_000_000,
    1_000_000_000,
    1_000_000,
    1_000,
    1,
];

/// The exact integer of the integral Number `x`, if its magnitude is below 2**100 (an `as`
/// cast is exact there). A larger field makes every record invalid.
fn exact_integer(x: f64) -> Option<i128> {
    (x.abs() < 1_267_650_600_228_229_401_496_703_205_376.0).then_some(x as i128)
}

/// IsIntegralNumber ( x ) for a Number: finite, and with no fraction. No float library call
/// (H36): every finite Number of magnitude 2**53 or more is an integer; below that an `i64`
/// cast is exact.
fn is_integral(x: f64) -> bool {
    if !x.is_finite() {
        return false;
    }
    x.abs() >= 9_007_199_254_740_992.0 || (x as i64) as f64 == x
}

impl Record {
    /// DurationSign ( duration ): the sign of the first nonzero field.
    pub fn sign(&self) -> i8 {
        for v in self.0 {
            if v < 0.0 {
                return -1;
            }
            if v > 0.0 {
                return 1;
            }
        }
        0
    }

    /// IsValidDuration ( years, ..., nanoseconds ) (Temporal): one sign for
    /// all fields; years, months and weeks below 2**32; the time part (days to nanoseconds)
    /// below 2**53 seconds. The time part is an exact `i128` sum in nanoseconds
    /// (INTL-V2-FP4): its bound is about 9.0072e24 ns.
    pub fn is_valid(&self) -> bool {
        let mut sign = 0i8;
        for v in self.0 {
            if !v.is_finite() {
                return false;
            }
            let s = if v < 0.0 {
                -1
            } else if v > 0.0 {
                1
            } else {
                0
            };
            if s != 0 {
                if sign != 0 && sign != s {
                    return false;
                }
                sign = s;
            }
        }
        if self.0[..3].iter().any(|v| v.abs() >= 4_294_967_296.0) {
            return false;
        }
        let mut total: i128 = 0;
        for (v, n) in self.0[3..].iter().zip(NANOS) {
            let Some(x) = exact_integer(*v) else {
                return false;
            };
            let Some(t) = x.checked_mul(n).and_then(|p| total.checked_add(p)) else {
                return false;
            };
            total = t;
        }
        total.abs() < MAX_SECONDS * 1_000_000_000
    }
}

/// ToIntegerIfIntegral ( argument ): ? ToNumber, a RangeError for a Number that is not
/// integral; -0 becomes +0.
fn to_integer_if_integral<'s>(cx: &mut Cx<'s>, v: Local<'s>, name: &str) -> JsResult<f64> {
    let x = cx.to_number(v)?;
    if !is_integral(x) {
        let text = cx.to_rust_string(Local::number(x))?;
        return Err(cx.throw_range_error(&format!("{name} must be an integer, not {text}")));
    }
    Ok(x + 0.0)
}

/// The field reads of ToDurationRecord, in their order (alphabetical, as Temporal's
/// ToTemporalPartialDurationRecord): days, hours, microseconds, milliseconds, minutes,
/// months, nanoseconds, seconds, weeks, years (indices in [`UNITS`]).
const READ_ORDER: [usize; 10] = [3, 4, 8, 7, 5, 1, 9, 6, 2, 0];

/// ToDurationRecord ( input ) (ECMA-402 §13). A String is a RangeError and any other
/// value that is not an Object a TypeError. Every Object, a Temporal.Duration too, is read
/// through its properties: the reads of a Temporal.Duration's internal slots and of a
/// duration String (Temporal's ToTemporalDuration) need an `ep9_temporal` reader with std
/// types, which this crate does not have yet (the `intl_view` reader of the I1 design §1.3).
pub fn to_duration_record<'s>(cx: &mut Cx<'s>, input: Local<'s>) -> JsResult<Record> {
    // 1. If input is not an Object, then
    let Some(o) = input.as_object() else {
        // a. If input is a String, throw a RangeError exception.
        if input.is_string() {
            return Err(cx.throw_range_error("Invalid duration string"));
        }
        // b. Throw a TypeError exception.
        return Err(cx.throw_type_error("A duration must be an object"));
    };
    // 2-24. Each field: ? Get, then ToIntegerIfIntegral if it is not undefined.
    let mut fields = [0.0; 10];
    let mut any = false;
    for i in READ_ORDER {
        let v = cx.get(o, UNITS[i])?;
        if !v.is_undefined() {
            fields[i] = to_integer_if_integral(cx, v, UNITS[i])?;
            any = true;
        }
    }
    // 25. If every field is undefined, throw a TypeError exception.
    if !any {
        return Err(cx.throw_type_error("A duration needs at least one duration field"));
    }
    // 26. If IsValidDuration(...) is false, throw a RangeError exception.
    let r = Record(fields);
    if !r.is_valid() {
        return Err(cx.throw_range_error("Invalid duration: a field is out of range"));
    }
    Ok(r)
}

// ---- the locale data (crate::cache) -----------------------------------------------------------

/// The `[[DigitalFormat]]` of one data locale: the separator between hours, minutes and seconds
/// (the ICU4X `UnitsDurationDigitalV1` data, one separator for both places).
pub struct Digital {
    separator: String,
}

impl Digital {
    /// The data of `data_locale` (canonical, no extension); `None` if the data has none (the
    /// compiled data falls back to root, so not the case).
    pub fn new(data_locale: &str) -> Option<Digital> {
        let locale = icu_locale(data_locale);
        let prefs = DurationFormatterPreferences::from(&locale).locale_preferences;
        let id = UnitsDurationDigitalV1::make_locale(prefs);
        let response = DataProvider::<UnitsDurationDigitalV1>::load(
            &Baked,
            DataRequest {
                id: DataIdentifierBorrowed::for_locale(&id),
                ..Default::default()
            },
        )
        .ok()?;
        Some(Digital {
            separator: response.payload.get().separator.to_string(),
        })
    }

    /// The time separator.
    pub fn separator(&self) -> &str {
        &self.separator
    }
}

/// The cached digital data of the `[[Locale]]` `locale` (":" if there is none).
fn digital_for(locale: &str) -> Option<Rc<Digital>> {
    let data_locale = tag::remove_unicode_extension(locale);
    cache::get_or_make(Service::DurationFormat, &data_locale, 0, || {
        Digital::new(&data_locale).or_else(|| Digital::new("und"))
    })
}

// ---- PartitionDurationFormatPattern ----------------------------------------------------

/// A part of a formatted duration: its `type`, `value` and, for the parts of a formatted
/// unit value, the NumberFormat unit (the `unit` of formatToParts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub ty: &'static str,
    pub value: String,
    pub unit: Option<&'static str>,
}

/// The exact value of the unit `unit` with the next units folded in as its fraction
/// (PartitionDurationFormatPattern step 4.h.ii.1 and FormatNumericUnits step 8:
/// value + ComputeFractionalDigits). The fractional units are the units after `unit` (a
/// "fractional" style makes every later unit "fractional"). INTL-V2-FP4: if they are all
/// zero, the Number of `unit` (its digits, as NumberFormat formats a Number); else one `i128`
/// sum in the smallest unit, then one `Decimal` moved by 10^-(3 x the fractional unit count).
/// No `f64` add, multiply or divide.
fn fractional_value(cx: &mut Cx<'_>, d: &Record, unit: usize) -> JsResult<IntlMv> {
    let rest = &d.0[unit + 1..];
    if rest.iter().all(|v| *v == 0.0) {
        return decimal::mv_of_number(cx, d.0[unit]);
    }
    let mut total: i128 = 0;
    for v in &d.0[unit..] {
        // A valid record: each field below 2**53 x 10^9, so the sum fits in i128.
        let x = exact_integer(*v).unwrap_or(0);
        total = total.saturating_mul(1000).saturating_add(x);
    }
    let mut dec = Decimal::from(total);
    dec.multiply_pow10(-(3 * rest.len() as i16));
    dec.absolute.trim_start();
    dec.absolute.trim_end();
    Ok(IntlMv::Finite(dec))
}

/// The parts of `x` formatted by `nf`, each with the NumberFormat unit `unit`.
fn number_parts(
    nf: &number_format::Resolved,
    x: &IntlMv,
    unit: &'static str,
    out: &mut Vec<Part>,
    work: &mut Work,
) -> Result<(), Interrupted> {
    for (t, v) in number_format::partition_numeric(nf, x, work)? {
        work.add(v.len())?;
        out.push(Part {
            ty: t.name(),
            value: v,
            unit: Some(unit),
        });
    }
    Ok(())
}

/// The value of the unit `unit` as an Intl mathematical value; -0 if `negative_zero`.
fn unit_value(cx: &mut Cx<'_>, x: f64, negative_zero: bool) -> JsResult<IntlMv> {
    decimal::mv_of_number(cx, if negative_zero { -0.0 } else { x })
}

/// The NumberFormat of FormatNumericHours, FormatNumericMinutes and FormatNumericSeconds
/// for the unit `unit`: the decimal style with the numbering system, 2 integer
/// digits for "2-digit", signDisplay "never" if the sign is shown already, no grouping; for
/// seconds the fraction digits of `[[FractionalDigits]]` (else 0 to 9) and roundingMode
/// "trunc".
fn numeric_format(r: &Resolved, unit: usize, sign_displayed: bool) -> number_format::Resolved {
    let mut nf = number_format::decimal_defaults(&r.locale, &r.numbering_system);
    if r.units[unit].style == TWO_DIGIT {
        nf.digits.minimum_integer_digits = 2;
    }
    if !sign_displayed {
        nf.sign_display = SignDisplay::Never;
    }
    nf.grouping = Grouping::Never;
    if unit == SECONDS {
        set_fraction_digits(r, &mut nf);
    }
    nf
}

/// The fraction digits of a value with a fraction: `[[FractionalDigits]]` for both bounds,
/// else 0 to 9, and roundingMode "trunc".
fn set_fraction_digits(r: &Resolved, nf: &mut number_format::Resolved) {
    let (min, max) = match r.fractional_digits {
        Some(d) => (d, d),
        None => (0, 9),
    };
    nf.digits.minimum_fraction_digits = min;
    nf.digits.maximum_fraction_digits = max;
    nf.digits.rounding_mode = RoundingMode::Trunc;
}

/// FormatNumericUnits ( durationFormat, duration, firstNumericUnit, signDisplayed ):
/// the parts of hours, minutes and seconds (from `first` on) with the time separators; empty
/// if no unit is shown.
fn format_numeric_units(
    cx: &mut Cx<'_>,
    r: &Resolved,
    d: &Record,
    first: usize,
    mut sign_displayed: bool,
) -> JsResult<Vec<Part>> {
    // 3-9. The values and displays; secondsValue has the sub-second fields as its fraction.
    let hours = d.0[HOURS];
    let minutes = d.0[MINUTES];
    let seconds = fractional_value(cx, d, SECONDS)?;
    let seconds_zero = d.0[SECONDS..].iter().all(|v| *v == 0.0);
    // 10-11. hoursFormatted.
    let hours_formatted = first == HOURS && (hours != 0.0 || r.units[HOURS].always);
    // 12-13. secondsFormatted.
    let seconds_formatted = !seconds_zero || r.units[SECONDS].always;
    // 14-15. minutesFormatted.
    let minutes_formatted = (first == HOURS || first == MINUTES)
        && ((hours_formatted && seconds_formatted) || minutes != 0.0 || r.units[MINUTES].always);
    let negative = d.sign() < 0;
    let separator = digital_for(&r.locale).map_or_else(|| ":".to_string(), |g| g.separator.clone());
    let mut out = Vec::new();
    // 16. Hours: -0 for a negative duration if the sign is not shown yet.
    if hours_formatted {
        let x = unit_value(cx, hours, sign_displayed && hours == 0.0 && negative)?;
        let nf = numeric_format(r, HOURS, sign_displayed);
        polled(cx, |w| number_parts(&nf, &x, NF_UNITS[HOURS], &mut out, w))?;
        sign_displayed = false;
    }
    // 17. Minutes, after the hour-minute separator if hours are shown.
    if minutes_formatted {
        if hours_formatted {
            out.push(Part {
                ty: "literal",
                value: separator.clone(),
                unit: None,
            });
        }
        let x = unit_value(cx, minutes, sign_displayed && minutes == 0.0 && negative)?;
        let nf = numeric_format(r, MINUTES, sign_displayed);
        polled(cx, |w| {
            number_parts(&nf, &x, NF_UNITS[MINUTES], &mut out, w)
        })?;
        sign_displayed = false;
    }
    // 18. Seconds, after the minute-second separator if minutes are shown.
    if seconds_formatted {
        if minutes_formatted {
            out.push(Part {
                ty: "literal",
                value: separator,
                unit: None,
            });
        }
        let nf = numeric_format(r, SECONDS, sign_displayed);
        polled(cx, |w| {
            number_parts(&nf, &seconds, NF_UNITS[SECONDS], &mut out, w)
        })?;
    }
    Ok(out)
}

/// NextUnitFractional ( durationFormat, unit ): the unit after seconds,
/// milliseconds or microseconds is "fractional".
fn next_unit_fractional(r: &Resolved, unit: usize) -> bool {
    (SECONDS..SECONDS + 3).contains(&unit) && r.units[unit + 1].style == FRACTIONAL
}

/// ListFormatParts ( durationFormat, partitionedPartsList ): the list of the
/// elements (type "unit", the style, "short" for "digital"), with the parts of each element
/// in the place of its "element" part.
fn list_format_parts(r: &Resolved, elements: Vec<Vec<Part>>) -> Vec<Part> {
    // 1-6. The ListFormat of [[Locale]], type "unit", style [[Style]] ("digital": "short").
    let style = if r.style as usize == DIGITAL {
        1
    } else {
        r.style as usize
    };
    let options = list_format::Packed::new(2, style);
    // 7. The text of each element.
    let strings: Vec<Vec<u16>> = elements
        .iter()
        .map(|parts| parts.iter().flat_map(|p| p.value.encode_utf16()).collect())
        .collect();
    // 8-11. CreatePartsFromList, then each element by its parts.
    let mut elements = elements.into_iter();
    let mut out = Vec::new();
    for (kind, value) in list_format::create_parts_from_list(&r.locale, options, &strings) {
        match kind {
            PartKind::Element => out.extend(elements.next().unwrap_or_default()),
            PartKind::Literal => out.push(Part {
                ty: "literal",
                value: String::from_utf16_lossy(&value),
                unit: None,
            }),
        }
    }
    out
}

/// PartitionDurationFormatPattern ( durationFormat, duration ).
pub fn partition_duration_format_pattern(
    cx: &mut Cx<'_>,
    r: &Resolved,
    d: &Record,
) -> JsResult<Vec<Part>> {
    // 1-3. result, signDisplayed, numericUnitFound.
    let mut result: Vec<Vec<Part>> = Vec::new();
    let mut sign_displayed = true;
    let negative = d.sign() < 0;
    // 4. While numericUnitFound is false, for each row of Table 20 in order:
    for (unit, &o) in r.units.iter().enumerate() {
        // g. A numeric or 2-digit unit: FormatNumericUnits, then stop.
        if o.is_numeric() {
            let parts = format_numeric_units(cx, r, d, unit, sign_displayed)?;
            if !parts.is_empty() {
                result.push(parts);
            }
            break;
        }
        // A fractional unit is in the value of the unit before it (not reached: the unit
        // before stops the loop).
        if o.style == FRACTIONAL {
            break;
        }
        // h. Else, the unit on its own with style "unit".
        let mut nf = number_format::unit_defaults(
            &r.locale,
            &r.numbering_system,
            NF_UNITS[unit],
            UNIT_STYLES[o.style as usize],
        );
        // ii. The next unit is fractional: the value has the next units as its fraction.
        let fractional = next_unit_fractional(r, unit);
        let (value, zero) = if fractional {
            set_fraction_digits(r, &mut nf);
            (
                Some(fractional_value(cx, d, unit)?),
                d.0[unit..].iter().all(|v| *v == 0.0),
            )
        } else {
            (None, d.0[unit] == 0.0)
        };
        // iii. If display is "always" or the value is not 0:
        if o.always || !zero {
            // 2-3. The first unit shown has the sign (-0 for a zero value of a negative
            //      duration); the others signDisplay "never".
            let value = if sign_displayed {
                sign_displayed = false;
                match value {
                    Some(v) if !zero => v,
                    _ => unit_value(cx, d.0[unit], zero && negative)?,
                }
            } else {
                nf.sign_display = SignDisplay::Never;
                match value {
                    Some(v) => v,
                    None => unit_value(cx, d.0[unit], false)?,
                }
            };
            // 4-11. The parts of the value, each with the NumberFormat unit.
            let mut parts = Vec::new();
            polled(cx, |w| {
                number_parts(&nf, &value, NF_UNITS[unit], &mut parts, w)
            })?;
            result.push(parts);
        }
        if fractional {
            break;
        }
    }
    // 5. Return ListFormatParts(durationFormat, result).
    Ok(list_format_parts(r, result))
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.DurationFormat ( [ locales [ , options ] ] ) (13.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.DurationFormat requires 'new'"));
    };
    // 2. Let durationFormat be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%Intl.DurationFormat.prototype%", « [[InitializedDurationFormat]], ... »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlDurationFormatPrototype)?;
    let df = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3-16.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(df, CLASS, SLOT_LOCALE, locale)?;
    let (a, b) = r.pack();
    cx.set_slot(df, CLASS, SLOT_OPTIONS, Local::int32(a))?;
    cx.set_slot(df, CLASS, SLOT_TIME_UNITS, Local::int32(b))?;
    let nu = cx.new_string(&r.numbering_system)?;
    cx.set_slot(df, CLASS, SLOT_NUMBERING_SYSTEM, nu)?;
    // 17. Return durationFormat.
    Ok(df.into())
}

/// Intl.DurationFormat.supportedLocalesOf ( locales [ , options ] ).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.DurationFormat%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The internal slots of the duration format `o` (TypeError if `o` is not an
/// Intl.DurationFormat: RequireInternalSlot(o, `[[InitializedDurationFormat]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<Resolved> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let a = cx.slot(o, CLASS, SLOT_OPTIONS)?.as_int32().unwrap_or(0);
    let b = cx.slot(o, CLASS, SLOT_TIME_UNITS)?.as_int32().unwrap_or(0);
    let nu = cx.slot(o, CLASS, SLOT_NUMBERING_SYSTEM)?;
    let locale = cx.to_rust_string(locale)?;
    let numbering_system = cx.to_rust_string(nu)?;
    Ok(Resolved::unpack(locale, numbering_system, a, b))
}

/// Steps 1-4 of format and formatToParts: the duration format of `this`, ToDurationRecord,
/// PartitionDurationFormatPattern.
fn read_parts<'s>(cx: &mut Cx<'s>, this: Local<'s>, args: Args<'s>) -> JsResult<Vec<Part>> {
    // 1-2. Let df be the this value; ? RequireInternalSlot(df, [[InitializedDurationFormat]]).
    let r = read(cx, this)?;
    // 3. Let record be ? ToDurationRecord(duration).
    let d = to_duration_record(cx, args.get(0))?;
    // 4. Let parts be PartitionDurationFormatPattern(df, record).
    partition_duration_format_pattern(cx, &r, &d)
}

/// Intl.DurationFormat.prototype.format ( duration ).
fn format<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let parts = read_parts(cx, this, args)?;
    // 5-7. The concatenation of the part values.
    let s = polled(cx, |w| {
        let mut out = String::new();
        for p in &parts {
            w.add(p.value.len())?;
            out.push_str(&p.value);
        }
        Ok(out)
    })?;
    Ok(cx.new_string(&s)?.into())
}

/// Intl.DurationFormat.prototype.formatToParts ( duration ).
fn format_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let parts = read_parts(cx, this, args)?;
    // 5-7. An Array of { type, value } and, for a part of a unit value, unit.
    let array = cx.new_array();
    for (i, p) in parts.iter().enumerate() {
        cx.poll_interrupt()?;
        let o = cx.new_object();
        let t = cx.new_string(p.ty)?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string(&p.value)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        if let Some(u) = p.unit {
            let u = cx.new_string(u)?;
            cx.create_data_property_or_throw(o, "unit", u)?;
        }
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Intl.DurationFormat.prototype.resolvedOptions ( ).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let df be the this value; ? RequireInternalSlot(df, [[InitializedDurationFormat]]).
    let r = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 21, in table order: locale, numberingSystem, style, each unit
    //    and its display ("fractional" shows as "numeric"), fractionalDigits if not undefined.
    let put = |cx: &mut Cx<'s>, key: &str, value: &str| -> JsResult<()> {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)
    };
    put(cx, "locale", &r.locale)?;
    put(cx, "numberingSystem", &r.numbering_system)?;
    put(cx, "style", STYLES[(r.style as usize).min(DIGITAL)])?;
    for (unit, u) in r.units.iter().enumerate() {
        let style = if u.style == FRACTIONAL {
            NUMERIC
        } else {
            u.style
        };
        put(cx, UNITS[unit], UNIT_STYLES[style as usize])?;
        put(
            cx,
            DISPLAY_NAMES[unit],
            if u.always { "always" } else { "auto" },
        )?;
    }
    if let Some(fd) = r.fractional_digits {
        cx.create_data_property_or_throw(o, "fractionalDigits", Local::int32(i32::from(fd)))?;
    }
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(f: [f64; 10]) -> Record {
        Record(f)
    }

    #[test]
    fn packed_options_round_trip() {
        let mut units = [UnitOptions {
            style: LONG,
            always: false,
        }; 10];
        for (i, u) in units.iter_mut().enumerate() {
            u.style = (i % 6) as u8;
            u.always = i % 3 == 0;
        }
        for fd in [None, Some(0), Some(9)] {
            for style in 0..4u8 {
                let r = Resolved {
                    locale: "en".into(),
                    numbering_system: "latn".into(),
                    style,
                    units,
                    fractional_digits: fd,
                };
                let (a, b) = r.pack();
                assert_eq!(Resolved::unpack("en".into(), "latn".into(), a, b), r);
            }
        }
    }

    /// IsValidDuration: one sign, the 2**32 bound of years, months and weeks, the 2**53 s
    /// bound of the time part (exact in i128; test262 duration-out-of-range-3.js cases).
    #[test]
    fn valid_durations() {
        assert!(record([0.0; 10]).is_valid());
        assert!(record([4_294_967_295.0, 0., 0., 0., 0., 0., 0., 0., 0., 0.]).is_valid());
        assert!(!record([4_294_967_296.0, 0., 0., 0., 0., 0., 0., 0., 0., 0.]).is_valid());
        assert!(!record([1.0, -1.0, 0., 0., 0., 0., 0., 0., 0., 0.]).is_valid());
        // seconds 2**53 - 1 and 999,999,999 ns: valid; one more ns: not.
        let mut f = [0.0; 10];
        f[SECONDS] = 9_007_199_254_740_991.0;
        f[9] = 999_999_999.0;
        assert!(record(f).is_valid());
        f[9] = 1_000_000_000.0;
        assert!(!record(f).is_valid());
        // Nanoseconds near 9e24: 2**53 x 10^9 (a Number) is the bound; the Number below it
        // (2**30 less) is valid.
        let mut f = [0.0; 10];
        f[9] = 9_007_199_254_740_992_000_000_000.0;
        assert!(!record(f).is_valid());
        f[9] = 9_007_199_254_740_990_926_258_176.0;
        assert!(record(f).is_valid());
        f[9] = -9_007_199_254_740_990_926_258_176.0;
        assert!(record(f).is_valid());
        f[9] = f64::MAX;
        assert!(!record(f).is_valid());
        f[9] = -f64::MAX;
        assert!(!record(f).is_valid());
    }

    #[test]
    fn integral_numbers() {
        for x in [0.0, -0.0, 1.0, -7.0, 9_007_199_254_740_993.0, 1e300] {
            assert!(is_integral(x), "{x}");
        }
        for x in [0.5, -1.25, f64::NAN, f64::INFINITY, 4_503_599_627_370_495.5] {
            assert!(!is_integral(x), "{x}");
        }
    }

    #[test]
    fn digital_separators() {
        assert_eq!(Digital::new("en").unwrap().separator(), ":");
        assert_eq!(Digital::new("da").unwrap().separator(), ".");
        assert!(Digital::new("und").is_some());
    }
}
