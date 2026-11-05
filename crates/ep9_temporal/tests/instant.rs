//! Temporal.Instant (WP T3) and Date.prototype.toTemporalInstant: the constructor, the
//! static functions, the getters, the methods and the error mapping, with and without GC
//! stress (the epoch value lives in two internal slots).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var i = new Temporal.Instant(1_000_000_000_123_456_789n);
            print(i.epochNanoseconds, i.epochMilliseconds, i.toString(), i.toJSON(), i.toLocaleString());
            print(new Temporal.Instant(-1n).toString(), new Temporal.Instant(-1n).epochMilliseconds, new Temporal.Instant(0n).toString());
            print(Object.prototype.toString.call(i), Temporal.Instant.length, Temporal.Instant.prototype.constructor === Temporal.Instant);
            print(i.toString({ smallestUnit: "second" }), i.toString({ fractionalSecondDigits: 4, roundingMode: "ceil" }), i.toString({ timeZone: "+01:00" }));"#
        ),
        lines(&[
            "1000000000123456789 1000000000123 2001-09-09T01:46:40.123456789Z 2001-09-09T01:46:40.123456789Z 2001-09-09T01:46:40.123456789Z",
            "1969-12-31T23:59:59.999999999Z -1 1970-01-01T00:00:00Z",
            "[object Temporal.Instant] 1 true",
            "2001-09-09T01:46:40Z 2001-09-09T01:46:40.1235Z 2001-09-09T02:46:40.123456789+01:00",
        ])
    );
}

#[test]
fn statics_and_arithmetic() {
    assert_eq!(
        run_both(
            r#"var a = Temporal.Instant.from("2020-01-01T00:00Z"), b = Temporal.Instant.fromEpochMilliseconds(1577840400000);
            print(a, b, Temporal.Instant.fromEpochNanoseconds(5n), Temporal.Instant.compare(a, b), Temporal.Instant.compare(b, a), a.equals("2020-01-01T01:00+01:00"));
            print(a.add("PT1H"), a.subtract({ minutes: 1 }), a.until(b), b.since(a, { largestUnit: "minutes" }), a.until(b, { smallestUnit: "hours" }));
            print(Temporal.Instant.from("2020-01-01T00:00:00.5Z").round("second"), Temporal.Instant.from("2020-01-01T00:29Z").round({ smallestUnit: "hour", roundingMode: "ceil" }));
            print(new Date(1577836800000).toTemporalInstant(), new Date(-1).toTemporalInstant().epochNanoseconds);"#
        ),
        lines(&[
            "2020-01-01T00:00:00Z 2020-01-01T01:00:00Z 1970-01-01T00:00:00.000000005Z -1 1 true",
            "2020-01-01T01:00:00Z 2019-12-31T23:59:00Z PT3600S PT60M PT1H",
            "2020-01-01T00:00:01Z 2020-01-01T01:00:00Z",
            "2020-01-01T00:00:00Z -1000000",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.Instant(0n));                        // no new
            t(() => new Temporal.Instant(0));                     // a Number is not a BigInt
            t(() => new Temporal.Instant(8_640_000_000_000_000_000_001n)); // out of range
            t(() => Temporal.Instant.from("2020-01-01T00:00"));   // no offset
            t(() => Temporal.Instant.from(12));                    // not a string
            t(() => Temporal.Instant.fromEpochMilliseconds(1.5)); // not integral
            t(() => Temporal.Instant.prototype.epochNanoseconds); // not an Instant
            t(() => new Temporal.Instant(0n).add({ days: 1 }));   // calendar unit
            t(() => new Temporal.Instant(0n).valueOf());          // valueOf throws
            t(() => new Temporal.Instant(0n).toString({ smallestUnit: "hour" }));
            t(() => new Date(NaN).toTemporalInstant());           // invalid time value
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError TypeError RangeError RangeError TypeError RangeError TypeError RangeError TypeError RangeError RangeError"
        ])
    );
}
