//! Intl.DateTimeFormat.prototype.formatRange and formatRangeToParts (ECMA-402 §11.3.5,
//! §11.3.6; ids 45.5 and 45.6; INTL W5 DTF-RANGE): PartitionDateTimeRangePattern on the
//! pieces of the W4 format path, in the ICU4C form that V8 and test262 follow.
//!
//! ```text
//! formatRange(x, y) ─ RequireInternalSlot; undefined -> TypeError (before ToNumber)
//!   ─ ToDateTimeFormattable x, y ─ Temporal: the same kind (else TypeError), the per-kind
//!     format and the ISO fields (HandleDateTimeValue); a number: TimeClip (NaN -> RangeError)
//!   ─ px, py: the pieces of format(x), format(y) (`format_pieces_at`, `pattern::adjust`)
//!   ─ no local field value differs down to the smallest field of the format ("practically
//!     equal"; the values, not the texts: `differences`) ─> px, all "shared"
//!   ─ date and time fields, the date the same ─> the date of px, its joiner, the range of
//!     the time pieces (ICU4C: the date plus the time range)
//!   ─ a time-only format on the same date (or the time of the same date): the collapse
//!     (`collapse`; a field that it writes once must be the same at both ends): the
//!     ICU4X range output of the time field set (DateRangeFormatter<TimeFieldSet>), read
//!     through writeable::PartsWrite: the fields of its RANGE_START part are a prefix of
//!     the time fields of px, the fields of its RANGE_END part a suffix of those of py, and
//!     the text between is the separator ("1:00 – 3:30 PM"); a template that does not fit
//!     the pieces (a field that the W4 path drops or adds, fractional seconds) ─> the fallback
//!   ─ else the fallback: px, the range glue of the locale (DatetimePatternsRangeGlueV1), py
//!     ("Jan 3, 2019 – Jan 5, 2019"; no date collapse: `range_formatter_for`)
//!   ─ sources: the ICU4C rule (FormattedValueFieldPositionIteratorImpl::addOverlapSpans):
//!     the field types that occur twice; from the first to the last of their first
//!     occurrences "startRange", of their second occurrences "endRange", the rest "shared"
//! ```
//!
//! No ICU4X object is in a slot: the range formatters and the glue live in `crate::cache`
//! (INTL-I1-D7). No float math (H36): a time value goes through `time_clip` only.

use std::fmt;
use std::rc::Rc;

use ep9_temporal::intl_view::{self, Kind};
use ep9_vm::{Args, Cx, JsResult, Local};
use icu_calendar::Iso;
use icu_datetime::DateTimeFormatterPreferences;
use icu_datetime::fieldsets::enums::{CompositeFieldSet, TimeFieldSet};
use icu_datetime::parts::{RANGE_END, RANGE_START};
use icu_datetime::range::DateRangeFormatter;
use icu_locale_core::Locale;
use icu_time::zone::models::AtTime;
use icu_time::{TimeZoneInfo, ZonedDateTime};
use writeable::{Part, PartsWrite, Writeable};

use super::pattern::{self, Piece};
use super::{
    Resolved, StyleCut, c, field_set, format_pieces_at, part_type, plain_input, prefs_tag, read,
    temporal_format, time_clip, zoned,
};
use crate::cache::{self, Service};
use crate::number_format::range::Source;
use crate::tag;

/// The ICU4X input of one end of the range.
type Input = ZonedDateTime<Iso, TimeZoneInfo<AtTime>>;

/// The cache bit of a range formatter (the `formatter_for` keys are below 2^36).
const RANGE_BIT: u64 = 1 << 40;

/// The cache options of the range glue of a locale (no formatter key has this value; the
/// decimal sign of `pattern::adjust` uses `u64::MAX`).
const GLUE_KEY: u64 = u64::MAX - 1;

/// Where a piece of the range comes from: the format of x, of y, or the range separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    X,
    Y,
    Separator,
}

/// A part of the range result: the Table 18 type, the value and the source.
type RangePart = (&'static str, String, Source);

// ---- the builtins ---------------------------------------------------------------------------

/// Intl.DateTimeFormat.prototype.formatRange ( startDate, endDate ) (11.3.5).
pub(super) fn format_range<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Return ? FormatDateTimeRange(dtf, startDate, endDate): the string of the parts.
    let parts = range_parts(cx, this, args)?;
    let s: String = parts.iter().map(|(_, v, _)| v.as_str()).collect();
    Ok(cx.new_string(&s)?.into())
}

/// Intl.DateTimeFormat.prototype.formatRangeToParts ( startDate, endDate ) (11.3.6).
pub(super) fn format_range_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Return ? FormatDateTimeRangeToParts(dtf, startDate, endDate) (11.5.11): an array of
    //    { type, value, source } in order.
    let parts = range_parts(cx, this, args)?;
    let array = cx.new_array();
    for (i, (t, v, s)) in parts.iter().enumerate() {
        cx.poll_interrupt()?;
        let o = cx.new_object();
        let t = cx.new_string(t)?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string(v)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        let s = cx.new_string(s.name())?;
        cx.create_data_property_or_throw(o, "source", s)?;
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Steps 1-3 of formatRange and formatRangeToParts, then PartitionDateTimeRangePattern
/// (11.5.9, ECMA-402 with Temporal) of the two arguments: the parts with their sources.
fn range_parts<'s>(cx: &mut Cx<'s>, this: Local<'s>, args: Args<'s>) -> JsResult<Vec<RangePart>> {
    // 1-2. Let dtf be the this value; ? RequireInternalSlot(dtf, [[InitializedDateTimeFormat]]).
    let r = read(cx, this)?;
    // 3. If startDate is undefined or endDate is undefined, throw a TypeError exception.
    let (start, end) = (args.get(0), args.get(1));
    if start.is_undefined() || end.is_undefined() {
        return Err(cx.throw_type_error(
            "Intl.DateTimeFormat: formatRange needs a start date and an end date",
        ));
    }
    // FormatDateTimeRange 1-2. Let x be ? ToDateTimeFormattable(startDate); let y be
    // ? ToDateTimeFormattable(endDate): a Temporal object other than a Duration stays, else
    // ? ToNumber (x first).
    let kx = temporal_kind(cx, start);
    let nx = match kx {
        Some(_) => 0.0,
        None => cx.to_number(start)?,
    };
    let ky = temporal_kind(cx, end);
    let ny = match ky {
        Some(_) => 0.0,
        None => cx.to_number(end)?,
    };
    // PartitionDateTimeRangePattern 1. If IsTemporalObject(x) or IsTemporalObject(y), then
    //   a. If SameTemporalType(x, y) is false, throw a TypeError exception.
    // 2-3. HandleDateTimeValue(dtf, x), then (dtf, y): the format and the ICU4X inputs.
    let (t, ix, iy) = match (kx, ky) {
        (None, None) => {
            // HandleDateTimeOthers: TimeClip; NaN is a RangeError (x first).
            let (Some(a), Some(b)) = (time_clip(nx), time_clip(ny)) else {
                return Err(cx.throw_range_error("Invalid time value"));
            };
            let (ix, iy) = (zoned(&r.time_zone, a), zoned(&r.time_zone, b));
            (r, ix, iy)
        }
        (Some(a), Some(b)) if a == b => {
            let (t, ix) = temporal_input(cx, &r, a, start)?;
            let (_, iy) = temporal_input(cx, &r, b, end)?;
            (t, ix, iy)
        }
        _ => {
            return Err(cx.throw_type_error(
                "Intl.DateTimeFormat: the start and end of a range must be of the same type",
            ));
        }
    };
    match range_pieces(&t, &ix, &iy) {
        Some((pieces, single)) => Ok(to_parts(&pieces, single)),
        None => Err(cx.throw_type_error("Intl.DateTimeFormat: no date-time data")),
    }
}

/// The Temporal kind of `v` if ToDateTimeFormattable keeps it (a Temporal object other than
/// a Duration).
fn temporal_kind(cx: &Cx<'_>, v: Local<'_>) -> Option<Kind> {
    intl_view::kind(cx, v).filter(|k| *k != Kind::Duration)
}

/// HandleDateTimeValue ( dateTimeFormat, x ) for the Temporal value `v` of the kind `kind`
/// (ECMA-402 with Temporal; the checks of `format_temporal`, in its order): a TypeError for a
/// ZonedDateTime, a RangeError for a calendar that is not the calendar of `r` (or iso8601
/// for PlainDate and PlainDateTime), a TypeError for a null format. The format of the kind
/// and the ICU4X input.
fn temporal_input<'s>(
    cx: &mut Cx<'s>,
    r: &Resolved,
    kind: Kind,
    v: Local<'s>,
) -> JsResult<(Resolved, Input)> {
    if kind == Kind::ZonedDateTime {
        return Err(cx.throw_type_error(
            "Intl.DateTimeFormat cannot format a Temporal.ZonedDateTime: use its toLocaleString",
        ));
    }
    if let Some(cal) = intl_view::calendar_id(cx, v)? {
        let iso_ok = matches!(kind, Kind::PlainDate | Kind::PlainDateTime);
        if cal != r.calendar && !(iso_ok && cal == "iso8601") {
            return Err(cx.throw_range_error(&format!(
                "the calendar {cal} of the Temporal value is not the calendar {} of the format",
                r.calendar
            )));
        }
    }
    let Some(t) = temporal_format(r, kind, false) else {
        return Err(cx.throw_type_error(
            "Intl.DateTimeFormat: the options have no field of this Temporal type",
        ));
    };
    let input = match intl_view::epoch_ns(cx, v)? {
        // An Instant: the epoch milliseconds (the floor) in the zone of the format.
        Some(ns) => zoned(&t.time_zone, ns.div_euclid(1_000_000) as i64),
        None => plain_input(cx, v)?,
    };
    Ok((t, input))
}

// ---- the range of two formatted values ------------------------------------------------------

/// Whether the UTS 35 field letter `ch` is a time field (a time zone name too: ICU4C puts it
/// in the time skeleton).
fn is_time_letter(ch: char) -> bool {
    matches!(
        ch,
        'h' | 'H'
            | 'K'
            | 'k'
            | 'm'
            | 's'
            | 'S'
            | 'A'
            | 'a'
            | 'b'
            | 'B'
            | 'z'
            | 'Z'
            | 'O'
            | 'v'
            | 'V'
            | 'X'
            | 'x'
    )
}

/// The part type of a piece ("literal" for a literal).
fn piece_type(p: &Piece) -> &'static str {
    p.field.map_or("literal", |(ch, _)| part_type(ch))
}

/// The index and part type of each field piece of `ps`.
fn fields(ps: &[Piece]) -> Vec<(usize, &'static str)> {
    ps.iter()
        .enumerate()
        .filter(|(_, p)| p.field.is_some())
        .map(|(i, p)| (i, piece_type(p)))
        .collect()
}

/// The Table 7 rank (ECMA-402 range pattern fields: era, year, month, day, AmPm and
/// dayPeriod, hour, minute, second, fractional second) of the UTS 35 field letter `ch`; the
/// weekday goes with the day, a year name with the year. `None`: not a Table 7 field (a time
/// zone name).
fn rank(ch: char) -> Option<u8> {
    Some(match ch {
        'G' => 0,
        'y' | 'Y' | 'u' | 'U' | 'r' => 1,
        'M' | 'L' => 2,
        'd' | 'E' | 'c' | 'e' => 3,
        'a' | 'b' | 'B' => 4,
        'h' | 'H' | 'K' | 'k' => 5,
        'm' => 6,
        's' => 7,
        'S' => 8,
        _ => return None,
    })
}

/// The local field values of an end of the range (ToLocalTime in the calendar of the
/// format), in Table 7 order: era, year, month, day, (day period: the text), hour, minute,
/// second, fractional second.
fn local_fields(t: &Resolved, input: &Input) -> [i64; 8] {
    use icu_calendar::{AnyCalendar, Ref};
    // The era code and the year, month and day of the calendar of the format (the ISO
    // values for a calendar that ICU4X does not have: not the case for a resolved calendar).
    let (era, year, month, day) = match pattern::calendar_kind(&t.calendar) {
        Some(kind) => {
            let calendar = AnyCalendar::new(kind);
            let d = input.date.to_calendar(Ref(&calendar));
            let era = d.year().era().map_or(0, |e| era_number(e.era.as_str()));
            (
                era,
                d.year().extended_year(),
                d.month().ordinal,
                d.day_of_month().0,
            )
        }
        None => (
            0,
            input.date.year().extended_year(),
            input.date.month().ordinal,
            input.date.day_of_month().0,
        ),
    };
    // FractionalSecondDigits: floor(ms × 10^(digits - 3)) (step 11.d.iv; 3 if none).
    let digits = u32::from(t.components.0[c::FRACTIONAL_SECOND_DIGITS]);
    let digits = if digits == 0 { 3 } else { digits.min(3) };
    let ms = input.time.subsecond.number() / 1_000_000;
    let fraction = ms / 10u32.pow(3 - digits);
    [
        era,
        i64::from(year),
        i64::from(month),
        i64::from(day),
        i64::from(input.time.hour.number()),
        i64::from(input.time.minute.number()),
        i64::from(input.time.second.number()),
        i64::from(fraction),
    ]
}

/// A number for the era code `code` (its bytes; the codes are short ASCII), so that two
/// codes compare as two numbers.
fn era_number(code: &str) -> i64 {
    code.bytes()
        .take(7)
        .fold(1i64, |n, b| n << 8 | i64::from(b))
}

/// Which fields differ between the ends `ix` and `iy` of the range of `t` (with the formats
/// `px` and `py`): (a date field, a time field). relevantFieldsEqual (PartitionDateTimeRange
/// Pattern step 11): the local field values of each Table 7 field, in table order, down to
/// the smallest field of the format (a smaller field has no range pattern: checkMoreFields
/// is false); a field of the format that is not shown counts (the year of {month, day}: no
/// two values of different years are equal); the day period is its text (step 11.d.iii: a
/// string of the locale), the fractional second floor(ms × 10^(digits - 3)). Not the shown
/// texts: the years 1920 and 2020 are both "20" with year "2-digit". Formats whose field
/// types do not line up: both.
fn differences(t: &Resolved, ix: &Input, iy: &Input, px: &[Piece], py: &[Piece]) -> (bool, bool) {
    let compared = |ps: &[Piece]| -> Vec<&'static str> {
        ps.iter()
            .filter_map(|p| {
                let (ch, _) = p.field?;
                rank(ch).map(|_| part_type(ch))
            })
            .collect()
    };
    if compared(px) != compared(py) {
        return (true, true);
    }
    let Some(smallest) = px.iter().filter_map(|p| rank(p.field?.0)).max() else {
        return (false, false);
    };
    let (vx, vy) = (local_fields(t, ix), local_fields(t, iy));
    // Table 7 rank of each value of `local_fields`.
    const RANKS: [u8; 8] = [0, 1, 2, 3, 5, 6, 7, 8];
    let mut out = (false, false);
    for ((a, b), r) in vx.iter().zip(&vy).zip(RANKS) {
        if r <= smallest && a != b {
            if r <= 3 {
                out.0 = true;
            } else {
                out.1 = true;
            }
        }
    }
    // The day period (AmPm and DayPeriod): the text of the field.
    let period = |ps: &[Piece]| {
        ps.iter()
            .find(|p| p.field.is_some_and(|(ch, _)| rank(ch) == Some(4)))
            .map(|p| p.text.clone())
    };
    if smallest >= 4 && period(px) != period(py) {
        out.1 = true;
    }
    out
}

/// The range pieces of `t` from `ix` to `iy` (PartitionDateTimeRangePattern after
/// HandleDateTimeValue), each with its origin; `true` if x and y are practically equal (all
/// parts "shared"). `None` if ICU4X has no data (not the case with the compiled data).
fn range_pieces(t: &Resolved, ix: &Input, iy: &Input) -> Option<(Vec<(Piece, Origin)>, bool)> {
    let px = format_pieces_at(t, ix)?;
    let py = format_pieces_at(t, iy)?;
    let (date_differs, time_differs) = differences(t, ix, iy, &px, &py);
    // 14. relevantFieldsEqual: the format of x, every part "shared".
    if !date_differs && !time_differs {
        return Some((px.into_iter().map(|p| (p, Origin::X)).collect(), true));
    }
    let letters = |ps: &[Piece]| -> (bool, bool) {
        let mut out = (false, false);
        for (ch, _) in ps.iter().filter_map(|p| p.field) {
            if is_time_letter(ch) {
                out.1 = true;
            } else {
                out.0 = true;
            }
        }
        out
    };
    let (has_date, has_time) = letters(&px);
    let out = match (has_date, has_time) {
        // The same date, another time: the date and the time range (ICU4C fallbackFormat
        // with fromToOnSameDay: the date-time glue of the date and the time range).
        (true, true) if !date_differs => match (split_date_time(&px), split_date_time(&py)) {
            (Some(sx), Some(sy)) if sx.time_first == sy.time_first => {
                let range = time_range(&time_part(t), ix, iy, sx.time, sy.time);
                let date = sx.date.iter().map(|p| (p.clone(), Origin::X));
                let joiner = sx.joiner.iter().map(|p| (p.clone(), Origin::X));
                if sx.time_first {
                    range.into_iter().chain(joiner).chain(date).collect()
                } else {
                    date.chain(joiner).chain(range).collect()
                }
            }
            _ => fallback(t, &px, &py),
        },
        // A time format with the same date (no date shown: the date of the ends counts;
        // another date is the [[Default]] range pattern, the fallback).
        (false, true) if !date_differs => time_range(t, ix, iy, &px, &py),
        // A date range: the fallback (no date collapse, `range_formatter_for`).
        _ => fallback(t, &px, &py),
    };
    Some((out, false))
}

/// The range of the time pieces `tx` and `ty` (the formats of `ix` and `iy` by the time-only
/// format `t`): the collapse, with a time zone name at the end kept once after the range
/// when it is the same ("1:00 – 3:30 PM UTC"; the ICU4C interval pattern "h:mm – h:mm a v");
/// else the fallback.
fn time_range(
    t: &Resolved,
    ix: &Input,
    iy: &Input,
    tx: &[Piece],
    ty: &[Piece],
) -> Vec<(Piece, Origin)> {
    if let (Some((core_x, zone_x)), Some((core_y, zone_y))) = (split_zone(tx), split_zone(ty))
        && zone_x == zone_y
        && let Some(mut out) = collapse(&without_zone(t), ix, iy, core_x, core_y)
    {
        out.extend(zone_x.iter().map(|p| (p.clone(), Origin::X)));
        return out;
    }
    collapse(t, ix, iy, tx, ty).unwrap_or_else(|| fallback(t, tx, ty))
}

/// `ps` cut after its last field before a time zone name that is its last field: (the
/// pieces before, the literal and the zone name). `None` if the last field is not a zone.
fn split_zone(ps: &[Piece]) -> Option<(&[Piece], &[Piece])> {
    let f = fields(ps);
    let [.., (before, _), (_, "timeZoneName")] = f.as_slice() else {
        return None;
    };
    Some(ps.split_at(before + 1))
}

/// The time-only format `t` without its time zone name.
fn without_zone(t: &Resolved) -> Resolved {
    let mut s = t.clone();
    s.components.0[c::TIME_ZONE_NAME] = 0;
    if s.styles.time_style.is_some() && s.cut == StyleCut::None {
        s.cut = StyleCut::Time;
    }
    s
}

/// A format cut into its date fields, the literal between, and its time fields.
struct DateTimeSplit<'a> {
    /// The date fields with the literals before them (or after them, if the time is first).
    date: &'a [Piece],
    /// The literals between the date and the time.
    joiner: &'a [Piece],
    /// The time fields with the literals after them (or before them, if the time is first).
    time: &'a [Piece],
    /// Whether the time comes first.
    time_first: bool,
}

/// `ps` cut into its date and its time (`None` if the date and time fields are mixed).
fn split_date_time(ps: &[Piece]) -> Option<DateTimeSplit<'_>> {
    let kinds: Vec<(usize, bool)> = ps
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.field.map(|(ch, _)| (i, is_time_letter(ch))))
        .collect();
    let mut changes = kinds.windows(2).filter(|w| w[0].1 != w[1].1);
    let w = changes.next()?;
    if changes.next().is_some() {
        return None;
    }
    let (a, b) = (w[0].0, w[1].0);
    let time_first = w[0].1;
    let (first, joiner, second) = (&ps[..=a], &ps[a + 1..b], &ps[b..]);
    let (date, time) = if time_first {
        (second, first)
    } else {
        (first, second)
    };
    Some(DateTimeSplit {
        date,
        joiner,
        time,
        time_first,
    })
}

/// The time-only format of the date-time format `t` (the field set of the time range).
fn time_part(t: &Resolved) -> Resolved {
    let mut s = t.clone();
    if s.styles.date_style.is_some() || s.styles.time_style.is_some() {
        s.styles.date_style = None;
        if s.cut == StyleCut::DateTime {
            s.cut = StyleCut::Time;
        }
    } else {
        for i in [c::WEEKDAY, c::ERA, c::YEAR, c::MONTH, c::DAY] {
            s.components.0[i] = 0;
        }
    }
    s
}

/// The fallback range: `px`, the range glue of the locale of `t` ("{0} – {1}"), `py`.
fn fallback(t: &Resolved, px: &[Piece], py: &[Piece]) -> Vec<(Piece, Origin)> {
    let glue = glue_of(t);
    let literal = |s: &str| {
        (
            Piece {
                field: None,
                text: s.to_string(),
            },
            Origin::Separator,
        )
    };
    let x = px.iter().map(|p| (p.clone(), Origin::X));
    let y = py.iter().map(|p| (p.clone(), Origin::Y));
    let mut out = vec![literal(&glue.before)];
    if glue.reversed {
        out.extend(y);
        out.push(literal(&glue.between));
        out.extend(x);
    } else {
        out.extend(x);
        out.push(literal(&glue.between));
        out.extend(y);
    }
    out.push(literal(&glue.after));
    out
}

/// The range glue of a locale (the CLDR intervalFormatFallback): the text before, between
/// and after the two dates, and whether the end date comes first.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Glue {
    before: String,
    between: String,
    after: String,
    reversed: bool,
}

/// The cached range glue of the data locale of `t` (ICU4X `DatetimePatternsRangeGlueV1`;
/// " – " if the data has none).
fn glue_of(t: &Resolved) -> Glue {
    let data_locale = tag::remove_unicode_extension(&t.locale);
    let glue = cache::get_or_make(Service::DateTimeFormat, &data_locale, GLUE_KEY, || {
        load_glue(&data_locale)
    });
    glue.map_or_else(
        || Glue {
            before: String::new(),
            between: " \u{2013} ".to_string(),
            after: String::new(),
            reversed: false,
        },
        |g| g.as_ref().clone(),
    )
}

/// The range glue of `data_locale` from the ICU4X data (the baked provider falls back to the
/// parent locales).
fn load_glue(data_locale: &str) -> Option<Glue> {
    use icu_datetime::provider::Baked;
    use icu_datetime::provider::pattern::GenericPatternItem;
    use icu_datetime::provider::range_patterns::DatetimePatternsRangeGlueV1;
    use icu_provider::prelude::*;
    let locale = DataLocale::from(&tag::icu_locale(data_locale));
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_locale(&locale),
        ..Default::default()
    };
    let data = DataProvider::<DatetimePatternsRangeGlueV1>::load(&Baked, req).ok()?;
    let mut texts = [String::new(), String::new(), String::new()];
    let mut placeholders = Vec::new();
    for item in data.payload.get().pattern.items.iter() {
        match item {
            GenericPatternItem::Placeholder(n) => placeholders.push(n),
            GenericPatternItem::Literal(ch) => texts[placeholders.len().min(2)].push(ch),
        }
    }
    let [before, between, after] = texts;
    let reversed = match placeholders.as_slice() {
        [0, 1] => false,
        [1, 0] => true,
        _ => return None,
    };
    Some(Glue {
        before: before.replace('\u{202f}', " "),
        between: between.replace('\u{202f}', " "),
        after: after.replace('\u{202f}', " "),
        reversed,
    })
}

/// The collapsed range of `px` and `py` (the formats of `ix` and `iy` by `t`): the ICU4X
/// range output of the same field set as a template, with its literals and with the field
/// texts of the W4 path. The fields of its RANGE_START part take the first fields of `px`,
/// the fields of its RANGE_END part the last fields of `py` (of the same types, together all
/// fields); its literals stay (the interval pattern has its own: ja "H時mm分～H時mm分" for the
/// format "H:mm"). `None` when the template does not fit the pieces (then the caller uses
/// the fallback): no range formatter for the field set, a field that the W4 path drops or
/// adds, fractional seconds (CLDR has no interval pattern with them), or a single format.
fn collapse(
    t: &Resolved,
    ix: &Input,
    iy: &Input,
    px: &[Piece],
    py: &[Piece],
) -> Option<Vec<(Piece, Origin)>> {
    if px
        .iter()
        .chain(py)
        .any(|p| p.field.is_some_and(|(ch, _)| ch == 'S'))
    {
        return None;
    }
    let (fx, fy) = (fields(px), fields(py));
    let n = fx.len();
    if fy.len() != n || fx.iter().zip(&fy).any(|(a, b)| a.1 != b.1) {
        return None;
    }
    let items = template(t, ix, iy)?;
    let side_types = |side: Source| -> Vec<&'static str> {
        items
            .iter()
            .filter(|i| i.side == side)
            .filter_map(|i| i.field)
            .collect()
    };
    let (start, end) = (side_types(Source::StartRange), side_types(Source::EndRange));
    let (n1, n2) = (start.len(), end.len());
    if n1 == 0 || n2 == 0 || n1 > n || n2 > n || n1 + n2 < n {
        return None;
    }
    let types = |f: &[(usize, &'static str)]| f.iter().map(|(_, t)| *t).collect::<Vec<_>>();
    if types(&fx[..n1]) != start || types(&fy[n - n2..]) != end {
        return None;
    }
    // A field that the template writes once (a field of x after its first n1, a field of y
    // before its last n2) stands for both ends: it must be the same in both after
    // `pattern::adjust` (ICU4X picks the template by the AM/PM field; "in the afternoon" and
    // "in the evening" are both PM).
    if (0..n).any(|i| (i >= n1 || i < n - n2) && px[fx[i].0].text != py[fy[i].0].text) {
        return None;
    }
    let (mut xs, mut ys) = (fx[..n1].iter(), fy[n - n2..].iter());
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let piece = match (item.field, item.side) {
            (Some(_), Source::StartRange) => (px[xs.next()?.0].clone(), Origin::X),
            (Some(_), Source::EndRange) => (py[ys.next()?.0].clone(), Origin::Y),
            (Some(_), Source::Shared) => return None,
            (None, side) => (
                Piece {
                    field: None,
                    text: item.text.replace('\u{202f}', " "),
                },
                match side {
                    Source::StartRange => Origin::X,
                    Source::EndRange => Origin::Y,
                    Source::Shared => Origin::Separator,
                },
            ),
        };
        out.push(piece);
    }
    Some(out)
}

/// The cached ICU4X range formatter of the time-only format `t` (`None`: a field set with
/// a date or a zone: no template).
///
/// Only the time field sets: the icu_datetime 2.4 DateRangeFormatter of a date or period
/// field set writes range patterns whose text fields have widths that the formatter did not
/// load (`NamesNotLoaded`: a `debug_assert!` panic in a debug build), in many locales and in
/// every length (the W5 DTF-RANGE probe in a debug build: 20,444 of 157,680 date ranges of
/// 73 locales and 12 calendars, 1,274 of the 13,140 gregorian ones; "MMM" in the long
/// "d. MMMM y" of de). The time field sets had none (0 of 886,950 time ranges: all CLDR
/// locales, with -u-hc, -u-nu and -u-ca keys, 3 lengths, 5 precisions). A date range is the
/// fallback.
fn range_formatter_for(t: &Resolved) -> Option<Rc<DateRangeFormatter<TimeFieldSet>>> {
    let CompositeFieldSet::Time(fs) = field_set(t) else {
        return None;
    };
    let key = prefs_tag(t);
    let bits = u64::from(t.components.pack())
        | u64::from(t.styles.pack()) << 24
        | (t.cut as u64) << 33
        | RANGE_BIT;
    cache::get_or_make(Service::DateTimeFormat, &key, bits, || {
        let make = |tag: &str| {
            let locale = Locale::try_from_str(tag).unwrap_or(Locale::UNKNOWN);
            DateRangeFormatter::try_new(DateTimeFormatterPreferences::from(&locale), fs).ok()
        };
        // A numbering system with no ICU4X digits: the locale's own (as `formatter_for`).
        make(&key).or_else(|| {
            let data_locale = tag::remove_unicode_extension(&t.locale);
            make(&format!("{data_locale}-u-ca-{}", t.calendar))
        })
    })
}

/// The text runs of the ICU4X range output of `t` from `ix` to `iy` (`None`: no range
/// formatter, a field outside both halves, or a start field after an end field).
fn template(t: &Resolved, ix: &Input, iy: &Input) -> Option<Vec<Item>> {
    let f = range_formatter_for(t)?;
    let mut sink = RangeSink::default();
    f.format(ix, iy).write_to_parts(&mut sink).ok()?;
    let mut seen_end = false;
    for item in &sink.items {
        match (item.field, item.side) {
            (Some(_), Source::Shared) => return None,
            (Some(_), Source::StartRange) if seen_end => return None,
            (Some(_), Source::EndRange) => seen_end = true,
            _ => {}
        }
    }
    Some(sink.items)
}

/// A text run of the ICU4X range output: its half, its field (the part value; `None` for a
/// literal) and its text.
#[derive(Debug)]
struct Item {
    side: Source,
    field: Option<&'static str>,
    text: String,
}

/// A `writeable::PartsWrite` sink that keeps the half (RANGE_START, RANGE_END) and the
/// outermost datetime part of each text run; a new field part starts a new item.
#[derive(Default)]
struct RangeSink {
    side: Option<Source>,
    field: Option<&'static str>,
    new_field: bool,
    items: Vec<Item>,
}

impl fmt::Write for RangeSink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let side = self.side.unwrap_or(Source::Shared);
        match self.items.last_mut() {
            Some(last) if !self.new_field && last.side == side && last.field == self.field => {
                last.text.push_str(s);
            }
            _ => self.items.push(Item {
                side,
                field: self.field,
                text: s.to_string(),
            }),
        }
        self.new_field = false;
        Ok(())
    }
}

impl PartsWrite for RangeSink {
    type SubPartsWrite = RangeSink;

    fn with_part(
        &mut self,
        part: Part,
        mut f: impl FnMut(&mut RangeSink) -> fmt::Result,
    ) -> fmt::Result {
        if part == RANGE_START || part == RANGE_END {
            let old = self.side;
            self.side = Some(if part == RANGE_START {
                Source::StartRange
            } else {
                Source::EndRange
            });
            let r = f(self);
            self.side = old;
            return r;
        }
        if self.field.is_none() && part.category == "datetime" {
            self.field = Some(part.value);
            self.new_field = true;
            let r = f(self);
            self.field = None;
            self.new_field = false;
            return r;
        }
        // A part inside a field (the digits of a number).
        f(self)
    }
}

// ---- the parts ------------------------------------------------------------------------------

/// The sources of `pieces` by the ICU4C rule (module doc): the field types that occur twice
/// give two spans; the span of their first occurrences takes the source of the value it
/// comes from (x: "startRange"), the other span the other source, the rest is "shared".
fn sources(pieces: &[(Piece, Origin)]) -> Vec<Source> {
    let types: Vec<Option<&'static str>> = pieces
        .iter()
        .map(|(p, _)| p.field.map(|(ch, _)| part_type(ch)))
        .collect();
    let (mut s1a, mut s1b, mut s2a, mut s2b) = (usize::MAX, 0, usize::MAX, 0);
    for (i, ti) in types.iter().enumerate() {
        let Some(ti) = ti else {
            continue;
        };
        if let Some(j) = (i + 1..types.len()).find(|&j| types[j] == Some(ti)) {
            s1a = s1a.min(i);
            s1b = s1b.max(i);
            s2a = s2a.min(j);
            s2b = s2b.max(j);
        }
    }
    if s1a == usize::MAX {
        return vec![Source::Shared; pieces.len()];
    }
    let (first, second) = if pieces[s1a].1 == Origin::Y {
        (Source::EndRange, Source::StartRange)
    } else {
        (Source::StartRange, Source::EndRange)
    };
    (0..pieces.len())
        .map(|i| {
            if (s1a..=s1b).contains(&i) {
                first
            } else if (s2a..=s2b).contains(&i) {
                second
            } else {
                Source::Shared
            }
        })
        .collect()
}

/// The parts of the range pieces `pieces` (`single`: all "shared"): a field's type by its
/// letter; literals next to each other with one source are one part; empty texts go.
fn to_parts(pieces: &[(Piece, Origin)], single: bool) -> Vec<RangePart> {
    let srcs = if single {
        vec![Source::Shared; pieces.len()]
    } else {
        sources(pieces)
    };
    let mut out: Vec<RangePart> = Vec::new();
    for ((p, _), s) in pieces.iter().zip(srcs) {
        if p.text.is_empty() {
            continue;
        }
        let t = piece_type(p);
        if t == "literal"
            && let Some(last) = out.last_mut()
            && last.0 == "literal"
            && last.2 == s
        {
            last.1.push_str(&p.text);
            continue;
        }
        out.push((t, p.text.clone(), s));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(field: Option<(char, usize)>, text: &str) -> Piece {
        Piece {
            field,
            text: text.to_string(),
        }
    }

    #[test]
    fn sources_follow_the_icu4c_overlap_spans() {
        // "Jan 3 – 5, 2019": only the day occurs twice.
        let ps = vec![
            (piece(Some(('M', 3)), "Jan"), Origin::X),
            (piece(None, " "), Origin::X),
            (piece(Some(('d', 1)), "3"), Origin::X),
            (piece(None, " – "), Origin::Separator),
            (piece(Some(('d', 1)), "5"), Origin::Y),
            (piece(None, ", "), Origin::Y),
            (piece(Some(('y', 1)), "2019"), Origin::Y),
        ];
        let got: Vec<String> = to_parts(&ps, false)
            .iter()
            .map(|(t, v, s)| format!("{t}:{v}:{}", s.name()))
            .collect();
        assert_eq!(
            got,
            [
                "month:Jan:shared",
                "literal: :shared",
                "day:3:startRange",
                "literal: – :shared",
                "day:5:endRange",
                "literal:, :shared",
                "year:2019:shared"
            ]
        );
        // No field twice: all shared.
        assert!(sources(&ps[..3]).iter().all(|s| *s == Source::Shared));
    }

    #[test]
    fn a_date_time_format_splits_at_the_joiner() {
        let ps = vec![
            piece(Some(('M', 1)), "8"),
            piece(None, "/"),
            piece(Some(('d', 1)), "4"),
            piece(None, ", "),
            piece(Some(('h', 1)), "12"),
            piece(None, ":"),
            piece(Some(('m', 2)), "30"),
            piece(None, " "),
            piece(Some(('a', 1)), "AM"),
        ];
        let s = split_date_time(&ps).expect("a split");
        assert!(!s.time_first);
        assert_eq!(s.date.len(), 3);
        assert_eq!(s.joiner, &ps[3..4]);
        assert_eq!(s.time.len(), 5);
        // Date, time, date: no split.
        let mixed = vec![
            piece(Some(('d', 1)), "4"),
            piece(Some(('h', 1)), "1"),
            piece(Some(('M', 1)), "8"),
        ];
        assert!(split_date_time(&mixed).is_none());
    }
}
