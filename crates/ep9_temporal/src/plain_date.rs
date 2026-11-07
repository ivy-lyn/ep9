//! Temporal.PlainDate (Temporal §3): module `plain_date`, block 23 (design §3.2-§3.4, WP T5).
//! Makes `%Temporal.PlainDate%` and `%Temporal.PlainDate.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalPlainDate, 4 slots (design §3.2):
//!   0 ISO year (-271,821..275,760), 1 ISO month (1..12), 2 ISO day (1..31): int32s
//!   3 calendar code (calendar.rs; 0 = iso8601): an int32
//! ```
//!
//! `toPlainDateTime` (id 30) came with PlainDateTime (WP T6), `toPlainYearMonth` and
//! `toPlainMonthDay` (ids 31-32) with WP T7, `toZonedDateTime` (id 33) with WP T8.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plain_date.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::PlainDate;
use temporal_rs::options::DisplayCalendar;
use temporal_rs::parsed_intermediates::ParsedDate;
use temporal_rs::partial::PartialDate;

use crate::calendar::{
    self, canonicalize_calendar, get_calendar_with_iso_default, prepare_fields, require_same,
    string_calendar, to_temporal_calendar_identifier,
};
use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object};
use crate::options::{get_difference_settings, get_options_object, get_parsed_option};
use crate::plain_time::get_overflow;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 23;

const SLOTS: usize = 4;
const CLASS: ObjectClass = ObjectClass::TemporalPlainDate;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PlainDate", 3, constructor), // 0
    BuiltinDef::method("from", 1, from),                  // 1
    BuiltinDef::method("compare", 2, compare),            // 2
    BuiltinDef::getter("calendarId", get_calendar_id),    // 3
    BuiltinDef::getter("era", get_era),                   // 4
    BuiltinDef::getter("eraYear", get_era_year),          // 5
    BuiltinDef::getter("year", get_year),                 // 6
    BuiltinDef::getter("month", get_month),               // 7
    BuiltinDef::getter("monthCode", get_month_code),      // 8
    BuiltinDef::getter("day", get_day),                   // 9
    BuiltinDef::getter("dayOfWeek", get_day_of_week),     // 10
    BuiltinDef::getter("dayOfYear", get_day_of_year),     // 11
    BuiltinDef::getter("weekOfYear", get_week_of_year),   // 12
    BuiltinDef::getter("yearOfWeek", get_year_of_week),   // 13
    BuiltinDef::getter("daysInWeek", get_days_in_week),   // 14
    BuiltinDef::getter("daysInMonth", get_days_in_month), // 15
    BuiltinDef::getter("daysInYear", get_days_in_year),   // 16
    BuiltinDef::getter("monthsInYear", get_months_in_year), // 17
    BuiltinDef::getter("inLeapYear", get_in_leap_year),   // 18
    BuiltinDef::method("add", 1, add),                    // 19
    BuiltinDef::method("subtract", 1, subtract),          // 20
    BuiltinDef::method("with", 1, with),                  // 21
    BuiltinDef::method("withCalendar", 1, with_calendar), // 22
    BuiltinDef::method("until", 1, until),                // 23
    BuiltinDef::method("since", 1, since),                // 24
    BuiltinDef::method("equals", 1, equals),              // 25
    BuiltinDef::method("toString", 0, to_string),         // 26
    BuiltinDef::method("toJSON", 0, to_json),             // 27
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 28
    BuiltinDef::method("valueOf", 0, value_of),           // 29
    BuiltinDef::method("toPlainDateTime", 0, to_plain_date_time), // 30 (T6)
    BuiltinDef::method("toPlainYearMonth", 0, to_plain_year_month), // 31 (T7)
    BuiltinDef::method("toPlainMonthDay", 0, to_plain_month_day), // 32 (T7)
    BuiltinDef::method(
        "toZonedDateTime",
        1,
        crate::zoned_date_time::plain_date_to_zoned_date_time,
    ), // 33 (T8)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainDate,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainDatePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalPlainDate,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalPlainDatePrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalPlainDatePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalPlainDate,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.PlainDate"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("calendarId"), Some(3), None),
            PropDef::accessor(PropKey::Str("era"), Some(4), None),
            PropDef::accessor(PropKey::Str("eraYear"), Some(5), None),
            PropDef::accessor(PropKey::Str("year"), Some(6), None),
            PropDef::accessor(PropKey::Str("month"), Some(7), None),
            PropDef::accessor(PropKey::Str("monthCode"), Some(8), None),
            PropDef::accessor(PropKey::Str("day"), Some(9), None),
            PropDef::accessor(PropKey::Str("dayOfWeek"), Some(10), None),
            PropDef::accessor(PropKey::Str("dayOfYear"), Some(11), None),
            PropDef::accessor(PropKey::Str("weekOfYear"), Some(12), None),
            PropDef::accessor(PropKey::Str("yearOfWeek"), Some(13), None),
            PropDef::accessor(PropKey::Str("daysInWeek"), Some(14), None),
            PropDef::accessor(PropKey::Str("daysInMonth"), Some(15), None),
            PropDef::accessor(PropKey::Str("daysInYear"), Some(16), None),
            PropDef::accessor(PropKey::Str("monthsInYear"), Some(17), None),
            PropDef::accessor(PropKey::Str("inLeapYear"), Some(18), None),
            PropDef::method(PropKey::Str("add"), 19),
            PropDef::method(PropKey::Str("subtract"), 20),
            PropDef::method(PropKey::Str("with"), 21),
            PropDef::method(PropKey::Str("withCalendar"), 22),
            PropDef::method(PropKey::Str("until"), 23),
            PropDef::method(PropKey::Str("since"), 24),
            PropDef::method(PropKey::Str("equals"), 25),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 26),
            PropDef::method(PropKey::Str("toJSON"), 27),
            PropDef::method(PropKey::Str("toLocaleString"), 28),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 29),
            PropDef::method(PropKey::Str("toPlainDateTime"), 30),
            PropDef::method(PropKey::Str("toPlainYearMonth"), 31),
            PropDef::method(PropKey::Str("toPlainMonthDay"), 32),
            PropDef::method(PropKey::Str("toZonedDateTime"), 33),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("PlainDate"),
            IntrinsicId::TemporalPlainDate,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plain_date",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The date in slots `base..base + 4` of `v` (RequireInternalSlot for `class`). PlainDate
/// and PlainDateTime keep the date the same way (design §3.2).
pub(crate) fn read_date_slots<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
    class: ObjectClass,
    base: usize,
) -> JsResult<PlainDate> {
    let mut f = [0.0f64; 4];
    for (i, x) in f.iter_mut().enumerate() {
        *x = cx.slot(v, class, base + i)?.as_number().unwrap_or(0.0);
    }
    PlainDate::try_new(
        f[0] as i32,
        f[1] as u8,
        f[2] as u8,
        calendar::from_slot(f[3]),
    )
    .or_throw(cx)
}

/// Writes the date `d` to slots `base..base + 4` of `o`.
pub(crate) fn write_date_slots<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    class: ObjectClass,
    base: usize,
    d: &PlainDate,
) -> JsResult<()> {
    // The ISO date, not the calendar fields (calendar.rs).
    let (y, m, day) = calendar::iso_of_date(d);
    cx.set_slot(o, class, base, Local::int32(y))?;
    cx.set_slot(o, class, base + 1, Local::int32(i32::from(m)))?;
    cx.set_slot(o, class, base + 2, Local::int32(i32::from(day)))?;
    cx.set_slot(
        o,
        class,
        base + 3,
        Local::int32(calendar::to_slot(d.calendar())),
    )
}

/// The PlainDate in `v` (RequireInternalSlot: a TypeError if `v` is not a PlainDate).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<PlainDate> {
    read_date_slots(cx, v, CLASS, 0)
}

/// CreateTemporalDate(isoDate, calendar, newTarget): a new PlainDate object. `d` is in
/// range (temporal_rs checks ISODateWithinLimits when it makes a PlainDate).
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    d: &PlainDate,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalPlainDatePrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalPlainDatePrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    write_date_slots(cx, o, CLASS, 0, d)?;
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// The date of a PlainDate, PlainDateTime or ZonedDateTime object, else `None`.
pub(crate) fn date_of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<PlainDate>> {
    match cx.object_class(v) {
        Some(CLASS) => read(cx, v).map(Some),
        Some(ObjectClass::TemporalPlainDateTime) => {
            crate::plain_date_time::read(cx, v).map(|dt| Some(dt.to_plain_date()))
        }
        Some(ObjectClass::TemporalZonedDateTime) => {
            crate::zoned_date_time::read(cx, v).map(|z| Some(z.to_plain_date()))
        }
        _ => Ok(None),
    }
}

/// ToTemporalDate(item [, options]) (§3.5.3). `options` is the raw argument (undefined
/// when the caller has none).
pub(crate) fn to_temporal_date<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<PlainDate> {
    // 2. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a-c. A PlainDate, ZonedDateTime or PlainDateTime: read the options, then its date.
        if let Some(d) = date_of_object(cx, item)? {
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return Ok(d);
        }
        // d. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(item).
        let cal = get_calendar_with_iso_default(cx, o)?;
        // e. Let fields be ? PrepareCalendarFields(calendar, item, « year, month, month-code,
        //    day », «», «»).
        let bag = prepare_fields(cx, o, fields::DATE, &cal)?;
        // f. Let resolvedOptions be ? GetOptionsObject(options).
        let options = get_options_object(cx, options)?;
        // g. Let overflow be ? GetTemporalOverflowOption(resolvedOptions).
        let overflow = get_overflow(cx, options)?;
        // h. Let isoDate be ? CalendarDateFromFields(calendar, fields, overflow).
        return PlainDate::from_partial(
            PartialDate {
                calendar_fields: bag.calendar_fields(),
                calendar: cal,
            },
            Some(overflow),
        )
        .or_throw(cx);
    }
    // 3. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("a date must be an object or a string"));
    };
    // 4. ParseISODateTime(item, « TemporalDateTimeString[~Zoned] »): the syntax, a valid ISO
    //    date and a known calendar id.
    let text = temporal_text(cx, s)?;
    let parsed = ParsedDate::from_utf8(text.as_bytes()).or_throw(cx)?;
    // 11. CreateTemporalDate (ISODateWithinLimits): built here, thrown after the options.
    let built = PlainDate::from_parsed(parsed);
    // 5-7. CanonicalizeCalendar (AvailableCalendars, calendar.rs), before the options.
    //      ParsedDate hides its calendar, so a date out of the limits reads it from the string.
    string_calendar(cx, &text, built.as_ref().ok().map(PlainDate::calendar))?;
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); GetTemporalOverflowOption.
    let options = get_options_object(cx, options)?;
    get_overflow(cx, options)?;
    built.or_throw(cx)
}

/// GetTemporalShowCalendarNameOption: the `calendarName` option.
pub(crate) fn get_show_calendar<'s>(
    cx: &mut Cx<'s>,
    options: crate::options::Options<'s>,
) -> JsResult<DisplayCalendar> {
    Ok(
        get_parsed_option::<DisplayCalendar>(cx, options, "calendarName")?
            .unwrap_or(DisplayCalendar::Auto),
    )
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.PlainDate ( isoYear, isoMonth, isoDay [ , calendar ] ) (§3.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.PlainDate requires 'new'"));
    };
    // 2-4. Let y, m, d be ? ToIntegerWithTruncation(isoYear, isoMonth, isoDay).
    let y = to_integer_with_truncation(cx, args.get(0))?;
    let m = to_integer_with_truncation(cx, args.get(1))?;
    let d = to_integer_with_truncation(cx, args.get(2))?;
    // 5. If calendar is undefined, set calendar to "iso8601".
    // 6. If calendar is not a String, throw a TypeError exception.
    // 7. Set calendar to ? CanonicalizeCalendar(calendar).
    let c = args.get(3);
    let cal = if c.is_undefined() {
        temporal_rs::Calendar::ISO
    } else {
        let Some(s) = c.as_string() else {
            return Err(cx.throw_type_error("calendar must be a string"));
        };
        let id = cx.to_rust_string(s)?;
        canonicalize_calendar(cx, &id)?
    };
    // 8. If IsValidISODate(y, m, d) is false, throw a RangeError exception.
    if !(1.0..=12.0).contains(&m) || !(1.0..=31.0).contains(&d) || y.abs() > 1e6 {
        return Err(cx.throw_range_error("date out of range"));
    }
    // 9-10. Let isoDate be CreateISODateRecord(y, m, d); CreateTemporalDate (the limits).
    let date = PlainDate::try_new(y as i32, m as u8, d as u8, cal).or_throw(cx)?;
    create(cx, &date, Some(nt))
}

/// Temporal.PlainDate.from ( item [ , options ] ) (§3.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? ToTemporalDate(item, options).
    let d = to_temporal_date(cx, args.get(0), args.get(1))?;
    create(cx, &d, None)
}

/// Temporal.PlainDate.compare ( one, two ) (§3.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set one to ? ToTemporalDate(one).
    let one = to_temporal_date(cx, args.get(0), Local::undefined())?;
    // 2. Set two to ? ToTemporalDate(two).
    let two = to_temporal_date(cx, args.get(1), Local::undefined())?;
    // 3. Return 𝔽(CompareISODate(one.[[ISODate]], two.[[ISODate]])).
    Ok(Local::int32(one.compare_iso(&two) as i32))
}

// ---- getters --------------------------------------------------------------------------------

/// A getter that maps the date to a value.
fn getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: impl FnOnce(&PlainDate) -> Option<i32>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    Ok(f(&d).map_or(Local::undefined(), Local::int32))
}

/// get Temporal.PlainDate.prototype.calendarId (§3.3.3).
fn get_calendar_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    let id = d.calendar().identifier();
    cx.new_string(id).map(|s| s.value())
}

/// get Temporal.PlainDate.prototype.era (§3.3.4): undefined for iso8601.
fn get_era<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    match d.era() {
        Some(e) => cx.new_string(e.as_str()).map(|s| s.value()),
        None => Ok(Local::undefined()),
    }
}

/// get Temporal.PlainDate.prototype.eraYear (§3.3.5): undefined for iso8601.
fn get_era_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, PlainDate::era_year)
}

fn get_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(d.year()))
}

fn get_month<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.month())))
}

/// get Temporal.PlainDate.prototype.monthCode (§3.3.8): "M01" .. "M12".
fn get_month_code<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    let mc = d.month_code();
    cx.new_string(mc.as_str()).map(|s| s.value())
}

fn get_day<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.day())))
}

fn get_day_of_week<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.day_of_week())))
}

fn get_day_of_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.day_of_year())))
}

fn get_week_of_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| d.week_of_year().map(i32::from))
}

fn get_year_of_week<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, PlainDate::year_of_week)
}

fn get_days_in_week<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.days_in_week())))
}

fn get_days_in_month<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.days_in_month())))
}

fn get_days_in_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.days_in_year())))
}

fn get_months_in_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    getter(cx, this, |d| Some(i32::from(d.months_in_year())))
}

fn get_in_leap_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    Ok(Local::boolean(d.in_leap_year()))
}

// ---- methods --------------------------------------------------------------------------------

/// AddDurationToDate(operation, temporalDate, temporalDurationLike, options) (§3.5.14).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 2. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let dur = to_temporal_duration(cx, args.get(0))?;
    // 5. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 6. Let overflow be ? GetTemporalOverflowOption(resolvedOptions).
    let overflow = get_overflow(cx, options)?;
    // 3-4, 7-8. ToDateDurationRecordWithoutTime and CalendarDateAdd (temporal_rs).
    let r = if subtract {
        d.subtract(&dur, Some(overflow))
    } else {
        d.add(&dur, Some(overflow))
    }
    .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainDate.prototype.add ( temporalDurationLike [ , options ] ) (§3.3.22).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, false)
}

/// Temporal.PlainDate.prototype.subtract ( temporalDurationLike [ , options ] ) (§3.3.23).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, true)
}

/// Temporal.PlainDate.prototype.with ( temporalDateLike [ , options ] ) (§3.3.24).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalDateLike) is false, throw a TypeError exception.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial date object"));
    };
    // 6. Let partialDate be ? PrepareCalendarFields(calendar, temporalDateLike, « year,
    //    month, month-code, day », « », partial).
    let bag = prepare_fields(cx, like, fields::DATE, d.calendar())?;
    if !bag.any() {
        return Err(cx.throw_type_error("with() needs at least one date field"));
    }
    // 8. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 9. Let overflow be ? GetTemporalOverflowOption(resolvedOptions).
    let overflow = get_overflow(cx, options)?;
    // 5, 7, 10. CalendarMergeFields and CalendarDateFromFields (temporal_rs).
    let r = d.with(bag.calendar_fields(), Some(overflow)).or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainDate.prototype.withCalendar ( calendarLike ) (§3.3.25).
fn with_calendar<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3. Let calendar be ? ToTemporalCalendarIdentifier(calendarLike).
    let cal = to_temporal_calendar_identifier(cx, args.get(0))?;
    // 4. Return ! CreateTemporalDate(temporalDate.[[ISODate]], calendar).
    create(cx, &d.with_calendar(cal), None)
}

/// DifferenceTemporalPlainDate(operation, temporalDate, other, options) (§3.5.13).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 1. Set other to ? ToTemporalDate(other).
    let other = to_temporal_date(cx, args.get(0), Local::undefined())?;
    // 2. If CalendarEquals(...) is false, throw a RangeError exception.
    require_same(cx, d.calendar(), other.calendar())?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 4. Let settings be ? GetDifferenceSettings(operation, resolvedOptions, date, « », day, day).
    let settings = get_difference_settings(cx, options)?;
    // 5-. The difference (temporal_rs).
    let r = if until {
        d.until(&other, settings)
    } else {
        d.since(&other, settings)
    }
    .or_throw(cx)?;
    create_duration(cx, &r, None)
}

/// Temporal.PlainDate.prototype.until ( other [ , options ] ) (§3.3.26).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.PlainDate.prototype.since ( other [ , options ] ) (§3.3.27).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.PlainDate.prototype.equals ( other ) (§3.3.28).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3. Set other to ? ToTemporalDate(other).
    let other = to_temporal_date(cx, args.get(0), Local::undefined())?;
    // 4. If CompareISODate(...) ≠ 0, return false.
    // 5. Return CalendarEquals(temporalDate.[[Calendar]], other.[[Calendar]]).
    Ok(Local::boolean(
        d.compare_iso(&other).is_eq() && d.calendar().identifier() == other.calendar().identifier(),
    ))
}

/// Temporal.PlainDate.prototype.toString ( \[ options \] ) (§3.3.30).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3. Let resolvedOptions be ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(0))?;
    // 4. Let showCalendar be ? GetTemporalShowCalendarNameOption(resolvedOptions).
    let show = get_show_calendar(cx, options)?;
    // 5. Return TemporalDateToString(temporalDate, showCalendar).
    let s = d.to_ixdtf_string(show);
    cx.new_string(&s).map(|s| s.value())
}

/// TemporalDateToString(temporalDate, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    let s = d.to_ixdtf_string(DisplayCalendar::Auto);
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.PlainDate.prototype.toJSON ( ) (§3.3.32).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainDate.prototype.toLocaleString ( ) (§3.3.31): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainDate.prototype.valueOf ( ) (§3.3.33): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.PlainDate cannot be converted to a primitive; use compare() or equals()",
    ))
}

/// Temporal.PlainDate.prototype.toPlainDateTime ( \[ temporalTime \] ).
fn to_plain_date_time<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3. Let time be ? ToTimeRecordOrMidnight(temporalTime).
    let t = crate::plain_date_time::to_time_or_midnight(cx, args.get(0))?;
    // 4-5. CombineISODateAndTimeRecord; CreateTemporalDateTime (the limits: RangeError).
    let dt = d.to_plain_date_time(t).or_throw(cx)?;
    crate::plain_date_time::create(cx, &dt, None)
}

/// Temporal.PlainDate.prototype.toPlainYearMonth ( ).
fn to_plain_year_month<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3-6. ISODateToFields; CalendarYearMonthFromFields(calendar, fields, constrain).
    let ym = d.to_plain_year_month().or_throw(cx)?;
    crate::plain_year_month::create(cx, &ym, None)
}

/// Temporal.PlainDate.prototype.toPlainMonthDay ( ).
fn to_plain_month_day<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = read(cx, this)?;
    // 3-6. ISODateToFields; CalendarMonthDayFromFields(calendar, fields, constrain).
    let md = d.to_plain_month_day().or_throw(cx)?;
    crate::plain_month_day::create(cx, &md, None)
}
