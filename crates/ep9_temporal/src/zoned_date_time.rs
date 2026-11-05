//! Temporal.ZonedDateTime (Temporal §6): module `zoned_date_time`, block 27 (design
//! §3.2-§3.4, WP T8). Makes `%Temporal.ZonedDateTime%` and
//! `%Temporal.ZonedDateTime.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalZonedDateTime, 4 slots (design §3.2):
//!   0 epoch seconds = floor(ns / 10^9): an integral Number      (as Instant)
//!   1 nanosecond of that second: an int32                      (as Instant)
//!   2 time zone identifier: a String ("Europe/Berlin", "+05:30"); parsed again with the
//!     provider at each call (design B8: speed only; no slot holds a provider index)
//!   3 calendar code (0 = iso8601): an int32
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-zoned_date_time.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::fields::ZonedDateTimeFields;
use temporal_rs::options::{
    Disambiguation, DisplayOffset, DisplayTimeZone, OffsetDisambiguation, RoundingMode,
    RoundingOptions, ToStringRoundingOptions, Unit, UnitGroup,
};
use temporal_rs::parsed_intermediates::ParsedZonedDateTime;
use temporal_rs::partial::PartialZonedDateTime;
use temporal_rs::provider::TransitionDirection;
use temporal_rs::{Instant, TimeZone, ZonedDateTime};

use crate::calendar::{
    self, constructor_calendar, get_calendar_with_iso_default, require_iso,
    to_temporal_calendar_identifier,
};
use crate::convert::temporal_text;
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object, to_temporal_time_zone_identifier};
use crate::instant::split_epoch_ns;
use crate::options::{
    Options, get_difference_settings, get_fractional_second_digits, get_options_object,
    get_parsed_option, get_rounding_increment, get_unit, string_or_options, validate_unit,
};
use crate::plain_date::get_show_calendar;
use crate::plain_date_time::to_time_or_midnight;
use crate::plain_time::get_overflow;
use crate::tzdb::provider;
use crate::{instant, plain_date, plain_date_time, plain_time};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 27;

const SLOTS: usize = 4;
const CLASS: ObjectClass = ObjectClass::TemporalZonedDateTime;
const NS_PER_SECOND: i128 = 1_000_000_000;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("ZonedDateTime", 2, constructor), // 0
    BuiltinDef::method("from", 1, from),                      // 1
    BuiltinDef::method("compare", 2, compare),                // 2
    BuiltinDef::getter("calendarId", get_calendar_id),        // 3
    BuiltinDef::getter("timeZoneId", get_time_zone_id),       // 4
    BuiltinDef::getter("era", get_era),                       // 5
    BuiltinDef::getter("eraYear", get_era_year),              // 6
    BuiltinDef::getter("year", get_year),                     // 7
    BuiltinDef::getter("month", get_month),                   // 8
    BuiltinDef::getter("monthCode", get_month_code),          // 9
    BuiltinDef::getter("day", get_day),                       // 10
    BuiltinDef::getter("hour", get_hour),                     // 11
    BuiltinDef::getter("minute", get_minute),                 // 12
    BuiltinDef::getter("second", get_second),                 // 13
    BuiltinDef::getter("millisecond", get_millisecond),       // 14
    BuiltinDef::getter("microsecond", get_microsecond),       // 15
    BuiltinDef::getter("nanosecond", get_nanosecond),         // 16
    BuiltinDef::getter("epochMilliseconds", get_epoch_milliseconds), // 17
    BuiltinDef::getter("epochNanoseconds", get_epoch_nanoseconds), // 18
    BuiltinDef::getter("dayOfWeek", get_day_of_week),         // 19
    BuiltinDef::getter("dayOfYear", get_day_of_year),         // 20
    BuiltinDef::getter("weekOfYear", get_week_of_year),       // 21
    BuiltinDef::getter("yearOfWeek", get_year_of_week),       // 22
    BuiltinDef::getter("hoursInDay", get_hours_in_day),       // 23
    BuiltinDef::getter("daysInWeek", get_days_in_week),       // 24
    BuiltinDef::getter("daysInMonth", get_days_in_month),     // 25
    BuiltinDef::getter("daysInYear", get_days_in_year),       // 26
    BuiltinDef::getter("monthsInYear", get_months_in_year),   // 27
    BuiltinDef::getter("inLeapYear", get_in_leap_year),       // 28
    BuiltinDef::getter("offsetNanoseconds", get_offset_nanoseconds), // 29
    BuiltinDef::getter("offset", get_offset),                 // 30
    BuiltinDef::method("with", 1, with),                      // 31
    BuiltinDef::method("withPlainTime", 0, with_plain_time),  // 32
    BuiltinDef::method("withTimeZone", 1, with_time_zone),    // 33
    BuiltinDef::method("withCalendar", 1, with_calendar),     // 34
    BuiltinDef::method("add", 1, add),                        // 35
    BuiltinDef::method("subtract", 1, subtract),              // 36
    BuiltinDef::method("until", 1, until),                    // 37
    BuiltinDef::method("since", 1, since),                    // 38
    BuiltinDef::method("round", 1, round),                    // 39
    BuiltinDef::method("equals", 1, equals),                  // 40
    BuiltinDef::method("toString", 0, to_string),             // 41
    BuiltinDef::method("toJSON", 0, to_json),                 // 42
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 43
    BuiltinDef::method("valueOf", 0, value_of),               // 44
    BuiltinDef::method("startOfDay", 0, start_of_day),        // 45
    BuiltinDef::method("getTimeZoneTransition", 1, get_time_zone_transition), // 46
    BuiltinDef::method("toInstant", 0, to_instant),           // 47
    BuiltinDef::method("toPlainDate", 0, to_plain_date),      // 48
    BuiltinDef::method("toPlainTime", 0, to_plain_time),      // 49
    BuiltinDef::method("toPlainDateTime", 0, to_plain_date_time), // 50
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalZonedDateTime,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalZonedDateTimePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalZonedDateTime,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalZonedDateTimePrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalZonedDateTimePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalZonedDateTime,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.ZonedDateTime"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("calendarId"), Some(3), None),
            PropDef::accessor(PropKey::Str("timeZoneId"), Some(4), None),
            PropDef::accessor(PropKey::Str("era"), Some(5), None),
            PropDef::accessor(PropKey::Str("eraYear"), Some(6), None),
            PropDef::accessor(PropKey::Str("year"), Some(7), None),
            PropDef::accessor(PropKey::Str("month"), Some(8), None),
            PropDef::accessor(PropKey::Str("monthCode"), Some(9), None),
            PropDef::accessor(PropKey::Str("day"), Some(10), None),
            PropDef::accessor(PropKey::Str("hour"), Some(11), None),
            PropDef::accessor(PropKey::Str("minute"), Some(12), None),
            PropDef::accessor(PropKey::Str("second"), Some(13), None),
            PropDef::accessor(PropKey::Str("millisecond"), Some(14), None),
            PropDef::accessor(PropKey::Str("microsecond"), Some(15), None),
            PropDef::accessor(PropKey::Str("nanosecond"), Some(16), None),
            PropDef::accessor(PropKey::Str("epochMilliseconds"), Some(17), None),
            PropDef::accessor(PropKey::Str("epochNanoseconds"), Some(18), None),
            PropDef::accessor(PropKey::Str("dayOfWeek"), Some(19), None),
            PropDef::accessor(PropKey::Str("dayOfYear"), Some(20), None),
            PropDef::accessor(PropKey::Str("weekOfYear"), Some(21), None),
            PropDef::accessor(PropKey::Str("yearOfWeek"), Some(22), None),
            PropDef::accessor(PropKey::Str("hoursInDay"), Some(23), None),
            PropDef::accessor(PropKey::Str("daysInWeek"), Some(24), None),
            PropDef::accessor(PropKey::Str("daysInMonth"), Some(25), None),
            PropDef::accessor(PropKey::Str("daysInYear"), Some(26), None),
            PropDef::accessor(PropKey::Str("monthsInYear"), Some(27), None),
            PropDef::accessor(PropKey::Str("inLeapYear"), Some(28), None),
            PropDef::accessor(PropKey::Str("offsetNanoseconds"), Some(29), None),
            PropDef::accessor(PropKey::Str("offset"), Some(30), None),
            PropDef::method(PropKey::Str("with"), 31),
            PropDef::method(PropKey::Str("withPlainTime"), 32),
            PropDef::method(PropKey::Str("withTimeZone"), 33),
            PropDef::method(PropKey::Str("withCalendar"), 34),
            PropDef::method(PropKey::Str("add"), 35),
            PropDef::method(PropKey::Str("subtract"), 36),
            PropDef::method(PropKey::Str("until"), 37),
            PropDef::method(PropKey::Str("since"), 38),
            PropDef::method(PropKey::Str("round"), 39),
            PropDef::method(PropKey::Str("equals"), 40),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 41),
            PropDef::method(PropKey::Str("toJSON"), 42),
            PropDef::method(PropKey::Str("toLocaleString"), 43),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 44),
            PropDef::method(PropKey::Str("startOfDay"), 45),
            PropDef::method(PropKey::Str("getTimeZoneTransition"), 46),
            PropDef::method(PropKey::Str("toInstant"), 47),
            PropDef::method(PropKey::Str("toPlainDate"), 48),
            PropDef::method(PropKey::Str("toPlainTime"), 49),
            PropDef::method(PropKey::Str("toPlainDateTime"), 50),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("ZonedDateTime"),
            IntrinsicId::TemporalZonedDateTime,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "zoned_date_time",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The ZonedDateTime in `v` (RequireInternalSlot: a TypeError if `v` is not one).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<ZonedDateTime> {
    let s = cx.slot(v, CLASS, 0)?.as_number().unwrap_or(0.0);
    let n = cx.slot(v, CLASS, 1)?.as_number().unwrap_or(0.0);
    let id = cx.slot(v, CLASS, 2)?;
    let id = cx.to_rust_string(id)?;
    let c = cx.slot(v, CLASS, 3)?.as_number().unwrap_or(0.0);
    let tz = TimeZone::try_from_identifier_str_with_provider(&id, provider()).or_throw(cx)?;
    ZonedDateTime::try_new_with_provider(
        s as i128 * NS_PER_SECOND + n as i128,
        tz,
        calendar::from_slot(c),
        provider(),
    )
    .or_throw(cx)
}

/// The time zone in `v` (a ZonedDateTime), without the rest.
fn read_time_zone<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<TimeZone> {
    let id = cx.slot(v, CLASS, 2)?;
    let id = cx.to_rust_string(id)?;
    TimeZone::try_from_identifier_str_with_provider(&id, provider()).or_throw(cx)
}

/// CreateTemporalZonedDateTime(epochNanoseconds, timeZone, calendar, newTarget).
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    z: &ZonedDateTime,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let id = z
        .time_zone()
        .identifier_with_provider(provider())
        .or_throw(cx)?;
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalZonedDateTimePrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalZonedDateTimePrototype),
    };
    let id = cx.new_string(&id)?;
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    let (s, n) = split_epoch_ns(z.epoch_nanoseconds().as_i128());
    cx.set_slot(o, CLASS, 0, Local::number(s))?;
    cx.set_slot(o, CLASS, 1, Local::int32(n))?;
    cx.set_slot(o, CLASS, 2, id)?;
    cx.set_slot(o, CLASS, 3, Local::int32(calendar::to_slot(z.calendar())))?;
    Ok(o.value())
}

/// The ZonedDateTime of `v` if it is one, else `None` (for the conversions of the other
/// classes: ToTemporalInstant, ToTemporalDate, ...).
pub(crate) fn of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<ZonedDateTime>> {
    match cx.object_class(v) {
        Some(CLASS) => read(cx, v).map(Some),
        _ => Ok(None),
    }
}

/// The time zone of `v` if it is a ZonedDateTime (ToTemporalTimeZoneIdentifier step 1).
pub(crate) fn time_zone_of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<TimeZone>> {
    match cx.object_class(v) {
        Some(CLASS) => read_time_zone(cx, v).map(Some),
        _ => Ok(None),
    }
}

// ---- abstract operations --------------------------------------------------------------------

/// GetTemporalDisambiguationOption, GetTemporalOffsetOption(fallback) and
/// GetTemporalOverflowOption, in this (alphabetical) order.
fn zoned_options<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
    offset_fallback: OffsetDisambiguation,
) -> JsResult<(
    Disambiguation,
    OffsetDisambiguation,
    temporal_rs::options::Overflow,
)> {
    let d = get_parsed_option::<Disambiguation>(cx, options, "disambiguation")?
        .unwrap_or(Disambiguation::Compatible);
    let o = get_parsed_option::<OffsetDisambiguation>(cx, options, "offset")?
        .unwrap_or(offset_fallback);
    let v = get_overflow(cx, options)?;
    Ok((d, o, v))
}

/// ToTemporalZonedDateTime(item [, options]) (§6.5.2). `options` is the raw argument.
pub(crate) fn to_temporal_zoned_date_time<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<ZonedDateTime> {
    // 4. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a. A ZonedDateTime: read the options, then its value.
        if let Some(z) = of_object(cx, item)? {
            let options = get_options_object(cx, options)?;
            zoned_options(cx, options, OffsetDisambiguation::Reject)?;
            return Ok(z);
        }
        // b. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(item).
        let cal = get_calendar_with_iso_default(cx, o)?;
        // c. Let fields be ? PrepareCalendarFields(calendar, item, « year, month, month-code,
        //    day », « hour, ..., nanosecond, offset, time-zone », « time-zone »).
        let bag = fields::read(
            cx,
            o,
            fields::DATE
                | fields::TIME
                | fields::OFFSET
                | fields::TIME_ZONE
                | fields::TIME_ZONE_REQUIRED,
        )?;
        // fields::read threw already for an undefined timeZone; kept as a defensive check.
        let Some(tz) = bag.time_zone else {
            return Err(cx.throw_type_error("a zoned date-time needs a timeZone"));
        };
        // f-j. Options: disambiguation, offset (reject), overflow.
        let options = get_options_object(cx, options)?;
        let (d, off, overflow) = zoned_options(cx, options, OffsetDisambiguation::Reject)?;
        // k-. InterpretTemporalDateTimeFields; InterpretISODateTimeOffset (temporal_rs; no
        //     offset field is the wall behaviour).
        let time = bag.partial_time(cx, overflow)?;
        return ZonedDateTime::from_partial_with_provider(
            PartialZonedDateTime {
                fields: ZonedDateTimeFields {
                    calendar_fields: bag.calendar_fields(),
                    time,
                    offset: bag.offset,
                },
                timezone: Some(tz),
                calendar: cal,
            },
            Some(overflow),
            Some(d),
            Some(off),
            provider(),
        )
        .or_throw(cx);
    }
    // 5. a. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("a zoned date-time must be an object or a string"));
    };
    // b-l. ParseISODateTime(item, « TemporalDateTimeString[+Zoned] ») and the time zone.
    let text = temporal_text(cx, s)?;
    let parsed =
        ParsedZonedDateTime::from_utf8_with_provider(text.as_bytes(), provider()).or_throw(cx)?;
    // m-p. Options: disambiguation, offset (reject), overflow.
    let options = get_options_object(cx, options)?;
    let (d, off, _) = zoned_options(cx, options, OffsetDisambiguation::Reject)?;
    // 6-8. InterpretISODateTimeOffset (temporal_rs); the calendar (iso8601 only).
    let z = ZonedDateTime::from_parsed_with_provider(parsed, d, off, provider()).or_throw(cx)?;
    require_iso(cx, z.calendar().clone())?;
    Ok(z)
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.ZonedDateTime ( epochNanoseconds, timeZone [ , calendar ] ) (§6.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.ZonedDateTime requires 'new'"));
    };
    // 2. Set epochNanoseconds to ? ToBigInt(epochNanoseconds).
    let b = cx.to_bigint(args.get(0))?;
    // 3. If IsValidEpochNanoseconds(epochNanoseconds) is false, throw a RangeError exception.
    let Some(ns) = cx.bigint_to_i128(b) else {
        return Err(cx.throw_range_error("epoch nanoseconds out of range"));
    };
    let i = Instant::try_new(ns).or_throw(cx)?;
    // 4. If timeZone is not a String, throw a TypeError exception.
    let Some(tz) = args.get(1).as_string() else {
        return Err(cx.throw_type_error("time zone must be a string"));
    };
    // 5-7. ParseTimeZoneIdentifier; an available named zone or an offset (RangeError).
    let tz = temporal_text(cx, tz)?;
    let tz = TimeZone::try_from_identifier_str_with_provider(&tz, provider()).or_throw(cx)?;
    // 8-10. The calendar.
    let cal = constructor_calendar(cx, args.get(2))?;
    // 11. Return ? CreateTemporalZonedDateTime(epochNanoseconds, timeZone, calendar, NewTarget).
    let z =
        ZonedDateTime::try_new_from_instant_with_provider(i, tz, cal, provider()).or_throw(cx)?;
    create(cx, &z, Some(nt))
}

/// Temporal.ZonedDateTime.from ( item [ , options ] ) (§6.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = to_temporal_zoned_date_time(cx, args.get(0), args.get(1))?;
    create(cx, &z, None)
}

/// Temporal.ZonedDateTime.compare ( one, two ) (§6.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let one = to_temporal_zoned_date_time(cx, args.get(0), Local::undefined())?;
    let two = to_temporal_zoned_date_time(cx, args.get(1), Local::undefined())?;
    // 3. Return 𝔽(CompareEpochNanoseconds(one.[[EpochNanoseconds]], two.[[EpochNanoseconds]])).
    Ok(Local::int32(one.compare_instant(&two) as i32))
}

// ---- getters --------------------------------------------------------------------------------

/// A getter of an int32 (or undefined) value.
fn int_getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: impl FnOnce(&ZonedDateTime) -> Option<i32>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    Ok(f(&z).map_or(Local::undefined(), Local::int32))
}

/// Defines getters `name(this) = int_getter(f)`.
macro_rules! int_getters {
    ($($name:ident => $f:expr;)*) => {$(
        fn $name<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _nt: Local<'s>,
        ) -> JsResult<Local<'s>> {
            int_getter(cx, this, $f)
        }
    )*};
}

int_getters! {
    get_era_year => ZonedDateTime::era_year;
    get_year => |z| Some(z.year());
    get_month => |z| Some(i32::from(z.month()));
    get_day => |z| Some(i32::from(z.day()));
    get_hour => |z| Some(i32::from(z.hour()));
    get_minute => |z| Some(i32::from(z.minute()));
    get_second => |z| Some(i32::from(z.second()));
    get_millisecond => |z| Some(i32::from(z.millisecond()));
    get_microsecond => |z| Some(i32::from(z.microsecond()));
    get_nanosecond => |z| Some(i32::from(z.nanosecond()));
    get_day_of_week => |z| Some(i32::from(z.day_of_week()));
    get_day_of_year => |z| Some(i32::from(z.day_of_year()));
    get_week_of_year => |z| z.week_of_year().map(i32::from);
    get_year_of_week => ZonedDateTime::year_of_week;
    get_days_in_week => |z| Some(i32::from(z.days_in_week()));
    get_days_in_month => |z| Some(i32::from(z.days_in_month()));
    get_days_in_year => |z| Some(i32::from(z.days_in_year()));
    get_months_in_year => |z| Some(i32::from(z.months_in_year()));
}

/// get Temporal.ZonedDateTime.prototype.calendarId (§6.3.3).
fn get_calendar_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    cx.new_string(z.calendar().identifier()).map(|s| s.value())
}

/// get Temporal.ZonedDateTime.prototype.timeZoneId (§6.3.4): the identifier in slot 2.
fn get_time_zone_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.slot(this, CLASS, 2)
}

/// get Temporal.ZonedDateTime.prototype.era (§6.3.5): undefined for iso8601.
fn get_era<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    match z.era() {
        Some(e) => cx.new_string(e.as_str()).map(|s| s.value()),
        None => Ok(Local::undefined()),
    }
}

/// get Temporal.ZonedDateTime.prototype.monthCode (§6.3.9).
fn get_month_code<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let mc = z.month_code();
    cx.new_string(mc.as_str()).map(|s| s.value())
}

/// get Temporal.ZonedDateTime.prototype.epochMilliseconds (§6.3.17).
fn get_epoch_milliseconds<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    Ok(Local::number(z.epoch_milliseconds() as f64 + 0.0))
}

/// get Temporal.ZonedDateTime.prototype.epochNanoseconds (§6.3.18): a BigInt.
fn get_epoch_nanoseconds<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    cx.bigint_from_i128(z.epoch_nanoseconds().as_i128())
}

/// get Temporal.ZonedDateTime.prototype.hoursInDay (§6.3.23).
fn get_hours_in_day<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let h = z.hours_in_day_with_provider(provider()).or_throw(cx)?;
    Ok(Local::number(h))
}

/// get Temporal.ZonedDateTime.prototype.inLeapYear (§6.3.29).
fn get_in_leap_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    Ok(Local::boolean(z.in_leap_year()))
}

/// get Temporal.ZonedDateTime.prototype.offsetNanoseconds (§6.3.30).
fn get_offset_nanoseconds<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    Ok(Local::number(z.offset_nanoseconds() as f64 + 0.0))
}

/// get Temporal.ZonedDateTime.prototype.offset (§6.3.31): "+01:00".
fn get_offset<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let s = z.offset();
    cx.new_string(&s).map(|s| s.value())
}

// ---- methods --------------------------------------------------------------------------------

/// Temporal.ZonedDateTime.prototype.with ( temporalZonedDateTimeLike [ , options ] ) (§6.3.32).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalZonedDateTimeLike) is false, throw a TypeError.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial zoned date-time object"));
    };
    // 12. Let partialZonedDateTime be ? PrepareCalendarFields(calendar, like, « year, month,
    //     month-code, day », « hour, ..., nanosecond, offset », partial).
    let bag = fields::read(cx, like, fields::DATE | fields::TIME | fields::OFFSET)?;
    if !bag.any {
        return Err(cx.throw_type_error("with() needs at least one field"));
    }
    // 14-17. Options: disambiguation, offset (prefer), overflow.
    let options = get_options_object(cx, args.get(1))?;
    let (d, off, overflow) = zoned_options(cx, options, OffsetDisambiguation::Prefer)?;
    // 13, 18-. CalendarMergeFields, InterpretTemporalDateTimeFields and
    //     InterpretISODateTimeOffset (temporal_rs).
    let time = bag.partial_time(cx, overflow)?;
    let r = z
        .with_with_provider(
            ZonedDateTimeFields {
                calendar_fields: bag.calendar_fields(),
                time,
                offset: bag.offset,
            },
            Some(d),
            Some(off),
            Some(overflow),
            provider(),
        )
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.withPlainTime ( [ plainTimeLike ] ) (§6.3.33).
fn with_plain_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 7-8. undefined: GetStartOfDay; else ToTemporalTime and GetEpochNanosecondsFor.
    let t = to_time_or_midnight(cx, args.get(0))?;
    let r = z.with_plain_time_and_provider(t, provider()).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.withTimeZone ( timeZoneLike ) (§6.3.34).
fn with_time_zone<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 3. Let timeZone be ? ToTemporalTimeZoneIdentifier(timeZoneLike).
    let tz = to_temporal_time_zone_identifier(cx, args.get(0))?;
    let r = z
        .with_time_zone_with_provider(tz, provider())
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.withCalendar ( calendarLike ) (§6.3.35).
fn with_calendar<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let cal = to_temporal_calendar_identifier(cx, args.get(0))?;
    create(cx, &z.with_calendar(cal), None)
}

/// AddDurationToZonedDateTime(operation, zonedDateTime, temporalDurationLike, options).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 1. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let dur = to_temporal_duration(cx, args.get(0))?;
    // 3-4. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    let r = if subtract {
        z.subtract_with_provider(&dur, Some(overflow), provider())
    } else {
        z.add_with_provider(&dur, Some(overflow), provider())
    }
    .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.add ( temporalDurationLike [ , options ] ) (§6.3.36).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, false)
}

/// Temporal.ZonedDateTime.prototype.subtract ( temporalDurationLike [ , options ] ) (§6.3.37).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, true)
}

/// DifferenceTemporalZonedDateTime(operation, zonedDateTime, other, options).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 1. Set other to ? ToTemporalZonedDateTime(other).
    let other = to_temporal_zoned_date_time(cx, args.get(0), Local::undefined())?;
    // 3-4. Let resolvedOptions be ? GetOptionsObject(options); GetDifferenceSettings(
    //      operation, resolvedOptions, datetime, « », nanosecond, hour).
    let options = get_options_object(cx, args.get(1))?;
    let settings = get_difference_settings(cx, options)?;
    let r = if until {
        z.until_with_provider(&other, settings, provider())
    } else {
        z.since_with_provider(&other, settings, provider())
    }
    .or_throw(cx)?;
    create_duration(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.until ( other [ , options ] ) (§6.3.38).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.ZonedDateTime.prototype.since ( other [ , options ] ) (§6.3.39).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.ZonedDateTime.prototype.round ( roundTo ) (§6.3.40).
fn round<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 3-5. roundTo: required; a String is { smallestUnit }.
    let round_to = string_or_options(cx, args.get(0), "smallestUnit", "roundTo")?;
    // 6-9. roundingIncrement, roundingMode, smallestUnit (required).
    let increment = get_rounding_increment(cx, round_to)?;
    let mode = get_parsed_option::<RoundingMode>(cx, round_to, "roundingMode")?;
    let Some(smallest) = get_unit(cx, round_to, "smallestUnit")? else {
        return Err(cx.throw_range_error("smallestUnit is required"));
    };
    // 10. Perform ? ValidateTemporalUnitValue(smallestUnit, time, « day »).
    validate_unit(cx, Some(smallest), UnitGroup::Time, Some(Unit::Day))?;
    // 11-. The increment check and the rounding (temporal_rs).
    let mut o = RoundingOptions::default();
    o.smallest_unit = Some(smallest);
    o.increment = increment;
    o.rounding_mode = mode;
    let r = z.round_with_provider(o, provider()).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.equals ( other ) (§6.3.41).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let other = to_temporal_zoned_date_time(cx, args.get(0), Local::undefined())?;
    let eq = z.equals_with_provider(&other, provider()).or_throw(cx)?;
    Ok(Local::boolean(eq))
}

/// Temporal.ZonedDateTime.prototype.toString ( [ options ] ) (§6.3.42).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4-9. calendarName, fractionalSecondDigits, offset, roundingMode, smallestUnit,
    //      timeZoneName (alphabetical).
    let show_calendar = get_show_calendar(cx, options)?;
    let precision = get_fractional_second_digits(cx, options)?;
    let show_offset =
        get_parsed_option::<DisplayOffset>(cx, options, "offset")?.unwrap_or(DisplayOffset::Auto);
    let rounding_mode = get_parsed_option::<RoundingMode>(cx, options, "roundingMode")?;
    let smallest_unit = get_unit(cx, options, "smallestUnit")?;
    let show_tz = get_parsed_option::<DisplayTimeZone>(cx, options, "timeZoneName")?
        .unwrap_or(DisplayTimeZone::Auto);
    // 10. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, smallest_unit, UnitGroup::Time, None)?;
    // 11. If smallestUnit is hour, throw a RangeError exception.
    if smallest_unit == Some(Unit::Hour) {
        return Err(cx.throw_range_error("smallestUnit must not be hour"));
    }
    // 12-. TemporalZonedDateTimeToString (temporal_rs).
    let s = z
        .to_ixdtf_string_with_provider(
            show_offset,
            show_tz,
            show_calendar,
            ToStringRoundingOptions {
                precision,
                smallest_unit,
                rounding_mode,
            },
            provider(),
        )
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// TemporalZonedDateTimeToString(zonedDateTime, auto, auto, auto, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let s = z.to_string_with_provider(provider()).or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.ZonedDateTime.prototype.toJSON ( ) (§6.3.44).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.ZonedDateTime.prototype.toLocaleString ( ) (§6.3.43): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.ZonedDateTime.prototype.valueOf ( ) (§6.3.45): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.ZonedDateTime cannot be converted to a primitive; use compare() or equals()",
    ))
}

/// Temporal.ZonedDateTime.prototype.startOfDay ( ) (§6.3.46).
fn start_of_day<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    let r = z.start_of_day_with_provider(provider()).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.ZonedDateTime.prototype.getTimeZoneTransition ( directionParam ) (§6.3.47).
fn get_time_zone_transition<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    // 3-5. directionParam: required; a String is { direction }.
    let o = string_or_options(cx, args.get(0), "direction", "directionParam")?;
    // 6. Let direction be ? GetDirectionOption(roundTo) (required).
    let Some(dir) = get_parsed_option::<TransitionDirection>(cx, o, "direction")? else {
        return Err(cx.throw_range_error("direction is required"));
    };
    // 7-. The next or previous transition, or null (temporal_rs).
    match z
        .get_time_zone_transition_with_provider(dir, provider())
        .or_throw(cx)?
    {
        Some(t) => create(cx, &t, None),
        None => Ok(Local::null()),
    }
}

/// Temporal.ZonedDateTime.prototype.toInstant ( ) (§6.3.48).
fn to_instant<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    instant::create(cx, &z.to_instant(), None)
}

/// Temporal.ZonedDateTime.prototype.toPlainDate ( ) (§6.3.49).
fn to_plain_date<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    plain_date::create(cx, &z.to_plain_date(), None)
}

/// Temporal.ZonedDateTime.prototype.toPlainTime ( ) (§6.3.50).
fn to_plain_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    plain_time::create(cx, &z.to_plain_time(), None)
}

/// Temporal.ZonedDateTime.prototype.toPlainDateTime ( ) (§6.3.51).
fn to_plain_date_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let z = read(cx, this)?;
    plain_date_time::create(cx, &z.to_plain_date_time(), None)
}

// ---- toZonedDateTime of the other classes ---------------------------------------------------

/// Temporal.Instant.prototype.toZonedDateTimeISO ( timeZone ).
pub(crate) fn instant_to_zoned_date_time_iso<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let i = instant::read(cx, this)?;
    // 3. Set timeZone to ? ToTemporalTimeZoneIdentifier(timeZone).
    let tz = to_temporal_time_zone_identifier(cx, args.get(0))?;
    // 4. Return ! CreateTemporalZonedDateTime(instant.[[EpochNanoseconds]], timeZone, "iso8601").
    let z =
        ZonedDateTime::try_new_iso_from_instant_with_provider(i, tz, provider()).or_throw(cx)?;
    create(cx, &z, None)
}

/// Temporal.PlainDate.prototype.toZonedDateTime ( item ).
pub(crate) fn plain_date_to_zoned_date_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = plain_date::read(cx, this)?;
    let item = args.get(0);
    // 3. If item is an Object, then
    let (tz, time_like) = if let Some(o) = item.as_object() {
        // a. Let timeZoneLike be ? Get(item, "timeZone").
        let k = cx.key("timeZone");
        let tz_like = cx.get(o, k)?;
        if tz_like.is_undefined() {
            // b. timeZone = ? ToTemporalTimeZoneIdentifier(item); temporalTime undefined.
            (
                to_temporal_time_zone_identifier(cx, item)?,
                Local::undefined(),
            )
        } else {
            // c. timeZone = ? ToTemporalTimeZoneIdentifier(timeZoneLike);
            //    temporalTime = ? Get(item, "plainTime").
            let tz = to_temporal_time_zone_identifier(cx, tz_like)?;
            let k = cx.key("plainTime");
            (tz, cx.get(o, k)?)
        }
    } else {
        // 4. Else, timeZone = ? ToTemporalTimeZoneIdentifier(item); temporalTime undefined.
        (
            to_temporal_time_zone_identifier(cx, item)?,
            Local::undefined(),
        )
    };
    // 5-6. undefined: GetStartOfDay; else ToTemporalTime, the limits and
    //      GetEpochNanosecondsFor(compatible).
    let t = to_time_or_midnight(cx, time_like)?;
    let z = d
        .to_zoned_date_time_with_provider(tz, t, provider())
        .or_throw(cx)?;
    create(cx, &z, None)
}

/// Temporal.PlainDateTime.prototype.toZonedDateTime ( temporalTimeZoneLike [ , options ] ).
pub(crate) fn plain_date_time_to_zoned_date_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = plain_date_time::read(cx, this)?;
    // 3. Let timeZone be ? ToTemporalTimeZoneIdentifier(temporalTimeZoneLike).
    let tz = to_temporal_time_zone_identifier(cx, args.get(0))?;
    // 4-5. Let resolvedOptions be ? GetOptionsObject(options); disambiguation.
    let options = get_options_object(cx, args.get(1))?;
    let d = get_parsed_option::<Disambiguation>(cx, options, "disambiguation")?
        .unwrap_or(Disambiguation::Compatible);
    // 6-7. GetEpochNanosecondsFor; CreateTemporalZonedDateTime.
    let z = dt
        .to_zoned_date_time_with_provider(tz, d, provider())
        .or_throw(cx)?;
    create(cx, &z, None)
}
