//! Temporal.Instant (Temporal §8): module `instant`, block 21 (design §3.2-§3.4, WP T3).
//! Makes `%Temporal.Instant%` and `%Temporal.Instant.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalInstant, 2 slots (design §3.2):
//!   0 epoch seconds = floor(ns / 10^9): an integral Number (|s| <= 8.64e12 < 2^53)
//!   1 nanosecond of that second, 0..999,999,999: an int32
//! epochNanoseconds (a BigInt) is made on read with Cx::bigint_from_i128; no BigInt is kept
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-instant.txt`.

use core::cmp::Ordering;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, ToPrimitiveHint, WellKnownSymbol,
};
use temporal_rs::options::{RoundingMode, RoundingOptions, ToStringRoundingOptions, UnitGroup};
use temporal_rs::{Instant, TimeZone};

use crate::convert::temporal_text;
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::to_temporal_time_zone_identifier;
use crate::options::{
    get_difference_settings, get_fractional_second_digits, get_options_object, get_parsed_option,
    get_rounding_increment, get_unit, string_or_options, validate_unit,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 21;

const SLOTS: usize = 2;
const CLASS: ObjectClass = ObjectClass::TemporalInstant;
const NS_PER_SECOND: i128 = 1_000_000_000;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Instant", 1, constructor), // 0
    BuiltinDef::method("from", 1, from),                // 1
    BuiltinDef::method("fromEpochMilliseconds", 1, from_epoch_milliseconds), // 2
    BuiltinDef::method("fromEpochNanoseconds", 1, from_epoch_nanoseconds), // 3
    BuiltinDef::method("compare", 2, compare),          // 4
    BuiltinDef::getter("epochMilliseconds", get_epoch_milliseconds), // 5
    BuiltinDef::getter("epochNanoseconds", get_epoch_nanoseconds), // 6
    BuiltinDef::method("add", 1, add),                  // 7
    BuiltinDef::method("subtract", 1, subtract),        // 8
    BuiltinDef::method("until", 1, until),              // 9
    BuiltinDef::method("since", 1, since),              // 10
    BuiltinDef::method("round", 1, round),              // 11
    BuiltinDef::method("equals", 1, equals),            // 12
    BuiltinDef::method("toString", 0, to_string),       // 13
    BuiltinDef::method("toJSON", 0, to_json),           // 14
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 15
    BuiltinDef::method("valueOf", 0, value_of),         // 16
    BuiltinDef::method(
        "toZonedDateTimeISO",
        1,
        crate::zoned_date_time::instant_to_zoned_date_time_iso,
    ), // 17 (T8)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalInstant,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalInstantPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalInstant,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalInstantPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("fromEpochMilliseconds"), 2),
            PropDef::method(PropKey::Str("fromEpochNanoseconds"), 3),
            PropDef::method(PropKey::Str("compare"), 4),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalInstantPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalInstant,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.Instant"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("epochMilliseconds"), Some(5), None),
            PropDef::accessor(PropKey::Str("epochNanoseconds"), Some(6), None),
            PropDef::method(PropKey::Str("add"), 7),
            PropDef::method(PropKey::Str("subtract"), 8),
            PropDef::method(PropKey::Str("until"), 9),
            PropDef::method(PropKey::Str("since"), 10),
            PropDef::method(PropKey::Str("round"), 11),
            PropDef::method(PropKey::Str("equals"), 12),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 13),
            PropDef::method(PropKey::Str("toJSON"), 14),
            PropDef::method(PropKey::Str("toLocaleString"), 15),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 16),
            PropDef::method(PropKey::Str("toZonedDateTimeISO"), 17),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("Instant"),
            IntrinsicId::TemporalInstant,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "instant",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The Instant in `v` (RequireInternalSlot: a TypeError if `v` is not an Instant).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Instant> {
    let s = cx.slot(v, CLASS, 0)?.as_number().unwrap_or(0.0);
    let n = cx.slot(v, CLASS, 1)?.as_number().unwrap_or(0.0);
    Instant::try_new(s as i128 * NS_PER_SECOND + n as i128).or_throw(cx)
}

/// The two slot values of the epoch nanoseconds `ns`.
pub(crate) fn split_epoch_ns(ns: i128) -> (f64, i32) {
    (
        ns.div_euclid(NS_PER_SECOND) as f64 + 0.0,
        ns.rem_euclid(NS_PER_SECOND) as i32,
    )
}

/// CreateTemporalInstant(i, newTarget): a new Instant object.
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    i: &Instant,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalInstantPrototype)?,
        None => cx.intrinsic(IntrinsicId::TemporalInstantPrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    let (s, n) = split_epoch_ns(i.as_i128());
    cx.set_slot(o, CLASS, 0, Local::number(s))?;
    cx.set_slot(o, CLASS, 1, Local::int32(n))?;
    Ok(o.value())
}

/// An Instant from a BigInt value of epoch nanoseconds: a RangeError if it is out of range
/// (IsValidEpochNanoseconds).
fn from_bigint<'s>(cx: &mut Cx<'s>, b: Local<'s>) -> JsResult<Instant> {
    match cx.bigint_to_i128(b) {
        Some(ns) => Instant::try_new(ns).or_throw(cx),
        None => Err(cx.throw_range_error("epoch nanoseconds out of range")),
    }
}

/// The Instant of `t` epoch milliseconds (a time value: integral, |t| <= 8.64e15). For
/// `Date.prototype.toTemporalInstant`.
pub fn create_from_epoch_milliseconds<'s>(cx: &mut Cx<'s>, t: f64) -> JsResult<Local<'s>> {
    let i = Instant::try_new(t as i128 * 1_000_000).or_throw(cx)?;
    create(cx, &i, None)
}

// ---- abstract operations --------------------------------------------------------------------

/// ToTemporalInstant (§8.5.2).
pub(crate) fn to_temporal_instant<'s>(cx: &mut Cx<'s>, item: Local<'s>) -> JsResult<Instant> {
    // 1. If item is an Object, then
    let item = if item.is_object() {
        // a. If item has an [[InitializedTemporalInstant]] or
        //    [[InitializedTemporalZonedDateTime]] internal slot, return its instant.
        if let Some(i) = instant_of_object(cx, item)? {
            return Ok(i);
        }
        // b. NOTE: This use of ToPrimitive allows Instant-like objects to be converted.
        // c. Set item to ? ToPrimitive(item, string).
        cx.to_primitive(item, ToPrimitiveHint::String)?
    } else {
        item
    };
    // 2. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("an instant must be an object or a string"));
    };
    // 3-8. ParseTemporalInstantString and the range checks (temporal_rs).
    let t = temporal_text(cx, s)?;
    Instant::from_utf8(t.as_bytes()).or_throw(cx)
}

/// The instant of an Instant or ZonedDateTime object, else `None`.
fn instant_of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<Instant>> {
    match cx.object_class(v) {
        Some(CLASS) => read(cx, v).map(Some),
        Some(ObjectClass::TemporalZonedDateTime) => {
            crate::zoned_date_time::read(cx, v).map(|z| Some(z.to_instant()))
        }
        _ => Ok(None),
    }
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.Instant ( epochNanoseconds ) (§8.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.Instant requires 'new'"));
    };
    // 2. Let epochNanoseconds be ? ToBigInt(epochNanoseconds).
    let b = cx.to_bigint(args.get(0))?;
    // 3. If IsValidEpochNanoseconds(epochNanoseconds) is false, throw a RangeError exception.
    let i = from_bigint(cx, b)?;
    // 4. Return ? CreateTemporalInstant(epochNanoseconds, NewTarget).
    create(cx, &i, Some(nt))
}

/// Temporal.Instant.from ( item ) (§8.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? ToTemporalInstant(item).
    let i = to_temporal_instant(cx, args.get(0))?;
    create(cx, &i, None)
}

/// Temporal.Instant.fromEpochMilliseconds ( epochMilliseconds ) (§8.2.3).
fn from_epoch_milliseconds<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set epochMilliseconds to ? ToNumber(epochMilliseconds).
    let ms = cx.to_number(args.get(0))?;
    // 2. Set epochMilliseconds to ? NumberToBigInt(epochMilliseconds).
    if !ms.is_finite() || ms.trunc() != ms {
        return Err(cx.throw_range_error("epochMilliseconds must be an integer"));
    }
    // 3. Let epochNanoseconds be epochMilliseconds × ℤ(10**6).
    // 4. If IsValidEpochNanoseconds(epochNanoseconds) is false, throw a RangeError exception.
    if ms.abs() > 8.64e15 {
        return Err(cx.throw_range_error("epochMilliseconds out of range"));
    }
    let i = Instant::try_new(ms as i128 * 1_000_000).or_throw(cx)?;
    // 5. Return ! CreateTemporalInstant(epochNanoseconds).
    create(cx, &i, None)
}

/// Temporal.Instant.fromEpochNanoseconds ( epochNanoseconds ) (§8.2.4).
fn from_epoch_nanoseconds<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set epochNanoseconds to ? ToBigInt(epochNanoseconds).
    let b = cx.to_bigint(args.get(0))?;
    // 2. If IsValidEpochNanoseconds(epochNanoseconds) is false, throw a RangeError exception.
    let i = from_bigint(cx, b)?;
    // 3. Return ! CreateTemporalInstant(epochNanoseconds).
    create(cx, &i, None)
}

/// -1, 0 or 1 for an ordering.
fn sign_of(o: Ordering) -> Local<'static> {
    Local::int32(match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}

/// Temporal.Instant.compare ( one, two ) (§8.2.5).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set one to ? ToTemporalInstant(one).
    let one = to_temporal_instant(cx, args.get(0))?;
    // 2. Set two to ? ToTemporalInstant(two).
    let two = to_temporal_instant(cx, args.get(1))?;
    // 3. Return 𝔽(CompareEpochNanoseconds(one.[[EpochNanoseconds]], two.[[EpochNanoseconds]])).
    Ok(sign_of(one.as_i128().cmp(&two.as_i128())))
}

// ---- getters --------------------------------------------------------------------------------

/// get Temporal.Instant.prototype.epochMilliseconds (§8.3.3): floor(ns / 10^6).
fn get_epoch_milliseconds<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    Ok(Local::number(
        i.as_i128().div_euclid(1_000_000) as f64 + 0.0,
    ))
}

/// get Temporal.Instant.prototype.epochNanoseconds (§8.3.4): a BigInt.
fn get_epoch_nanoseconds<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    cx.bigint_from_i128(i.as_i128())
}

// ---- methods --------------------------------------------------------------------------------

/// AddDurationToInstant(operation, instant, temporalDurationLike) (§8.5.10).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    like: Local<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    // 1. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let d = to_temporal_duration(cx, like)?;
    // 2-7. (temporal_rs: calendar units are a RangeError; the result range is checked.)
    let r = if subtract { i.subtract(&d) } else { i.add(&d) }.or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.Instant.prototype.add ( temporalDurationLike ) (§8.3.5).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), false)
}

/// Temporal.Instant.prototype.subtract ( temporalDurationLike ) (§8.3.6).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args.get(0), true)
}

/// DifferenceTemporalInstant(operation, instant, other, options) (§8.5.9).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    // 1. Set other to ? ToTemporalInstant(other).
    let other = to_temporal_instant(cx, args.get(0))?;
    // 2. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 3. Let settings be ? GetDifferenceSettings(operation, resolvedOptions, time, « », nanosecond, second).
    let settings = get_difference_settings(cx, options)?;
    // 4-. The difference (temporal_rs).
    let d = if until {
        i.until(&other, settings)
    } else {
        i.since(&other, settings)
    }
    .or_throw(cx)?;
    create_duration(cx, &d, None)
}

/// Temporal.Instant.prototype.until ( other [ , options ] ) (§8.3.7).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.Instant.prototype.since ( other [ , options ] ) (§8.3.8).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.Instant.prototype.round ( roundTo ) (§8.3.9).
fn round<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let i = read(cx, this)?;
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
    // 11-17. The maximum, the increment check and RoundTemporalInstant (temporal_rs).
    let mut o = RoundingOptions::default();
    o.smallest_unit = Some(smallest);
    o.increment = increment;
    o.rounding_mode = mode;
    let r = i.round(o).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.Instant.prototype.equals ( other ) (§8.3.10).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    // 3. Set other to ? ToTemporalInstant(other).
    let other = to_temporal_instant(cx, args.get(0))?;
    // 4. If instant.[[EpochNanoseconds]] ≠ other.[[EpochNanoseconds]], return false.
    Ok(Local::boolean(i.as_i128() == other.as_i128()))
}

/// Temporal.Instant.prototype.toString ( [ options ] ) (§8.3.11).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot.
    let i = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4-7. fractionalSecondDigits, roundingMode, smallestUnit, then Get(timeZone).
    let precision = get_fractional_second_digits(cx, options)?;
    let rounding_mode = get_parsed_option::<RoundingMode>(cx, options, "roundingMode")?;
    let smallest_unit = get_unit(cx, options, "smallestUnit")?;
    let tz_value = match options {
        Some(o) => {
            let k = cx.key("timeZone");
            cx.get(o, k)?
        }
        None => Local::undefined(),
    };
    // 8. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, smallest_unit, UnitGroup::Time, None)?;
    // 9. If smallestUnit is hour, throw a RangeError exception.
    if smallest_unit == Some(temporal_rs::options::Unit::Hour) {
        return Err(cx.throw_range_error("smallestUnit must not be hour"));
    }
    // 10. If timeZone is not undefined, set timeZone to ? ToTemporalTimeZoneIdentifier(timeZone).
    let tz: Option<TimeZone> = if tz_value.is_undefined() {
        None
    } else {
        Some(to_temporal_time_zone_identifier(cx, tz_value)?)
    };
    // 11-. Rounding and TemporalInstantToString (temporal_rs).
    let s = i
        .to_ixdtf_string_with_provider(
            tz,
            ToStringRoundingOptions {
                precision,
                smallest_unit,
                rounding_mode,
            },
            crate::tzdb::provider(),
        )
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// TemporalInstantToString(instant, undefined, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let i = read(cx, this)?;
    let s = i
        .to_ixdtf_string_with_provider(
            None,
            ToStringRoundingOptions::default(),
            crate::tzdb::provider(),
        )
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.Instant.prototype.toJSON ( ) (§8.3.13).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.Instant.prototype.toLocaleString ( ) (§8.3.12): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.Instant.prototype.valueOf ( ) (§8.3.14): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.Instant cannot be converted to a primitive; use compare() or equals()",
    ))
}
