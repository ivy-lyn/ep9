//! The wall clock and the local time zone, for Date (ECMA-262 §21.4) through `ep9_vm::Cx`
//! (C20: builtins reach the OS only through the VM).
//!
//! ```text
//! now_epoch_ns()               SystemTime::now() - UNIX_EPOCH, in ns (negative before 1970)
//! local_offset_ms(t, true)     the offset at the UTC time t: localtime of t, then
//!                                unix     tm_gmtoff (localtime_r)
//!                                windows  _mkgmtime64(_localtime64_s(t)) - t (C runtime)
//! local_offset_ms(t, false)    t is a local time: the offset o with offset(t - o) = o
//!                              (o from the offsets one day before and after t); two
//!                              matches (a repeated hour): the earlier instant; no match
//!                              (a skipped hour): the offset before the change (§21.4.1.25)
//! a time the C runtime cannot convert (windows: before 1970 or after 3000)
//!                              -> the same day in an equivalent year (same leap rule, same
//!                                 weekday of 1 January) in 2008-2035, as V8 does
//! ```
//!
//! Offsets are in milliseconds, positive east of Greenwich. Later the body of
//! `Cx::local_tz_offset_ms` calls the tz provider that Temporal shares
//! (docs/plan/06-object-model-and-spec.md "Same tz provider as Temporal"); this module is the
//! M0 source.

use std::time::{SystemTime, UNIX_EPOCH};

const MS_PER_DAY: f64 = 86_400_000.0;

/// The current time in nanoseconds since 1970-01-01T00:00:00Z (negative if the clock is
/// before the epoch).
pub fn now_epoch_ns() -> i128 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}

/// The local time zone offset in milliseconds at the time `t_ms` (LocalTZA, ECMA-262
/// §21.4.1.25). `is_utc`: `t_ms` is a UTC time value; else it is a local time. `t_ms` is a
/// finite time value; NaN or an infinity gives 0.
pub fn local_offset_ms(t_ms: f64, is_utc: bool) -> f64 {
    if !t_ms.is_finite() {
        return 0.0;
    }
    if is_utc {
        return offset_at_utc(t_ms);
    }
    // The offsets in force one day before and one day after t (as if t were UTC). Time zone
    // offsets are under one day, and two changes are more than two days apart.
    let before = offset_at_utc(t_ms - MS_PER_DAY);
    let after = offset_at_utc(t_ms + MS_PER_DAY);
    // A candidate offset o is right if the instant t - o has the offset o.
    let fits = |o: f64| offset_at_utc(t_ms - o) == o;
    match (fits(before), fits(after)) {
        // A repeated local time: the earlier instant (the offset before the change).
        (true, true) => before.max(after),
        (true, false) => before,
        (false, true) => after,
        // A skipped local time: the offset before the change.
        (false, false) => before,
    }
}

/// The offset in milliseconds at the UTC time `t_ms` (finite). Integer day math only: float
/// `%` lowers to the C `fmod`, which engine code does not call (H36).
fn offset_at_utc(t_ms: f64) -> f64 {
    let secs = (t_ms / 1000.0).floor() as i64;
    if let Some(off) = sys_offset_secs(secs) {
        return off as f64 * 1000.0;
    }
    // The C runtime cannot convert this time: use the same day of an equivalent year.
    let year = year_from_day(secs.div_euclid(86_400));
    let shift = days_from_year(equivalent_year(year)) - days_from_year(year);
    sys_offset_secs(secs + shift * 86_400).map_or(0.0, |off| off as f64 * 1000.0)
}

/// The days from 1970-01-01 to 1 January of `y` (DayFromYear, §21.4.1.3).
fn days_from_year(y: i64) -> i64 {
    365 * (y - 1970) + (y - 1969).div_euclid(4) - (y - 1901).div_euclid(100)
        + (y - 1601).div_euclid(400)
}

/// The year that holds day number `day` (days since 1970-01-01).
fn year_from_day(day: i64) -> i64 {
    let mut y = (day * 400).div_euclid(146_097) + 1970;
    while days_from_year(y) > day {
        y -= 1;
    }
    while days_from_year(y + 1) <= day {
        y += 1;
    }
    y
}

fn is_leap(y: i64) -> bool {
    y.rem_euclid(4) == 0 && (y.rem_euclid(100) != 0 || y.rem_euclid(400) == 0)
}

/// A year in 2008-2035 with the leap rule of `y` and the weekday of its 1 January (V8's
/// `DateCache::EquivalentYear`).
fn equivalent_year(y: i64) -> i64 {
    // 1970-01-01 was a Thursday (4).
    let week_day = (days_from_year(y) + 4).rem_euclid(7);
    let recent = if is_leap(y) { 1956 } else { 1967 } + (week_day * 12) % 28;
    2008 + (recent + 3 * 28 - 2008) % 28
}

/// The offset in seconds at `secs` seconds since the epoch, or `None` when the C runtime
/// cannot convert it.
#[cfg(unix)]
fn sys_offset_secs(secs: i64) -> Option<i64> {
    let t = libc::time_t::try_from(secs).ok()?;
    // SAFETY: an all-zero `tm` is a valid value (integers and, on some targets, a null
    // `tm_zone` pointer).
    let mut tm: libc::tm = unsafe { core::mem::zeroed() };
    // SAFETY: `t` and `tm` are valid for the call; localtime_r is the thread-safe form.
    let r = unsafe { libc::localtime_r(&t, &mut tm) };
    if r.is_null() {
        return None;
    }
    // `c_long`: i64 on 64-bit targets, i32 on 32-bit ones.
    Some(tm.tm_gmtoff as i64)
}

/// `struct tm` of the Microsoft C runtime.
#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
}

#[cfg(windows)]
unsafe extern "C" {
    /// Local time of `*time` into `*tm`; 0 on success (1970-01-01 to 3000-12-31 UTC).
    fn _localtime64_s(tm: *mut Tm, time: *const i64) -> i32;
    /// The UTC time of the broken-down UTC time `*tm`; -1 on error.
    fn _mkgmtime64(tm: *mut Tm) -> i64;
}

/// The offset in seconds at `secs` seconds since the epoch, or `None` when the C runtime
/// cannot convert it.
#[cfg(windows)]
fn sys_offset_secs(secs: i64) -> Option<i64> {
    let mut tm = Tm::default();
    // SAFETY: `tm` and `secs` are valid for the call.
    if unsafe { _localtime64_s(&mut tm, &secs) } != 0 {
        return None;
    }
    // The local wall time read as a UTC time, minus the UTC time: the offset.
    // SAFETY: `tm` holds a valid broken-down time from the call above.
    let local = unsafe { _mkgmtime64(&mut tm) };
    if local == -1 {
        return None;
    }
    Some(local - secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2020_and_moves_forward() {
        let a = now_epoch_ns();
        // 2020-01-01T00:00:00Z in ns.
        assert!(a > 1_577_836_800_000_000_000);
        let b = now_epoch_ns();
        assert!(b >= a);
    }

    #[test]
    fn offsets_are_whole_seconds_under_one_day() {
        for t in [0.0, 1.6e12, -1e12, 8.64e15, -8.64e15, 4.1e12, 3.3e13] {
            for is_utc in [true, false] {
                let o = local_offset_ms(t, is_utc);
                assert!(o.abs() < MS_PER_DAY, "offset {o} at {t}");
                assert_eq!(o % 1000.0, 0.0, "offset {o} at {t}");
            }
        }
        assert_eq!(local_offset_ms(f64::NAN, true), 0.0);
        assert_eq!(local_offset_ms(f64::INFINITY, false), 0.0);
    }

    #[test]
    fn local_to_utc_inverts_utc_to_local() {
        // At every hour of 2021 (two DST changes in most zones that have DST), a local time
        // made from a UTC time maps back to it, except in a repeated hour, where the earlier
        // instant wins: then the result is one offset change earlier.
        let start = 1_609_459_200_000.0; // 2021-01-01T00:00:00Z
        for h in 0..(365 * 24) {
            let t = start + f64::from(h) * 3_600_000.0;
            let local = t + local_offset_ms(t, true);
            let back = local - local_offset_ms(local, false);
            if back != t {
                assert!(back < t, "{t}: back {back}");
                assert_eq!(local_offset_ms(back, true) + back, local);
            }
        }
    }

    #[test]
    fn equivalent_year_keeps_leap_rule_and_weekday() {
        for y in [-271_821, -1, 0, 1600, 1900, 1969, 2100, 3001, 275_760] {
            let e = equivalent_year(y);
            assert!((2008..=2035).contains(&e), "{y} -> {e}");
            assert_eq!(is_leap(y), is_leap(e), "{y} -> {e}");
            let wd = |y: i64| (days_from_year(y) + 4).rem_euclid(7);
            assert_eq!(wd(y), wd(e), "{y} -> {e}");
        }
        assert_eq!(year_from_day(-1), 1969);
        assert_eq!(year_from_day(0), 1970);
        assert_eq!(year_from_day(365), 1971);
        assert_eq!(year_from_day(-100_000_000), -271_821);
        assert_eq!(year_from_day(100_000_000), 275_760);
    }

    #[test]
    #[cfg(unix)]
    fn unix_offset_matches_localtime() {
        // tm_gmtoff for t = 0 is the offset at the epoch; the module must give the same.
        assert_eq!(
            local_offset_ms(0.0, true),
            sys_offset_secs(0).unwrap() as f64 * 1000.0
        );
    }
}
