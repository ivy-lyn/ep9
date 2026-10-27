//! Tests of the clock and time-zone methods (`abi/time.rs`) (owner M3-P1 B-DATE): the `Cx`
//! methods give what ep9_platform gives, the clock moves forward, and a local time maps back
//! to its UTC time.

use super::{config, isolate};
use crate::{Cx, Dispatch, Isolate};

/// Runs `f` with a builtin-ABI context of a fresh isolate.
fn with_cx<R>(f: impl FnOnce(&mut Cx<'_>) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let iso: &mut Isolate = &mut iso;
    let mut cx = Cx::new(&mut iso.cx);
    f(&mut cx)
}

#[test]
fn now_epoch_ns_is_the_platform_clock() {
    with_cx(|cx| {
        let before = ep9_platform::time::now_epoch_ns();
        let now = cx.now_epoch_ns();
        let after = ep9_platform::time::now_epoch_ns();
        assert!(before <= now && now <= after, "{before} {now} {after}");
        // After 2020-01-01T00:00:00Z.
        assert!(now > 1_577_836_800_000_000_000);
    });
}

#[test]
fn local_tz_offset_ms_is_the_platform_offset() {
    with_cx(|cx| {
        // 2021-01-15 and 2021-07-15 (UTC noon): a zone with DST has two offsets here.
        for t in [
            1_610_712_000_000.0,
            1_626_350_400_000.0,
            0.0,
            -8.64e15,
            8.64e15,
        ] {
            for is_utc in [true, false] {
                assert_eq!(
                    cx.local_tz_offset_ms(t, is_utc),
                    ep9_platform::time::local_offset_ms(t, is_utc)
                );
            }
            // LocalTime then UTC gives t back outside a repeated hour.
            let local = t + cx.local_tz_offset_ms(t, true);
            assert_eq!(local - cx.local_tz_offset_ms(local, false), t);
        }
        assert_eq!(cx.local_tz_offset_ms(f64::NAN, true), 0.0);
    });
}
