//! Temporal.Duration (Temporal §7): module `duration`, block 20 (design §3.2-§3.4, WP T2).
//! Makes `%Temporal.Duration%` and `%Temporal.Duration.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalDuration, 10 slots, each an integral Number (never -0):
//!   0 years  1 months  2 weeks  3 days  4 hours  5 minutes  6 seconds
//!   7 milliseconds  8 microseconds  9 nanoseconds
//! each call: slots -> temporal_rs::Duration::new (exact: the feature
//!   float64_representable_durations keeps every field a float64 integer) -> work -> new object
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-duration.txt`.

use core::cmp::Ordering;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::options::{
    RoundingMode, RoundingOptions, ToStringRoundingOptions, Unit, UnitGroup,
};
use temporal_rs::{Duration, Sign};

use crate::convert::{temporal_text, to_integer_if_integral};
use crate::error::OrThrow;
use crate::options::{
    get_fractional_second_digits, get_options_object, get_parsed_option, get_rounding_increment,
    get_unit, validate_unit,
};
use crate::relative_to::get_relative_to_option;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 20;

/// The internal slots of a Duration object.
const SLOTS: usize = 10;
const CLASS: ObjectClass = ObjectClass::TemporalDuration;

/// The field names, in slot order (also the getter names).
const FIELDS: [&str; SLOTS] = [
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

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Duration", 0, constructor), // 0
    BuiltinDef::method("from", 1, from),                 // 1
    BuiltinDef::method("compare", 2, compare),           // 2
    BuiltinDef::getter("years", get_years),              // 3
    BuiltinDef::getter("months", get_months),            // 4
    BuiltinDef::getter("weeks", get_weeks),              // 5
    BuiltinDef::getter("days", get_days),                // 6
    BuiltinDef::getter("hours", get_hours),              // 7
    BuiltinDef::getter("minutes", get_minutes),          // 8
    BuiltinDef::getter("seconds", get_seconds),          // 9
    BuiltinDef::getter("milliseconds", get_milliseconds), // 10
    BuiltinDef::getter("microseconds", get_microseconds), // 11
    BuiltinDef::getter("nanoseconds", get_nanoseconds),  // 12
    BuiltinDef::getter("sign", get_sign),                // 13
    BuiltinDef::getter("blank", get_blank),              // 14
    BuiltinDef::method("with", 1, with),                 // 15
    BuiltinDef::method("negated", 0, negated),           // 16
    BuiltinDef::method("abs", 0, abs),                   // 17
    BuiltinDef::method("add", 1, add),                   // 18
    BuiltinDef::method("subtract", 1, subtract),         // 19
    BuiltinDef::method("round", 1, round),               // 20
    BuiltinDef::method("total", 1, total),               // 21
    BuiltinDef::method("toString", 0, to_string),        // 22
    BuiltinDef::method("toJSON", 0, to_json),            // 23
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 24
    BuiltinDef::method("valueOf", 0, value_of),          // 25
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalDuration,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalDurationPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalDuration,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalDurationPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalDurationPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalDuration,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.Duration"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("years"), Some(3), None),
            PropDef::accessor(PropKey::Str("months"), Some(4), None),
            PropDef::accessor(PropKey::Str("weeks"), Some(5), None),
            PropDef::accessor(PropKey::Str("days"), Some(6), None),
            PropDef::accessor(PropKey::Str("hours"), Some(7), None),
            PropDef::accessor(PropKey::Str("minutes"), Some(8), None),
            PropDef::accessor(PropKey::Str("seconds"), Some(9), None),
            PropDef::accessor(PropKey::Str("milliseconds"), Some(10), None),
            PropDef::accessor(PropKey::Str("microseconds"), Some(11), None),
            PropDef::accessor(PropKey::Str("nanoseconds"), Some(12), None),
            PropDef::accessor(PropKey::Str("sign"), Some(13), None),
            PropDef::accessor(PropKey::Str("blank"), Some(14), None),
            PropDef::method(PropKey::Str("with"), 15),
            PropDef::method(PropKey::Str("negated"), 16),
            PropDef::method(PropKey::Str("abs"), 17),
            PropDef::method(PropKey::Str("add"), 18),
            PropDef::method(PropKey::Str("subtract"), 19),
            PropDef::method(PropKey::Str("round"), 20),
            PropDef::method(PropKey::Str("total"), 21),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 22),
            PropDef::method(PropKey::Str("toJSON"), 23),
            PropDef::method(PropKey::Str("toLocaleString"), 24),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 25),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("Duration"),
            IntrinsicId::TemporalDuration,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "duration",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The ten fields of `d` as Numbers, in slot order.
fn fields_of(d: &Duration) -> [f64; SLOTS] {
    [
        d.years() as f64,
        d.months() as f64,
        d.weeks() as f64,
        d.days() as f64,
        d.hours() as f64,
        d.minutes() as f64,
        d.seconds() as f64,
        d.milliseconds() as f64,
        d.microseconds() as f64,
        d.nanoseconds() as f64,
    ]
}

/// A Duration from ten integral Numbers; a RangeError if they are not a valid duration
/// (IsValidDuration). The `as` casts saturate, so an out-of-range value stays invalid.
fn from_fields(cx: &mut Cx<'_>, f: [f64; SLOTS]) -> JsResult<Duration> {
    Duration::new(
        f[0] as i64,
        f[1] as i64,
        f[2] as i64,
        f[3] as i64,
        f[4] as i64,
        f[5] as i64,
        f[6] as i64,
        f[7] as i64,
        f[8] as i128,
        f[9] as i128,
    )
    .or_throw(cx)
}

/// The Duration in `v` (RequireInternalSlot: a TypeError if `v` is not a Duration).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Duration> {
    let mut f = [0.0; SLOTS];
    for (i, x) in f.iter_mut().enumerate() {
        *x = cx.slot(v, CLASS, i)?.as_number().unwrap_or(0.0);
    }
    from_fields(cx, f)
}

/// CreateTemporalDuration(d, newTarget): a new Duration object.
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    d: &Duration,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalDurationPrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalDurationPrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    for (i, x) in fields_of(d).into_iter().enumerate() {
        cx.set_slot(o, CLASS, i, Local::number(x + 0.0))?;
    }
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// ToTemporalPartialDurationRecord (§7.5.18): the fields in alphabetical order, each
/// ToIntegerIfIntegral; a TypeError if `like` is not an object or has no field.
fn to_partial_duration<'s>(cx: &mut Cx<'s>, like: Local<'s>) -> JsResult<[Option<f64>; SLOTS]> {
    // 1. If temporalDurationLike is not an Object, throw a TypeError exception.
    let Some(o) = like.as_object() else {
        return Err(cx.throw_type_error("a duration-like value must be an object"));
    };
    // 2-14. days, hours, microseconds, milliseconds, minutes, months, nanoseconds, seconds,
    //       weeks, years (alphabetical).
    const ORDER: [usize; SLOTS] = [3, 4, 8, 7, 5, 1, 9, 6, 2, 0];
    let mut out = [None; SLOTS];
    for i in ORDER {
        let k = cx.key(FIELDS[i]);
        let v = cx.get(o, k)?;
        if !v.is_undefined() {
            out[i] = Some(to_integer_if_integral(cx, v)?);
        }
    }
    // 15. If every field is undefined, throw a TypeError exception.
    if out.iter().all(Option::is_none) {
        return Err(cx.throw_type_error("a duration-like object needs at least one field"));
    }
    Ok(out)
}

/// ToTemporalDuration (§7.5.12).
pub(crate) fn to_temporal_duration<'s>(cx: &mut Cx<'s>, item: Local<'s>) -> JsResult<Duration> {
    // 1. If item is an Object with [[InitializedTemporalDuration]], return a copy of it.
    if cx.object_class(item) == Some(CLASS) {
        return read(cx, item);
    }
    // 2. If item is not an Object, then
    if !item.is_object() {
        // a. If item is not a String, throw a TypeError exception.
        let Some(s) = item.as_string() else {
            return Err(cx.throw_type_error("a duration must be an object or a string"));
        };
        // b. Return ? ParseTemporalDurationString(item).
        let t = temporal_text(cx, s)?;
        return Duration::from_utf8(t.as_bytes()).or_throw(cx);
    }
    // 3-5. ToTemporalPartialDurationRecord, missing fields 0, then CreateTemporalDuration.
    let p = to_partial_duration(cx, item)?;
    from_fields(cx, p.map(|x| x.unwrap_or(0.0)))
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.Duration ( [ years [ , ... [ , nanoseconds ] ] ] ) (§7.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.Duration requires 'new'"));
    };
    // 2-11. Each argument: undefined is 0, else ? ToIntegerIfIntegral.
    let mut f = [0.0; SLOTS];
    for (i, x) in f.iter_mut().enumerate() {
        let v = args.get(i);
        if !v.is_undefined() {
            *x = to_integer_if_integral(cx, v)?;
        }
    }
    // 12. Return ? CreateTemporalDuration(..., NewTarget).
    let d = from_fields(cx, f)?;
    create(cx, &d, Some(nt))
}

/// Temporal.Duration.from ( item ) (§7.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? ToTemporalDuration(item).
    let d = to_temporal_duration(cx, args.get(0))?;
    create(cx, &d, None)
}

/// Temporal.Duration.compare ( one, two [ , options ] ) (§7.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set one to ? ToTemporalDuration(one).
    let one = to_temporal_duration(cx, args.get(0))?;
    // 2. Set two to ? ToTemporalDuration(two).
    let two = to_temporal_duration(cx, args.get(1))?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(2))?;
    // 4. Let relativeToRecord be ? GetTemporalRelativeToOption(resolvedOptions).
    let relative_to = get_relative_to_option(cx, options)?;
    // 5-12. Compare (temporal_rs).
    let ord = one
        .compare_with_provider(&two, relative_to, crate::tzdb::provider())
        .or_throw(cx)?;
    Ok(Local::int32(match ord {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }))
}

// ---- getters --------------------------------------------------------------------------------

/// The getter of field `i`: the slot value itself (an integral Number).
fn field<'s>(cx: &mut Cx<'s>, this: Local<'s>, i: usize) -> JsResult<Local<'s>> {
    cx.slot(this, CLASS, i)
}

macro_rules! getters {
    ($($name:ident = $i:expr;)*) => {$(
        fn $name<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            field(cx, this, $i)
        }
    )*};
}

getters! {
    get_years = 0;
    get_months = 1;
    get_weeks = 2;
    get_days = 3;
    get_hours = 4;
    get_minutes = 5;
    get_seconds = 6;
    get_milliseconds = 7;
    get_microseconds = 8;
    get_nanoseconds = 9;
}

/// get Temporal.Duration.prototype.sign (§7.3.12).
fn get_sign<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    Ok(Local::int32(match d.sign() {
        Sign::Positive => 1,
        Sign::Zero => 0,
        Sign::Negative => -1,
    }))
}

/// get Temporal.Duration.prototype.blank (§7.3.13).
fn get_blank<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    Ok(Local::boolean(d.sign() == Sign::Zero))
}

// ---- methods --------------------------------------------------------------------------------

/// Temporal.Duration.prototype.with ( temporalDurationLike ) (§7.3.14).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let d = read(cx, this)?;
    // 3. Let temporalDurationLike be ? ToTemporalPartialDurationRecord(temporalDurationLike).
    let p = to_partial_duration(cx, args.get(0))?;
    // 4-23. Each field: the new value if present, else the old one.
    let old = fields_of(&d);
    let mut f = [0.0; SLOTS];
    for i in 0..SLOTS {
        f[i] = p[i].unwrap_or(old[i]);
    }
    // 24. Return ? CreateTemporalDuration(...).
    let r = from_fields(cx, f)?;
    create(cx, &r, None)
}

/// Temporal.Duration.prototype.negated ( ) (§7.3.15).
fn negated<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    create(cx, &d.negated(), None)
}

/// Temporal.Duration.prototype.abs ( ) (§7.3.16).
fn abs<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    create(cx, &d.abs(), None)
}

/// AddDurations(operation, duration, other) (§7.5.43).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    other: Local<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 1. Set other to ? ToTemporalDuration(other).
    let other = to_temporal_duration(cx, other)?;
    let r = if subtract {
        d.subtract(&other)
    } else {
        d.add(&other)
    }
    .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.Duration.prototype.add ( other ) (§7.3.17).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), false)
}

/// Temporal.Duration.prototype.subtract ( other ) (§7.3.18).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), true)
}

/// Temporal.Duration.prototype.round ( roundTo ) (§7.3.19).
fn round<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let d = read(cx, this)?;
    // 3-5. roundTo: required; a String is { smallestUnit }.
    let round_to = crate::options::string_or_options(cx, args.get(0), "smallestUnit", "roundTo")?;
    // 8. Let largestUnit be ? GetTemporalUnitValuedOption(roundTo, "largestUnit", unset).
    let largest = get_unit(cx, round_to, "largestUnit")?;
    // 9. Let relativeToRecord be ? GetTemporalRelativeToOption(roundTo).
    let relative_to = get_relative_to_option(cx, round_to)?;
    // 10. Let roundingIncrement be ? GetRoundingIncrementOption(roundTo).
    let increment = get_rounding_increment(cx, round_to)?;
    // 11. Let roundingMode be ? GetRoundingModeOption(roundTo, half-expand).
    let mode = get_parsed_option::<RoundingMode>(cx, round_to, "roundingMode")?;
    // 12. Let smallestUnit be ? GetTemporalUnitValuedOption(roundTo, "smallestUnit", unset).
    let smallest = get_unit(cx, round_to, "smallestUnit")?;
    // 13. Perform ? ValidateTemporalUnitValue(smallestUnit, datetime).
    validate_unit(cx, smallest, UnitGroup::DateTime, None)?;
    // 17-18. ValidateTemporalUnitValue(largestUnit, datetime, « auto »).
    validate_unit(cx, largest, UnitGroup::DateTime, Some(Unit::Auto))?;
    // 19-. The rest of the steps (temporal_rs).
    let mut o = RoundingOptions::default();
    o.largest_unit = largest;
    o.smallest_unit = smallest;
    o.increment = increment;
    o.rounding_mode = mode;
    let r = d
        .round_with_provider(o, relative_to, crate::tzdb::provider())
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.Duration.prototype.total ( totalOf ) (§7.3.20).
fn total<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let d = read(cx, this)?;
    // 3-5. totalOf: required; a String is { unit }.
    let total_of = crate::options::string_or_options(cx, args.get(0), "unit", "totalOf")?;
    // 6. Let relativeToRecord be ? GetTemporalRelativeToOption(totalOf).
    let relative_to = get_relative_to_option(cx, total_of)?;
    // 7. Let unit be ? GetTemporalUnitValuedOption(totalOf, "unit", required).
    let Some(unit) = get_unit(cx, total_of, "unit")? else {
        return Err(cx.throw_range_error("unit is required"));
    };
    // 8. Perform ? ValidateTemporalUnitValue(unit, datetime).
    validate_unit(cx, Some(unit), UnitGroup::DateTime, None)?;
    // 9-. The total (temporal_rs).
    let t = d
        .total_with_provider(unit, relative_to, crate::tzdb::provider())
        .or_throw(cx)?;
    Ok(Local::number(t.as_inner()))
}

/// Temporal.Duration.prototype.toString ( \[ options \] ) (§7.3.21).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let d = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4-6. fractionalSecondDigits, roundingMode, smallestUnit (alphabetical).
    let precision = get_fractional_second_digits(cx, options)?;
    let rounding_mode = get_parsed_option::<RoundingMode>(cx, options, "roundingMode")?;
    let smallest_unit = get_unit(cx, options, "smallestUnit")?;
    // 7. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, smallest_unit, UnitGroup::Time, None)?;
    // 8-. ToSecondsStringPrecisionRecord, rounding and the text (temporal_rs).
    let s = d
        .as_temporal_string(ToStringRoundingOptions {
            precision,
            smallest_unit,
            rounding_mode,
        })
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// The default text of `this` (toJSON and toLocaleString).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    let s = d
        .as_temporal_string(ToStringRoundingOptions::default())
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.Duration.prototype.toJSON ( ) (§7.3.22).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.Duration.prototype.toLocaleString ( [ locales [ , options ] ] ) (§7.3.23): ep9 has
/// no ECMA-402, so this is the toString() text (design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.Duration.prototype.valueOf ( ) (§7.3.24): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.Duration cannot be converted to a primitive; use compare() or toString()",
    ))
}
