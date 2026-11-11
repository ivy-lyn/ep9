//! Calendars (design SPEC-T-D17 as changed by INTL-I1-D2): AvailableCalendars of ECMA-402 is
//! [`AVAILABLE_CALENDARS`], the 16 CLDR ids that temporal_rs maps to an icu_calendar
//! `AnyCalendarKind`. Any other calendar id, in an argument, a property bag or a string
//! annotation, is a RangeError. temporal_rs also accepts `islamic` (as `islamic-civil`); ep9
//! checks the id before it calls temporal_rs, so `islamic` and `islamic-rgsa` are RangeErrors.
//!
//! icu_calendar computes every one of these calendars with tables and integer arithmetic: the
//! astronomy code of `calendrical_calculations` has no runtime caller (INTL-I1-D2;
//! `tools/calprobe` counts the calls).
//!
//! A Temporal object keeps its calendar in a slot as an ep9 code (append-only; `CODES`):
//!
//! ```text
//!  0 iso8601    4 dangi      8 hebrew          12 islamic-umalqura
//!  1 buddhist   5 ethioaa    9 indian          13 japanese
//!  2 chinese    6 ethiopic  10 islamic-civil   14 persian
//!  3 coptic     7 gregory   11 islamic-tbla    15 roc
//! ```
//!
//! The date slots of PlainDate, PlainDateTime, PlainYearMonth and PlainMonthDay hold the ISO
//! date (the `[[ISODate]]` slot), never the calendar fields.

use core::str::FromStr;

use ep9_vm::{Cx, JsObject, JsResult, Local, ObjectClass};
use temporal_rs::fields::{CalendarFields, YearMonthCalendarFields};
use temporal_rs::parsers::parse_allowed_calendar_formats;
use temporal_rs::{Calendar, PlainDate, PlainYearMonth, TinyAsciiStr};

use crate::convert::{temporal_text, to_integer_with_truncation};
use crate::fields::{self, Bag};

/// AvailableCalendars (ECMA-402 §6.10 AvailableCalendars): the canonical calendar ids that ep9
/// supports, sorted as `Intl.supportedValuesOf("calendar")` returns them. The one list for
/// Temporal and Intl (design INTL-I1 §4.4).
pub const AVAILABLE_CALENDARS: [&str; 16] = [
    "buddhist",
    "chinese",
    "coptic",
    "dangi",
    "ethioaa",
    "ethiopic",
    "gregory",
    "hebrew",
    "indian",
    "islamic-civil",
    "islamic-tbla",
    "islamic-umalqura",
    "iso8601",
    "japanese",
    "persian",
    "roc",
];

/// The calendar of each slot code: code `i` is `CODES[i]` (append-only; the module comment).
const CODES: [Calendar; 16] = [
    Calendar::ISO,
    Calendar::BUDDHIST,
    Calendar::CHINESE,
    Calendar::COPTIC,
    Calendar::DANGI,
    Calendar::ETHIOPIAN_AMETE_ALEM,
    Calendar::ETHIOPIAN,
    Calendar::GREGORIAN,
    Calendar::HEBREW,
    Calendar::INDIAN,
    Calendar::HIJRI_TABULAR_FRIDAY,
    Calendar::HIJRI_TABULAR_THURSDAY,
    Calendar::HIJRI_UMM_AL_QURA,
    Calendar::JAPANESE,
    Calendar::PERSIAN,
    Calendar::ROC,
];

/// The calendar code of `iso8601` in a calendar slot (ep9 codes, append-only).
pub(crate) const CAL_ISO8601: i32 = 0;

/// The classes with a `[[Calendar]]` slot (PlainDate, PlainDateTime, PlainYearMonth,
/// PlainMonthDay, ZonedDateTime). Each keeps its calendar code in slot [`CALENDAR_SLOT`].
const CALENDAR_CLASSES: &[ObjectClass] = &[
    ObjectClass::TemporalPlainDate,
    ObjectClass::TemporalPlainDateTime,
    ObjectClass::TemporalPlainYearMonth,
    ObjectClass::TemporalPlainMonthDay,
    ObjectClass::TemporalZonedDateTime,
];

/// The slot of the calendar code in every class of [`CALENDAR_CLASSES`].
const CALENDAR_SLOT: usize = 3;

/// The slot code of `cal` (a calendar from [`canonicalize_calendar`], [`string_calendar`] or
/// a slot).
pub(crate) fn to_slot(cal: &Calendar) -> i32 {
    let kind = cal.kind();
    match CODES.iter().position(|c| c.kind() == kind) {
        Some(i) => i as i32,
        None => {
            debug_assert!(false, "calendar {} has no slot code", cal.identifier());
            CAL_ISO8601
        }
    }
}

/// The calendar of a slot code (a code that [`to_slot`] made).
pub(crate) fn from_slot(code: f64) -> Calendar {
    let i = code as usize;
    debug_assert!(code >= 0.0 && i < CODES.len(), "calendar code {code}");
    CODES.get(i).cloned().unwrap_or(Calendar::ISO)
}

/// CalendarSupportsEra (ECMA-402 Temporal): every available calendar but `chinese`, `dangi`
/// and `iso8601` has eras.
pub(crate) fn supports_era(cal: &Calendar) -> bool {
    let kind = cal.kind();
    kind != Calendar::ISO.kind()
        && kind != Calendar::CHINESE.kind()
        && kind != Calendar::DANGI.kind()
}

/// CanonicalizeCalendar (§12.2.1) of the String `id`: its ASCII-lowercase form must be an
/// available calendar or one of its aliases (`islamicc`, `ethiopic-amete-alem`), else a
/// RangeError. Returns the canonical calendar.
pub(crate) fn canonicalize_calendar(cx: &mut Cx<'_>, id: &str) -> JsResult<Calendar> {
    let lower = id.to_ascii_lowercase();
    // temporal_rs maps `islamic` to `islamic-civil`; ECMA-402 has no `islamic` (and no
    // `islamic-rgsa`) in AvailableCalendars, so ep9 rejects it before it asks temporal_rs.
    let known = !matches!(lower.as_str(), "islamic" | "islamic-rgsa");
    match Calendar::try_from_utf8(lower.as_bytes()) {
        Ok(cal) if known && AVAILABLE_CALENDARS.contains(&cal.identifier()) => Ok(cal),
        _ => Err(cx.throw_range_error(&format!("calendar {id} is not supported"))),
    }
}

/// The calendar of a Temporal string that the caller parsed (`text`): the `u-ca` annotation of
/// `text`, or `iso8601` without one, then CanonicalizeCalendar. `parsed` is the calendar
/// temporal_rs kept when it built the value (`None` when the build failed, for example out of
/// the ISO limits). temporal_rs reads the annotation with the same parser
/// (`parse_allowed_calendar_formats`) and the same canonical form, so its calendar is the one
/// of the annotation; only `islamic-civil` can also come from the rejected `islamic`, so that
/// one reads the annotation again.
pub(crate) fn string_calendar(
    cx: &mut Cx<'_>,
    text: &str,
    parsed: Option<&Calendar>,
) -> JsResult<Calendar> {
    if let Some(cal) = parsed
        && cal.identifier() != "islamic-civil"
    {
        return Ok(cal.clone());
    }
    match parse_allowed_calendar_formats(text.as_bytes()) {
        Some([]) => Ok(Calendar::ISO),
        Some(id) => {
            let id = String::from_utf8_lossy(id).into_owned();
            canonicalize_calendar(cx, &id)
        }
        // Not reached: the caller parsed `text` as a Temporal string.
        None => Calendar::from_str(text).map_err(|e| crate::error::throw(cx, e)),
    }
}

/// "If CalendarEquals(one, two) is false, throw a RangeError exception" (the `until`,
/// `since` step before the options are read).
pub(crate) fn require_same(cx: &mut Cx<'_>, one: &Calendar, two: &Calendar) -> JsResult<()> {
    if one.kind() == two.kind() {
        Ok(())
    } else {
        let msg = format!(
            "calendars {} and {} are not the same",
            one.identifier(),
            two.identifier()
        );
        Err(cx.throw_range_error(&msg))
    }
}

/// Whether `v` is a Temporal object with a `[[Calendar]]` slot.
pub(crate) fn has_calendar_slot(cx: &Cx<'_>, v: Local<'_>) -> bool {
    cx.object_class(v)
        .is_some_and(|c| CALENDAR_CLASSES.contains(&c))
}

/// The `[[Calendar]]` slot of `v`, or `None` when `v` has none.
fn calendar_of_object<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Option<Calendar>> {
    match cx.object_class(v) {
        Some(class) if CALENDAR_CLASSES.contains(&class) => {
            let code = cx.slot(v, class, CALENDAR_SLOT)?.as_number().unwrap_or(0.0);
            Ok(Some(from_slot(code)))
        }
        _ => Ok(None),
    }
}

/// ToTemporalCalendarIdentifier (§12.2.9).
pub(crate) fn to_temporal_calendar_identifier<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
) -> JsResult<Calendar> {
    // 1. If temporalCalendarLike is an Object, then
    //    a. If it has a [[Calendar]] slot, return temporalCalendarLike.[[Calendar]].
    if let Some(cal) = calendar_of_object(cx, v)? {
        return Ok(cal);
    }
    // 2. If temporalCalendarLike is not a String, throw a TypeError exception.
    let Some(s) = v.as_string() else {
        return Err(cx.throw_type_error("calendar must be a string"));
    };
    // 3. Let identifier be ? ParseTemporalCalendarString(temporalCalendarLike).
    let t = temporal_text(cx, s)?;
    let id = match parse_allowed_calendar_formats(t.as_bytes()) {
        // A Temporal string: its annotation, or iso8601 without one.
        Some([]) => return Ok(Calendar::ISO),
        Some(a) => String::from_utf8_lossy(a).into_owned(),
        // Else the string itself must be a calendar id (AnnotationValue).
        None => t,
    };
    // 4. Return ? CanonicalizeCalendar(identifier).
    canonicalize_calendar(cx, &id)
}

/// GetTemporalCalendarIdentifierWithISODefault (§12.2.10).
pub(crate) fn get_calendar_with_iso_default<'s>(
    cx: &mut Cx<'s>,
    item: Local<'s, JsObject>,
) -> JsResult<Calendar> {
    // 1. If item has a [[Calendar]] slot, return item.[[Calendar]].
    if let Some(cal) = calendar_of_object(cx, item.value())? {
        return Ok(cal);
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
/// PlainMonthDay, ZonedDateTime): undefined is `iso8601`; not a String is a TypeError; else
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

// ---- property bags with eras ----------------------------------------------------------------

/// The fields of a property bag for a calendar: the fields of [`fields::read`] plus `era` and
/// `eraYear` (CalendarExtraFields).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CalendarBag {
    pub(crate) bag: Bag,
    pub(crate) era: Option<TinyAsciiStr<19>>,
    pub(crate) era_year: Option<i32>,
}

/// An era text that no calendar has (an era longer than 19 ASCII chars, or not ASCII): the
/// calendar then throws its RangeError for an unknown era, at the spec's step.
const NO_ERA: TinyAsciiStr<19> = match TinyAsciiStr::try_from_str("-") {
    Ok(s) => s,
    Err(_) => panic!("an ASCII era text"),
};

/// PrepareCalendarFields(calendar, o, fieldNames, ...) (§12.2.3) over the fields in `want`
/// (bits of [`fields`]): the fields of [`fields::read`] and, when `want` has `year` and the
/// calendar has eras, `era` (ToString) and `eraYear` (ToIntegerWithTruncation)
/// (CalendarExtraFields). All are read in alphabetical order: `day`, `era`, `eraYear`, then
/// the others.
pub(crate) fn prepare_fields<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    want: u16,
    cal: &Calendar,
) -> JsResult<CalendarBag> {
    if want & fields::YEAR == 0 || !supports_era(cal) {
        return Ok(CalendarBag {
            bag: fields::read(cx, o, want)?,
            ..CalendarBag::default()
        });
    }
    // `day` comes before `era` in the alphabetical order.
    let mut bag = fields::read(cx, o, want & fields::DAY)?;
    let mut r = CalendarBag::default();
    let k = cx.key("era");
    let v = cx.get(o, k)?;
    if !v.is_undefined() {
        let s = cx.to_rust_string(v)?;
        r.era = Some(TinyAsciiStr::try_from_str(&s).unwrap_or(NO_ERA));
    }
    let k = cx.key("eraYear");
    let v = cx.get(o, k)?;
    if !v.is_undefined() {
        let n = to_integer_with_truncation(cx, v)?;
        r.era_year = Some(n.clamp(i32::MIN as f64, i32::MAX as f64) as i32);
    }
    let rest = fields::read(cx, o, want & !fields::DAY)?;
    bag = Bag {
        day: bag.day,
        any: bag.any || rest.any || r.era.is_some() || r.era_year.is_some(),
        ..rest
    };
    r.bag = bag;
    Ok(r)
}

impl CalendarBag {
    /// Whether any field that was read is present (not undefined).
    pub(crate) fn any(&self) -> bool {
        self.bag.any
    }

    /// The date fields as a temporal_rs record.
    pub(crate) fn calendar_fields(&self) -> CalendarFields {
        CalendarFields {
            era: self.era,
            era_year: self.era_year,
            ..self.bag.calendar_fields()
        }
    }

    /// The year-month fields as a temporal_rs record.
    pub(crate) fn year_month_fields(&self) -> YearMonthCalendarFields {
        let mut f = self.bag.year_month_fields();
        f.era = self.era;
        f.era_year = self.era_year;
        f
    }
}

// ---- ISO dates of temporal_rs values ---------------------------------------------------------

/// The ISO date (year, month, day) of `d` (its `[[ISODate]]`; `d.year()` is the calendar year,
/// equal to the ISO year only for iso8601).
pub(crate) fn iso_of_date(d: &PlainDate) -> (i32, u8, u8) {
    if d.calendar().is_iso() {
        return (d.year(), d.month(), d.day());
    }
    let iso = d.with_calendar(Calendar::ISO);
    (iso.year(), iso.month(), iso.day())
}

/// The ISO date (year, month, reference day) of `ym` (its `[[ISODate]]`). temporal_rs keeps it
/// private; `epoch_ns_for_utc` is that date at noon UTC, so its day number gives it back.
pub(crate) fn iso_of_year_month(ym: &PlainYearMonth) -> (i32, u8, u8) {
    const NS_PER_DAY: i128 = 86_400_000_000_000;
    // iso8601: the calendar fields are the ISO fields.
    if ym.calendar().is_iso() {
        return (ym.year(), ym.month(), ym.reference_day());
    }
    let days = ym.epoch_ns_for_utc().0.div_euclid(NS_PER_DAY);
    iso_date_of_epoch_days(days as i64)
}

/// The ISO date of the day `days` after 1970-01-01 (proleptic Gregorian; integer arithmetic:
/// the days-to-civil algorithm of H. Hinnant, `chrono`-compatible).
fn iso_date_of_epoch_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // 0..=146_096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // 0..=399
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..=365
    let mp = (5 * doy + 2) / 153; // 0..=11, from March
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_available_calendars() {
        // Every available id has one code, and code 0 is iso8601 (kept from SPEC-T-D17).
        assert_eq!(CODES[CAL_ISO8601 as usize].identifier(), "iso8601");
        let mut ids: Vec<&str> = CODES.iter().map(Calendar::identifier).collect();
        ids.sort_unstable();
        assert_eq!(ids, AVAILABLE_CALENDARS);
        for (i, c) in CODES.iter().enumerate() {
            assert_eq!(to_slot(c), i as i32);
            assert_eq!(from_slot(i as f64).identifier(), c.identifier());
        }
    }

    #[test]
    fn available_calendars_are_sorted() {
        let mut s = AVAILABLE_CALENDARS;
        s.sort_unstable();
        assert_eq!(s, AVAILABLE_CALENDARS);
    }

    #[test]
    fn epoch_days_to_iso() {
        assert_eq!(iso_date_of_epoch_days(0), (1970, 1, 1));
        assert_eq!(iso_date_of_epoch_days(-1), (1969, 12, 31));
        assert_eq!(iso_date_of_epoch_days(11_016), (2000, 2, 29));
        assert_eq!(iso_date_of_epoch_days(-719_468), (0, 3, 1));
        // The Temporal limits (±10^8 days around the epoch).
        assert_eq!(iso_date_of_epoch_days(-100_000_001), (-271_821, 4, 19));
        assert_eq!(iso_date_of_epoch_days(100_000_000), (275_760, 9, 13));
    }
}
