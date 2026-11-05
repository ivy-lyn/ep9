//! Property bags: PrepareCalendarFields (§12.2.3) and ToTemporalTimeRecord (§4.5.12) for the
//! ISO calendar. The fields are read in alphabetical order; each value is converted right
//! after its Get (both are observable):
//!
//! | Field | Conversion |
//! |---|---|
//! | `day`, `month` | ToPositiveIntegerWithTruncation |
//! | `hour`, `microsecond`, `millisecond`, `minute`, `nanosecond`, `second`, `year` | ToIntegerWithTruncation |
//! | `monthCode` | ToPrimitive(string), a String (TypeError), a well-formed month code (RangeError) |
//! | `offset` | ToOffsetString: ToPrimitive(string), a String (TypeError), ParseDateTimeUTCOffset (RangeError) |
//! | `timeZone` | ToTemporalTimeZoneIdentifier |
//!
//! `era` and `eraYear` are not read: the ISO calendar has no eras (CalendarExtraFields).

use ep9_vm::{Cx, JsObject, JsResult, Local};
use temporal_rs::fields::{CalendarFields, YearMonthCalendarFields};
use temporal_rs::options::Overflow;
use temporal_rs::partial::PartialTime;
use temporal_rs::{MonthCode, TimeZone, UtcOffset};

use crate::convert::{
    temporal_text, to_integer_with_truncation, to_positive_integer_with_truncation,
    to_primitive_string,
};
use crate::error::OrThrow;

/// Field sets (bits) for [`read`].
pub(crate) const YEAR: u16 = 1 << 0;
pub(crate) const MONTH: u16 = 1 << 1;
pub(crate) const MONTH_CODE: u16 = 1 << 2;
pub(crate) const DAY: u16 = 1 << 3;
pub(crate) const TIME: u16 = 1 << 4;
pub(crate) const OFFSET: u16 = 1 << 5;
pub(crate) const TIME_ZONE: u16 = 1 << 6;
/// `timeZone` is in requiredFieldNames: undefined is a TypeError at its Get (with [`TIME_ZONE`]).
pub(crate) const TIME_ZONE_REQUIRED: u16 = 1 << 7;
/// year, month, monthCode, day.
pub(crate) const DATE: u16 = YEAR | MONTH | MONTH_CODE | DAY;
/// year, month, monthCode.
pub(crate) const YEAR_MONTH: u16 = YEAR | MONTH | MONTH_CODE;

/// The fields of a property bag, after conversion. Integral values stay `f64` until the
/// overflow option is known (a negative time field is a RangeError only with `reject`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Bag {
    pub(crate) year: Option<f64>,
    pub(crate) month: Option<f64>,
    pub(crate) month_code: Option<MonthCode>,
    pub(crate) day: Option<f64>,
    pub(crate) hour: Option<f64>,
    pub(crate) minute: Option<f64>,
    pub(crate) second: Option<f64>,
    pub(crate) millisecond: Option<f64>,
    pub(crate) microsecond: Option<f64>,
    pub(crate) nanosecond: Option<f64>,
    pub(crate) offset: Option<UtcOffset>,
    pub(crate) time_zone: Option<TimeZone>,
    /// Whether any field that was read is present (not undefined).
    pub(crate) any: bool,
}

/// One field read: Get, then the conversion when the value is not undefined.
fn get<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, name: &str) -> JsResult<Option<Local<'s>>> {
    let k = cx.key(name);
    let v = cx.get(o, k)?;
    Ok(if v.is_undefined() { None } else { Some(v) })
}

/// PrepareCalendarFields for the ISO calendar over the fields in `want`, in alphabetical
/// order. No required-field check: the caller does it, except for `timeZone` with
/// [`TIME_ZONE_REQUIRED`] (step 9: the TypeError comes before the later fields are read).
pub(crate) fn read<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, want: u16) -> JsResult<Bag> {
    let mut b = Bag::default();
    let int = |cx: &mut Cx<'s>, v| to_integer_with_truncation(cx, v);
    if want & DAY != 0
        && let Some(v) = get(cx, o, "day")?
    {
        b.day = Some(to_positive_integer_with_truncation(cx, v)?);
    }
    if want & TIME != 0 {
        if let Some(v) = get(cx, o, "hour")? {
            b.hour = Some(int(cx, v)?);
        }
        if let Some(v) = get(cx, o, "microsecond")? {
            b.microsecond = Some(int(cx, v)?);
        }
        if let Some(v) = get(cx, o, "millisecond")? {
            b.millisecond = Some(int(cx, v)?);
        }
        if let Some(v) = get(cx, o, "minute")? {
            b.minute = Some(int(cx, v)?);
        }
    }
    if want & MONTH != 0
        && let Some(v) = get(cx, o, "month")?
    {
        b.month = Some(to_positive_integer_with_truncation(cx, v)?);
    }
    if want & MONTH_CODE != 0
        && let Some(v) = get(cx, o, "monthCode")?
    {
        let s = to_primitive_string(cx, v, "monthCode")?;
        b.month_code = Some(MonthCode::try_from_utf8(s.as_bytes()).or_throw(cx)?);
    }
    if want & TIME != 0
        && let Some(v) = get(cx, o, "nanosecond")?
    {
        b.nanosecond = Some(int(cx, v)?);
    }
    if want & OFFSET != 0
        && let Some(v) = get(cx, o, "offset")?
    {
        let s = to_primitive_string(cx, v, "offset")?;
        b.offset = Some(UtcOffset::from_utf8(s.as_bytes()).or_throw(cx)?);
    }
    if want & TIME != 0
        && let Some(v) = get(cx, o, "second")?
    {
        b.second = Some(int(cx, v)?);
    }
    if want & TIME_ZONE != 0 {
        match get(cx, o, "timeZone")? {
            Some(v) => b.time_zone = Some(to_temporal_time_zone_identifier(cx, v)?),
            // 9. A required field that is undefined: TypeError.
            None if want & TIME_ZONE_REQUIRED != 0 => {
                return Err(cx.throw_type_error("a zoned date-time needs a timeZone"));
            }
            None => {}
        }
    }
    if want & YEAR != 0
        && let Some(v) = get(cx, o, "year")?
    {
        b.year = Some(int(cx, v)?);
    }
    b.any = b.year.is_some()
        || b.month.is_some()
        || b.month_code.is_some()
        || b.day.is_some()
        || b.has_time()
        || b.offset.is_some()
        || b.time_zone.is_some();
    Ok(b)
}

/// A clamped `u8` (the field is integral; temporal_rs checks or constrains the range).
fn u8_of(n: f64) -> u8 {
    n.clamp(0.0, 255.0) as u8
}

/// A clamped `u16`.
fn u16_of(n: f64) -> u16 {
    n.clamp(0.0, 65535.0) as u16
}

impl Bag {
    pub(crate) fn has_time(&self) -> bool {
        self.hour.is_some()
            || self.minute.is_some()
            || self.second.is_some()
            || self.millisecond.is_some()
            || self.microsecond.is_some()
            || self.nanosecond.is_some()
    }

    /// The date fields as a temporal_rs record (ISO: no era).
    pub(crate) fn calendar_fields(&self) -> CalendarFields {
        CalendarFields {
            year: self
                .year
                .map(|y| y.clamp(i32::MIN as f64, i32::MAX as f64) as i32),
            month: self.month.map(u8_of),
            month_code: self.month_code,
            day: self.day.map(u8_of),
            era: None,
            era_year: None,
        }
    }

    /// The year-month fields as a temporal_rs record (ISO: no era).
    pub(crate) fn year_month_fields(&self) -> YearMonthCalendarFields {
        let c = self.calendar_fields();
        let mut f = YearMonthCalendarFields::new();
        f.year = c.year;
        f.month = c.month;
        f.month_code = c.month_code;
        f
    }

    /// The time fields as a temporal_rs record. With `reject`, a negative field is a
    /// RangeError here (temporal_rs gets clamped integers, so it would see 0).
    pub(crate) fn partial_time(
        &self,
        cx: &mut Cx<'_>,
        overflow: Overflow,
    ) -> JsResult<PartialTime> {
        if overflow == Overflow::Reject {
            for v in [
                self.hour,
                self.minute,
                self.second,
                self.millisecond,
                self.microsecond,
                self.nanosecond,
            ]
            .into_iter()
            .flatten()
            {
                if v < 0.0 {
                    return Err(cx.throw_range_error("time field out of range"));
                }
            }
        }
        Ok(PartialTime::new()
            .with_hour(self.hour.map(u8_of))
            .with_minute(self.minute.map(u8_of))
            .with_second(self.second.map(u8_of))
            .with_millisecond(self.millisecond.map(u16_of))
            .with_microsecond(self.microsecond.map(u16_of))
            .with_nanosecond(self.nanosecond.map(u16_of)))
    }
}

/// ToTemporalTimeZoneIdentifier (§11.1.5).
pub(crate) fn to_temporal_time_zone_identifier<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<TimeZone> {
    // 1. If temporalTimeZoneLike is an Object and has [[InitializedTemporalZonedDateTime]],
    //    return temporalTimeZoneLike.[[TimeZone]].
    if let Some(tz) = crate::zoned_date_time::time_zone_of_object(cx, v)? {
        return Ok(tz);
    }
    // 2. If temporalTimeZoneLike is not a String, throw a TypeError exception.
    let Some(s) = v.as_string() else {
        return Err(cx.throw_type_error("time zone must be a string"));
    };
    // 3-9. ParseTemporalTimeZoneString, then the offset or the available named zone.
    let t = temporal_text(cx, s)?;
    TimeZone::try_from_str_with_provider(&t, crate::tzdb::provider()).or_throw(cx)
}

/// IsPartialTemporalObject (§13.42): the object, or `None` when it is not one (not an
/// object, a Temporal date or time object, or it has a `calendar` or `timeZone` property).
pub(crate) fn is_partial_temporal_object<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<Option<Local<'s, JsObject>>> {
    // 1. If value is not an Object, return false.
    let Some(o) = v.as_object() else {
        return Ok(None);
    };
    // 2. If value has an [[InitializedTemporalDate]], [[InitializedTemporalDateTime]],
    //    [[InitializedTemporalMonthDay]], [[InitializedTemporalTime]],
    //    [[InitializedTemporalYearMonth]] or [[InitializedTemporalZonedDateTime]] internal
    //    slot, return false.
    if crate::calendar::has_calendar_slot(cx, v)
        || cx.object_class(v) == Some(ep9_vm::ObjectClass::TemporalPlainTime)
    {
        return Ok(None);
    }
    // 3-6. If Get(value, "calendar") or Get(value, "timeZone") is not undefined, return false.
    for name in ["calendar", "timeZone"] {
        let k = cx.key(name);
        if !cx.get(o, k)?.is_undefined() {
            return Ok(None);
        }
    }
    // 7. Return true.
    Ok(Some(o))
}
