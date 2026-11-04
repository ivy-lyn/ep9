//! Temporal.PlainDateTime (WP T6) and PlainDate.prototype.toPlainDateTime: the constructor,
//! `from`, `compare`, the getters, the methods and the error mapping, with and without GC
//! stress (the date-time and the calendar code live in six internal slots).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var dt = new Temporal.PlainDateTime(2024, 2, 29, 13, 7, 9, 123, 456, 789);
            print(dt.calendarId, dt.era, dt.year, dt.month, dt.monthCode, dt.day, dt.hour, dt.minute, dt.second, dt.millisecond, dt.microsecond, dt.nanosecond);
            print(dt.dayOfWeek, dt.dayOfYear, dt.weekOfYear, dt.yearOfWeek, dt.daysInWeek, dt.daysInMonth, dt.daysInYear, dt.monthsInYear, dt.inLeapYear);
            print(dt.toString(), dt.toJSON(), dt.toLocaleString(), dt.toString({ smallestUnit: "minute", calendarName: "always" }), dt.toString({ fractionalSecondDigits: 0 }));
            print(Object.prototype.toString.call(dt), Temporal.PlainDateTime.length, dt.toPlainDate(), dt.toPlainTime());"#
        ),
        lines(&[
            "iso8601 undefined 2024 2 M02 29 13 7 9 123 456 789",
            "4 60 9 2024 7 29 366 12 true",
            "2024-02-29T13:07:09.123456789 2024-02-29T13:07:09.123456789 2024-02-29T13:07:09.123456789 2024-02-29T13:07[u-ca=iso8601] 2024-02-29T13:07:09",
            "[object Temporal.PlainDateTime] 3 2024-02-29 13:07:09.123456789",
        ])
    );
}

#[test]
fn from_compare_and_arithmetic() {
    assert_eq!(
        run_both(
            r#"var a = Temporal.PlainDateTime.from("2020-01-31T12:00"), b = Temporal.PlainDateTime.from({ year: 2020, month: 3, day: 1, hour: 6 });
            print(a, b, Temporal.PlainDateTime.compare(a, b), a.equals("2020-01-31T12:00:00"), Temporal.PlainDateTime.from(Temporal.PlainDate.from("2020-01-01")));
            print(a.add({ months: 1, hours: 13 }), a.subtract("PT12H"), a.with({ day: 1, minute: 30 }), a.withPlainTime("08:15"), a.withPlainTime(), a.until(b), b.since(a, { largestUnit: "months" }));
            print(a.round("hour"), Temporal.PlainDateTime.from("2020-01-01T12:00").round({ smallestUnit: "day" }), Temporal.PlainDate.from("2020-05-05").toPlainDateTime("10:00"), Temporal.PlainDate.from("2020-05-05").toPlainDateTime());
            print(Temporal.PlainDate.from(a), Temporal.PlainTime.from(a), Temporal.Duration.from("P1M").total({ unit: "days", relativeTo: a }));"#
        ),
        lines(&[
            "2020-01-31T12:00:00 2020-03-01T06:00:00 -1 true 2020-01-01T00:00:00",
            "2020-03-01T01:00:00 2020-01-31T00:00:00 2020-01-01T12:30:00 2020-01-31T08:15:00 2020-01-31T00:00:00 P29DT18H P1MT18H",
            "2020-01-31T12:00:00 2020-01-02T00:00:00 2020-05-05T10:00:00 2020-05-05T00:00:00",
            "2020-01-31 12:00:00 29",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.PlainDateTime(2020, 1, 1));                   // no new
            t(() => new Temporal.PlainDateTime(2020, 1, 1, 24));           // hour out of range
            t(() => new Temporal.PlainDateTime(-271821, 4, 19));           // before the limits
            t(() => new Temporal.PlainDateTime(2020, 1, 1, 0, 0, 0, 0, 0, 0, "gregory"));
            t(() => Temporal.PlainDateTime.from("2020-01-01T25:00"));      // bad string
            t(() => Temporal.PlainDateTime.from({ year: 2020, month: 1 })); // no day
            t(() => Temporal.PlainDateTime.from({ year: 2020, month: 1, day: 1, hour: -1 }, { overflow: "reject" }));
            t(() => new Temporal.PlainDateTime(2020, 1, 1).with({ timeZone: "UTC" }));
            t(() => new Temporal.PlainDateTime(2020, 1, 1).round({ smallestUnit: "month" }));
            t(() => new Temporal.PlainDateTime(2020, 1, 1).toString({ smallestUnit: "hour" }));
            t(() => new Temporal.PlainDateTime(2020, 1, 1).valueOf());
            t(() => Temporal.PlainDateTime.prototype.hour);                // not a PlainDateTime
            t(() => new Temporal.PlainDate(-271821, 4, 19).toPlainDateTime()); // midnight is outside
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError RangeError RangeError RangeError RangeError TypeError RangeError TypeError RangeError RangeError TypeError TypeError RangeError"
        ])
    );
}
