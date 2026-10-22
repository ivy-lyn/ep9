//! The date math of ECMA-262 §21.4.1 over f64 time values (milliseconds since
//! 1970-01-01T00:00:00Z), with no Value and no time zone: Day, TimeWithinDay, DayFromYear,
//! YearFromTime, MonthFromTime, DateFromTime, WeekDay, the hour/minute/second/ms parts,
//! MakeTime, MakeDay, MakeDate, MakeFullYear and TimeClip.
//!
//! Every function takes the spec's mathematical values as f64. A NaN in gives a NaN out where
//! the spec says so (MakeTime, MakeDay, MakeDate, TimeClip); the field functions take a finite
//! time value (callers test NaN first).

pub const MS_PER_SECOND: f64 = 1000.0;
pub const MS_PER_MINUTE: f64 = 60_000.0;
pub const MS_PER_HOUR: f64 = 3_600_000.0;
pub const MS_PER_DAY: f64 = 86_400_000.0;

/// The largest time value magnitude (§21.4.1.1): 100,000,000 days.
pub const MAX_TIME: f64 = 8.64e15;

/// MakeDay accepts years in this range only (as V8 does: a wider year cannot give a time
/// value in range, and the day arithmetic stays exact).
const MAX_YEAR: f64 = 1_000_000.0;

/// x modulo y with the sign of y (the spec's "modulo"). `ep9_math::fmod`, not float `%`
/// (H36: `%` lowers to the C `fmod`).
fn modulo(x: f64, y: f64) -> f64 {
    let r = ep9_math::fmod(x, y);
    if r < 0.0 { r + y } else { r + 0.0 }
}

/// Day(t) (§21.4.1.3).
pub fn day(t: f64) -> f64 {
    (t / MS_PER_DAY).floor()
}

/// TimeWithinDay(t) (§21.4.1.3).
pub fn time_within_day(t: f64) -> f64 {
    modulo(t, MS_PER_DAY)
}

/// DaysInYear(y) (§21.4.1.4).
pub fn days_in_year(y: f64) -> f64 {
    if ep9_math::fmod(y, 4.0) != 0.0 {
        365.0
    } else if ep9_math::fmod(y, 100.0) != 0.0 {
        366.0
    } else if ep9_math::fmod(y, 400.0) != 0.0 {
        365.0
    } else {
        366.0
    }
}

/// DayFromYear(y) (§21.4.1.5).
pub fn day_from_year(y: f64) -> f64 {
    365.0 * (y - 1970.0) + ((y - 1969.0) / 4.0).floor() - ((y - 1901.0) / 100.0).floor()
        + ((y - 1601.0) / 400.0).floor()
}

/// YearFromTime(t) (§21.4.1.7): the largest integral y with TimeFromYear(y) ≤ t.
pub fn year_from_time(t: f64) -> f64 {
    let d = day(t);
    let mut y = (d / 365.2425).floor() + 1970.0;
    while day_from_year(y) > d {
        y -= 1.0;
    }
    while day_from_year(y + 1.0) <= d {
        y += 1.0;
    }
    y
}

/// DayWithinYear(t) (§21.4.1.8).
pub fn day_within_year(t: f64) -> f64 {
    day(t) - day_from_year(year_from_time(t))
}

/// InLeapYear(t) (§21.4.1.9), as 0 or 1.
fn in_leap_year(t: f64) -> f64 {
    days_in_year(year_from_time(t)) - 365.0
}

/// The first day within the year of each month (non-leap year), and of the next year.
const MONTH_START: [f64; 13] = [
    0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0, 365.0,
];

/// The first day within the year of month `m` (0-11), with `leap` (0 or 1).
fn month_start(m: usize, leap: f64) -> f64 {
    MONTH_START[m] + if m >= 2 { leap } else { 0.0 }
}

/// MonthFromTime(t) (§21.4.1.11), 0-11.
pub fn month_from_time(t: f64) -> f64 {
    let d = day_within_year(t);
    let leap = in_leap_year(t);
    let mut m = 0;
    while m < 11 && d >= month_start(m + 1, leap) {
        m += 1;
    }
    m as f64
}

/// DateFromTime(t) (§21.4.1.12), 1-31.
pub fn date_from_time(t: f64) -> f64 {
    let m = month_from_time(t) as usize;
    day_within_year(t) - month_start(m, in_leap_year(t)) + 1.0
}

/// WeekDay(t) (§21.4.1.13), 0 (Sunday) - 6.
pub fn week_day(t: f64) -> f64 {
    modulo(day(t) + 4.0, 7.0)
}

/// HourFromTime(t) (§21.4.1.14).
pub fn hour_from_time(t: f64) -> f64 {
    modulo((t / MS_PER_HOUR).floor(), 24.0)
}

/// MinFromTime(t) (§21.4.1.15).
pub fn min_from_time(t: f64) -> f64 {
    modulo((t / MS_PER_MINUTE).floor(), 60.0)
}

/// SecFromTime(t) (§21.4.1.16).
pub fn sec_from_time(t: f64) -> f64 {
    modulo((t / MS_PER_SECOND).floor(), 60.0)
}

/// msFromTime(t) (§21.4.1.17).
pub fn ms_from_time(t: f64) -> f64 {
    modulo(t, MS_PER_SECOND)
}

/// ToIntegerOrInfinity of a number (§7.1.5): NaN gives 0 (callers test finiteness first).
fn to_integer(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.trunc() + 0.0 }
}

/// MakeTime(hour, min, sec, ms) (§21.4.1.27).
pub fn make_time(hour: f64, min: f64, sec: f64, ms: f64) -> f64 {
    // 1. If hour, min, sec or ms is not finite, return NaN.
    if !(hour.is_finite() && min.is_finite() && sec.is_finite() && ms.is_finite()) {
        return f64::NAN;
    }
    // 2-5. h, m, s, milli = ToIntegerOrInfinity of each.
    let (h, m, s, milli) = (
        to_integer(hour),
        to_integer(min),
        to_integer(sec),
        to_integer(ms),
    );
    // 6. Return ((h × msPerHour + m × msPerMinute) + s × msPerSecond) + milli, in IEEE 754
    //    arithmetic (the ECMAScript * and +).
    ((h * MS_PER_HOUR + m * MS_PER_MINUTE) + s * MS_PER_SECOND) + milli
}

/// MakeDay(year, month, date) (§21.4.1.28).
pub fn make_day(year: f64, month: f64, date: f64) -> f64 {
    // 1. If year, month or date is not finite, return NaN.
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    // 2-4. y, m, dt = ToIntegerOrInfinity of each.
    let (y, m, dt) = (to_integer(year), to_integer(month), to_integer(date));
    // 5. Let ym be y + floor(m / 12).
    let ym = y + (m / 12.0).floor();
    // 6. If ym is not finite, return NaN. (Also NaN where no time value can be in range.)
    if !ym.is_finite() || ym.abs() > MAX_YEAR {
        return f64::NAN;
    }
    // 7. Let mn be m modulo 12.
    let mn = modulo(m, 12.0) as usize;
    // 8. Find t with YearFromTime(t) = ym, MonthFromTime(t) = mn, DateFromTime(t) = 1.
    let leap = days_in_year(ym) - 365.0;
    let first = day_from_year(ym) + month_start(mn, leap);
    // 9. Return Day(t) + dt - 1𝔽.
    first + dt - 1.0
}

/// MakeDate(day, time) (§21.4.1.29).
pub fn make_date(day: f64, time: f64) -> f64 {
    // 1. If day or time is not finite, return NaN.
    if !(day.is_finite() && time.is_finite()) {
        return f64::NAN;
    }
    // 2. Let tv be day × msPerDay + time.
    let tv = day * MS_PER_DAY + time;
    // 3. If tv is not finite, return NaN.
    if !tv.is_finite() {
        return f64::NAN;
    }
    tv
}

/// MakeFullYear(year) (§21.4.1.30): years 0-99 are 1900-1999.
pub fn make_full_year(year: f64) -> f64 {
    // 1. If year is NaN, return NaN.
    if year.is_nan() {
        return f64::NAN;
    }
    // 2. Let truncated be ToIntegerOrInfinity(year).
    let truncated = if year.is_infinite() {
        year
    } else {
        year.trunc() + 0.0
    };
    // 3. If truncated is in the inclusive interval from 0 to 99, return 1900𝔽 + truncated.
    if (0.0..=99.0).contains(&truncated) {
        return 1900.0 + truncated;
    }
    // 4. Return truncated.
    truncated
}

/// TimeClip(time) (§21.4.1.31).
pub fn time_clip(time: f64) -> f64 {
    // 1. If time is not finite, return NaN.
    // 2. If abs(time) > 8.64 × 10^15, return NaN.
    if !time.is_finite() || time.abs() > MAX_TIME {
        return f64::NAN;
    }
    // 3. Return 𝔽(! ToIntegerOrInfinity(time)) (-0 becomes +0).
    time.trunc() + 0.0
}

/// The fields of a finite time value: (year, month 0-11, date 1-31, hours, minutes, seconds,
/// ms, weekday 0-6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fields {
    pub year: f64,
    pub month: f64,
    pub date: f64,
    pub hour: f64,
    pub min: f64,
    pub sec: f64,
    pub ms: f64,
    pub week_day: f64,
}

impl Fields {
    /// The fields of the finite time value `t`.
    pub fn of(t: f64) -> Fields {
        Fields {
            year: year_from_time(t),
            month: month_from_time(t),
            date: date_from_time(t),
            hour: hour_from_time(t),
            min: min_from_time(t),
            sec: sec_from_time(t),
            ms: ms_from_time(t),
            week_day: week_day(t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_at_the_edges() {
        // -8.64e15 is -271821-04-20T00:00:00.000Z (a Tuesday); 8.64e15 is
        // +275760-09-13T00:00:00.000Z (a Saturday).
        let lo = Fields::of(-MAX_TIME);
        assert_eq!(
            (lo.year, lo.month, lo.date, lo.week_day),
            (-271_821.0, 3.0, 20.0, 2.0)
        );
        let hi = Fields::of(MAX_TIME);
        assert_eq!(
            (hi.year, hi.month, hi.date, hi.week_day),
            (275_760.0, 8.0, 13.0, 6.0)
        );
        let e = Fields::of(-1.0);
        assert_eq!(
            (e.year, e.month, e.date, e.hour, e.min, e.sec, e.ms),
            (1969.0, 11.0, 31.0, 23.0, 59.0, 59.0, 999.0)
        );
    }

    #[test]
    fn make_day_round_trips_every_day_of_400_years() {
        let mut d = day_from_year(1600.0);
        for y in 1600..2000 {
            for m in 0..12 {
                let n = if m == 1 {
                    days_in_year(f64::from(y)) - 337.0
                } else {
                    [
                        31.0, 0.0, 31.0, 30.0, 31.0, 30.0, 31.0, 31.0, 30.0, 31.0, 30.0, 31.0,
                    ][m]
                };
                for dt in 1..=(n as i32) {
                    assert_eq!(make_day(f64::from(y), m as f64, f64::from(dt)), d);
                    let f = Fields::of(d * MS_PER_DAY);
                    assert_eq!(
                        (f.year, f.month, f.date),
                        (f64::from(y), m as f64, f64::from(dt))
                    );
                    d += 1.0;
                }
            }
        }
    }

    #[test]
    fn make_day_month_overflow_and_nan() {
        assert_eq!(make_day(2020.0, 12.0, 1.0), make_day(2021.0, 0.0, 1.0));
        assert_eq!(make_day(2020.0, -1.0, 1.0), make_day(2019.0, 11.0, 1.0));
        assert_eq!(make_day(2020.0, 1.0, 30.0), make_day(2020.0, 2.0, 1.0));
        assert!(make_day(f64::INFINITY, 0.0, 1.0).is_nan());
        assert!(make_day(1e300, 0.0, 1.0).is_nan());
        assert!(make_day(2020.0, f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn time_clip_edges() {
        assert_eq!(time_clip(MAX_TIME), MAX_TIME);
        assert_eq!(time_clip(-MAX_TIME), -MAX_TIME);
        assert!(time_clip(MAX_TIME + 1.0).is_nan());
        assert!(time_clip(-MAX_TIME - 1.0).is_nan());
        assert!(time_clip(f64::NAN).is_nan());
        assert!(time_clip(f64::INFINITY).is_nan());
        let z = time_clip(-0.5);
        assert!(z == 0.0 && z.is_sign_positive());
        assert_eq!(time_clip(1.9), 1.0);
        assert_eq!(time_clip(-1.9), -1.0);
    }

    #[test]
    fn make_time_and_full_year() {
        assert_eq!(make_time(1.0, 2.0, 3.0, 4.5), 3_723_004.0);
        assert!(make_time(f64::NAN, 0.0, 0.0, 0.0).is_nan());
        assert_eq!(make_full_year(99.9), 1999.0);
        assert_eq!(make_full_year(100.0), 100.0);
        assert_eq!(make_full_year(-0.5), 1900.0);
        assert!(make_full_year(f64::NAN).is_nan());
    }
}
