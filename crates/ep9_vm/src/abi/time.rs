//! The clock and the local time zone on the builtin ABI (owner M3-P1 B-DATE; V8-C2): builtins
//! reach the platform only through ep9_vm (C20).
//!
//! ```text
//! Cx::now_epoch_ns()               -> ep9_platform::time::now_epoch_ns()   (SystemTime)
//! Cx::local_tz_offset_ms(t, utc)   -> ep9_platform::time::local_offset_ms  (localtime_r /
//!                                     _localtime64_s; LocalTZA, §21.4.1.25)
//! Cx::system_time_zone_id()        -> ep9_platform::time::system_time_zone_id  (TZ or the
//!                                     /etc/localtime link; Temporal.Now)
//! ```
//!
//! This is the seam of docs/plan/06-object-model-and-spec.md "Same tz provider as Temporal":
//! when `ep9_temporal` exists, the body of `local_tz_offset_ms` calls its tz provider, and the
//! signature stays (H20 part 2 is append-only).

use super::Cx;

impl<'s> Cx<'s> {
    /// The current time: nanoseconds since the epoch (1970-01-01T00:00:00Z). Date divides by
    /// 10^6 (§21.4.1.1 time values are milliseconds); Temporal.Now can use the nanoseconds.
    pub fn now_epoch_ns(&mut self) -> i128 {
        ep9_platform::time::now_epoch_ns()
    }

    /// The local time zone offset in milliseconds at the time value `t_ms` (§21.4.1.25
    /// LocalTZA): `t_ms` is a UTC time if `is_utc`, else a local time. A local time in a
    /// repeated hour gives the offset before the change, and so does a local time in a
    /// skipped hour (the spec's rule). NaN or an infinity gives 0.
    pub fn local_tz_offset_ms(&mut self, t_ms: f64, is_utc: bool) -> f64 {
        ep9_platform::time::local_offset_ms(t_ms, is_utc)
    }

    /// The host time zone identifier (the host part of SystemTimeZoneIdentifier, Temporal
    /// §11.1.?): an IANA name, or `None` when the host gives none. The caller (Temporal.Now)
    /// checks it with its tz data and uses "UTC" for `None` or an unknown name.
    pub fn system_time_zone_id(&mut self) -> Option<String> {
        ep9_platform::time::system_time_zone_id()
    }
}
