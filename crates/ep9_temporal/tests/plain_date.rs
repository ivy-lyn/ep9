//! Temporal.PlainDate (WP T5): the constructor, `from`, `compare`, the getters, the methods,
//! Duration relativeTo a PlainDate and the error mapping, with and without GC stress (the
//! date and the calendar code live in four internal slots).

mod common;
use common::{lines, run_both};

#[test]
fn constructor_getters_and_to_string() {
    assert_eq!(
        run_both(
            r#"var d = new Temporal.PlainDate(2024, 2, 29);
            print(d.calendarId, d.era, d.eraYear, d.year, d.month, d.monthCode, d.day);
            print(d.dayOfWeek, d.dayOfYear, d.weekOfYear, d.yearOfWeek, d.daysInWeek, d.daysInMonth, d.daysInYear, d.monthsInYear, d.inLeapYear);
            print(d.toString(), d.toJSON(), d.toLocaleString(), d.toString({ calendarName: "always" }), new Temporal.PlainDate(-1, 1, 1, "ISO8601").toString());
            print(Object.prototype.toString.call(d), Temporal.PlainDate.length, Temporal.PlainDate.prototype.constructor === Temporal.PlainDate);"#
        ),
        lines(&[
            "iso8601 undefined undefined 2024 2 M02 29",
            "4 60 9 2024 7 29 366 12 true",
            "2024-02-29 2024-02-29 2024-02-29 2024-02-29[u-ca=iso8601] -000001-01-01",
            "[object Temporal.PlainDate] 3 true",
        ])
    );
}

#[test]
fn from_compare_arithmetic_and_relative_to() {
    assert_eq!(
        run_both(
            r#"var a = Temporal.PlainDate.from("2020-01-31"), b = Temporal.PlainDate.from({ year: 2020, monthCode: "M03", day: 31 });
            print(a, b, Temporal.PlainDate.compare(a, b), Temporal.PlainDate.compare("2020-01-01", "2020-01-01"), a.equals("2020-01-31[u-ca=iso8601]"));
            print(a.add({ months: 1 }), a.subtract("P1M"), a.with({ day: 1 }), a.with({ month: 2 }), a.withCalendar("iso8601"), a.until(b), b.since(a, { largestUnit: "months" }));
            print(Temporal.PlainDate.from({ year: 2021, month: 2, day: 31 }), Temporal.Duration.from("P1M").total({ unit: "days", relativeTo: a }), Temporal.Duration.from({ days: 40 }).round({ largestUnit: "months", relativeTo: a }));"#
        ),
        lines(&[
            "2020-01-31 2020-03-31 -1 0 true",
            "2020-02-29 2019-12-31 2020-01-01 2020-02-29 2020-01-31 P60D P2M",
            "2021-02-28 29 P1M11D", // 2020-01-31 + P1M is 2020-02-29 (constrain)
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.PlainDate(2020, 1, 1));                     // no new
            t(() => new Temporal.PlainDate(2020, 13, 1));                // month out of range
            t(() => new Temporal.PlainDate(2020, 2, 30));                // day out of range
            t(() => new Temporal.PlainDate(2020, 1, 1, "gregory"));      // only iso8601
            t(() => new Temporal.PlainDate(2020, 1, 1, 1));              // calendar not a string
            t(() => new Temporal.PlainDate(275761, 1, 1));               // past the limits
            t(() => Temporal.PlainDate.from("2020-01-01[u-ca=japanese]"));
            t(() => Temporal.PlainDate.from({ month: 1, day: 1 }));      // no year
            t(() => Temporal.PlainDate.from({ year: 2020, month: 2, day: 30 }, { overflow: "reject" }));
            t(() => Temporal.PlainDate.from({ year: 2020, month: 1, monthCode: "M02", day: 1 }));
            t(() => Temporal.PlainDate.from(20200101));                  // not a string or object
            t(() => new Temporal.PlainDate(2020, 1, 1).with({ calendar: "iso8601" }));
            t(() => new Temporal.PlainDate(2020, 1, 1).with({}));
            t(() => new Temporal.PlainDate(2020, 1, 1).withCalendar());
            t(() => new Temporal.PlainDate(2020, 1, 1).until("2021-01-01", { largestUnit: "hours" }));
            t(() => new Temporal.PlainDate(2020, 1, 1).valueOf());
            t(() => Temporal.PlainDate.prototype.year);                  // not a PlainDate
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError RangeError RangeError RangeError TypeError RangeError RangeError TypeError RangeError RangeError TypeError TypeError TypeError TypeError RangeError TypeError TypeError"
        ])
    );
}
