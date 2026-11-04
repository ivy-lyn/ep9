//! Calendars (design SPEC-T-D17): ep9 has no ECMA-402, so the only calendar is `iso8601`
//! (AvailableCalendars is « "iso8601" »). Any other calendar id, in an argument, a property
//! bag or a string annotation, is a RangeError.
//!
//! A Temporal object keeps its calendar in a slot as an ep9 code (append-only):
//! [`CAL_ISO8601`] = 0 = `iso8601`.

use core::str::FromStr;

use ep9_vm::{Cx, JsObject, JsResult, Local, ObjectClass};
use temporal_rs::Calendar;

/// `cal` if it is `iso8601`, else a RangeError (SPEC-T-D17).
pub(crate) fn require_iso(cx: &mut Cx<'_>, cal: Calendar) -> JsResult<Calendar> {
    if cal.is_iso() {
        Ok(cal)
    } else {
        let msg = format!(
            "calendar {} is not supported (ep9 has only iso8601)",
            cal.identifier()
        );
        Err(cx.throw_range_error(&msg))
    }
}

/// The classes with a `[[Calendar]]` slot (PlainDate, PlainDateTime, PlainYearMonth,
/// PlainMonthDay, ZonedDateTime), as they are built.
const CALENDAR_CLASSES: &[ObjectClass] = &[
    ObjectClass::TemporalPlainDate,
    ObjectClass::TemporalPlainDateTime,
    ObjectClass::TemporalPlainYearMonth,
    ObjectClass::TemporalPlainMonthDay,
    ObjectClass::TemporalZonedDateTime,
];

/// The calendar code of `iso8601` in a calendar slot (ep9 codes, append-only).
pub(crate) const CAL_ISO8601: i32 = 0;

/// The slot code of `cal` (a calendar that passed [`require_iso`]).
pub(crate) fn to_slot(cal: &Calendar) -> i32 {
    debug_assert!(cal.is_iso());
    CAL_ISO8601
}

/// The calendar of a slot code.
pub(crate) fn from_slot(code: f64) -> Calendar {
    debug_assert_eq!(code, f64::from(CAL_ISO8601));
    Calendar::ISO
}

/// CanonicalizeCalendar (§12.2.1) of the String `id`: its ASCII-lowercase form must be an
/// available calendar (only `iso8601`), else a RangeError.
pub(crate) fn canonicalize_calendar(cx: &mut Cx<'_>, id: &str) -> JsResult<Calendar> {
    if id.eq_ignore_ascii_case("iso8601") {
        Ok(Calendar::ISO)
    } else {
        Err(cx.throw_range_error(&format!(
            "calendar {id} is not supported (ep9 has only iso8601)"
        )))
    }
}

/// Whether `v` is a Temporal object with a `[[Calendar]]` slot.
pub(crate) fn has_calendar_slot(cx: &Cx<'_>, v: Local<'_>) -> bool {
    cx.object_class(v)
        .is_some_and(|c| CALENDAR_CLASSES.contains(&c))
}

/// ToTemporalCalendarIdentifier (§12.2.9).
pub(crate) fn to_temporal_calendar_identifier<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<Calendar> {
    // 1. If temporalCalendarLike is an Object, then
    //    a. If it has a [[Calendar]] slot, return temporalCalendarLike.[[Calendar]].
    if has_calendar_slot(cx, v) {
        return Ok(Calendar::ISO);
    }
    // 2. If temporalCalendarLike is not a String, throw a TypeError exception.
    let Some(s) = v.as_string() else {
        return Err(cx.throw_type_error("calendar must be a string"));
    };
    // 3. Let identifier be ? ParseTemporalCalendarString(temporalCalendarLike).
    // 4. Return ? CanonicalizeCalendar(identifier).
    let t = crate::convert::temporal_text(cx, s)?;
    let cal = match Calendar::from_str(&t) {
        Ok(c) => c,
        Err(e) => return Err(crate::error::throw(cx, e)),
    };
    require_iso(cx, cal)
}

/// GetTemporalCalendarIdentifierWithISODefault (§12.2.10).
pub(crate) fn get_calendar_with_iso_default<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s, JsObject>,
) -> JsResult<Calendar> {
    // 1. If item has a [[Calendar]] slot, return item.[[Calendar]].
    if has_calendar_slot(cx, item.value()) {
        return Ok(Calendar::ISO);
    }
    // 2. Let calendarLike be ? Get(item, "calendar").
    let k = cx.key("calendar");
    let like = cx.get(item, k)?;
    // 3. If calendarLike is undefined, return "iso8601".
    if like.is_undefined() {
        return Ok(Calendar::ISO);
    }
    // 4. Return ? ToTemporalCalendarIdentifier(calendarLike).
    to_temporal_calendar_identifier(cx, like)
}

/// The `calendar` argument of a constructor (PlainDate, PlainDateTime, PlainYearMonth,
/// PlainMonthDay): undefined is `iso8601`; not a String is a TypeError; else
/// CanonicalizeCalendar.
pub(crate) fn constructor_calendar<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Calendar> {
    if v.is_undefined() {
        return Ok(Calendar::ISO);
    }
    let Some(s) = v.as_string() else {
        return Err(cx.throw_type_error("calendar must be a string"));
    };
    let id = cx.to_rust_string(s)?;
    canonicalize_calendar(cx, &id)
}
