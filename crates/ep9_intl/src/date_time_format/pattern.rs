//! The pattern work of Intl.DateTimeFormat (INTL W4 DTF-PATTERN; INTL-V2 plan 2.2 row 13,
//! INTL-V2-D18 per-string overrides): ICU4X picks a pattern by semantic field set, with one
//! length for all fields; test262 and the other engines follow ICU4C (DateTimePatternGenerator
//! over the CLDR availableFormats). This module takes the ICU4X output, cuts it into pieces by
//! its pattern, and changes the pieces where the two differ.
//!
//! ```text
//! ICU4X string + pattern ─ pieces ─┬─ U+202F -> U+0020 in literals (the V8 and SM output)
//!                                  ├─ h24: hour 0 is "24" (ICU4X formats h24 as h23)
//!                                  ├─ styles: a 2-digit year where the locale pattern has "yy"
//!                                  └─ components:
//!                                     drop the time fields ICU4X adds ({minute, second} -> "mm:ss")
//!                                     dayPeriod: the T3 flexible day period "B" (in the morning)
//!                                     weekday / month text width per field (a second ICU4X run;
//!                                       narrow: the ICU4X names data)
//!                                     numeric month / day width of the CLDR availableFormats
//!                                       (T3 yMd, yM, Md: de "d.M.y")
//!                                     year "2-digit"; one numeric field alone ("6" for second)
//!                                     the decimal sign of the numbering system before the
//!                                       fractional seconds ("٫" for arab)
//! ```
//!
//! Each change reads only the pieces, the resolved options and the T3 table (`crate::cldr`);
//! the ICU4X formatter stays in `crate::cache`. No float math (H36): the hour, minute and
//! second are integers of the ICU4X input.

use icu_calendar::Iso;
use icu_decimal::DecimalFormatter;
use icu_decimal::options::DecimalFormatterOptions;
use icu_decimal::preferences::DecimalFormatterPreferences;
use icu_locale_core::Locale;
use icu_time::zone::models::AtTime;
use icu_time::{TimeZoneInfo, ZonedDateTime};

use super::{HOUR_CYCLES, Resolved, c, formatter_for, part_type};
use crate::cache::{self, Service};
use crate::cldr;
use crate::tag;

/// A piece of a formatted string: a field (its UTS 35 letter and count) or a literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Piece {
    /// The field letter and count; `None` for a literal.
    pub(super) field: Option<(char, usize)>,
    /// The text.
    pub(super) text: String,
}

impl Piece {
    fn literal(text: &str) -> Piece {
        Piece {
            field: None,
            text: text.to_string(),
        }
    }

    /// The field letter (`None` for a literal).
    fn letter(&self) -> Option<char> {
        self.field.map(|(ch, _)| ch)
    }
}

/// The items of the UTS 35 pattern `p`: a field (letter, count) or a literal text. Quoted
/// text ('' for a quote) and other characters are literals.
fn items(p: &str) -> Vec<(Option<(char, usize)>, String)> {
    let mut items: Vec<(Option<(char, usize)>, String)> = Vec::new();
    let mut literal = String::new();
    let mut chars = p.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                literal.push('\'');
            } else {
                quoted = !quoted;
            }
            continue;
        }
        if !quoted && ch.is_ascii_alphabetic() {
            if !literal.is_empty() {
                items.push((None, std::mem::take(&mut literal)));
            }
            let mut n = 1;
            while chars.peek() == Some(&ch) {
                chars.next();
                n += 1;
            }
            items.push((Some((ch, n)), String::new()));
        } else {
            literal.push(ch);
        }
    }
    if !literal.is_empty() {
        items.push((None, literal));
    }
    items
}

/// The count of the first field `ch` of the UTS 35 pattern `p`, if it has one.
pub(super) fn field_count(p: &str, ch: char) -> Option<usize> {
    items(p)
        .into_iter()
        .find_map(|(f, _)| f.filter(|(x, _)| *x == ch).map(|(_, n)| n))
}

/// The pieces of the formatted string `s` of the pattern `p`: each field takes the text up to
/// the next literal of the pattern; two fields with no literal between them split at the
/// digit boundary (the `super::parts` rule). Text that the pattern does not explain is a
/// literal.
pub(super) fn pieces(s: &str, p: &str) -> Vec<Piece> {
    let items = items(p);
    let mut out: Vec<Piece> = Vec::new();
    let mut rest = s;
    for (i, (field, literal)) in items.iter().enumerate() {
        match field {
            None => {
                if let Some(r) = rest.strip_prefix(literal.as_str()) {
                    out.push(Piece::literal(literal));
                    rest = r;
                }
            }
            Some(f) => {
                let end = match items.get(i + 1) {
                    None => rest.len(),
                    Some((Some(_), _)) => super::run_len(rest),
                    Some((None, l)) => rest.find(l.as_str()).unwrap_or(rest.len()),
                };
                if end > 0 {
                    out.push(Piece {
                        field: Some(*f),
                        text: rest[..end].to_string(),
                    });
                }
                rest = &rest[end..];
            }
        }
    }
    if !rest.is_empty() {
        out.push(Piece::literal(rest));
    }
    out
}

/// The parts (Table 18 type, value) of `pieces`: a field's type by its letter; literals next
/// to each other are one part.
pub(super) fn parts(pieces: &[Piece]) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for p in pieces {
        if p.text.is_empty() {
            continue;
        }
        let t = p.letter().map_or("literal", part_type);
        if t == "literal"
            && let Some(last) = out.last_mut()
            && last.0 == "literal"
        {
            last.1.push_str(&p.text);
            continue;
        }
        out.push((t, p.text.clone()));
    }
    out
}

/// Whether `ch` is an hour field letter.
fn is_hour(ch: char) -> bool {
    matches!(ch, 'h' | 'H' | 'K' | 'k')
}

/// Whether `ch` is a day period field letter.
fn is_day_period(ch: char) -> bool {
    matches!(ch, 'a' | 'b' | 'B')
}

/// The time fields of `pieces` that the components of `r` do not ask for (ICU4X adds an hour
/// to {minute, second} and a day period to an h11/h12 hour; CLDR availableFormats "ms" is
/// "mm:ss"), and the literal that joined each to the rest. A day period stays when the
/// components ask for an hour or a day period.
fn drop_unrequested(r: &Resolved, pieces: Vec<Piece>) -> Vec<Piece> {
    let cs = r.components;
    let wanted = |ch: char| -> bool {
        match ch {
            _ if is_hour(ch) => cs.0[c::HOUR] != 0,
            'm' => cs.0[c::MINUTE] != 0,
            's' => cs.0[c::SECOND] != 0,
            'S' => cs.0[c::FRACTIONAL_SECOND_DIGITS] != 0,
            _ if is_day_period(ch) => cs.0[c::HOUR] != 0 || cs.0[c::DAY_PERIOD] != 0,
            _ => true,
        }
    };
    if pieces.iter().filter_map(Piece::letter).all(&wanted) {
        return pieces;
    }
    let mut out: Vec<Piece> = Vec::new();
    let mut dropped = false;
    for p in pieces {
        match p.letter() {
            Some(ch) if !wanted(ch) => dropped = true,
            Some(_) => {
                out.push(p);
                dropped = false;
            }
            None => {
                // The literal after a dropped field joined it to the next field: drop it at the
                // start, or when the literal before the dropped field is kept.
                let keep = !dropped || out.last().is_some_and(|l| l.field.is_some());
                if keep {
                    out.push(p);
                }
                dropped = false;
            }
        }
    }
    if dropped && out.last().is_some_and(|l| l.field.is_none()) {
        out.pop();
    }
    out
}

/// The flexible day period (UTS 35 "B") of `data_locale` at `hour`:`minute`:`second` in the
/// T3 width `width` (0 abbreviated, 1 narrow, 2 wide), the ICU4C rule
/// (`SimpleDateFormat::subFormat`, UDAT_FLEXIBLE_DAY_PERIOD_FIELD): "noon" at 12:00:00 if the
/// rule has it (the minute and second count only if the pattern shows them); "midnight" is
/// never written (ICU 57 and later), so 00:00:00 takes the period of hour 0. `None`: no rule,
/// a rule of am/pm only, or no name: the caller keeps the AM/PM text (ICU4C falls back to
/// "a").
fn flexible_day_period(
    data_locale: &str,
    width: usize,
    hour: u8,
    minute: u8,
    second: u8,
) -> Option<&'static str> {
    let (hours, noon, _) = cldr::day_period_rule(data_locale)?;
    let hour = usize::from(hour).min(23);
    if hour == 12
        && minute == 0
        && second == 0
        && noon
        && let Some(name) = cldr::day_period_name(data_locale, width, cldr::DAY_PERIOD_NOON)
    {
        return Some(name);
    }
    let period = usize::from(hours[hour]);
    if period >= cldr::DAY_PERIOD_NOON {
        return None;
    }
    cldr::day_period_name(data_locale, width, period)
}

/// The zero digit of the numbering system `nu` (`None` for a system with no simple digits).
fn zero_digit(nu: &str) -> Option<char> {
    cldr::numbering_system_digits(nu).map(|d| d[0])
}

/// The numeric text `text` (digits of `nu`) with `width` digits: one zero added in front of a
/// one-digit text for 2, one leading zero cut from a two-digit text for 1. Other texts stay.
fn numeric_width(text: &str, width: usize, nu: &str) -> String {
    let Some(zero) = zero_digit(nu) else {
        return text.to_string();
    };
    let digits: Vec<char> = text.chars().collect();
    if !digits
        .iter()
        .all(|d| cldr::numbering_system_digits(nu).is_some_and(|ds| ds.contains(d)))
    {
        return text.to_string();
    }
    match (digits.len(), width) {
        (1, 2) => format!("{zero}{text}"),
        (2, 1) if digits[0] == zero => digits[1].to_string(),
        _ => text.to_string(),
    }
}

/// "24" in the digits of the hour text `text` (hour 0 of an h23 pattern: "0" or "00"): the
/// digits of `nu` when the text uses them, else the digits after the text's zero (a decimal
/// digit block is contiguous). A text that is not a zero stays.
fn hour_24(text: &str, nu: &str) -> String {
    if let Some(ds) = cldr::numbering_system_digits(nu)
        && !text.is_empty()
        && text.chars().all(|ch| ch == ds[0])
    {
        return [ds[2], ds[4]].iter().collect();
    }
    let mut chars = text.chars();
    if let Some(zero) = chars.next()
        && zero.is_numeric()
        && chars.all(|ch| ch == zero)
        && let (Some(two), Some(four)) = (
            char::from_u32(u32::from(zero) + 2),
            char::from_u32(u32::from(zero) + 4),
        )
    {
        return [two, four].iter().collect();
    }
    text.to_string()
}

/// The last two digits of the numeric text `text` (UTS 35 "yy"; one zero in front of a
/// one-digit text). A text with a non-digit (a sign) stays.
fn two_digit_year(text: &str, nu: &str) -> String {
    let Some(ds) = cldr::numbering_system_digits(nu) else {
        return text.to_string();
    };
    let digits: Vec<char> = text.chars().collect();
    if digits.is_empty() || !digits.iter().all(|d| ds.contains(d)) {
        return text.to_string();
    }
    match digits.len() {
        1 => format!("{}{text}", ds[0]),
        n => digits[n - 2..].iter().collect(),
    }
}

/// The decimal sign that ICU4C writes before the fractional seconds for `data_locale` and
/// the numbering system `nu`: the locale's symbols for `nu` (ICU4X has them in the locales
/// that use `nu`), else for the locale's default system its own sign, else the root sign of
/// `nu` (T3: "٫" for arab), else ".".
fn decimal_sign(data_locale: &str, nu: &str) -> String {
    use icu_decimal::provider::{Baked, DecimalSymbolsV1};
    use icu_provider::prelude::*;
    let silent = || {
        let mut m = DataRequestMetadata::default();
        m.silent = true;
        m
    };
    if let Ok(attributes) = DataMarkerAttributes::try_from_str(nu) {
        for l in cldr::locale_chain(data_locale) {
            if l == "und" {
                break;
            }
            let locale = DataLocale::from(&tag::icu_locale(&l));
            let req = DataRequest {
                id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale),
                metadata: silent(),
            };
            if let Ok(r) = DataProvider::<DecimalSymbolsV1>::load(&Baked, req) {
                return r.payload.get().decimal_separator().to_string();
            }
        }
    }
    if crate::number_format::decimal::default_numbering_system(data_locale) == nu
        && let Ok(locale) = Locale::try_from_str(data_locale)
        && let Ok(f) = DecimalFormatter::try_new(
            DecimalFormatterPreferences::from(&locale),
            DecimalFormatterOptions::default(),
        )
        && let Ok(half) = fixed_decimal::Decimal::try_from_str("0.5")
    {
        // "0", the sign, "5": the sign is the text between the first and the last char.
        let s = f.format(&half).to_string();
        let first = s.chars().next().map_or(0, char::len_utf8);
        let last = s.chars().next_back().map_or(0, char::len_utf8);
        if let Some(sign) = s.get(first..s.len().saturating_sub(last))
            && !sign.is_empty()
        {
            return sign.to_string();
        }
    }
    cldr::numbering_system_decimal(nu)
        .unwrap_or(".")
        .to_string()
}

/// The text width class of the UTS 35 count `n` of a weekday or text month field: 3
/// abbreviated (counts 1-3 and 6), 4 wide, 5 narrow.
fn text_class(n: usize) -> usize {
    match n {
        4 => 4,
        5 => 5,
        _ => 3,
    }
}

/// The text width (UTS 35 count: 3 abbreviated, 4 wide, 5 narrow) that the components of `r`
/// ask for the field letter `ch` (weekday or text month), if any.
fn wanted_text_width(r: &Resolved, ch: char) -> Option<usize> {
    let cs = r.components;
    match ch {
        'E' | 'c' => match cs.0[c::WEEKDAY] {
            1 => Some(5),
            2 => Some(3),
            3 => Some(4),
            _ => None,
        },
        'M' | 'L' => match cs.0[c::MONTH] {
            3 => Some(5),
            4 => Some(3),
            5 => Some(4),
            _ => None,
        },
        _ => None,
    }
}

/// The text of the field `ch` from an ICU4X run of `r` whose text fields (weekday and text
/// month) all have the width `width` (3 abbreviated, 4 wide): the field bag gives one length
/// to all fields, so a run per width gives each field its own width.
fn text_in_width(
    r: &Resolved,
    input: &ZonedDateTime<Iso, TimeZoneInfo<AtTime>>,
    ch: char,
    width: usize,
) -> Option<String> {
    let mut r2 = r.clone();
    let cs = &mut r2.components.0;
    if cs[c::WEEKDAY] != 0 {
        cs[c::WEEKDAY] = if width == 4 { 3 } else { 2 };
    }
    if cs[c::MONTH] >= 3 {
        cs[c::MONTH] = if width == 4 { 5 } else { 4 };
    }
    let f = formatter_for(&r2)?;
    let formatted = f.format(input);
    let ps = pieces(&formatted.to_string(), &formatted.pattern().to_string());
    let same =
        |x: char| x == ch || matches!((x, ch), ('E', 'c') | ('c', 'E') | ('M', 'L') | ('L', 'M'));
    ps.into_iter()
        .find(|p| {
            p.field
                .is_some_and(|(x, n)| same(x) && text_class(n) == width)
        })
        .map(|p| p.text)
}

/// The narrow name (UTS 35 count 5) of the weekday or month field `ch` ('E' or 'c', 'M' or
/// 'L': the format or the stand-alone names) of `input` in the locale and calendar of `r`,
/// read from the ICU4X names data (the baked provider does the locale fallback). `None` when
/// the data has no plain name for the month (a leap month, or a leap-pattern table): the
/// caller keeps the ICU4X text.
fn narrow_text(
    r: &Resolved,
    input: &ZonedDateTime<Iso, TimeZoneInfo<AtTime>>,
    ch: char,
) -> Option<String> {
    use icu_calendar::{AnyCalendar, AnyCalendarKind, Ref};
    use icu_datetime::provider::Baked;
    use icu_datetime::provider::names::{
        DatetimeNamesMonthBuddhistV1, DatetimeNamesMonthChineseV1, DatetimeNamesMonthCopticV1,
        DatetimeNamesMonthDangiV1, DatetimeNamesMonthEthiopianV1, DatetimeNamesMonthGregorianV1,
        DatetimeNamesMonthHebrewV1, DatetimeNamesMonthHijriV1, DatetimeNamesMonthIndianV1,
        DatetimeNamesMonthJapaneseV1, DatetimeNamesMonthPersianV1, DatetimeNamesMonthRocV1,
        DatetimeNamesWeekdayV1,
    };
    use icu_datetime::provider::semantic_skeletons::marker_attrs::{NARROW, NARROW_STANDALONE};
    use icu_provider::prelude::*;

    let attributes = if matches!(ch, 'L' | 'c') {
        NARROW_STANDALONE
    } else {
        NARROW
    };
    let locale = DataLocale::from(&tag::icu_locale(&tag::remove_unicode_extension(&r.locale)));
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale),
        ..Default::default()
    };
    if matches!(ch, 'E' | 'c') {
        let names = DataProvider::<DatetimeNamesWeekdayV1>::load(&Baked, req).ok()?;
        return names
            .payload
            .get()
            .get(input.date.weekday())
            .map(str::to_string);
    }
    let kind = match r.calendar.as_str() {
        "buddhist" => AnyCalendarKind::Buddhist,
        "chinese" => AnyCalendarKind::Chinese,
        "coptic" => AnyCalendarKind::Coptic,
        "dangi" => AnyCalendarKind::Dangi,
        "ethioaa" => AnyCalendarKind::EthiopianAmeteAlem,
        "ethiopic" => AnyCalendarKind::Ethiopian,
        "gregory" | "iso8601" => AnyCalendarKind::Gregorian,
        "hebrew" => AnyCalendarKind::Hebrew,
        "indian" => AnyCalendarKind::Indian,
        "islamic-civil" => AnyCalendarKind::HijriTabularTypeIIFriday,
        "islamic-tbla" => AnyCalendarKind::HijriTabularTypeIIThursday,
        "islamic-umalqura" => AnyCalendarKind::HijriUmmAlQura,
        "japanese" => AnyCalendarKind::Japanese,
        "persian" => AnyCalendarKind::Persian,
        "roc" => AnyCalendarKind::Roc,
        _ => return None,
    };
    let calendar = AnyCalendar::new(kind);
    let month = input.date.to_calendar(Ref(&calendar)).month();
    let nu = r.numbering_system.as_str();
    match kind {
        AnyCalendarKind::Buddhist => month_name::<DatetimeNamesMonthBuddhistV1>(req, month, nu),
        AnyCalendarKind::Chinese => month_name::<DatetimeNamesMonthChineseV1>(req, month, nu),
        AnyCalendarKind::Coptic => month_name::<DatetimeNamesMonthCopticV1>(req, month, nu),
        AnyCalendarKind::Dangi => month_name::<DatetimeNamesMonthDangiV1>(req, month, nu),
        AnyCalendarKind::Ethiopian | AnyCalendarKind::EthiopianAmeteAlem => {
            month_name::<DatetimeNamesMonthEthiopianV1>(req, month, nu)
        }
        AnyCalendarKind::Hebrew => month_name::<DatetimeNamesMonthHebrewV1>(req, month, nu),
        AnyCalendarKind::Indian => month_name::<DatetimeNamesMonthIndianV1>(req, month, nu),
        AnyCalendarKind::HijriTabularTypeIIFriday
        | AnyCalendarKind::HijriTabularTypeIIThursday
        | AnyCalendarKind::HijriUmmAlQura => {
            month_name::<DatetimeNamesMonthHijriV1>(req, month, nu)
        }
        AnyCalendarKind::Japanese => month_name::<DatetimeNamesMonthJapaneseV1>(req, month, nu),
        AnyCalendarKind::Persian => month_name::<DatetimeNamesMonthPersianV1>(req, month, nu),
        AnyCalendarKind::Roc => month_name::<DatetimeNamesMonthRocV1>(req, month, nu),
        _ => month_name::<DatetimeNamesMonthGregorianV1>(req, month, nu),
    }
}

/// The name of `month` in the month names of the marker `M` for `req`: the plain name of a
/// month that is not a leap month or a leap base (ICU4X `get_name_for_month`), or the month
/// number (digits of `nu`) when the table is numeric. `None` for a leap month or a leap base
/// (their name is a pattern around the plain name; the ICU4X text stays).
fn month_name<M>(
    req: icu_provider::DataRequest,
    month: icu_calendar::types::MonthInfo,
    nu: &str,
) -> Option<String>
where
    M: icu_provider::DataMarker<DataStruct = icu_datetime::provider::names::MonthNames<'static>>,
    icu_datetime::provider::Baked: icu_provider::DataProvider<M>,
{
    use icu_calendar::types::LeapStatus;
    use icu_datetime::provider::names::MonthNames;
    let names = icu_provider::DataProvider::<M>::load(&icu_datetime::provider::Baked, req).ok()?;
    let normal = month.leap_status() == LeapStatus::Normal;
    let index = usize::from(month.number()).checked_sub(1)?;
    match names.payload.get() {
        MonthNames::Linear(l) if normal => l.get(index).map(str::to_string),
        // The last two entries are the leap and leap-base patterns (not names).
        MonthNames::LeapPattern(l) if normal && index + 2 < l.len() => {
            l.get(index).map(str::to_string)
        }
        MonthNames::Numeric => Some(number_text(month.number(), nu)),
        MonthNames::LeapNumeric(_) | MonthNames::LeapNumericWithBase(_) if normal => {
            Some(number_text(month.number(), nu))
        }
        _ => None,
    }
}

/// The decimal digits of `n` in the numbering system `nu` (ASCII digits for a system with
/// no simple digits).
fn number_text(n: u8, nu: &str) -> String {
    let ascii = n.to_string();
    match cldr::numbering_system_digits(nu) {
        Some(ds) => ascii.bytes().map(|b| ds[usize::from(b - b'0')]).collect(),
        None => ascii,
    }
}

/// The local time of the ICU4X input: hour, minute, second.
pub(super) struct Clock {
    pub(super) hour: u8,
    pub(super) minute: u8,
    pub(super) second: u8,
}

/// The pieces of the ICU4X output `pieces` of `r`, changed to the ICU4C form (module doc).
/// `input` is the ICU4X input, `clock` its local time, `reference_year_is_two_digit` whether
/// the same formatter writes the year with 2 digits for a date of the year 2000 (a style
/// format: ICU4X writes the full year far from the present, ICU4C the pattern's "yy").
pub(super) fn adjust(
    r: &Resolved,
    input: &ZonedDateTime<Iso, TimeZoneInfo<AtTime>>,
    clock: &Clock,
    reference_year_is_two_digit: bool,
    mut pieces: Vec<Piece>,
) -> Vec<Piece> {
    // U+202F (narrow no-break space) -> U+0020 in the literals: V8 and SpiderMonkey write a
    // space before the day period (web compatibility; test262 numbering-system.js).
    for p in pieces.iter_mut().filter(|p| p.field.is_none()) {
        if p.text.contains('\u{202f}') {
            p.text = p.text.replace('\u{202f}', " ");
        }
    }
    let nu = r.numbering_system.as_str();
    // hourCycle "h24": ICU4X has no h24 and formats with h23 (`prefs_tag`), so the hour at
    // midnight is "00". FormatDateTimePattern (11.5.5), p = "hour": if [[HourCycle]] is "h24"
    // and v is 0, let v be 24. For the components and the styles.
    if clock.hour == 0 && r.styles.hour_cycle.is_some_and(|h| HOUR_CYCLES[h] == "h24") {
        for p in pieces.iter_mut() {
            if let Some(('H', n)) = p.field {
                p.text = hour_24(&p.text, nu);
                p.field = Some(('k', n));
            }
        }
    }
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let styles = r.styles.date_style.is_some() || r.styles.time_style.is_some();
    let year_is_two_digit = if styles {
        reference_year_is_two_digit
    } else {
        r.components.0[c::YEAR] == 1
    };
    if year_is_two_digit {
        for p in pieces.iter_mut() {
            if let Some(('y', n)) = p.field
                && n != 2
            {
                p.text = two_digit_year(&p.text, nu);
                p.field = Some(('y', 2));
            }
        }
    }
    if styles {
        return pieces;
    }
    let cs = r.components;
    let original = pieces.clone();
    let mut pieces = drop_unrequested(r, pieces);

    // dayPeriod: the flexible day period in place of the AM/PM field.
    if cs.0[c::DAY_PERIOD] != 0 {
        let width = match cs.0[c::DAY_PERIOD] {
            1 => 1,
            2 => 0,
            _ => 2,
        };
        let has = |ch: char| pieces.iter().any(|p| p.letter() == Some(ch));
        let minute = if has('m') { clock.minute } else { 0 };
        let second = if has('s') { clock.second } else { 0 };
        if let Some(name) = flexible_day_period(&data_locale, width, clock.hour, minute, second) {
            let count = [3, 5, 4][width];
            if let Some(p) = pieces
                .iter_mut()
                .find(|p| p.letter().is_some_and(is_day_period))
            {
                p.text = name.to_string();
                p.field = Some(('B', count));
            } else if pieces.is_empty() {
                // dayPeriod alone in a locale whose ICU4X pattern has a 24-hour hour.
                pieces.push(Piece {
                    field: Some(('B', count)),
                    text: name.to_string(),
                });
            }
        }
        if pieces.is_empty() {
            // No rule and no AM/PM field left: the ICU4X output.
            return original;
        }
    }

    // The text width of each weekday and text month field.
    for p in pieces.iter_mut() {
        let Some((ch, n)) = p.field else {
            continue;
        };
        let Some(want) = wanted_text_width(r, ch) else {
            continue;
        };
        if (ch == 'M' || ch == 'L') && n < 3 {
            continue;
        }
        if text_class(n) == want {
            continue;
        }
        if want == 5 {
            // ICU4X has no narrow run (the field bag maps narrow to the abbreviated length):
            // the narrow name from the names data. No name: the ICU4X text stays.
            if let Some(text) = narrow_text(r, input, ch) {
                p.text = text;
                p.field = Some((ch, 5));
            }
            continue;
        }
        if let Some(text) = text_in_width(r, input, ch, want) {
            p.text = text;
            p.field = Some((ch, want));
        }
    }

    // The numeric month and day widths of the CLDR availableFormats pattern of the numeric
    // date skeleton (ICU4C keeps the pattern's width when the skeleton field has the
    // requested length: de "d.M.y", en-GB "dd/MM/y"); "2-digit" is 2 digits.
    let numeric = |i: usize| matches!(cs.0[i], 1 | 2);
    let (y, m, d) = (cs.0[c::YEAR] != 0, cs.0[c::MONTH] != 0, cs.0[c::DAY] != 0);
    let skeleton = match (y, m, d) {
        (true, true, true) => Some(0),
        (true, true, false) => Some(1),
        (false, true, true) => Some(2),
        _ => None,
    };
    if let Some(skeleton) = skeleton
        && cs.0[c::WEEKDAY] == 0
        && cs.0[c::ERA] == 0
        && numeric(c::MONTH)
        && (!d || numeric(c::DAY))
        && let Some(cldr_pattern) = cldr::numeric_date_pattern(&data_locale, skeleton)
    {
        let count = |letters: &[char]| {
            items(cldr_pattern)
                .iter()
                .find_map(|(f, _)| f.filter(|(ch, _)| letters.contains(ch)).map(|(_, n)| n))
        };
        let width_of = |i: usize, letters: &[char]| match cs.0[i] {
            1 => Some(2),
            _ => count(letters).filter(|n| *n <= 2),
        };
        let month_width = width_of(c::MONTH, &['M', 'L']);
        let day_width = width_of(c::DAY, &['d']);
        for p in pieces.iter_mut() {
            let (ch, want) = match p.field {
                Some((ch @ ('M' | 'L'), n)) if n <= 2 => (ch, month_width),
                Some(('d', _)) => ('d', day_width),
                _ => continue,
            };
            if let Some(w) = want {
                p.text = numeric_width(&p.text, w, nu);
                p.field = Some((ch, w));
            }
        }
    }

    // One numeric field alone ({second: "numeric"}: "6"; ICU4C's pattern is the skeleton).
    if let [only] = pieces.as_mut_slice()
        && let Some((ch @ ('m' | 's' | 'd'), _)) = only.field
    {
        let i = match ch {
            'm' => c::MINUTE,
            's' => c::SECOND,
            _ => c::DAY,
        };
        let w = if cs.0[i] == 1 { 2 } else { 1 };
        only.text = numeric_width(&only.text, w, nu);
        only.field = Some((ch, w));
    }

    // The decimal sign of the numbering system between the seconds and the fraction.
    if cs.0[c::FRACTIONAL_SECOND_DIGITS] != 0
        && let Some(i) = pieces.iter().position(|p| p.letter() == Some('S'))
        && i >= 2
        && pieces[i - 1].field.is_none()
        && pieces[i - 2].letter() == Some('s')
    {
        // The sign lives in crate::cache with the formatters (a key no formatter has: the
        // formatter keys are below 2^33).
        let key = format!("{data_locale} {nu}");
        let sign = cache::get_or_make(Service::DateTimeFormat, &key, u64::MAX, || {
            Some(decimal_sign(&data_locale, nu))
        });
        if let Some(sign) = sign {
            pieces[i - 1].text = sign.as_ref().clone();
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(ps: &[Piece]) -> Vec<String> {
        ps.iter()
            .map(|p| match p.field {
                Some((ch, n)) => format!("{}{n}:{}", ch, p.text),
                None => format!("'{}'", p.text),
            })
            .collect()
    }

    #[test]
    fn pieces_keep_the_field_counts() {
        let ps = pieces("1:02:03.4\u{202f}AM", "h:mm:ss.S\u{202f}a");
        assert_eq!(
            texts(&ps),
            [
                "h1:1",
                "':'",
                "m2:02",
                "':'",
                "s2:03",
                "'.'",
                "S1:4",
                "'\u{202f}'",
                "a1:AM"
            ]
        );
        let ps = pieces("Sa., 24. Dez.", "EEE, d. MMM");
        assert_eq!(texts(&ps), ["E3:Sa.", "', '", "d1:24", "'. '", "M3:Dez."]);
    }

    #[test]
    fn parts_follow_the_pattern() {
        let parts_of = |s: &str, p: &str| parts(&pieces(s, p));
        let p = parts_of("1/2/2026, 3:04:05\u{202f}PM", "M/d/y, h:mm:ss\u{202f}a");
        let text: Vec<String> = p.iter().map(|(t, v)| format!("{t}:{v}")).collect();
        assert_eq!(
            text,
            [
                "month:1",
                "literal:/",
                "day:2",
                "literal:/",
                "year:2026",
                "literal:, ",
                "hour:3",
                "literal::",
                "minute:04",
                "literal::",
                "second:05",
                "literal:\u{202f}",
                "dayPeriod:PM"
            ]
        );
        let p = parts_of("Tuesday, January 2", "EEEE, MMMM d");
        assert_eq!(p[0], ("weekday", "Tuesday".to_string()));
        assert_eq!(p[2], ("month", "January".to_string()));
        let p = parts_of("2026年1月2日", "y年M月d日");
        assert_eq!(p.len(), 6);
        assert_eq!(p[1], ("literal", "年".to_string()));
        let p = parts_of("12 o'clock", "h 'o''clock'");
        assert_eq!(p[1], ("literal", " o'clock".to_string()));
        assert_eq!(field_count("M/d/yy", 'y'), Some(2));
        assert_eq!(field_count("'y' d", 'y'), None);
    }

    #[test]
    fn widths_and_years() {
        assert_eq!(numeric_width("02", 1, "latn"), "2");
        assert_eq!(numeric_width("12", 1, "latn"), "12");
        assert_eq!(numeric_width("2", 2, "latn"), "02");
        assert_eq!(numeric_width("٠٦", 1, "arab"), "٦");
        assert_eq!(two_digit_year("1886", "latn"), "86");
        assert_eq!(two_digit_year("5", "latn"), "05");
        assert_eq!(two_digit_year("-5", "latn"), "-5");
        assert_eq!(two_digit_year("٢٠٢٦", "arab"), "٢٦");
        assert_eq!(hour_24("00", "latn"), "24");
        assert_eq!(hour_24("0", "latn"), "24");
        assert_eq!(hour_24("٠٠", "arab"), "٢٤");
        assert_eq!(hour_24("〇", "hanidec"), "二四");
        assert_eq!(hour_24("०", "latn"), "२४");
        assert_eq!(hour_24("12", "latn"), "12");
    }

    #[test]
    fn flexible_day_periods_follow_icu4c() {
        let en = |h, m, s, w| flexible_day_period("en", w, h, m, s);
        assert_eq!(en(0, 0, 0, 2), Some("in the morning"), "no midnight");
        assert_eq!(en(12, 0, 0, 2), Some("noon"));
        assert_eq!(en(12, 0, 0, 1), Some("n"));
        assert_eq!(en(12, 30, 0, 2), Some("in the afternoon"));
        assert_eq!(en(20, 0, 0, 0), Some("in the evening"));
        assert_eq!(en(23, 0, 0, 2), Some("at night"));
        assert_eq!(flexible_day_period("de", 2, 3, 0, 0), Some("nachts"));
        assert_eq!(flexible_day_period("und", 2, 3, 0, 0), None, "am/pm only");
    }
}
