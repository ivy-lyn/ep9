//! The read-only view of Temporal objects for Intl (design INTL-I1 §1.3 item 1; INTL W5,
//! V14-D33): `ep9_intl` reads the internal slots of a Temporal object through these
//! functions, which return std types only (`i32`, `u8`, `i128`, `f64`, `&'static str`,
//! `String`). So `ep9_intl` needs no `temporal_rs`, `timezone_provider` or `zoneinfo64` edge
//! (dep-guards check 19), and the slot layouts stay in this crate (one owner per fact).
//!
//! ```text
//! kind(v)                 the Temporal class of v (8 kinds), by its ObjectClass
//! iso_date(v)             PlainDate, PlainDateTime, PlainYearMonth (reference day),
//!                         PlainMonthDay (reference year): the ISO year, month and day
//! iso_time(v)             PlainTime, PlainDateTime: hour .. nanosecond
//! calendar_id(v)          the calendar identifier of the 5 classes with a [[Calendar]]
//! epoch_ns(v)             Instant, ZonedDateTime: the epoch nanoseconds
//! time_zone_id(v)         ZonedDateTime: the time zone identifier as the slot keeps it
//! duration_fields(v)      Duration: the 10 fields (integral Numbers, slot order)
//! to_temporal_duration(v) ToTemporalDuration (a Duration, a duration String, a
//!                         property bag): the 10 fields of the result
//! ```
//!
//! Each reader returns `None` for a value of another kind; none calls a getter or other
//! JS code (only `to_temporal_duration` does, for a property bag, as the spec says).

use ep9_vm::{Cx, JsResult, Local, ObjectClass};

use crate::{calendar, duration};

/// The 8 Temporal kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Temporal.PlainDate.
    PlainDate,
    /// Temporal.PlainTime.
    PlainTime,
    /// Temporal.PlainDateTime.
    PlainDateTime,
    /// Temporal.PlainYearMonth.
    PlainYearMonth,
    /// Temporal.PlainMonthDay.
    PlainMonthDay,
    /// Temporal.ZonedDateTime.
    ZonedDateTime,
    /// Temporal.Instant.
    Instant,
    /// Temporal.Duration.
    Duration,
}

/// An ISO 8601 calendar date (the ISO fields of a slot, not the calendar fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsoDate {
    /// The ISO year (-271,821 ..= 275,760).
    pub year: i32,
    /// The ISO month (1 ..= 12).
    pub month: u8,
    /// The ISO day (1 ..= 31).
    pub day: u8,
}

/// A wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsoTime {
    /// 0 ..= 23.
    pub hour: u8,
    /// 0 ..= 59.
    pub minute: u8,
    /// 0 ..= 59.
    pub second: u8,
    /// 0 ..= 999.
    pub millisecond: u16,
    /// 0 ..= 999.
    pub microsecond: u16,
    /// 0 ..= 999.
    pub nanosecond: u16,
}

/// Nanoseconds per second.
const NS_PER_SECOND: i128 = 1_000_000_000;

/// The Temporal kind of `v` (`None`: not a Temporal object).
pub fn kind(cx: &Cx<'_>, v: Local<'_>) -> Option<Kind> {
    Some(match cx.object_class(v)? {
        ObjectClass::TemporalPlainDate => Kind::PlainDate,
        ObjectClass::TemporalPlainTime => Kind::PlainTime,
        ObjectClass::TemporalPlainDateTime => Kind::PlainDateTime,
        ObjectClass::TemporalPlainYearMonth => Kind::PlainYearMonth,
        ObjectClass::TemporalPlainMonthDay => Kind::PlainMonthDay,
        ObjectClass::TemporalZonedDateTime => Kind::ZonedDateTime,
        ObjectClass::TemporalInstant => Kind::Instant,
        ObjectClass::TemporalDuration => Kind::Duration,
        _ => return None,
    })
}

/// An int32 slot value as an `i32` (every slot that this module reads this way holds one).
fn int_slot<'s>(cx: &mut Cx<'s>, v: Local<'s>, class: ObjectClass, i: usize) -> JsResult<i32> {
    Ok(cx.slot(v, class, i)?.as_number().unwrap_or(0.0) as i32)
}

/// The ISO date of a PlainDate, PlainDateTime, PlainYearMonth (its reference ISO day) or
/// PlainMonthDay (its reference ISO year): slots 0-2 of each (design §3.2).
pub fn iso_date<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<IsoDate>> {
    let class = match kind(cx, v) {
        Some(
            Kind::PlainDate | Kind::PlainDateTime | Kind::PlainYearMonth | Kind::PlainMonthDay,
        ) => cx.object_class(v).expect("a Temporal class"),
        _ => return Ok(None),
    };
    Ok(Some(IsoDate {
        year: int_slot(cx, v, class, 0)?,
        month: int_slot(cx, v, class, 1)? as u8,
        day: int_slot(cx, v, class, 2)? as u8,
    }))
}

/// The wall-clock time of a PlainTime (slots 0-1) or PlainDateTime (slots 4-5): the second of
/// the day and the nanosecond of that second.
pub fn iso_time<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<IsoTime>> {
    let (class, base) = match kind(cx, v) {
        Some(Kind::PlainTime) => (ObjectClass::TemporalPlainTime, 0),
        Some(Kind::PlainDateTime) => (ObjectClass::TemporalPlainDateTime, 4),
        _ => return Ok(None),
    };
    let s = int_slot(cx, v, class, base)?;
    let n = int_slot(cx, v, class, base + 1)?;
    Ok(Some(IsoTime {
        hour: (s / 3600) as u8,
        minute: (s / 60 % 60) as u8,
        second: (s % 60) as u8,
        millisecond: (n / 1_000_000) as u16,
        microsecond: (n / 1_000 % 1_000) as u16,
        nanosecond: (n % 1_000) as u16,
    }))
}

/// The calendar identifier (`"iso8601"`, `"gregory"`, `"islamic-tbla"` ..) of a PlainDate,
/// PlainDateTime, PlainYearMonth, PlainMonthDay or ZonedDateTime (slot 3 of each).
pub fn calendar_id<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<&'static str>> {
    let class = match kind(cx, v) {
        Some(
            Kind::PlainDate
            | Kind::PlainDateTime
            | Kind::PlainYearMonth
            | Kind::PlainMonthDay
            | Kind::ZonedDateTime,
        ) => cx.object_class(v).expect("a Temporal class"),
        _ => return Ok(None),
    };
    let code = cx.slot(v, class, 3)?.as_number().unwrap_or(0.0);
    Ok(Some(calendar::from_slot(code).identifier()))
}

/// The epoch nanoseconds of an Instant or ZonedDateTime (slots 0-1: the epoch second and the
/// nanosecond of it).
pub fn epoch_ns<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<i128>> {
    let class = match kind(cx, v) {
        Some(Kind::Instant) => ObjectClass::TemporalInstant,
        Some(Kind::ZonedDateTime) => ObjectClass::TemporalZonedDateTime,
        _ => return Ok(None),
    };
    let s = cx.slot(v, class, 0)?.as_number().unwrap_or(0.0);
    let n = cx.slot(v, class, 1)?.as_number().unwrap_or(0.0);
    Ok(Some(s as i128 * NS_PER_SECOND + n as i128))
}

/// The time zone identifier of a ZonedDateTime, as its slot 2 keeps it (an IANA name or an
/// offset "+05:30").
pub fn time_zone_id<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<String>> {
    if kind(cx, v) != Some(Kind::ZonedDateTime) {
        return Ok(None);
    }
    let id = cx.slot(v, ObjectClass::TemporalZonedDateTime, 2)?;
    cx.to_rust_string(id).map(Some)
}

/// The 10 fields of a Duration (years .. nanoseconds; integral Numbers, never -0), read from
/// its slots: no getter is called.
pub fn duration_fields<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<[f64; 10]>> {
    if kind(cx, v) != Some(Kind::Duration) {
        return Ok(None);
    }
    let mut f = [0.0; 10];
    for (i, x) in f.iter_mut().enumerate() {
        *x = cx
            .slot(v, ObjectClass::TemporalDuration, i)?
            .as_number()
            .unwrap_or(0.0);
    }
    Ok(Some(f))
}

/// ToTemporalDuration ( item ) (Temporal §7.5.12): the 10 fields of the Duration of `item`
/// (a Duration: its slots; a String: ParseTemporalDurationString; another Object: a property
/// bag; else a TypeError). A RangeError for a String that is not a duration or an invalid
/// duration.
pub fn to_temporal_duration<'s>(cx: &mut Cx<'s>, item: Local<'s>) -> JsResult<[f64; 10]> {
    if let Some(f) = duration_fields(cx, item)? {
        return Ok(f);
    }
    let d = duration::to_temporal_duration(cx, item)?;
    // Each field is a float64 integer (the temporal_rs feature
    // float64_representable_durations), so the casts are exact.
    Ok([
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
    ])
}
