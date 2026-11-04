//! Temporal.PlainYearMonth (Temporal §9): module `plain_year_month`, block 25 (design
//! §3.2-§3.4, WP T7). Makes `%Temporal.PlainYearMonth%` and
//! `%Temporal.PlainYearMonth.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalPlainYearMonth, 4 int32 slots (design §3.2):
//!   0 ISO year, 1 ISO month, 2 reference ISO day, 3 calendar code (0 = iso8601)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plain_year_month.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::PlainYearMonth;
use temporal_rs::options::{DisplayCalendar, Overflow};
use temporal_rs::partial::PartialYearMonth;

use crate::calendar::{self, constructor_calendar, get_calendar_with_iso_default, require_iso};
use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::duration::{create as create_duration, to_temporal_duration};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object};
use crate::options::{get_difference_settings, get_options_object};
use crate::plain_date::{self, get_show_calendar};
use crate::plain_time::get_overflow;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 25;

const SLOTS: usize = 4;
const CLASS: ObjectClass = ObjectClass::TemporalPlainYearMonth;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PlainYearMonth", 2, constructor), // 0
    BuiltinDef::method("from", 1, from),                       // 1
    BuiltinDef::method("compare", 2, compare),                 // 2
    BuiltinDef::getter("calendarId", get_calendar_id),         // 3
    BuiltinDef::getter("era", get_era),                        // 4
    BuiltinDef::getter("eraYear", get_era_year),               // 5
    BuiltinDef::getter("year", get_year),                      // 6
    BuiltinDef::getter("month", get_month),                    // 7
    BuiltinDef::getter("monthCode", get_month_code),           // 8
    BuiltinDef::getter("daysInYear", get_days_in_year),        // 9
    BuiltinDef::getter("daysInMonth", get_days_in_month),      // 10
    BuiltinDef::getter("monthsInYear", get_months_in_year),    // 11
    BuiltinDef::getter("inLeapYear", get_in_leap_year),        // 12
    BuiltinDef::method("with", 1, with),                       // 13
    BuiltinDef::method("add", 1, add),                         // 14
    BuiltinDef::method("subtract", 1, subtract),               // 15
    BuiltinDef::method("until", 1, until),                     // 16
    BuiltinDef::method("since", 1, since),                     // 17
    BuiltinDef::method("equals", 1, equals),                   // 18
    BuiltinDef::method("toString", 0, to_string),              // 19
    BuiltinDef::method("toJSON", 0, to_json),                  // 20
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 21
    BuiltinDef::method("valueOf", 0, value_of),                // 22
    BuiltinDef::method("toPlainDate", 1, to_plain_date),       // 23
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainYearMonth,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainYearMonthPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalPlainYearMonth,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalPlainYearMonthPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
            PropDef::method(PropKey::Str("compare"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalPlainYearMonthPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalPlainYearMonth,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.PlainYearMonth"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("calendarId"), Some(3), None),
            PropDef::accessor(PropKey::Str("era"), Some(4), None),
            PropDef::accessor(PropKey::Str("eraYear"), Some(5), None),
            PropDef::accessor(PropKey::Str("year"), Some(6), None),
            PropDef::accessor(PropKey::Str("month"), Some(7), None),
            PropDef::accessor(PropKey::Str("monthCode"), Some(8), None),
            PropDef::accessor(PropKey::Str("daysInYear"), Some(9), None),
            PropDef::accessor(PropKey::Str("daysInMonth"), Some(10), None),
            PropDef::accessor(PropKey::Str("monthsInYear"), Some(11), None),
            PropDef::accessor(PropKey::Str("inLeapYear"), Some(12), None),
            PropDef::method(PropKey::Str("with"), 13),
            PropDef::method(PropKey::Str("add"), 14),
            PropDef::method(PropKey::Str("subtract"), 15),
            PropDef::method(PropKey::Str("until"), 16),
            PropDef::method(PropKey::Str("since"), 17),
            PropDef::method(PropKey::Str("equals"), 18),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 19),
            PropDef::method(PropKey::Str("toJSON"), 20),
            PropDef::method(PropKey::Str("toLocaleString"), 21),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 22),
            PropDef::method(PropKey::Str("toPlainDate"), 23),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("PlainYearMonth"),
            IntrinsicId::TemporalPlainYearMonth,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plain_year_month",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The PlainYearMonth in `v` (RequireInternalSlot: a TypeError if `v` is not one).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<PlainYearMonth> {
    let mut f = [0.0f64; SLOTS];
    for (i, x) in f.iter_mut().enumerate() {
        *x = cx.slot(v, CLASS, i)?.as_number().unwrap_or(0.0);
    }
    PlainYearMonth::new_with_overflow(
        f[0] as i32,
        f[1] as u8,
        Some(f[2] as u8),
        calendar::from_slot(f[3]),
        Overflow::Reject,
    )
    .or_throw(cx)
}

/// CreateTemporalYearMonth(isoDate, calendar, newTarget): a new PlainYearMonth object.
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    ym: &PlainYearMonth,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalPlainYearMonthPrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalPlainYearMonthPrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // iso8601 only (SPEC-T-D17): the calendar fields are the ISO fields.
    cx.set_slot(o, CLASS, 0, Local::int32(ym.year()))?;
    cx.set_slot(o, CLASS, 1, Local::int32(i32::from(ym.month())))?;
    cx.set_slot(o, CLASS, 2, Local::int32(i32::from(ym.reference_day())))?;
    cx.set_slot(o, CLASS, 3, Local::int32(calendar::to_slot(ym.calendar())))?;
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// ToTemporalYearMonth(item [, options]) (§9.5.2). `options` is the raw argument.
pub(crate) fn to_temporal_year_month<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<PlainYearMonth> {
    // 2. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a. A PlainYearMonth: read the options, then its value.
        if cx.object_class(item) == Some(CLASS) {
            let ym = read(cx, item)?;
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return Ok(ym);
        }
        // b. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(item).
        let cal = get_calendar_with_iso_default(cx, o)?;
        // c. Let fields be ? PrepareCalendarFields(calendar, item, « year, month,
        //    month-code », «», «»).
        let bag = fields::read(cx, o, fields::YEAR_MONTH)?;
        // d-e. Let resolvedOptions be ? GetOptionsObject(options); overflow.
        let options = get_options_object(cx, options)?;
        let overflow = get_overflow(cx, options)?;
        // f. Let isoDate be ? CalendarYearMonthFromFields(calendar, fields, overflow).
        return PlainYearMonth::from_partial(
            PartialYearMonth {
                calendar_fields: bag.year_month_fields(),
                calendar: cal,
            },
            Some(overflow),
        )
        .or_throw(cx);
    }
    // 3. If item is not a String, throw a TypeError exception.
    let Some(s) = item.as_string() else {
        return Err(cx.throw_type_error("a year-month must be an object or a string"));
    };
    // 4-7. ParseISODateTime(item, « TemporalYearMonthString »); the calendar.
    let text = temporal_text(cx, s)?;
    let ym = PlainYearMonth::from_utf8(text.as_bytes()).or_throw(cx)?;
    require_iso(cx, ym.calendar().clone())?;
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); GetTemporalOverflowOption.
    let options = get_options_object(cx, options)?;
    get_overflow(cx, options)?;
    // 10-. ISOYearMonthWithinLimits; CalendarYearMonthFromFields (temporal_rs, above).
    Ok(ym)
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.PlainYearMonth ( isoYear, isoMonth [ , calendar [ , referenceISODay ] ] ) (§9.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.PlainYearMonth requires 'new'"));
    };
    // 3-4. Let y, m be ? ToIntegerWithTruncation(isoYear, isoMonth).
    let y = to_integer_with_truncation(cx, args.get(0))?;
    let m = to_integer_with_truncation(cx, args.get(1))?;
    // 5-7. The calendar.
    let cal = constructor_calendar(cx, args.get(2))?;
    // 2, 8. Let ref be ? ToIntegerWithTruncation(referenceISODay) (undefined is 1).
    let r = args.get(3);
    let ref_day = if r.is_undefined() {
        1.0
    } else {
        to_integer_with_truncation(cx, r)?
    };
    // 9. If IsValidISODate(y, m, ref) is false, throw a RangeError exception.
    if !(1.0..=12.0).contains(&m) || !(1.0..=31.0).contains(&ref_day) || y.abs() > 1e6 {
        return Err(cx.throw_range_error("date out of range"));
    }
    // 10. CreateTemporalYearMonth (ISOYearMonthWithinLimits).
    let ym = PlainYearMonth::new_with_overflow(
        y as i32,
        m as u8,
        Some(ref_day as u8),
        cal,
        Overflow::Reject,
    )
    .or_throw(cx)?;
    create(cx, &ym, Some(nt))
}

/// Temporal.PlainYearMonth.from ( item [ , options ] ) (§9.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = to_temporal_year_month(cx, args.get(0), args.get(1))?;
    create(cx, &ym, None)
}

/// Temporal.PlainYearMonth.compare ( one, two ) (§9.2.3).
fn compare<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let one = to_temporal_year_month(cx, args.get(0), Local::undefined())?;
    let two = to_temporal_year_month(cx, args.get(1), Local::undefined())?;
    // 3. Return 𝔽(CompareISODate(one.[[ISODate]], two.[[ISODate]])).
    Ok(Local::int32(one.compare_iso(&two) as i32))
}

// ---- getters --------------------------------------------------------------------------------

/// A getter of an int32 (or undefined) value.
fn int_getter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: impl FnOnce(&PlainYearMonth) -> Option<i32>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    Ok(f(&ym).map_or(Local::undefined(), Local::int32))
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
    get_era_year => PlainYearMonth::era_year;
    get_year => |d| Some(d.year());
    get_month => |d| Some(i32::from(d.month()));
    get_days_in_year => |d| Some(i32::from(d.days_in_year()));
    get_days_in_month => |d| Some(i32::from(d.days_in_month()));
    get_months_in_year => |d| Some(i32::from(d.months_in_year()));
}

/// get Temporal.PlainYearMonth.prototype.calendarId (§9.3.3).
fn get_calendar_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    cx.new_string(ym.calendar_id()).map(|s| s.value())
}

/// get Temporal.PlainYearMonth.prototype.era (§9.3.4): undefined for iso8601.
fn get_era<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    match ym.era() {
        Some(e) => cx.new_string(e.as_str()).map(|s| s.value()),
        None => Ok(Local::undefined()),
    }
}

/// get Temporal.PlainYearMonth.prototype.monthCode (§9.3.8).
fn get_month_code<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    let mc = ym.month_code();
    cx.new_string(mc.as_str()).map(|s| s.value())
}

/// get Temporal.PlainYearMonth.prototype.inLeapYear (§9.3.12).
fn get_in_leap_year<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    Ok(Local::boolean(ym.in_leap_year()))
}

// ---- methods --------------------------------------------------------------------------------

/// Temporal.PlainYearMonth.prototype.with ( temporalYearMonthLike [ , options ] ) (§9.3.13).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalYearMonthLike) is false, throw a TypeError.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial year-month object"));
    };
    // 6. Let partialYearMonth be ? PrepareCalendarFields(calendar, temporalYearMonthLike,
    //    « year, month, month-code », « », partial).
    let bag = fields::read(cx, like, fields::YEAR_MONTH)?;
    if !bag.any {
        return Err(cx.throw_type_error("with() needs at least one year or month field"));
    }
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    // 7, 10. CalendarMergeFields and CalendarYearMonthFromFields (temporal_rs).
    let r = ym
        .with(bag.year_month_fields(), Some(overflow))
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// AddDurationToYearMonth(operation, yearMonth, temporalDurationLike, options).
fn add_or_subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    subtract: bool,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    // 1. Let duration be ? ToTemporalDuration(temporalDurationLike).
    let dur = to_temporal_duration(cx, args.get(0))?;
    // 3-4. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    // 5-. The addition (temporal_rs).
    let r = if subtract {
        ym.subtract(&dur, overflow)
    } else {
        ym.add(&dur, overflow)
    }
    .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainYearMonth.prototype.add ( temporalDurationLike [ , options ] ) (§9.3.14).
fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, false)
}

/// Temporal.PlainYearMonth.prototype.subtract ( temporalDurationLike [ , options ] ) (§9.3.15).
fn subtract<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    add_or_subtract(cx, this, args, true)
}

/// DifferenceTemporalPlainYearMonth(operation, yearMonth, other, options).
fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    until: bool,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    // 1. Set other to ? ToTemporalYearMonth(other).
    let other = to_temporal_year_month(cx, args.get(0), Local::undefined())?;
    // 3-4. Let resolvedOptions be ? GetOptionsObject(options); GetDifferenceSettings(
    //      operation, resolvedOptions, date, « week, day », month, year).
    let options = get_options_object(cx, args.get(1))?;
    let settings = get_difference_settings(cx, options)?;
    let r = if until {
        ym.until(&other, settings)
    } else {
        ym.since(&other, settings)
    }
    .or_throw(cx)?;
    create_duration(cx, &r, None)
}

/// Temporal.PlainYearMonth.prototype.until ( other [ , options ] ) (§9.3.16).
fn until<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, true)
}

/// Temporal.PlainYearMonth.prototype.since ( other [ , options ] ) (§9.3.17).
fn since<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    difference(cx, this, args, false)
}

/// Temporal.PlainYearMonth.prototype.equals ( other ) (§9.3.18).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    let other = to_temporal_year_month(cx, args.get(0), Local::undefined())?;
    Ok(Local::boolean(
        ym.compare_iso(&other).is_eq() && ym.calendar_id() == other.calendar_id(),
    ))
}

/// Temporal.PlainYearMonth.prototype.toString ( [ options ] ) (§9.3.19).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    let options = get_options_object(cx, args.get(0))?;
    let show = get_show_calendar(cx, options)?;
    let s = ym.to_ixdtf_string(show);
    cx.new_string(&s).map(|s| s.value())
}

/// TemporalYearMonthToString(yearMonth, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    let s = ym.to_ixdtf_string(DisplayCalendar::Auto);
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.PlainYearMonth.prototype.toJSON ( ) (§9.3.21).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainYearMonth.prototype.toLocaleString ( ) (§9.3.20): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainYearMonth.prototype.valueOf ( ) (§9.3.22): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.PlainYearMonth cannot be converted to a primitive; use compare() or equals()",
    ))
}

/// Temporal.PlainYearMonth.prototype.toPlainDate ( item ) (§9.3.23).
fn to_plain_date<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ym = read(cx, this)?;
    // 3. If item is not an Object, throw a TypeError exception.
    let Some(item) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("toPlainDate() needs an object with a day"));
    };
    // 6. Let inputFields be ? PrepareCalendarFields(calendar, item, « day », « », « »).
    let bag = fields::read(cx, item, fields::DAY)?;
    // 7-9. CalendarMergeFields; CalendarDateFromFields(calendar, mergedFields, constrain).
    let d = ym.to_plain_date(Some(bag.calendar_fields())).or_throw(cx)?;
    plain_date::create(cx, &d, None)
}
