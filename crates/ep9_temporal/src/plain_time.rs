//! Temporal.PlainTime (Temporal §4): module `plain_time`, block 22 (design §3.2-§3.4, WP T4).
//! Makes `%Temporal.PlainTime%` and `%Temporal.PlainTime.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalPlainTime, 2 slots (design §3.2):
//!   0 second of the day, 0..86,399: an int32 (hour * 3600 + minute * 60 + second)
//!   1 nanosecond of that second, 0..999,999,999: an int32 (ms * 10^6 + us * 10^3 + ns)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plain_time.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::PlainTime;
use temporal_rs::options::{
    Overflow, RoundingMode, RoundingOptions, ToStringRoundingOptions, Unit, UnitGroup,
};

use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object};
use crate::options::{
    Options, get_difference_settings, get_fractional_second_digits, get_options_object,
    get_parsed_option, get_rounding_increment, get_unit, string_or_options, validate_unit,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 22;

const SLOTS: usize = 2;
const CLASS: ObjectClass = ObjectClass::TemporalPlainTime;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PlainTime", 0, constructor), // 0
    BuiltinDef::method("from", 1, from),                  // 1
    BuiltinDef::method("compare", 2, compare),            // 2
    BuiltinDef::getter("hour", get_hour),                 // 3
    BuiltinDef::getter("minute", get_minute),             // 4
    BuiltinDef::getter("second", get_second),             // 5
    BuiltinDef::getter("millisecond", get_millisecond),   // 6
    BuiltinDef::getter("microsecond", get_microsecond),   // 7
    BuiltinDef::getter("nanosecond", get_nanosecond),     // 8
    BuiltinDef::method("add", 1, add),                    // 9
    BuiltinDef::method("subtract", 1, subtract),          // 10
    BuiltinDef::method("with", 1, with),                  // 11
    BuiltinDef::method("until", 1, until),                // 12
    BuiltinDef::method("since", 1, since),                // 13
    BuiltinDef::method("round", 1, round),                // 14
    BuiltinDef::method("equals", 1, equals),              // 15
    BuiltinDef::method("toString", 0, to_string),         // 16
    BuiltinDef::method("toJSON", 0, to_json),             // 17
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 18
    BuiltinDef::method("valueOf", 0, value_of),           // 19
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainTime,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainTimePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalPlainTime,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalPlainTimePrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalPlainTimePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalPlainTime,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.PlainTime"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("hour"), Some(3), None),
            PropDef::accessor(PropKey::Str("minute"), Some(4), None),
            PropDef::accessor(PropKey::Str("second"), Some(5), None),
            PropDef::accessor(PropKey::Str("millisecond"), Some(6), None),
            PropDef::accessor(PropKey::Str("microsecond"), Some(7), None),
            PropDef::accessor(PropKey::Str("nanosecond"), Some(8), None),
            PropDef::method(PropKey::Str("add"), 9),
            PropDef::method(PropKey::Str("subtract"), 10),
            PropDef::method(PropKey::Str("with"), 11),
            PropDef::method(PropKey::Str("until"), 12),
            PropDef::method(PropKey::Str("since"), 13),
            PropDef::method(PropKey::Str("round"), 14),
            PropDef::method(PropKey::Str("equals"), 15),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 16),
            PropDef::method(PropKey::Str("toJSON"), 17),
            PropDef::method(PropKey::Str("toLocaleString"), 18),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 19),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("PlainTime"),
            IntrinsicId::TemporalPlainTime,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plain_time",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The two slot values of `t`.
pub(crate) fn to_slots(t: &PlainTime) -> (i32, i32) {
    (
        i32::from(t.hour()) * 3600 + i32::from(t.minute()) * 60 + i32::from(t.second()),
        i32::from(t.millisecond()) * 1_000_000
            + i32::from(t.microsecond()) * 1_000
            + i32::from(t.nanosecond()),
    )
}

/// The time of the two slot values (both in range: they were made by [`to_slots`]).
pub(crate) fn from_slots(cx: &mut Cx<'_>, s: f64, n: f64) -> JsResult<PlainTime> {
    let (s, n) = (s as i32, n as i32);
    PlainTime::try_new(
        (s / 3600) as u8,
        (s / 60 % 60) as u8,
        (s % 60) as u8,
        (n / 1_000_000) as u16,
        (n / 1_000 % 1_000) as u16,
        (n % 1_000) as u16,
    )
    .or_throw(cx)
}

/// The PlainTime in `v` (RequireInternalSlot: a TypeError if `v` is not a PlainTime).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<PlainTime> {
    let s = cx.slot(v, CLASS, 0)?.as_number().unwrap_or(0.0);
    let n = cx.slot(v, CLASS, 1)?.as_number().unwrap_or(0.0);
    from_slots(cx, s, n)
}

/// CreateTemporalTime(time, newTarget): a new PlainTime object.
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    t: &PlainTime,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalPlainTimePrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalPlainTimePrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    let (s, n) = to_slots(t);
    cx.set_slot(o, CLASS, 0, Local::int32(s))?;
    cx.set_slot(o, CLASS, 1, Local::int32(n))?;
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// GetTemporalOverflowOption (§13.8).
pub(crate) fn get_overflow<'s>(cx: &mut Cx<'s>, options: Options<'s>) -> JsResult<Overflow> {
    Ok(get_parsed_option::<Overflow>(cx, options, "overflow")?.unwrap_or(Overflow::Constrain))
}

/// The time of a PlainTime, PlainDateTime or ZonedDateTime object, else `None`.
fn time_of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<PlainTime>> {
    match cx.object_class(v) {
        Some(CLASS) => read(cx, v).map(Some),
        Some(ObjectClass::TemporalPlainDateTime) => {
            crate::plain_date_time::read(cx, v).map(|dt| Some(dt.to_plain_time()))
        }
        Some(ObjectClass::TemporalZonedDateTime) => {
            crate::zoned_date_time::read(cx, v).map(|z| Some(z.to_plain_time()))
        }
        _ => Ok(None),
    }
}

/// ToTemporalTime(item [, options]) (§4.5.6). `options` is the raw argument (undefined when
/// the caller has none).
pub(crate) fn to_temporal_time<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<PlainTime> {
    // 2. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a-c. A PlainTime, ZonedDateTime or PlainDateTime: read the options, then its time.
        if let Some(t) = time_of_object(cx, item)? {
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return Ok(t);
        }
        // d. Let result be ? ToTemporalTimeRecord(item).
        let bag = fields::read(cx, o, fields::TIME)?;
        if !bag.has_time() {
            return Err(cx.throw_type_error("a time object needs at least one time field"));
        }
        // e. Let resolvedOptions be ? GetOptionsObject(options).
        let options = get_options_object(cx, options)?;
        // f. Let overflow be ? GetTemporalOverflowOption(resolvedOptions).
        let overflow = get_overflow(cx, options)?;
        // g. Set result to ? RegulateTime(...).
        let partial = bag.partial_time(cx, overflow)?;
        return PlainTime::from_partial(partial, Some(overflow)).or_throw(cx);
    }
    // 3. Else,
    //    a. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("a time must be an object or a string"));
    };
    //    b-c. ParseTemporalTimeString (temporal_rs).
    let text = temporal_text(cx, s)?;
    let t = PlainTime::from_utf8(text.as_bytes()).or_throw(cx)?;
    //    d-e. Let resolvedOptions be ? GetOptionsObject(options); GetTemporalOverflowOption.
    let options = get_options_object(cx, options)?;
    get_overflow(cx, options)?;
    Ok(t)
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.PlainTime ( [ hour [ , minute [ , second [ , millisecond [ , microsecond [ ,
/// nanosecond ] ] ] ] ] ] ) (§4.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.PlainTime requires 'new'"));
    };
    // 2-7. Each argument: undefined is 0, else ? ToIntegerWithTruncation.
    let mut f = [0.0f64; 6];
    for (i, slot) in f.iter_mut().enumerate() {
        let v = args.get(i);
        if !v.is_undefined() {
            *slot = to_integer_with_truncation(cx, v)?;
        }
    }
    // 8. If IsValidTime(hour, minute, second, millisecond, microsecond, nanosecond) is false,
    //    throw a RangeError exception.
    let max = [23.0, 59.0, 59.0, 999.0, 999.0, 999.0];
    if f.iter().zip(max).any(|(v, m)| *v < 0.0 || *v > m) {
        return Err(cx.throw_range_error("time field out of range"));
    }
    let t = PlainTime::try_new(
        f[0] as u8,
        f[1] as u8,
        f[2] as u8,
        f[3] as u16,
        f[4] as u16,
        f[5] as u16,
    )
    .or_throw(cx)?;
    // 9-10. Return ? CreateTemporalTime(time, NewTarget).
    create(cx, &t, Some(nt))
}

/// Temporal.PlainTime.from ( item [ , options ] ) (§4.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? ToTemporalTime(item, options).
    let t = to_temporal_time(cx, args.get(0), args.get(1))?;
    create(cx, &t, None)
}

/// -1, 0 or 1 for an ordering.
fn sign_of(o: core::cmp::Ordering) -> Local<'static> {
    Local::int32(o as i32)
}

/// Temporal.PlainTime.compare ( one, two ) (§4.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set one to ? ToTemporalTime(one).
    let one = to_temporal_time(cx, args.get(0), Local::undefined())?;
    // 2. Set two to ? ToTemporalTime(two).
    let two = to_temporal_time(cx, args.get(1), Local::undefined())?;
    // 3. Return 𝔽(CompareTimeRecord(one.[[Time]], two.[[Time]])).
    Ok(sign_of(one.cmp(&two)))
}

// ---- getters --------------------------------------------------------------------------------

/// The getters (§4.3.3-§4.3.8): one field of the time.
fn field<'s>(cx: &mut Cx<'s>, this: Local<'s>, f: fn(&PlainTime) -> u16) -> JsResult<Local<'s>> {
    let t = read(cx, this)?;
    Ok(Local::int32(i32::from(f(&t))))
}

fn get_hour<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, |t| u16::from(t.hour()))
}

fn get_minute<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, |t| u16::from(t.minute()))
}

fn get_second<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, |t| u16::from(t.second()))
}

fn get_millisecond<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, PlainTime::millisecond)
}

fn get_microsecond<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, PlainTime::microsecond)
}

fn get_nanosecond<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    field(cx, this, PlainTime::nanosecond)
}

// ---- methods --------------------------------------------------------------------------------

/// AddDurationToTime(operation, temporalTime, temporalDurationLike) (§4.5.18).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    like: Local<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let t = read(cx, this)?;
    // 1. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let d = to_temporal_duration(cx, like)?;
    // 2-6. AddTime (temporal_rs; the result wraps around the day).
    let r = if subtract { t.subtract(&d) } else { t.add(&d) }.or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainTime.prototype.add ( temporalDurationLike ) (§4.3.9).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), false)
}

/// Temporal.PlainTime.prototype.subtract ( temporalDurationLike ) (§4.3.10).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), true)
}

/// Temporal.PlainTime.prototype.with ( temporalTimeLike [ , options ] ) (§4.3.11).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let t = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalTimeLike) is false, throw a TypeError exception.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial time object"));
    };
    // 4. Let partialTime be ? ToTemporalTimeRecord(temporalTimeLike, partial).
    let bag = fields::read(cx, like, fields::TIME)?;
    if !bag.has_time() {
        return Err(cx.throw_type_error("with() needs at least one time field"));
    }
    // 11. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 12. Let overflow be ? GetTemporalOverflowOption(resolvedOptions).
    let overflow = get_overflow(cx, options)?;
    // 5-10, 13. Fill the missing fields from temporalTime; RegulateTime (temporal_rs).
    let partial = bag.partial_time(cx, overflow)?;
    let r = t.with(partial, Some(overflow)).or_throw(cx)?;
    create(cx, &r, None)
}

/// DifferenceTemporalPlainTime(operation, temporalTime, other, options) (§4.5.17).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let t = read(cx, this)?;
    // 1. Set other to ? ToTemporalTime(other).
    let other = to_temporal_time(cx, args.get(0), Local::undefined())?;
    // 2. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 3. Let settings be ? GetDifferenceSettings(operation, resolvedOptions, time, « »,
    //    nanosecond, hour).
    let settings = get_difference_settings(cx, options)?;
    // 4-. The difference (temporal_rs).
    let d = if until {
        t.until(&other, settings)
    } else {
        t.since(&other, settings)
    }
    .or_throw(cx)?;
    create_duration(cx, &d, None)
}

/// Temporal.PlainTime.prototype.until ( other [ , options ] ) (§4.3.12).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.PlainTime.prototype.since ( other [ , options ] ) (§4.3.13).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.PlainTime.prototype.round ( roundTo ) (§4.3.14).
fn round<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let t = read(cx, this)?;
    // 3-5. roundTo: required; a String is { smallestUnit }.
    let round_to = string_or_options(cx, args.get(0), "smallestUnit", "roundTo")?;
    // 6. NOTE: reads in alphabetical order.
    // 7. Let roundingIncrement be ? GetRoundingIncrementOption(roundTo).
    let increment = get_rounding_increment(cx, round_to)?;
    // 8. Let roundingMode be ? GetRoundingModeOption(roundTo, half-expand).
    let mode = get_parsed_option::<RoundingMode>(cx, round_to, "roundingMode")?;
    // 9. Let smallestUnit be ? GetTemporalUnitValuedOption(roundTo, "smallestUnit", required).
    let Some(smallest) = get_unit(cx, round_to, "smallestUnit")? else {
        return Err(cx.throw_range_error("smallestUnit is required"));
    };
    // 10. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, Some(smallest), UnitGroup::Time, None)?;
    // 11-14. The maximum, the increment check and RoundTime (temporal_rs).
    let mut o = RoundingOptions::default();
    o.smallest_unit = Some(smallest);
    o.increment = increment;
    o.rounding_mode = mode;
    let r = t.round(o).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainTime.prototype.equals ( other ) (§4.3.15).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let t = read(cx, this)?;
    // 3. Set other to ? ToTemporalTime(other).
    let other = to_temporal_time(cx, args.get(0), Local::undefined())?;
    // 4. If CompareTimeRecord(temporalTime.[[Time]], other.[[Time]]) = 0, return true.
    Ok(Local::boolean(t == other))
}

/// Temporal.PlainTime.prototype.toString ( [ options ] ) (§4.3.16).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let t = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4-6. fractionalSecondDigits, roundingMode, smallestUnit.
    let precision = get_fractional_second_digits(cx, options)?;
    let rounding_mode = get_parsed_option::<RoundingMode>(cx, options, "roundingMode")?;
    let smallest_unit = get_unit(cx, options, "smallestUnit")?;
    // 7. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, smallest_unit, UnitGroup::Time, None)?;
    // 8. If smallestUnit is hour, throw a RangeError exception.
    if smallest_unit == Some(Unit::Hour) {
        return Err(cx.throw_range_error("smallestUnit must not be hour"));
    }
    // 9-. ToSecondsStringPrecisionRecord, RoundTime and TimeRecordToString (temporal_rs).
    let s = t
        .to_ixdtf_string(ToStringRoundingOptions {
            precision,
            smallest_unit,
            rounding_mode,
        })
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// TimeRecordToString(temporalTime.[[Time]], auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let t = read(cx, this)?;
    let s = t
        .to_ixdtf_string(ToStringRoundingOptions::default())
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.PlainTime.prototype.toJSON ( ) (§4.3.18).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainTime.prototype.toLocaleString ( ) (§4.3.17): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainTime.prototype.valueOf ( ) (§4.3.19): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.PlainTime cannot be converted to a primitive; use compare() or equals()",
    ))
}
