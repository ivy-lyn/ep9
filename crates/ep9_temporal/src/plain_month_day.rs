//! Temporal.PlainMonthDay (Temporal §10): module `plain_month_day`, block 26 (design
//! §3.2-§3.4, WP T7). Makes `%Temporal.PlainMonthDay%` and
//! `%Temporal.PlainMonthDay.prototype%`.
//!
//! ```text
//! object: ObjectClass::TemporalPlainMonthDay, 4 int32 slots (design §3.2):
//!   0 reference ISO year, 1 ISO month, 2 ISO day, 3 calendar code (0 = iso8601)
//! ```
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plain_month_day.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use temporal_rs::PlainMonthDay;
use temporal_rs::options::{DisplayCalendar, Overflow};
use temporal_rs::partial::PartialDate;

use crate::calendar::{self, constructor_calendar, get_calendar_with_iso_default, require_iso};
use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::error::OrThrow;
use crate::fields::{self, is_partial_temporal_object};
use crate::options::get_options_object;
use crate::plain_date::{self, get_show_calendar};
use crate::plain_time::get_overflow;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 26;

const SLOTS: usize = 4;
const CLASS: ObjectClass = ObjectClass::TemporalPlainMonthDay;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PlainMonthDay", 2, constructor), // 0
    BuiltinDef::method("from", 1, from),                      // 1
    BuiltinDef::getter("calendarId", get_calendar_id),        // 2
    BuiltinDef::getter("monthCode", get_month_code),          // 3
    BuiltinDef::getter("day", get_day),                       // 4
    BuiltinDef::method("with", 1, with),                      // 5
    BuiltinDef::method("equals", 1, equals),                  // 6
    BuiltinDef::method("toString", 0, to_string),             // 7
    BuiltinDef::method("toJSON", 0, to_json),                 // 8
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 9
    BuiltinDef::method("valueOf", 0, value_of),               // 10
    BuiltinDef::method("toPlainDate", 1, to_plain_date),      // 11
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainMonthDay,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::TemporalPlainMonthDayPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::TemporalPlainMonthDay,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::TemporalPlainMonthDayPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("from"), 1),
        ],
    },
    ObjectProps {
        target: IntrinsicId::TemporalPlainMonthDayPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::TemporalPlainMonthDay,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Temporal.PlainMonthDay"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("calendarId"), Some(2), None),
            PropDef::accessor(PropKey::Str("monthCode"), Some(3), None),
            PropDef::accessor(PropKey::Str("day"), Some(4), None),
            PropDef::method(PropKey::Str("with"), 5),
            PropDef::method(PropKey::Str("equals"), 6),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 7),
            PropDef::method(PropKey::Str("toJSON"), 8),
            PropDef::method(PropKey::Str("toLocaleString"), 9),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 10),
            PropDef::method(PropKey::Str("toPlainDate"), 11),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Temporal,
        props: &[PropDef::intrinsic(
            PropKey::Str("PlainMonthDay"),
            IntrinsicId::TemporalPlainMonthDay,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plain_month_day",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- slots ----------------------------------------------------------------------------------

/// The PlainMonthDay in `v` (RequireInternalSlot: a TypeError if `v` is not one).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<PlainMonthDay> {
    let mut f = [0.0f64; SLOTS];
    for (i, x) in f.iter_mut().enumerate() {
        *x = cx.slot(v, CLASS, i)?.as_number().unwrap_or(0.0);
    }
    PlainMonthDay::new_with_overflow(
        f[1] as u8,
        f[2] as u8,
        calendar::from_slot(f[3]),
        Overflow::Reject,
        Some(f[0] as i32),
    )
    .or_throw(cx)
}

/// CreateTemporalMonthDay(isoDate, calendar, newTarget): a new PlainMonthDay object.
pub(crate) fn create<'s>(
    cx: &mut Cx<'s>,
    md: &PlainMonthDay,
    new_target: Option<Local<'s, JsObject>>,
) -> JsResult<Local<'s>> {
    let proto = match new_target {
        Some(nt) => {
            cx.get_prototype_from_constructor(nt, IntrinsicId::TemporalPlainMonthDayPrototype)?
        }
        None => cx.intrinsic(IntrinsicId::TemporalPlainMonthDayPrototype),
    };
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // iso8601 only (SPEC-T-D17): the month code M01..M12 gives the ISO month.
    let month = md.month_code().to_month_integer();
    cx.set_slot(o, CLASS, 0, Local::int32(md.reference_year()))?;
    cx.set_slot(o, CLASS, 1, Local::int32(i32::from(month)))?;
    cx.set_slot(o, CLASS, 2, Local::int32(i32::from(md.day())))?;
    cx.set_slot(o, CLASS, 3, Local::int32(calendar::to_slot(md.calendar())))?;
    Ok(o.value())
}

// ---- abstract operations --------------------------------------------------------------------

/// ToTemporalMonthDay(item [, options]) (§10.5.1). `options` is the raw argument.
fn to_temporal_month_day<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s>,
    options: Local<'s>,
) -> JsResult<PlainMonthDay> {
    // 2. If item is an Object, then
    if let Some(o) = item.as_object() {
        // a. A PlainMonthDay: read the options, then its value.
        if cx.object_class(item) == Some(CLASS) {
            let md = read(cx, item)?;
            let options = get_options_object(cx, options)?;
            get_overflow(cx, options)?;
            return Ok(md);
        }
        // b. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(item).
        let cal = get_calendar_with_iso_default(cx, o)?;
        // c. Let fields be ? PrepareCalendarFields(calendar, item, « year, month, month-code,
        //    day », «», «»).
        let bag = fields::read(cx, o, fields::DATE)?;
        // d-e. Let resolvedOptions be ? GetOptionsObject(options); overflow.
        let options = get_options_object(cx, options)?;
        let overflow = get_overflow(cx, options)?;
        // f. Let isoDate be ? CalendarMonthDayFromFields(calendar, fields, overflow).
        return PlainMonthDay::from_partial(
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
        return Err(cx.throw_type_error("a month-day must be an object or a string"));
    };
    // 4-7. ParseISODateTime(item, « TemporalMonthDayString »); the calendar.
    let text = temporal_text(cx, s)?;
    let md = PlainMonthDay::from_utf8(text.as_bytes()).or_throw(cx)?;
    require_iso(cx, md.calendar().clone())?;
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); GetTemporalOverflowOption.
    let options = get_options_object(cx, options)?;
    get_overflow(cx, options)?;
    // 10-. iso8601: the reference year is 1972 (temporal_rs, above).
    Ok(md)
}

// ---- constructor and statics ----------------------------------------------------------------

/// Temporal.PlainMonthDay ( isoMonth, isoDay [ , calendar [ , referenceISOYear ] ] ) (§10.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Temporal.PlainMonthDay requires 'new'"));
    };
    // 3-4. Let m, d be ? ToIntegerWithTruncation(isoMonth, isoDay).
    let m = to_integer_with_truncation(cx, args.get(0))?;
    let d = to_integer_with_truncation(cx, args.get(1))?;
    // 5-7. The calendar.
    let cal = constructor_calendar(cx, args.get(2))?;
    // 2, 8. Let y be ? ToIntegerWithTruncation(referenceISOYear) (undefined is 1972).
    let r = args.get(3);
    let y = if r.is_undefined() {
        1972.0
    } else {
        to_integer_with_truncation(cx, r)?
    };
    // 9. If IsValidISODate(y, m, d) is false, throw a RangeError exception.
    if !(1.0..=12.0).contains(&m) || !(1.0..=31.0).contains(&d) || y.abs() > 1e6 {
        return Err(cx.throw_range_error("date out of range"));
    }
    // 10. CreateTemporalMonthDay (ISODateWithinLimits).
    let md =
        PlainMonthDay::new_with_overflow(m as u8, d as u8, cal, Overflow::Reject, Some(y as i32))
            .or_throw(cx)?;
    if !plain_date_in_limits(y as i32, m as u8, d as u8) {
        return Err(cx.throw_range_error("date out of range"));
    }
    create(cx, &md, Some(nt))
}

/// ISODateWithinLimits for the reference date (temporal_rs does not check it here).
fn plain_date_in_limits(y: i32, m: u8, d: u8) -> bool {
    temporal_rs::PlainDate::try_new_iso(y, m, d).is_ok()
}

/// Temporal.PlainMonthDay.from ( item [ , options ] ) (§10.2.2).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = to_temporal_month_day(cx, args.get(0), args.get(1))?;
    create(cx, &md, None)
}

// ---- getters --------------------------------------------------------------------------------

/// get Temporal.PlainMonthDay.prototype.calendarId (§10.3.3).
fn get_calendar_id<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    cx.new_string(md.calendar_id()).map(|s| s.value())
}

/// get Temporal.PlainMonthDay.prototype.monthCode (§10.3.4).
fn get_month_code<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    let mc = md.month_code();
    cx.new_string(mc.as_str()).map(|s| s.value())
}

/// get Temporal.PlainMonthDay.prototype.day (§10.3.5).
fn get_day<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    Ok(Local::int32(i32::from(md.day())))
}

// ---- methods --------------------------------------------------------------------------------

/// Temporal.PlainMonthDay.prototype.with ( temporalMonthDayLike [ , options ] ) (§10.3.6).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    // 3. If ? IsPartialTemporalObject(temporalMonthDayLike) is false, throw a TypeError.
    let Some(like) = is_partial_temporal_object(cx, args.get(0))? else {
        return Err(cx.throw_type_error("with() needs a partial month-day object"));
    };
    // 6. Let partialMonthDay be ? PrepareCalendarFields(calendar, temporalMonthDayLike,
    //    « year, month, month-code, day », « », partial).
    let bag = fields::read(cx, like, fields::DATE)?;
    if !bag.any {
        return Err(cx.throw_type_error("with() needs at least one date field"));
    }
    // 8-9. Let resolvedOptions be ? GetOptionsObject(options); overflow.
    let options = get_options_object(cx, args.get(1))?;
    let overflow = get_overflow(cx, options)?;
    // 5, 7, 10. CalendarMergeFields and CalendarMonthDayFromFields (temporal_rs).
    let r = md
        .with(bag.calendar_fields(), Some(overflow))
        .or_throw(cx)?;
    create(cx, &r, None)
}

/// Temporal.PlainMonthDay.prototype.equals ( other ) (§10.3.7).
fn equals<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    // 3. Set other to ? ToTemporalMonthDay(other).
    let other = to_temporal_month_day(cx, args.get(0), Local::undefined())?;
    // 4-5. CompareISODate (with the reference years) = 0 and CalendarEquals.
    Ok(Local::boolean(
        md.reference_year() == other.reference_year()
            && md.month_code() == other.month_code()
            && md.day() == other.day()
            && md.calendar_id() == other.calendar_id(),
    ))
}

/// Temporal.PlainMonthDay.prototype.toString ( [ options ] ) (§10.3.8).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    let options = get_options_object(cx, args.get(0))?;
    let show = get_show_calendar(cx, options)?;
    let s = md.to_ixdtf_string(show);
    cx.new_string(&s).map(|s| s.value())
}

/// TemporalMonthDayToString(monthDay, auto).
fn default_text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    let s = md.to_ixdtf_string(DisplayCalendar::Auto);
    cx.new_string(&s).map(|s| s.value())
}

/// Temporal.PlainMonthDay.prototype.toJSON ( ) (§10.3.10).
fn to_json<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainMonthDay.prototype.toLocaleString ( ) (§10.3.9): the toString() text (no
/// ECMA-402, design SPEC-T-D18).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    default_text(cx, this)
}

/// Temporal.PlainMonthDay.prototype.valueOf ( ) (§10.3.11): always a TypeError.
fn value_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error(
        "Temporal.PlainMonthDay cannot be converted to a primitive; use equals()",
    ))
}

/// Temporal.PlainMonthDay.prototype.toPlainDate ( item ) (§10.3.12).
fn to_plain_date<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let md = read(cx, this)?;
    // 3. If item is not an Object, throw a TypeError exception.
    let Some(item) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("toPlainDate() needs an object with a year"));
    };
    // 6. Let inputFields be ? PrepareCalendarFields(calendar, item, « year », « », « »).
    let bag = fields::read(cx, item, fields::YEAR)?;
    // 7-9. CalendarMergeFields; CalendarDateFromFields(calendar, mergedFields, constrain).
    let d = md.to_plain_date(Some(bag.calendar_fields())).or_throw(cx)?;
    plain_date::create(cx, &d, None)
}
