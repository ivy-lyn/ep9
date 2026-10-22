#![allow(dead_code)] // m3-stub
//! The clock and the local time zone on the builtin ABI (owner M3-P1 B-DATE; V8-C2): builtins
//! reach the platform only through ep9_vm (C20). M3-P0 stubs: the epoch and UTC.

use super::Cx;

impl<'s> Cx<'s> {
    /// The current time: nanoseconds since the epoch (1970-01-01T00:00:00Z). Date divides by
    /// 10^6 (§21.4.1.1 time values are milliseconds); Temporal.Now can use the nanoseconds.
    /// M3-P0 stub: 0.
    pub fn now_epoch_ns(&mut self) -> i128 {
        0
    }

    /// The local time zone offset in milliseconds at the time value `t_ms` (§21.4.1.26
    /// LocalTZA): `t_ms` is a UTC time if `is_utc`, else a local time. M3-P0 stub: 0.0 (UTC).
    pub fn local_tz_offset_ms(&mut self, t_ms: f64, is_utc: bool) -> f64 {
        let _ = (t_ms, is_utc);
        0.0
    }
}
