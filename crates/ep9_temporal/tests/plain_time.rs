//! Temporal.PlainTime (WP T4): the constructor, `from`, `compare`, the getters, the methods
//! and the error mapping, with and without GC stress (the time lives in two internal slots).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var t = new Temporal.PlainTime(13, 7, 9, 123, 456, 789);
            print(t.hour, t.minute, t.second, t.millisecond, t.microsecond, t.nanosecond);
            print(t.toString(), t.toJSON(), t.toLocaleString(), new Temporal.PlainTime().toString(), new Temporal.PlainTime(23, 59, 59, 999, 999, 999).toString());
            print(Object.prototype.toString.call(t), Temporal.PlainTime.length, Temporal.PlainTime.prototype.constructor === Temporal.PlainTime);
            print(t.toString({ smallestUnit: "minute" }), t.toString({ fractionalSecondDigits: 2, roundingMode: "ceil" }), t.toString({ smallestUnit: "second" }));"#
        ),
        lines(&[
            "13 7 9 123 456 789",
            "13:07:09.123456789 13:07:09.123456789 13:07:09.123456789 00:00:00 23:59:59.999999999",
            "[object Temporal.PlainTime] 0 true",
            "13:07 13:07:09.13 13:07:09",
        ])
    );
}

#[test]
fn from_compare_and_arithmetic() {
    assert_eq!(
        run_both(
            r#"var a = Temporal.PlainTime.from("12:30"), b = Temporal.PlainTime.from({ hour: 25, minute: 70 });
            print(a, b, Temporal.PlainTime.compare(a, b), Temporal.PlainTime.compare("01:00", "01:00"), a.equals({ hour: 12, minute: 30 }));
            print(a.add("PT12H"), a.subtract({ minutes: 45 }), a.with({ second: 5 }), a.with({ hour: 30 }), a.until(b), b.since(a, { largestUnit: "minutes" }));
            print(Temporal.PlainTime.from("23:59:59.5").round("second"), a.round({ smallestUnit: "hour", roundingMode: "floor" }), a.round({ smallestUnit: "minute", roundingIncrement: 15 }));
            print(Temporal.PlainTime.from("2020-01-01T10:20:30Z[UTC]".slice(0, 19)), Temporal.PlainTime.from({ hour: -1 }).hour);"#
        ),
        lines(&[
            "12:30:00 23:59:00 -1 0 true",
            "00:30:00 11:45:00 12:30:05 23:30:00 PT11H29M PT689M",
            "00:00:00 12:00:00 12:30:00",
            "10:20:30 0",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.PlainTime());                         // no new
            t(() => new Temporal.PlainTime(24));                    // hour out of range
            t(() => new Temporal.PlainTime(-1));                    // negative
            t(() => new Temporal.PlainTime(Infinity));              // not finite
            t(() => Temporal.PlainTime.from("25:00"));              // bad string
            t(() => Temporal.PlainTime.from(12));                   // not a string or object
            t(() => Temporal.PlainTime.from({}));                   // no field
            t(() => Temporal.PlainTime.from({ hour: 24 }, { overflow: "reject" }));
            t(() => Temporal.PlainTime.from({ hour: -1 }, { overflow: "reject" }));
            t(() => Temporal.PlainTime.from("12:00", { overflow: "bad" }));
            t(() => new Temporal.PlainTime().with({ calendar: "iso8601", hour: 1 }));
            t(() => new Temporal.PlainTime().with({}));
            t(() => new Temporal.PlainTime().valueOf());
            t(() => new Temporal.PlainTime().toString({ smallestUnit: "hour" }));
            t(() => new Temporal.PlainTime().add({ days: 1 }));     // days are fine: they wrap
            t(() => Temporal.PlainTime.prototype.hour);             // not a PlainTime
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError RangeError RangeError RangeError RangeError TypeError TypeError RangeError RangeError RangeError TypeError TypeError TypeError RangeError ok TypeError"
        ])
    );
}
