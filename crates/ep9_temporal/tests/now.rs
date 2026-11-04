//! Temporal.Now (WP T9): the clock (`Cx::now_epoch_ns`) and the host zone
//! (`Cx::system_time_zone_id`, checked with the tz data and mapped to its primary identifier;
//! "UTC" when the host gives none or an unknown name), with and without GC stress.

mod common;
use common::{lines, run_both};

#[test]
fn now_functions_give_the_current_time() {
    assert_eq!(
        run_both(
            r#"var i = Temporal.Now.instant(), z = Temporal.Now.zonedDateTimeISO("UTC");
            print(typeof Temporal.Now.timeZoneId(), i instanceof Temporal.Instant, i.epochNanoseconds > 1_700_000_000_000_000_000n);
            print(z.timeZoneId, z.calendarId, Temporal.Instant.compare(z.toInstant(), i) >= 0, Temporal.Now.zonedDateTimeISO().timeZoneId === Temporal.Now.timeZoneId());
            print(Temporal.Now.plainDateTimeISO("+01:00") instanceof Temporal.PlainDateTime, Temporal.Now.plainDateISO() instanceof Temporal.PlainDate, Temporal.Now.plainTimeISO("Asia/Tokyo") instanceof Temporal.PlainTime);
            var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.Now.plainDateISO(1));          // not a string
            t(() => Temporal.Now.zonedDateTimeISO("Mars")); // unknown zone
            t(() => new Temporal.Now.instant());            // not a constructor
            print(r.join(" "), Object.prototype.toString.call(Temporal.Now));"#
        ),
        lines(&[
            "string true true",
            "UTC iso8601 true true",
            "true true true",
            "TypeError RangeError TypeError [object Temporal.Now]",
        ])
    );
}

/// `TZ` in a child process: a known zone is the system zone (a link gives its primary
/// identifier); an unknown one gives "UTC".
#[test]
#[cfg(unix)]
fn system_zone_from_tz_in_a_child() {
    const CHILD: &str = "EP9_TEMPORAL_TZ_CHILD";
    if let Some(want) = std::env::var_os(CHILD) {
        assert_eq!(
            run_both(
                "print(Temporal.Now.timeZoneId(), Temporal.Now.zonedDateTimeISO().timeZoneId);"
            ),
            Ok(vec![format!("{0} {0}", want.to_string_lossy())])
        );
        return;
    }
    for (tz, want) in [
        ("Asia/Tokyo", "Asia/Tokyo"),
        (":America/New_York", "America/New_York"),
        ("US/Pacific", "America/Los_Angeles"),
        ("Asia/Calcutta", "Asia/Kolkata"),
        ("Mars/Olympus_Mons", "UTC"),
        ("EST5EDT,M3.2.0,M11.1.0", "UTC"),
    ] {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["system_zone_from_tz_in_a_child", "--exact", "--nocapture"])
            .env(CHILD, want)
            .env("TZ", tz)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && text.contains("1 passed"),
            "child with TZ={tz} failed:\n{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
