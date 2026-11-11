//! Date.parse (ECMA-262 §21.4.3.2) without the time zone: the Date Time String Format
//! (§21.4.1.32), the forms that `toString`, `toDateString` and `toUTCString` produce, and the
//! numeric slash dates of V8's legacy parser. Any other string gives `None` (NaN). The rest of
//! V8's legacy parser (the other strings browsers accept) is not built.
//!
//! ```text
//! ISO     YYYY | ±YYYYYY  [-MM [-DD]]  [THH:mm [:ss [.s+]] [Z | ±HH:mm]]
//!         ("-000000" is not a year; a date-only form is UTC, a date-time form without an
//!          offset is local time; T24:00 is the end of the day)
//! legacy  [Www[,]]  (Mmm DD Y | DD Mmm Y | M/D/Y | Y/M/D)
//!         [HH:mm[:ss]]  [GMT[±HHMM] | UTC | Z]  [(text)]
//!         (no GMT: local time)
//! slash   M/D/Y: M and D 1-2 digits, Y 1-6 digits; Y/M/D: Y 3-6 digits, M and D 1-2
//!         digits. A year below 100 is 2000 + Y (0-49) or 1900 + Y (50-99), as V8's
//!         DayComposer. M is 1-12 and D is 1-31; a D past the month end runs on into the
//!         next month (MakeDay), as in V8.
//! ```
//!
//! §21.4.3.2 lets a string that is not in the Date Time String Format fall back to
//! "implementation-specific heuristics or implementation-specific date formats". The slash
//! forms are the ones the web uses most (JetStream validatorjs: `isBefore` with
//! "08/04/2011"); every slash string this parser takes gives the value that node gives.

use super::math::{make_date, make_day, make_time};

/// A parsed date: a UTC time value, or a local time value that the caller converts with
/// UTC(t) (§21.4.1.26). Either may be out of range; the caller applies TimeClip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Parsed {
    Utc(f64),
    Local(f64),
}

/// Parses `s`: `None` if it has none of the forms above.
pub fn parse(s: &str) -> Option<Parsed> {
    parse_iso(s.as_bytes()).or_else(|| parse_legacy(s))
}

/// A cursor over ASCII bytes.
struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// Exactly `n` decimal digits.
    fn digits(&mut self, n: usize) -> Option<f64> {
        let mut v = 0.0;
        for _ in 0..n {
            let c = self.peek().filter(u8::is_ascii_digit)?;
            v = v * 10.0 + f64::from(c - b'0');
            self.i += 1;
        }
        Some(v)
    }

    fn at_end(&self) -> bool {
        self.i == self.b.len()
    }
}

/// The Date Time String Format (§21.4.1.32).
fn parse_iso(b: &[u8]) -> Option<Parsed> {
    let mut c = Cursor { b, i: 0 };
    // The year: four digits, or a sign and six digits (§21.4.1.32.1); -000000 is not a year.
    let year = match c.peek()? {
        sign @ (b'+' | b'-') => {
            c.i += 1;
            let y = c.digits(6)?;
            if sign == b'-' {
                if y == 0.0 {
                    return None;
                }
                -y
            } else {
                y
            }
        }
        _ => c.digits(4)?,
    };
    let (mut month, mut day) = (1.0, 1.0);
    if c.eat(b'-') {
        month = c.digits(2)?;
        if c.eat(b'-') {
            day = c.digits(2)?;
        }
    }
    if !(1.0..=12.0).contains(&month) || !(1.0..=31.0).contains(&day) {
        return None;
    }
    let date = make_day(year, month - 1.0, day);
    if c.at_end() {
        // A date-only form is a UTC time.
        return Some(Parsed::Utc(make_date(date, 0.0)));
    }
    if !c.eat(b'T') {
        return None;
    }
    let hour = c.digits(2)?;
    if !c.eat(b':') {
        return None;
    }
    let min = c.digits(2)?;
    let (mut sec, mut ms) = (0.0, 0.0);
    if c.eat(b':') {
        sec = c.digits(2)?;
        if c.eat(b'.') {
            // One or more digits; the first three are the milliseconds (as V8).
            let start = c.i;
            let mut scale = 100.0;
            while let Some(d) = c.peek().filter(u8::is_ascii_digit) {
                ms += f64::from(d - b'0') * scale;
                scale /= 10.0;
                c.i += 1;
            }
            if c.i == start {
                return None;
            }
            ms = ms.trunc();
        }
    }
    if hour > 24.0 || min > 59.0 || sec > 59.0 {
        return None;
    }
    // 24:00 is the end of the day: only with zero minutes, seconds and ms.
    if hour == 24.0 && (min != 0.0 || sec != 0.0 || ms != 0.0) {
        return None;
    }
    let tv = make_date(date, make_time(hour, min, sec, ms));
    if c.at_end() {
        // A date-time form without an offset is a local time.
        return Some(Parsed::Local(tv));
    }
    let offset = match c.peek()? {
        b'Z' => {
            c.i += 1;
            0.0
        }
        sign @ (b'+' | b'-') => {
            c.i += 1;
            let oh = c.digits(2)?;
            if !c.eat(b':') {
                return None;
            }
            let om = c.digits(2)?;
            if oh > 23.0 || om > 59.0 {
                return None;
            }
            let o = make_time(oh, om, 0.0, 0.0);
            if sign == b'-' { -o } else { o }
        }
        _ => return None,
    };
    if !c.at_end() {
        return None;
    }
    Some(Parsed::Utc(tv - offset))
}

/// The index of `w` in `names`, ignoring ASCII case.
fn name_index(names: &[&str], w: &str) -> Option<usize> {
    names.iter().position(|n| n.eq_ignore_ascii_case(w))
}

/// A run of 1-`max` decimal digits as a number.
fn number(w: &str, max: usize) -> Option<f64> {
    if w.is_empty() || w.len() > max || !w.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    w.parse().ok()
}

/// The forms of `toString`, `toDateString` and `toUTCString` (§21.4.4.41, §21.4.4.43), with
/// an optional `(text)` tail.
fn parse_legacy(s: &str) -> Option<Parsed> {
    use super::format::{DAY_NAMES, MONTH_NAMES};
    let blanked;
    let s = match s.find('(') {
        // A slash date: each "(...)" is a space (V8 SkipParentheses), so the words after a
        // comment are still read and checked.
        Some(i) if s[..i].contains('/') => {
            blanked = blank_comments(s)?;
            blanked.as_str()
        }
        // Drop a "(time zone name)" tail.
        Some(i) => {
            if !s.ends_with(')') {
                return None;
            }
            &s[..i]
        }
        None => s,
    };
    let mut words = s.split(' ').filter(|w| !w.is_empty()).peekable();
    // An optional weekday name (not checked against the date, as in other engines).
    if let Some(w) = words.peek() {
        let name = w.strip_suffix(',').unwrap_or(w);
        if name_index(&DAY_NAMES, name).is_some() {
            words.next();
        }
    }
    let a = words.next()?;
    let date = if a.contains('/') {
        // "M/D/Y" or "Y/M/D" (V8's legacy parser).
        slash_date(a)?
    } else {
        // "Mmm DD" (toString) or "DD Mmm" (toUTCString).
        let b = words.next()?;
        let (month, day) = match name_index(&MONTH_NAMES, a) {
            Some(m) => (m, number(b, 2)?),
            None => (name_index(&MONTH_NAMES, b)?, number(a, 2)?),
        };
        if !(1.0..=31.0).contains(&day) {
            return None;
        }
        // The year: an optional '-' and digits.
        let y = words.next()?;
        let year = match y.strip_prefix('-') {
            Some(digits) => -number(digits, 6)?,
            None => number(y, 6)?,
        };
        make_day(year, month as f64, day)
    };
    let mut time = None;
    let mut offset = None;
    for w in words {
        if let Some(rest) = w.strip_prefix("GMT").or_else(|| w.strip_prefix("UTC")) {
            if offset.is_some() {
                return None;
            }
            offset = Some(if rest.is_empty() {
                0.0
            } else {
                hhmm_offset(rest)?
            });
        } else if w == "Z" && offset.is_none() {
            offset = Some(0.0);
        } else if w.contains(':') && time.is_none() && offset.is_none() {
            let mut parts = w.split(':');
            let h = number(parts.next()?, 2)?;
            let m = number(parts.next()?, 2)?;
            let sec = match parts.next() {
                Some(p) => number(p, 2)?,
                None => 0.0,
            };
            if parts.next().is_some() || h > 24.0 || m > 59.0 || sec > 59.0 {
                return None;
            }
            // 24:00 is the end of the day; V8 gives NaN for 24:30 and 24:00:01.
            if h == 24.0 && (m != 0.0 || sec != 0.0) {
                return None;
            }
            time = Some(make_time(h, m, sec, 0.0));
        } else {
            return None;
        }
    }
    let tv = make_date(date, time.unwrap_or(0.0));
    Some(match offset {
        Some(o) => Parsed::Utc(tv - o),
        None => Parsed::Local(tv),
    })
}

/// `s` with each balanced "(...)" (nested too) as one space; `None` if a parenthesis is not
/// balanced (NaN; V8 reads an open one to the end).
fn blank_comments(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' => {
                if depth == 0 {
                    out.push(' ');
                }
                depth += 1;
            }
            ')' => depth = depth.checked_sub(1)?,
            _ if depth > 0 => {}
            _ => out.push(c),
        }
    }
    (depth == 0).then_some(out)
}

/// A numeric slash date as a day number (MakeDay): "Y/M/D" when the first value is 0 or above
/// 31, else "M/D/Y" (V8 DayComposer::Write, by value: "001/02/03" is M/D/Y). The first field
/// has 1-6 digits, M and D 1-2, Y 1-6. Exactly three fields, each only ASCII digits, so an
/// empty field ("8//4/2011"), a sign or a fourth field gives `None`. V8 takes more forms
/// ("8/4", "08/004/2011", "8/4/2011/"); this function gives `None` for them (NaN), never a
/// value that differs from V8's.
fn slash_date(w: &str) -> Option<f64> {
    let mut parts = w.split('/');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let first = number(a, 6)?;
    let (year, month, day) = if !(1.0..=31.0).contains(&first) {
        (first, number(b, 2)?, number(c, 2)?)
    } else {
        (number(c, 6)?, first, number(b, 2)?)
    };
    if !(1.0..=12.0).contains(&month) || !(1.0..=31.0).contains(&day) {
        return None;
    }
    // V8 DayComposer::Write: a year of 0-49 is 20YY, 50-99 is 19YY (by value, so "0099" too).
    let year = if year < 50.0 {
        year + 2000.0
    } else if year < 100.0 {
        year + 1900.0
    } else {
        year
    };
    Some(make_day(year, month - 1.0, day))
}

/// "+HHMM" or "-HHMM" as milliseconds.
fn hhmm_offset(w: &str) -> Option<f64> {
    let (sign, digits) = match w.as_bytes().first()? {
        b'+' => (1.0, &w[1..]),
        b'-' => (-1.0, &w[1..]),
        _ => return None,
    };
    // Four ASCII digits: the slices below then cut at char boundaries (a 4-byte word with a
    // multibyte char in it would panic there).
    if digits.len() != 4 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h = number(&digits[..2], 2)?;
    let m = number(&digits[2..], 2)?;
    if h > 23.0 || m > 59.0 {
        return None;
    }
    Some(sign * make_time(h, m, 0.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::format::{date_string, iso_string, time_string, time_zone_string, utc_string};
    use crate::date::math::MAX_TIME;

    fn utc(s: &str) -> Option<f64> {
        match parse(s)? {
            Parsed::Utc(t) => Some(t),
            Parsed::Local(_) => panic!("{s}: a local time"),
        }
    }

    #[test]
    fn iso_forms() {
        assert_eq!(utc("1970-01-01T00:00:00.000Z"), Some(0.0));
        assert_eq!(utc("1970"), Some(0.0));
        assert_eq!(utc("1970-02"), Some(31.0 * 86_400_000.0));
        assert_eq!(utc("2021-01-01T10:00+01:00"), Some(1_609_491_600_000.0));
        assert_eq!(utc("2021-01-01T24:00Z"), Some(1_609_545_600_000.0));
        assert_eq!(
            utc("2021-01-01T00:00:00.123456Z"),
            Some(1_609_459_200_123.0)
        );
        assert_eq!(utc("2021-01-01T00:00:00.5Z"), Some(1_609_459_200_500.0));
        assert_eq!(utc("+275760-09-13T00:00:00.000Z"), Some(MAX_TIME));
        assert_eq!(utc("-271821-04-20T00:00:00.000Z"), Some(-MAX_TIME));
        assert_eq!(parse("1970-01-01T00:00"), Some(Parsed::Local(0.0)));
        for bad in [
            "-000000-03-31T00:45Z",
            "-000000-03-31T01:45",
            "2021-13-01",
            "2021-00-01",
            "2021-01-00",
            "2021-01-01T24:00:01Z",
            "2021-01-01T10:60Z",
            "2021-01-01T10:00:60Z",
            "2021-01-01T00:00:00.Z",
            "2021-01-01T00:00:00.000+24:00",
            "2021-01-01T",
            "2021-1-01",
            "202",
            "",
            "x",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn legacy_forms_round_trip() {
        for t in [
            0.0,
            -1000.0,
            1.6e12,
            MAX_TIME,
            -MAX_TIME,
            -62_198_755_200_000.0,
        ] {
            // toUTCString.
            assert_eq!(utc(&utc_string(t)), Some(t), "{}", utc_string(t));
            // toString in a zone at +05:30 (local = t + offset).
            let o = 19_800_000.0;
            let local = t + o;
            let s = format!(
                "{} {}{} (Some Zone)",
                date_string(local),
                time_string(local),
                time_zone_string(o)
            );
            assert_eq!(utc(&s), Some(t), "{s}");
            // toDateString: local midnight.
            let d = date_string(t);
            assert!(matches!(parse(&d), Some(Parsed::Local(_))), "{d}");
        }
        assert_eq!(utc(&iso_string(1.6e12)), Some(1.6e12));
        assert_eq!(utc("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0.0));
        assert_eq!(utc("Thu Jan 01 1970 07:30:00 GMT+0730"), Some(0.0));
        assert_eq!(parse("Thu Jan 01 1970 07:30:00 GMT+0730 (x"), None);
        assert_eq!(parse("Jan 01 2011 24:30"), None);
        assert_eq!(parse("Jan 01 2011 24:00:01"), None);
        assert_eq!(parse("Jan 01 1970 07:30:00 PST"), None);
        assert_eq!(parse("Foo Jan 01 1970"), None);
        // A 4-byte offset word with a multibyte char in it (no panic; node gives NaN).
        for s in [
            "Jan 01 1970 GMT+1\u{20ac}",
            "Jan 01 1970 GMT+\u{20ac}1",
            "Jan 01 1970 UTC-1\u{e9}1",
            "Jan 01 1970 GMT+1\u{fffd}",
            "Jan 01 1970 GMT+1\u{800}",
        ] {
            assert_eq!(parse(s), None, "{s}");
        }
    }

    /// The slash forms (route JS3-VALIDATOR). Each value is node's (v26, `TZ=UTC`).
    #[test]
    fn slash_forms() {
        let day = |y: f64, m: f64, d: f64| make_date(make_day(y, m - 1.0, d), 0.0);
        let local = |s: &str| match parse(s) {
            Some(Parsed::Local(t)) => t,
            other => panic!("{s}: {other:?}"),
        };
        assert_eq!(local("08/04/2011"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("8/4/2011"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("Thu 08/04/2011"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("Thu, 08/04/2011"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("  08/04/2011  "), day(2011.0, 8.0, 4.0));
        assert_eq!(local("08/04/2011 (x)"), day(2011.0, 8.0, 4.0));
        // Each "(...)" is a space (V8 SkipParentheses): the words after it are read.
        assert_eq!(
            local("8/4/2011 (x) 12:00 (y)"),
            day(2011.0, 8.0, 4.0) + 43_200_000.0
        );
        assert_eq!(utc("8/4/2011 (a(b)) GMT"), Some(day(2011.0, 8.0, 4.0)));
        assert_eq!(local("2011/08/04"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("2011/8/4"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("08/04/11"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("8/4/99"), day(1999.0, 8.0, 4.0));
        assert_eq!(local("8/4/49"), day(2049.0, 8.0, 4.0));
        assert_eq!(local("8/4/50"), day(1950.0, 8.0, 4.0));
        assert_eq!(local("8/4/0"), day(2000.0, 8.0, 4.0));
        assert_eq!(local("1/1/0099"), day(1999.0, 1.0, 1.0));
        assert_eq!(local("0099/1/1"), day(1999.0, 1.0, 1.0));
        assert_eq!(local("1/1/100"), day(100.0, 1.0, 1.0));
        assert_eq!(local("100/1/1"), day(100.0, 1.0, 1.0));
        assert_eq!(local("08/04/12345"), day(12345.0, 8.0, 4.0));
        // A day past the month end runs on (MakeDay), as in V8.
        assert_eq!(local("02/30/2011"), day(2011.0, 3.0, 2.0));
        // Y/M/D when the first value is not a day (0 or above 31), by value: leading zeros do
        // not count (V8 DayComposer::Write; node's values).
        assert_eq!(local("001/02/03"), day(2003.0, 1.0, 2.0));
        assert_eq!(local("008/04/2011"), day(2011.0, 8.0, 4.0));
        assert_eq!(local("32/1/1"), day(2032.0, 1.0, 1.0));
        assert_eq!(local("99/1/1"), day(1999.0, 1.0, 1.0));
        assert_eq!(local("0/1/1"), day(2000.0, 1.0, 1.0));
        assert_eq!(
            local("8/4/2011 1:2:3"),
            day(2011.0, 8.0, 4.0) + make_time(1.0, 2.0, 3.0, 0.0)
        );
        assert_eq!(local("08/04/2011 24:00"), day(2011.0, 8.0, 5.0));
        assert_eq!(
            utc("08/04/2011 10:00 GMT"),
            Some(day(2011.0, 8.0, 4.0) + make_time(10.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(
            utc("08/04/2011 GMT+0200"),
            Some(day(2011.0, 8.0, 3.0) + make_time(22.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(
            utc("12/31/1969 23:59:59 GMT-0100"),
            Some(make_time(0.0, 59.0, 59.0, 0.0))
        );
        assert_eq!(utc("1/1/1970 00:00 UTC"), Some(0.0));
        assert_eq!(
            utc("8/4/2011 12:00:00 Z"),
            Some(day(2011.0, 8.0, 4.0) + 43_200_000.0)
        );
        // node gives NaN for these.
        for bad in [
            "13/04/2011",
            "08/32/2011",
            "00/04/2011",
            "08/00/2011",
            "2011/13/01",
            "08/04/-2011",
            "1/+1/2011",
            "1/1/1e3",
            "8/4/2011 24:30",
            "8/4/2011 10:60",
            "8/4/2011 10:00:60",
            "8/4/2011 10:00 10:00",
            "8/4/2011 Thu",
            "\u{ff11}/1/2011",
            "013/01/01",
            "8/4/2011 (x) 99 (y)",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        // node gives a value for these; this parser does not take them (NaN, a subset).
        for gap in ["08/04", "08/04/2011/", "08//04/2011", "08/004/2011"] {
            assert_eq!(parse(gap), None, "{gap}");
        }
    }
}
