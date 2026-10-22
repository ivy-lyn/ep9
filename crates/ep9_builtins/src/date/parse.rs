//! Date.parse (ECMA-262 §21.4.3.2) without the time zone: the Date Time String Format
//! (§21.4.1.32) and the forms that `toString`, `toDateString` and `toUTCString` produce. Any
//! other string gives `None` (NaN). V8's legacy parser (the other strings browsers accept) is
//! milestone M3 work (docs/plan/06-object-model-and-spec.md), not M0.
//!
//! ```text
//! ISO     YYYY | ±YYYYYY  [-MM [-DD]]  [THH:mm [:ss [.s+]] [Z | ±HH:mm]]
//!         ("-000000" is not a year; a date-only form is UTC, a date-time form without an
//!          offset is local time; T24:00 is the end of the day)
//! legacy  [Www[,]]  (Mmm DD | DD Mmm)  [-]Y+  [HH:mm[:ss]]  [GMT[±HHMM] | UTC | Z]  [(text)]
//!         (no GMT: local time)
//! ```

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
    // Drop a "(time zone name)" tail.
    let s = match s.find('(') {
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
    // "Mmm DD" (toString) or "DD Mmm" (toUTCString).
    let a = words.next()?;
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
    let date = make_day(year, month as f64, day);
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

/// "+HHMM" or "-HHMM" as milliseconds.
fn hhmm_offset(w: &str) -> Option<f64> {
    let (sign, digits) = match w.as_bytes().first()? {
        b'+' => (1.0, &w[1..]),
        b'-' => (-1.0, &w[1..]),
        _ => return None,
    };
    if digits.len() != 4 {
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
        assert_eq!(parse("Jan 01 1970 07:30:00 PST"), None);
        assert_eq!(parse("Foo Jan 01 1970"), None);
    }
}
