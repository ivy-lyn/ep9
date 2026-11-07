//! GetTemporalRelativeToOption (Duration `compare`, `round`, `total`).
//!
//! ```text
//! relativeTo undefined          -> none
//! a ZonedDateTime / PlainDate   -> that value (PlainDateTime -> its date)   (as the classes land)
//! another object (property bag) -> calendar, PrepareCalendarFields(date, time, offset,
//!                                  timeZone, and era with eraYear for a calendar with eras),
//!                                  constrain; no timeZone: a PlainDate, else a ZonedDateTime
//!                                  (offset "reject", disambiguation "compatible")
//! a String                      -> ParseTemporalRelativeToString (temporal_rs), then the
//!                                  calendar (AvailableCalendars, calendar.rs)
//! anything else                 -> TypeError
//! ```

use ep9_vm::{Cx, JsResult, Local};
use temporal_rs::fields::ZonedDateTimeFields;
use temporal_rs::options::{Overflow, RelativeTo};
use temporal_rs::partial::{PartialDate, PartialZonedDateTime};
use temporal_rs::{PlainDate, ZonedDateTime};

use crate::calendar::{get_calendar_with_iso_default, prepare_fields, string_calendar};
use crate::convert::temporal_text;
use crate::error::OrThrow;
use crate::fields;
use crate::options::Options;

/// GetTemporalRelativeToOption(options).
pub(crate) fn get_relative_to_option<'s>(
    cx: &mut Cx<'s>,
    options: Options<'s>,
) -> JsResult<Option<RelativeTo>> {
    let Some(o) = options else {
        return Ok(None);
    };
    // 1. Let value be ? Get(options, "relativeTo").
    let k = cx.key("relativeTo");
    let value = cx.get(o, k)?;
    // 2. If value is undefined, return { undefined, undefined }.
    if value.is_undefined() {
        return Ok(None);
    }
    // 5. If value is an Object, then
    if let Some(obj) = value.as_object() {
        // a-c. A ZonedDateTime, PlainDate or PlainDateTime gives its value.
        if let Some(r) = of_object(cx, value)? {
            return Ok(Some(r));
        }
        // d. Let calendar be ? GetTemporalCalendarIdentifierWithISODefault(value).
        let calendar = get_calendar_with_iso_default(cx, obj)?;
        // e. Let fields be ? PrepareCalendarFields(calendar, value, « year, month, month-code,
        //    day », « hour, minute, second, millisecond, microsecond, nanosecond, offset,
        //    time-zone », «»).
        let bag = prepare_fields(
            cx,
            obj,
            fields::DATE | fields::TIME | fields::OFFSET | fields::TIME_ZONE,
            &calendar,
        )?;
        // f. Let result be ? InterpretTemporalDateTimeFields(calendar, fields, constrain).
        let time = bag.bag.partial_time(cx, Overflow::Constrain)?;
        // i. If timeZone is unset: a PlainDate.
        let Some(tz) = bag.bag.time_zone else {
            let date = PlainDate::from_partial(
                PartialDate {
                    calendar_fields: bag.calendar_fields(),
                    calendar,
                },
                Some(Overflow::Constrain),
            )
            .or_throw(cx)?;
            return Ok(Some(RelativeTo::PlainDate(date)));
        };
        // j-m. Else a ZonedDateTime: offsetBehaviour option (wall without an offset),
        //      match-exactly, compatible, reject.
        let zdt = ZonedDateTime::from_partial_with_provider(
            PartialZonedDateTime {
                fields: ZonedDateTimeFields {
                    calendar_fields: bag.calendar_fields(),
                    time,
                    offset: bag.bag.offset,
                },
                timezone: Some(tz),
                calendar,
            },
            Some(Overflow::Constrain),
            None,
            None,
            crate::tzdb::provider(),
        )
        .or_throw(cx)?;
        return Ok(Some(RelativeTo::ZonedDateTime(zdt)));
    }
    // 6. Else, a. If value is not a String, throw a TypeError exception.
    let Some(s) = value.as_string() else {
        return Err(cx.throw_type_error("relativeTo must be an object or a string"));
    };
    // b-k. ParseTemporalRelativeToString and the rest.
    let t = temporal_text(cx, s)?;
    let r = RelativeTo::try_from_str_with_provider(&t, crate::tzdb::provider()).or_throw(cx)?;
    // CanonicalizeCalendar of the annotation (AvailableCalendars, calendar.rs).
    let cal = match &r {
        RelativeTo::PlainDate(d) => d.calendar(),
        RelativeTo::ZonedDateTime(z) => z.calendar(),
    };
    string_calendar(cx, &t, Some(cal))?;
    Ok(Some(r))
}

/// The relativeTo value of a ZonedDateTime, PlainDate or PlainDateTime object (steps 5.a-c),
/// else `None`. The classes join this list as they are built.
fn of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<RelativeTo>> {
    // b. A ZonedDateTime gives itself.
    if let Some(z) = crate::zoned_date_time::of_object(cx, v)? {
        return Ok(Some(RelativeTo::ZonedDateTime(z)));
    }
    // c. A PlainDate, or a PlainDateTime's date.
    Ok(crate::plain_date::date_of_object(cx, v)?.map(RelativeTo::PlainDate))
}
