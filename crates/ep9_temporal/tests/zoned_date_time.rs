//! Temporal.ZonedDateTime (WP T8) and the toZonedDateTime* methods of Instant, PlainDate and
//! PlainDateTime, with and without GC stress (the epoch value, the time zone id String and
//! the calendar code live in four internal slots). The zone data is the vendored
//! zoneinfo64.res (tzdb 2026f).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var z = new Temporal.ZonedDateTime(1_000_000_000_123_456_789n, "Europe/Berlin");
            print(z.calendarId, z.timeZoneId, z.year, z.month, z.monthCode, z.day, z.hour, z.minute, z.second, z.millisecond, z.microsecond, z.nanosecond);
            print(z.epochMilliseconds, z.epochNanoseconds, z.offset, z.offsetNanoseconds, z.hoursInDay, z.dayOfWeek, z.inLeapYear);
            print(z.toString(), z.toJSON() === z.toString(), z.toString({ smallestUnit: "minute", offset: "never", timeZoneName: "never", calendarName: "always" }));
            print(Object.prototype.toString.call(z), Temporal.ZonedDateTime.length, new Temporal.ZonedDateTime(0n, "+05:30").toString(), new Temporal.ZonedDateTime(0n, "america/new_york").timeZoneId);"#
        ),
        lines(&[
            "iso8601 Europe/Berlin 2001 9 M09 9 3 46 40 123 456 789",
            "1000000000123 1000000000123456789 +02:00 7200000000000 24 7 false",
            "2001-09-09T03:46:40.123456789+02:00[Europe/Berlin] true 2001-09-09T03:46[u-ca=iso8601]",
            "[object Temporal.ZonedDateTime] 2 1970-01-01T05:30:00+05:30[+05:30] America/New_York",
        ])
    );
}

#[test]
fn transitions_arithmetic_and_conversions() {
    assert_eq!(
        run_both(
            r#"var z = Temporal.ZonedDateTime.from("2024-03-31T00:30+01:00[Europe/Berlin]");
            print(z.hoursInDay, z.add({ hours: 2 }), z.add({ days: 1 }), z.startOfDay(), z.getTimeZoneTransition("next"), z.getTimeZoneTransition({ direction: "previous" }));
            print(z.with({ hour: 2, minute: 30 }), z.with({ hour: 2, minute: 30 }, { disambiguation: "earlier" }), z.withPlainTime("12:00"), z.withTimeZone("UTC"), z.round("hour"));
            var a = Temporal.ZonedDateTime.from({ timeZone: "UTC", year: 2020, month: 1, day: 1 }), b = Temporal.ZonedDateTime.from("2020-03-01T12:00Z[UTC]");
            print(Temporal.ZonedDateTime.compare(a, b), a.equals("2020-01-01T00:00+00:00[UTC]"), a.until(b), b.since(a, { largestUnit: "months" }), Temporal.Duration.from("P1D").total({ unit: "hours", relativeTo: z }));
            print(a.toInstant(), a.toPlainDate(), a.toPlainTime(), a.toPlainDateTime(), new Temporal.Instant(0n).toZonedDateTimeISO("Asia/Tokyo"));
            print(Temporal.PlainDate.from("2024-03-31").toZonedDateTime("Europe/Berlin"), Temporal.PlainDate.from("2024-03-31").toZonedDateTime({ timeZone: "Europe/Berlin", plainTime: "02:30" }), Temporal.PlainDateTime.from("2024-10-27T02:30").toZonedDateTime("Europe/Berlin", { disambiguation: "later" }));
            print(Temporal.Instant.from(a), Temporal.PlainDate.from(a), Temporal.PlainTime.from(b), Temporal.PlainDateTime.from(b), Temporal.Instant.from("2020-01-01T00:00Z").toString({ timeZone: a }));"#
        ),
        lines(&[
            "23 2024-03-31T03:30:00+02:00[Europe/Berlin] 2024-04-01T00:30:00+02:00[Europe/Berlin] 2024-03-31T00:00:00+01:00[Europe/Berlin] 2024-03-31T03:00:00+02:00[Europe/Berlin] 2023-10-29T02:00:00+01:00[Europe/Berlin]",
            "2024-03-31T03:30:00+02:00[Europe/Berlin] 2024-03-31T01:30:00+01:00[Europe/Berlin] 2024-03-31T12:00:00+02:00[Europe/Berlin] 2024-03-30T23:30:00+00:00[UTC] 2024-03-31T01:00:00+01:00[Europe/Berlin]",
            "-1 true PT1452H P2MT12H 23",
            "2020-01-01T00:00:00Z 2020-01-01 00:00:00 2020-01-01T00:00:00 1970-01-01T09:00:00+09:00[Asia/Tokyo]",
            "2024-03-31T00:00:00+01:00[Europe/Berlin] 2024-03-31T03:30:00+02:00[Europe/Berlin] 2024-10-27T02:30:00+01:00[Europe/Berlin]",
            "2020-01-01T00:00:00Z 2020-01-01 12:00:00 2020-03-01T12:00:00 2020-01-01T00:00:00+00:00",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.ZonedDateTime(0n, "UTC"));                       // no new
            t(() => new Temporal.ZonedDateTime(0, "UTC"));                    // not a BigInt
            t(() => new Temporal.ZonedDateTime(0n));                          // no time zone
            t(() => new Temporal.ZonedDateTime(0n, "Mars/Olympus_Mons"));     // unknown zone
            t(() => new Temporal.ZonedDateTime(0n, "2020-01-01T00:00Z[UTC]")); // not an identifier
            t(() => new Temporal.ZonedDateTime(0n, "UTC", "gregory"));        // only iso8601
            t(() => Temporal.ZonedDateTime.from("2020-01-01T00:00"));         // no time zone
            t(() => Temporal.ZonedDateTime.from({ year: 2020, month: 1, day: 1 })); // no timeZone
            t(() => Temporal.ZonedDateTime.from("2020-01-01T00:00+01:00[UTC]")); // offset mismatch
            t(() => Temporal.ZonedDateTime.from("2024-03-31T02:30[Europe/Berlin]", { disambiguation: "reject" }));
            t(() => new Temporal.ZonedDateTime(0n, "UTC").getTimeZoneTransition());
            t(() => new Temporal.ZonedDateTime(0n, "UTC").getTimeZoneTransition({}));
            t(() => new Temporal.ZonedDateTime(0n, "UTC").with({ timeZone: "UTC" }));
            t(() => new Temporal.ZonedDateTime(0n, "UTC").valueOf());
            t(() => Temporal.ZonedDateTime.prototype.year);                   // not a ZonedDateTime
            t(() => new Temporal.Instant(0n).toZonedDateTimeISO());           // no time zone
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError TypeError TypeError RangeError RangeError RangeError RangeError TypeError RangeError RangeError TypeError RangeError TypeError TypeError TypeError TypeError"
        ])
    );
}
