//! Temporal.Duration (WP T2): the constructor, `from`, `compare`, the getters, the methods
//! and the error mapping, with and without GC stress (the fields live in internal slots).
//! The expected texts follow the Temporal spec (TemporalDurationToString).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var d = new Temporal.Duration(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
            print(d.years, d.months, d.weeks, d.days, d.hours, d.minutes, d.seconds, d.milliseconds, d.microseconds, d.nanoseconds, d.sign, d.blank);
            print(d.toString(), d.toJSON(), d.toLocaleString(), String(new Temporal.Duration()), new Temporal.Duration().blank);
            print(Object.prototype.toString.call(d), typeof Temporal.Duration, Temporal.Duration.length, Temporal.Duration.prototype.constructor === Temporal.Duration);
            print(new Temporal.Duration(0, 0, 0, 0, -1).toString(), new Temporal.Duration(0, 0, 0, 0, 0, 0, 1, 500).toString({ fractionalSecondDigits: 1 }));"#
        ),
        lines(&[
            "1 2 3 4 5 6 7 8 9 10 1 false",
            "P1Y2M3W4DT5H6M7.00800901S P1Y2M3W4DT5H6M7.00800901S P1Y2M3W4DT5H6M7.00800901S PT0S true",
            "[object Temporal.Duration] function 0 true",
            "-PT1H PT1.5S",
        ])
    );
}

#[test]
fn from_compare_and_arithmetic() {
    assert_eq!(
        run_both(
            r#"var a = Temporal.Duration.from("PT1H30M"), b = Temporal.Duration.from({ minutes: 90 });
            print(a, b, Temporal.Duration.compare(a, b), Temporal.Duration.compare("PT1S", "PT2S"), Temporal.Duration.compare({ hours: 2 }, "PT1H"));
            print(a.add(b), a.subtract("PT2H"), a.negated(), a.negated().abs(), a.with({ seconds: 5 }));
            print(Temporal.Duration.from("P1DT12H").round({ largestUnit: "hours" }), Temporal.Duration.from("PT130M").round("hours"));
            print(Temporal.Duration.from("PT90M").total("hours"), Temporal.Duration.from("P1D").total({ unit: "minutes" }));"#
        ),
        lines(&[
            "PT1H30M PT90M 0 -1 1",
            "PT3H -PT30M -PT1H30M PT1H30M PT1H30M5S",
            "PT36H PT2H",
            "1.5 1440",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.Duration());                       // no new
            t(() => new Temporal.Duration(1.5));                 // not integral
            t(() => new Temporal.Duration(1, -1));               // mixed signs
            t(() => new Temporal.Duration(2 ** 32));             // years too big
            t(() => Temporal.Duration.from("P1Q"));               // bad string
            t(() => Temporal.Duration.from(12));                  // not a string or object
            t(() => Temporal.Duration.from({}));                  // no field
            t(() => Temporal.Duration.from("PT1H−"));        // bad string
            t(() => Temporal.Duration.prototype.years);           // not a Duration
            t(() => new Temporal.Duration().valueOf());           // valueOf
            t(() => new Temporal.Duration().round());             // roundTo required
            t(() => new Temporal.Duration().total({}));           // unit required
            t(() => new Temporal.Duration().toString({ smallestUnit: "hour" }));
            t(() => Temporal.Duration.from("P1Y").add("P1M"));    // calendar units, no relativeTo
            t(() => Temporal.Duration.compare("P1Y", "P1M"));     // the same
            t(() => Temporal.Duration.from("PT1H").total({ unit: "day", relativeTo: 5 }));
            print(r.join());"#
        ),
        lines(&[
            "TypeError,RangeError,RangeError,RangeError,RangeError,TypeError,TypeError,RangeError,TypeError,TypeError,TypeError,RangeError,RangeError,RangeError,RangeError,TypeError"
        ])
    );
}

#[test]
fn relative_to_strings_and_property_bags() {
    assert_eq!(
        run_both(
            r#"var d = Temporal.Duration.from("P1M");
            print(d.total({ unit: "days", relativeTo: "2020-02-01" }), d.total({ unit: "days", relativeTo: { year: 2021, month: 2, day: 1 } }));
            print(Temporal.Duration.compare("P1M", "P30D", { relativeTo: "2020-01-01" }), Temporal.Duration.compare("P1M", "P30D", { relativeTo: "2020-04-01" }));
            print(Temporal.Duration.from("P1Y").round({ largestUnit: "days", relativeTo: "2020-01-01[UTC]" }));
            print(Temporal.Duration.from("PT24H").round({ largestUnit: "days", relativeTo: { year: 2020, month: 3, day: 8, timeZone: "America/New_York" } }));"#
        ),
        lines(&["29 28", "1 0", "P366D", "P1DT1H"])
    );
}

/// Design §8 A8: `Duration.prototype.total` with a time numerator above 2^53 takes temporal_rs
/// `Fraction::to_finite_f64_slow`, the one FMA site (libm `fma`, soft with
/// `force-soft-floats`). The result bits are the same on every target; CI runs this test on
/// the 4 targets.
#[test]
fn total_slow_path_bits_are_pinned() {
    let out = run_both(
        r#"function bits(x) { return x.toString(); }
        var a = new Temporal.Duration(0, 0, 0, 0, 0, 0, 0, 0, 0, 2 ** 60);
        var b = new Temporal.Duration(0, 0, 0, 0, 0, 0, 0, 0, 2 ** 55 + 8, 2 ** 61 + 3 * 2 ** 9);
        var c = new Temporal.Duration(0, 0, 0, 0, 0, 0, 2 ** 52, 999, 999, 999);
        print(bits(a.total("seconds")), bits(a.total("hours")), bits(a.total("milliseconds")));
        print(bits(b.total("seconds")), bits(b.total("days")), bits(c.total("minutes")));"#,
    )
    .unwrap();
    assert_eq!(
        out,
        PINNED.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    );
}

/// The pinned texts of [`total_slow_path_bits_are_pinned`] (JS Number-to-String is the
/// shortest round-trip text, so equal texts are equal bits). Each value is the correctly
/// rounded quotient: Python `float(Fraction(numerator_ns, unit_ns))` gives the same 6 texts.
const PINNED: [&str; 2] = [
    "1152921504.606847 320255.97350190196 1152921504606.847",
    "38334640028.17767 443687.9632890934 75059993789508.28",
];

/// A relativeTo string with a calendar annotation: CanonicalizeCalendar, as for a property bag
/// (spec-temporal Codex r1 T1). `islamic` and `islamic-rgsa` are not in AvailableCalendars
/// (INTL-I1-D2); `gregory` and `chinese` are, and the month length of the calendar counts
/// (node 26 gives the same text, except that V8 accepts `islamic` in a string).
#[test]
fn relative_to_strings_check_the_calendar() {
    assert_eq!(
        run_both(
            r#"var r = [];
            for (var rt of ["2020-01-01[u-ca=islamic]", "2020-01-01T00:00[UTC][u-ca=islamic-rgsa]", "2020-01-01[u-ca=gregory]", "2020-01-01T00:00[UTC][u-ca=chinese]", "2020-01-01[u-ca=iso8601]", "2020-01-01T00:00[UTC][u-ca=iso8601]"]) {
              for (var f of [() => Temporal.Duration.compare("P40D", "P39D", { relativeTo: rt }), () => Temporal.Duration.from("P40D").round({ largestUnit: "months", relativeTo: rt }), () => Temporal.Duration.from("P40D").total({ unit: "months", relativeTo: rt })]) {
                try { r.push(String(f())); } catch (e) { r.push(e.constructor.name); }
              }
            }
            print(r.join(" "));"#
        ),
        lines(&[
            "RangeError RangeError RangeError RangeError RangeError RangeError 1 P1M9D 1.3103448275862069 1 P1M10D 1.3448275862068966 1 P1M9D 1.3103448275862069 1 P1M9D 1.3103448275862069"
        ])
    );
}
