//! Temporal.PlainDateTime (Temporal §5): module `plain_date_time`, block 24 (design
//! §3.2-§3.4, WP T6). Makes `%Temporal.PlainDateTime%` and
//! `%Temporal.PlainDateTime.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalPlainDateTime, 6 int32 slots (design §3.2):
//!   0-3 as PlainDate (ISO year, month, day, calendar code)   plain_date::read_date_slots
//!   4-5 as PlainTime (second of the day, nanosecond of it)   plain_time::{to,from}_slots
//! ```
//!
//! `toZonedDateTime` (id 40) came with ZonedDateTime (WP T8).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plain_date_time.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::options::{
    RoundingMode, RoundingOptions, ToStringRoundingOptions, Unit, UnitGroup,
};
use temporal_rs::parsed_intermediates::ParsedDateTime;
use temporal_rs::partial::PartialDateTime;
use temporal_rs::{Calendar, PlainDate, PlainDateTime, PlainTime};

use crate::calendar::{
    canonicalize_calendar, get_calendar_with_iso_default, parsed_string_calendar, require_iso,
    to_temporal_calendar_identifier,
};
use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object};
use crate::options::{
    get_difference_settings, get_fractional_second_digits, get_options_object, get_parsed_option,
    get_rounding_increment, get_unit, string_or_options, validate_unit,
};
use crate::plain_date::{self, get_show_calendar, read_date_slots, write_date_slots};
use crate::plain_time::{self, get_overflow, to_temporal_time};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 24;

const SLOTS: usize = 6;
const CLASS: ObjectClass = ObjectClass::TemporalPlainDateTime;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PlainDateTime", 3, constructor), // 0
    BuiltinDef::method("from", 1, from),                      // 1
    BuiltinDef::method("compare", 2, compare),                // 2
    BuiltinDef::getter("calendarId", get_calendar_id),        // 3
    BuiltinDef::getter("era", get_era),                       // 4
    BuiltinDef::getter("eraYear", get_era_year),              // 5
    BuiltinDef::getter("year", get_year),                     // 6
    BuiltinDef::getter("month", get_month),                   // 7
    BuiltinDef::getter("monthCode", get_month_code),          // 8
    BuiltinDef::getter("day", get_day),                       // 9
    BuiltinDef::getter("hour", get_hour),                     // 10
    BuiltinDef::getter("minute", get_minute),                 // 11
    BuiltinDef::getter("second", get_second),                 // 12
    BuiltinDef::getter("millisecond", get_millisecond),       // 13
    BuiltinDef::getter("microsecond", get_microsecond),       // 14
    BuiltinDef::getter("nanosecond", get_nanosecond),         // 15
    BuiltinDef::getter("dayOfWeek", get_day_of_week),         // 16
    BuiltinDef::getter("dayOfYear", get_day_of_year),         // 17
    BuiltinDef::getter("weekOfYear", get_week_of_year),       // 18
    BuiltinDef::getter("yearOfWeek", get_year_of_week),       // 19
    BuiltinDef::getter("daysInWeek", get_days_in_week),       // 20
    BuiltinDef::getter("daysInMonth", get_days_in_month),     // 21
    BuiltinDef::getter("daysInYear", get_days_in_year),       // 22
    BuiltinDef::getter("monthsInYear", get_months_in_year),   // 23
    BuiltinDef::getter("inLeapYear", get_in_leap_year),       // 24
    BuiltinDef::method("with", 1, with),                      // 25
    BuiltinDef::method("withPlainTime", 0, with_plain_time),  // 26
    BuiltinDef::method("withCalendar", 1, with_calendar),     // 27
    BuiltinDef::method("add", 1, add),                        // 28
    BuiltinDef::method("subtract", 1, subtract),              // 29
    BuiltinDef::method("until", 1, until),                    // 30
    BuiltinDef::method("since", 1, since),                    // 31
    BuiltinDef::method("round", 1, round),                    // 32
    BuiltinDef::method("equals", 1, equals),                  // 33
    BuiltinDef::method("toString", 0, to_string),             // 34
    BuiltinDef::method("toJSON", 0, to_json),                 // 35
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 36
    BuiltinDef::method("valueOf", 0, value_of),               // 37
    BuiltinDef::method("toPlainDate", 0, to_plain_date),      // 38
    BuiltinDef::method("toPlainTime", 0, to_plain_time),      // 39
    BuiltinDef::method(
        "toZonedDateTime",
        1,
        crate::zoned_date_time::plain_date_time_to_zoned_date_time,
    ), // 40 (T8)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainDateTime,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainDateTimePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalPlainDateTime,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalPlainDateTimePrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalPlainDateTimePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalPlainDateTime,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.PlainDateTime"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("calendarId"), Some(3), None),
            PropDef::accessor(PropKey::Str("era"), Some(4), None),
            PropDef::accessor(PropKey::Str("eraYear"), Some(5), None),
            PropDef::accessor(PropKey::Str("year"), Some(6), None),
            PropDef::accessor(PropKey::Str("month"), Some(7), None),
            PropDef::accessor(PropKey::Str("monthCode"), Some(8), None),
            PropDef::accessor(PropKey::Str("day"), Some(9), None),
            PropDef::accessor(PropKey::Str("hour"), Some(10), None),
            PropDef::accessor(PropKey::Str("minute"), Some(11), None),
            PropDef::accessor(PropKey::Str("second"), Some(12), None),
            PropDef::accessor(PropKey::Str("millisecond"), Some(13), None),
            PropDef::accessor(PropKey::Str("microsecond"), Some(14), None),
            PropDef::accessor(PropKey::Str("nanosecond"), Some(15), None),
            PropDef::accessor(PropKey::Str("dayOfWeek"), Some(16), None),
            PropDef::accessor(PropKey::Str("dayOfYear"), Some(17), None),
            PropDef::accessor(PropKey::Str("weekOfYear"), Some(18), None),
            PropDef::accessor(PropKey::Str("yearOfWeek"), Some(19), None),
            PropDef::accessor(PropKey::Str("daysInWeek"), Some(20), None),
            PropDef::accessor(PropKey::Str("daysInMonth"), Some(21), None),
            PropDef::accessor(PropKey::Str("daysInYear"), Some(22), None),
            PropDef::accessor(PropKey::Str("monthsInYear"), Some(23), None),
            PropDef::accessor(PropKey::Str("inLeapYear"), Some(24), None),
            PropDef::method(PropKey::Str("with"), 25),
            PropDef::method(PropKey::Str("withPlainTime"), 26),
            PropDef::method(PropKey::Str("withCalendar"), 27),
            PropDef::method(PropKey::Str("add"), 28),
            PropDef::method(PropKey::Str("subtract"), 29),
            PropDef::method(PropKey::Str("until"), 30),
            PropDef::method(PropKey::Str("since"), 31),
            PropDef::method(PropKey::Str("round"), 32),
            PropDef::method(PropKey::Str("equals"), 33),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 34),
            PropDef::method(PropKey::Str("toJSON"), 35),
            PropDef::method(PropKey::Str("toLocaleString"), 36),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 37),
            PropDef::method(PropKey::Str("toPlainDate"), 38),
            PropDef::method(PropKey::Str("toPlainTime"), 39),
            PropDef::method(PropKey::Str("toZonedDateTime"), 40),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("PlainDateTime"),
            IntrinsicId::TemporalPlainDateTime,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plain_date_time",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The PlainDateTime in `v` (RequireInternalSlot: a TypeError if `v` is not one).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<PlainDateTime> {
    let d = read_date_slots(cx, v, CLASS, 0)?;
    let s = cx.slot(v, CLASS, 4)?.as_number().unwrap_or(0.0);
    let n = cx.slot(v, CLASS, 5)?.as_number().unwrap_or(0.0);
    let t = plain_time::from_slots(cx, s, n)?;
    PlainDateTime::from_date_and_time(d, t).or_throw(cx)
}

/// CreateTemporalDateTime(isoDateTime, calendar, newTarget): a new PlainDateTime object.
/// `dt` is in range (temporal_rs checks ISODateTimeWithinLimits when it makes one).
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    dt: &PlainDateTime,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalPlainDateTimePrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalPlainDateTimePrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    write_date_slots(cx, o, CLASS, 0, &dt.to_plain_date())?;
    let (s, n) = plain_time::to_slots(&dt.to_plain_time());
    cx.set_slot(o, CLASS, 4, Local::int32(s))?;
    cx.set_slot(o, CLASS, 5, Local::int32(n))?;
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// The date-time of a PlainDateTime or ZonedDateTime object, else `None`.
pub(crate) fn date_time_of_object<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<Option<PlainDateTime>> {
    match cx.object_class(v) {
        Some(CLASS) => read(cx, v).map(Some),
        Some(ObjectClass::TemporalZonedDateTime) => {
            crate::zoned_date_time::read(cx, v).map(|z| Some(z.to_plain_date_time()))
        }
        _ => Ok(None),
    }
}

/// ToTemporalDateTime(item [, options]) (§5.5.4). `options` is the raw argument.
pub(crate) fn to_temporal_date_time<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<PlainDateTime> {
    // 2. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a-b. A PlainDateTime or ZonedDateTime: read the options, then its date-time.
        if let Some(dt) = date_time_of_object(cx, item)? {
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return Ok(dt);
        }
        // c. A PlainDate: read the options, then its date at midnight.
        if let Some(d) = plain_date::date_of_object(cx, item)? {
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return d.to_plain_date_time(None).or_throw(cx);
        }
        // d. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(item).
        let cal = get_calendar_with_iso_default(cx, o)?;
        // e. Let fields be ? PrepareCalendarFields(calendar, item, « year, month, month-code,
        //    day », « hour, minute, second, millisecond, microsecond, nanosecond », «»).
        let bag = fields::read(cx, o, fields::DATE | fields::TIME)?;
        // f-g. Let resolvedOptions be ? GetOptionsObject(options); overflow.
        let options = get_options_object(cx, options)?;
        let overflow = get_overflow(cx, options)?;
        // h. Let result be ? InterpretTemporalDateTimeFields(calendar, fields, overflow).
        let time = bag.partial_time(cx, overflow)?;
        return PlainDateTime::from_partial(
            PartialDateTime {
                fields: temporal_rs::fields::DateTimeFields {
                    calendar_fields: bag.calendar_fields(),
                    time,
                },
                calendar: cal,
            },
            Some(overflow),
        )
        .or_throw(cx);
    }
    // 3. a. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("a date-time must be an object or a string"));
    };
    // b-c. ParseISODateTime(item, « TemporalDateTimeString[~Zoned] »): the syntax, a valid ISO
    //      date and time and a known calendar id.
    let text = temporal_text(cx, s)?;
    let parsed = ParsedDateTime::from_utf8(text.as_bytes()).or_throw(cx)?;
    // i-j, 4. CreateTemporalDateTime (ISODateTimeWithinLimits): built here, thrown after the
    //         options.
    let built = PlainDateTime::from_parsed(parsed);
    // d-f. CanonicalizeCalendar: iso8601 only (SPEC-T-D17), before the options.
    //      ParsedDateTime hides its calendar, so a date-time out of the limits reads it from
    //      the string (as in ToTemporalDate).
    let cal = match &built {
        Ok(dt) => dt.calendar().clone(),
        Err(_) => parsed_string_calendar(cx, &text)?,
    };
    require_iso(cx, cal)?;
    // g-h. Let resolvedOptions be ? GetOptionsObject(options); GetTemporalOverflowOption.
    let options = get_options_object(cx, options)?;
    get_overflow(cx, options)?;
    built.or_throw(cx)
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.PlainDateTime ( isoYear, isoMonth, isoDay [ , hour [ , minute [ , second [ ,
/// millisecond [ , microsecond [ , nanosecond [ , calendar ] ] ] ] ] ] ] ) (§5.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.PlainDateTime requires 'new'"));
    };
    // 2-4. Let isoYear, isoMonth, isoDay be ? ToIntegerWithTruncation(...).
    let y = to_integer_with_truncation(cx, args.get(0))?;
    let m = to_integer_with_truncation(cx, args.get(1))?;
    let d = to_integer_with_truncation(cx, args.get(2))?;
    // 5-10. Each time argument: undefined is 0, else ? ToIntegerWithTruncation.
    let mut t = [0.0f64; 6];
    for (i, x) in t.iter_mut().enumerate() {
        let v = args.get(3 + i);
        if !v.is_undefined() {
            *x = to_integer_with_truncation(cx, v)?;
        }
    }
    // 11-13. The calendar: undefined is iso8601; not a String is a TypeError; then
    //        CanonicalizeCalendar.
    let c = args.get(9);
    let cal = if c.is_undefined() {
        Calendar::ISO
    } else {
        let Some(s) = c.as_string() else {
            return Err(cx.throw_type_error("calendar must be a string"));
        };
        let id = cx.to_rust_string(s)?;
        canonicalize_calendar(cx, &id)?
    };
    // 14. If IsValidISODate(isoYear, isoMonth, isoDay) is false, throw a RangeError exception.
    if !(1.0..=12.0).contains(&m) || !(1.0..=31.0).contains(&d) || y.abs() > 1e6 {
        return Err(cx.throw_range_error("date out of range"));
    }
    let date = PlainDate::try_new(y as i32, m as u8, d as u8, cal).or_throw(cx)?;
    // 16. If IsValidTime(hour, ..., nanosecond) is false, throw a RangeError exception.
    let max = [23.0, 59.0, 59.0, 999.0, 999.0, 999.0];
    if t.iter().zip(max).any(|(v, m)| *v < 0.0 || *v > m) {
        return Err(cx.throw_range_error("time field out of range"));
    }
    let time = PlainTime::try_new(
        t[0] as u8,
        t[1] as u8,
        t[2] as u8,
        t[3] as u16,
        t[4] as u16,
        t[5] as u16,
    )
    .or_throw(cx)?;
    // 17-18. CombineISODateAndTimeRecord; CreateTemporalDateTime (the limits).
    let dt = PlainDateTime::from_date_and_time(date, time).or_throw(cx)?;
    create(cx, &dt, Some(nt))
}

/// Temporal.PlainDateTime.from ( item [ , options ] ) (§5.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? ToTemporalDateTime(item, options).
    let dt = to_temporal_date_time(cx, args.get(0), args.get(1))?;
    create(cx, &dt, None)
}

/// Temporal.PlainDateTime.compare ( one, two ) (§5.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Set one and two to ? ToTemporalDateTime(...).
    let one = to_temporal_date_time(cx, args.get(0), Local::undefined())?;
    let two = to_temporal_date_time(cx, args.get(1), Local::undefined())?;
    // 3. Return 𝔽(CompareISODateTime(one.[[ISODateTime]], two.[[ISODateTime]])).
    Ok(Local::int32(one.compare_iso(&two) as i32))
}

// ---- getters --------------------------------------------------------------------------------

/// A getter of an int32 (or undefined) value of the date-time.
fn int_getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: impl FnOnce(&PlainDateTime) -> Option<i32>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    Ok(f(&dt).map_or(Local::undefined(), Local::int32))
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
    get_era_year => PlainDateTime::era_year;
    get_year => |d| Some(d.year());
    get_month => |d| Some(i32::from(d.month()));
    get_day => |d| Some(i32::from(d.day()));
    get_hour => |d| Some(i32::from(d.hour()));
    get_minute => |d| Some(i32::from(d.minute()));
    get_second => |d| Some(i32::from(d.second()));
    get_millisecond => |d| Some(i32::from(d.millisecond()));
    get_microsecond => |d| Some(i32::from(d.microsecond()));
    get_nanosecond => |d| Some(i32::from(d.nanosecond()));
    get_day_of_week => |d| Some(i32::from(d.day_of_week()));
    get_day_of_year => |d| Some(i32::from(d.day_of_year()));
    get_week_of_year => |d| d.week_of_year().map(i32::from);
    get_year_of_week => PlainDateTime::year_of_week;
    get_days_in_week => |d| Some(i32::from(d.days_in_week()));
    get_days_in_month => |d| Some(i32::from(d.days_in_month()));
    get_days_in_year => |d| Some(i32::from(d.days_in_year()));
    get_months_in_year => |d| Some(i32::from(d.months_in_year()));
}

/// get Temporal.PlainDateTime.prototype.calendarId (§5.3.3).
fn get_calendar_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    let id = dt.calendar().identifier();
    cx.new_string(id).map(|s| s.value())
}

/// get Temporal.PlainDateTime.prototype.era (§5.3.4): undefined for iso8601.
fn get_era<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    match dt.era() {
        Some(e) => cx.new_string(e.as_str()).map(|s| s.value()),
        None => Ok(Local::undefined()),
    }
}

/// get Temporal.PlainDateTime.prototype.monthCode (§5.3.8).
fn get_month_code<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    let mc = dt.month_code();
    cx.new_string(mc.as_str()).map(|s| s.value())
}

/// get Temporal.PlainDateTime.prototype.inLeapYear (§5.3.25).
fn get_in_leap_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    Ok(Local::boolean(dt.in_leap_year()))
}

// ---- methods --------------------------------------------------------------------------------

/// Temporal.PlainDateTime.prototype.with ( temporalDateTimeLike [ , options ] ) (§5.3.26).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalDateTimeLike) is false, throw a TypeError.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial date-time object"));
    };
    // 6. Let partialDateTime be ? PrepareCalendarFields(calendar, temporalDateTimeLike,
    //    « year, month, month-code, day », « hour, ..., nanosecond », partial).
    let bag = fields::read(cx, like, fields::DATE | fields::TIME)?;
    if !bag.any {
        return Err(cx.throw_type_error("with() needs at least one date or time field"));
    }
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    // 4-5, 7, 10. CalendarMergeFields and InterpretTemporalDateTimeFields (temporal_rs).
    let time = bag.partial_time(cx, overflow)?;
    let r = dt
        .with(
            temporal_rs::fields::DateTimeFields {
                calendar_fields: bag.calendar_fields(),
                time,
            },
            Some(overflow),
        )
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// ToTimeRecordOrMidnight(item): undefined is midnight, else ToTemporalTime.
pub(crate) fn to_time_or_midnight<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
) -> JsResult<Option<PlainTime>> {
    if item.is_undefined() {
        return Ok(None);
    }
    to_temporal_time(cx, item, Local::undefined()).map(Some)
}

/// Temporal.PlainDateTime.prototype.withPlainTime ( [ plainTimeLike ] ) (§5.3.27).
fn with_plain_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3. Let time be ? ToTimeRecordOrMidnight(plainTimeLike).
    let t = to_time_or_midnight(cx, args.get(0))?;
    // 4-5. CombineISODateAndTimeRecord; CreateTemporalDateTime (the limits).
    let r = dt.with_time(t).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainDateTime.prototype.withCalendar ( calendarLike ) (§5.3.28).
fn with_calendar<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3. Let calendar be ? ToTemporalCalendarIdentifier(calendarLike).
    let cal = to_temporal_calendar_identifier(cx, args.get(0))?;
    create(cx, &dt.with_calendar(cal), None)
}

/// AddDurationToDateTime(operation, dateTime, temporalDurationLike, options).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 1. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let dur = to_temporal_duration(cx, args.get(0))?;
    // 3-4. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    // 5-. The addition (temporal_rs).
    let r = if subtract {
        dt.subtract(&dur, Some(overflow))
    } else {
        dt.add(&dur, Some(overflow))
    }
    .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainDateTime.prototype.add ( temporalDurationLike [ , options ] ) (§5.3.29).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, false)
}

/// Temporal.PlainDateTime.prototype.subtract ( temporalDurationLike [ , options ] ) (§5.3.30).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, true)
}

/// DifferenceTemporalPlainDateTime(operation, dateTime, other, options).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 1. Set other to ? ToTemporalDateTime(other).
    let other = to_temporal_date_time(cx, args.get(0), Local::undefined())?;
    // 2. CalendarEquals (always equal: iso8601).
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 4. Let settings be ? GetDifferenceSettings(operation, resolvedOptions, datetime, « »,
    //    nanosecond, day).
    let settings = get_difference_settings(cx, options)?;
    let r = if until {
        dt.until(&other, settings)
    } else {
        dt.since(&other, settings)
    }
    .or_throw(cx)?;
    create_duration(cx, &r, None)
}

/// Temporal.PlainDateTime.prototype.until ( other [ , options ] ) (§5.3.31).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.PlainDateTime.prototype.since ( other [ , options ] ) (§5.3.32).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.PlainDateTime.prototype.round ( roundTo ) (§5.3.33).
fn round<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3-5. roundTo: required; a String is { smallestUnit }.
    let round_to = string_or_options(cx, args.get(0), "smallestUnit", "roundTo")?;
    // 6-9. roundingIncrement, roundingMode, smallestUnit (required), in this order.
    let increment = get_rounding_increment(cx, round_to)?;
    let mode = get_parsed_option::<RoundingMode>(cx, round_to, "roundingMode")?;
    let Some(smallest) = get_unit(cx, round_to, "smallestUnit")? else {
        return Err(cx.throw_range_error("smallestUnit is required"));
    };
    // 10. Perform ? ValidateTemporalUnitValue(smallestUnit, time, « day »).
    validate_unit(cx, Some(smallest), UnitGroup::Time, Some(Unit::Day))?;
    // 11-. The maximum, the increment check and RoundISODateTime (temporal_rs).
    let mut o = RoundingOptions::default();
    o.smallest_unit = Some(smallest);
    o.increment = increment;
    o.rounding_mode = mode;
    let r = dt.round(o).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainDateTime.prototype.equals ( other ) (§5.3.34).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3. Set other to ? ToTemporalDateTime(other).
    let other = to_temporal_date_time(cx, args.get(0), Local::undefined())?;
    // 4-5. CompareISODateTime = 0 and CalendarEquals.
    Ok(Local::boolean(
        dt.compare_iso(&other).is_eq()
            && dt.calendar().identifier() == other.calendar().identifier(),
    ))
}

/// Temporal.PlainDateTime.prototype.toString ( [ options ] ) (§5.3.35).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4-7. calendarName, fractionalSecondDigits, roundingMode, smallestUnit.
    let show = get_show_calendar(cx, options)?;
    let precision = get_fractional_second_digits(cx, options)?;
    let rounding_mode = get_parsed_option::<RoundingMode>(cx, options, "roundingMode")?;
    let smallest_unit = get_unit(cx, options, "smallestUnit")?;
    // 8. Perform ? ValidateTemporalUnitValue(smallestUnit, time).
    validate_unit(cx, smallest_unit, UnitGroup::Time, None)?;
    // 9. If smallestUnit is hour, throw a RangeError exception.
    if smallest_unit == Some(Unit::Hour) {
        return Err(cx.throw_range_error("smallestUnit must not be hour"));
    }
    // 10-. Round, check the limits (RangeError), ISODateTimeToString (temporal_rs).
    let s = dt
        .to_ixdtf_string(
            ToStringRoundingOptions {
                precision,
                smallest_unit,
                rounding_mode,
            },
            show,
        )
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// ISODateTimeToString(dateTime, calendar, auto, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    let s = dt
        .to_ixdtf_string(
            ToStringRoundingOptions::default(),
            temporal_rs::options::DisplayCalendar::Auto,
        )
        .or_throw(cx)?;
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.PlainDateTime.prototype.toJSON ( ) (§5.3.37).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainDateTime.prototype.toLocaleString ( ) (§5.3.36): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainDateTime.prototype.valueOf ( ) (§5.3.38): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.PlainDateTime cannot be converted to a primitive; use compare() or equals()",
    ))
}

/// Temporal.PlainDateTime.prototype.toPlainDate ( ) (§5.3.40).
fn to_plain_date<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    plain_date::create(cx, &dt.to_plain_date(), None)
}

/// Temporal.PlainDateTime.prototype.toPlainTime ( ) (§5.3.41).
fn to_plain_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let dt = read(cx, this)?;
    plain_time::create(cx, &dt.to_plain_time(), None)
}
