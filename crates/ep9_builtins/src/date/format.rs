//! The string forms of a time value (ECMA-262 §21.4.4.35-41, §21.4.4.43), as Rust text. The
//! caller gives the local time (t + LocalTZA) and the offset; this module has no time zone.
//!
//! ```text
//! date_string(t)         "Thu Jan 01 1970"                    DateString (§21.4.4.41.2)
//! time_string(t)         "07:30:00 GMT"                       TimeString (§21.4.4.41.1)
//! time_zone_string(o)    "+0730"                              TimeZoneString (§21.4.4.41.3), empty tzName
//! utc_string(t)          "Thu, 01 Jan 1970 00:00:00 GMT"      toUTCString (§21.4.4.43)
//! iso_string(t)          "1970-01-01T00:00:00.000Z"           toISOString (§21.4.4.36), Date Time String Format
//!                        "+275760-09-13T00:00:00.000Z"        (expanded years outside 0-9999)
//! ```

use super::math::{Fields, hour_from_time, min_from_time};

/// The day names of Table 63 (§21.4.4.41.2).
pub const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The month names of Table 64 (§21.4.4.41.2).
pub const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// yearSign + ToZeroPaddedDecimalString(abs(yv), 4).
fn year_text(y: f64) -> String {
    let sign = if y < 0.0 { "-" } else { "" };
    format!("{sign}{:04}", y.abs() as u64)
}

/// DateString(tv) (§21.4.4.41.2) of the finite (local) time value `t`.
pub fn date_string(t: f64) -> String {
    let f = Fields::of(t);
    format!(
        "{} {} {:02} {}",
        DAY_NAMES[f.week_day as usize],
        MONTH_NAMES[f.month as usize],
        f.date as u32,
        year_text(f.year)
    )
}

/// TimeString(tv) (§21.4.4.41.1) of the finite (local) time value `t`.
pub fn time_string(t: f64) -> String {
    let f = Fields::of(t);
    format!(
        "{:02}:{:02}:{:02} GMT",
        f.hour as u32, f.min as u32, f.sec as u32
    )
}

/// TimeZoneString(tv) (§21.4.4.41.3) for the offset `offset_ms`, with an empty tzName (the
/// spec allows it; a time-zone name needs ECMA-402).
pub fn time_zone_string(offset_ms: f64) -> String {
    let sign = if offset_ms >= 0.0 { '+' } else { '-' };
    let abs = offset_ms.abs();
    format!(
        "{sign}{:02}{:02}",
        hour_from_time(abs) as u32,
        min_from_time(abs) as u32
    )
}

/// The toUTCString text (§21.4.4.43) of the finite time value `t`.
pub fn utc_string(t: f64) -> String {
    let f = Fields::of(t);
    format!(
        "{}, {:02} {} {} {}",
        DAY_NAMES[f.week_day as usize],
        f.date as u32,
        MONTH_NAMES[f.month as usize],
        year_text(f.year),
        time_string(t)
    )
}

/// The toISOString text (§21.4.4.36) of the finite time value `t` (in range).
pub fn iso_string(t: f64) -> String {
    let f = Fields::of(t);
    let year = if (0.0..=9999.0).contains(&f.year) {
        format!("{:04}", f.year as u32)
    } else {
        let sign = if f.year < 0.0 { '-' } else { '+' };
        format!("{sign}{:06}", f.year.abs() as u32)
    };
    format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        f.month as u32 + 1,
        f.date as u32,
        f.hour as u32,
        f.min as u32,
        f.sec as u32,
        f.ms as u32
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::math::MAX_TIME;

    #[test]
    fn strings_of_known_values() {
        assert_eq!(date_string(0.0), "Thu Jan 01 1970");
        assert_eq!(time_string(27_000_000.0), "07:30:00 GMT");
        assert_eq!(time_zone_string(27_000_000.0), "+0730");
        assert_eq!(time_zone_string(-18_000_000.0), "-0500");
        assert_eq!(time_zone_string(0.0), "+0000");
        assert_eq!(time_zone_string(24_925_000.0), "+0655");
        assert_eq!(utc_string(0.0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(iso_string(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_string(MAX_TIME), "+275760-09-13T00:00:00.000Z");
        assert_eq!(iso_string(-MAX_TIME), "-271821-04-20T00:00:00.000Z");
        assert_eq!(
            iso_string(-62_198_755_200_000.0),
            "-000001-01-01T00:00:00.000Z"
        );
        assert_eq!(
            utc_string(-62_198_755_200_000.0),
            "Fri, 01 Jan -0001 00:00:00 GMT"
        );
        assert_eq!(date_string(MAX_TIME), "Sat Sep 13 275760");
        assert_eq!(iso_string(-1.0), "1969-12-31T23:59:59.999Z");
    }
}
