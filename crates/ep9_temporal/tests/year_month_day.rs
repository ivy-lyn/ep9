//! Temporal.PlainYearMonth and Temporal.PlainMonthDay (WP T7), and PlainDate's
//! toPlainYearMonth / toPlainMonthDay, with and without GC stress (each value lives in four
//! internal slots).

mod common;
use common::{lines, run_both};

#[test]
fn year_month() {
    assert_eq!(
        run_both(
            r#"var ym = new Temporal.PlainYearMonth(2024, 2);
            print(ym.calendarId, ym.era, ym.eraYear, ym.year, ym.month, ym.monthCode, ym.daysInYear, ym.daysInMonth, ym.monthsInYear, ym.inLeapYear);
            print(ym, ym.toJSON(), ym.toString({ calendarName: "always" }), new Temporal.PlainYearMonth(2024, 2, "iso8601", 15).toString({ calendarName: "always" }), Object.prototype.toString.call(ym));
            var a = Temporal.PlainYearMonth.from("2020-05"), b = Temporal.PlainYearMonth.from({ year: 2021, monthCode: "M03" });
            print(a, b, Temporal.PlainYearMonth.compare(a, b), a.equals("2020-05-31"), a.with({ month: 12 }), a.add({ months: 10 }), a.subtract("P1Y"), a.until(b), b.since(a, { largestUnit: "months" }));
            print(a.toPlainDate({ day: 31 }), Temporal.PlainDate.from("2020-05-17").toPlainYearMonth(), Temporal.PlainYearMonth.from({ year: 2020, month: 13 }));"#
        ),
        lines(&[
            "iso8601 undefined undefined 2024 2 M02 366 29 12 true",
            "2024-02 2024-02 2024-02-01[u-ca=iso8601] 2024-02-15[u-ca=iso8601] [object Temporal.PlainYearMonth]",
            "2020-05 2021-03 -1 true 2020-12 2021-03 2019-05 P10M P10M",
            "2020-05-31 2020-05 2020-12",
        ])
    );
}

#[test]
fn month_day() {
    assert_eq!(
        run_both(
            r#"var md = new Temporal.PlainMonthDay(2, 29);
            print(md.calendarId, md.monthCode, md.day, md, md.toJSON(), md.toString({ calendarName: "always" }), Object.prototype.toString.call(md));
            var a = Temporal.PlainMonthDay.from("12-25"), b = Temporal.PlainMonthDay.from({ monthCode: "M02", day: 30 });
            print(a, b, a.equals("--12-25"), a.with({ day: 24 }), a.toPlainDate({ year: 2020 }), md.toPlainDate({ year: 2023 }), Temporal.PlainDate.from("2020-05-17").toPlainMonthDay());"#
        ),
        lines(&[
            "iso8601 M02 29 02-29 02-29 1972-02-29[u-ca=iso8601] [object Temporal.PlainMonthDay]",
            "12-25 02-29 true 12-24 2020-12-25 2023-02-28 05-17",
        ])
    );
}

#[test]
fn errors_have_the_spec_types() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.PlainYearMonth(2020, 1));                    // no new
            t(() => new Temporal.PlainYearMonth(2020, 13));               // month out of range
            t(() => new Temporal.PlainYearMonth(275760, 10));             // past the limits
            t(() => new Temporal.PlainYearMonth(2020, 1, "islamic"));     // not available
            t(() => Temporal.PlainYearMonth.from({ year: 2020 }));        // no month
            t(() => Temporal.PlainYearMonth.from({ year: 2020, month: 13 }, { overflow: "reject" }));
            t(() => new Temporal.PlainYearMonth(2020, 1).toPlainDate("1"));
            t(() => new Temporal.PlainYearMonth(2020, 1).until("2021-01", { largestUnit: "days" }));
            t(() => Temporal.PlainMonthDay(1, 1));                        // no new
            t(() => new Temporal.PlainMonthDay(2, 30));                   // day out of range
            t(() => Temporal.PlainMonthDay.from({ month: 1 })); // no day
            t(() => Temporal.PlainMonthDay.from("2020-13-01"));           // bad string
            t(() => new Temporal.PlainMonthDay(1, 1).toPlainDate({}));    // no year
            t(() => new Temporal.PlainMonthDay(1, 1).valueOf());
            t(() => Temporal.PlainMonthDay.prototype.day);                // not a PlainMonthDay
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError RangeError RangeError RangeError TypeError RangeError TypeError RangeError TypeError RangeError TypeError RangeError TypeError TypeError TypeError"
        ])
    );
}

/// ToTemporalDate, ToTemporalDateTime and ToTemporalYearMonth of a string read the options
/// (GetOptionsObject, then the overflow Get) before the ISO limits check (CreateTemporalDate,
/// CreateTemporalDateTime, ISOYearMonthWithinLimits) (spec-temporal Codex r1 T2).
#[test]
fn string_limits_are_checked_after_the_options() {
    assert_eq!(
        run_both(
            r#"var r = [];
            var cases = [[Temporal.PlainDate, "+999999-01-01"], [Temporal.PlainDate, "+275760-09-14"], [Temporal.PlainDateTime, "+999999-01-01T00:00"], [Temporal.PlainDateTime, "-271821-04-19T00:00"], [Temporal.PlainYearMonth, "+999999-01"]];
            for (var [C, s] of cases) {
              try { C.from(s, null); r.push("ok"); } catch (e) { r.push(e.constructor.name); }
              var n = 0;
              try { C.from(s, { get overflow() { n++; return "constrain"; } }); r.push("ok"); } catch (e) { r.push(e.constructor.name + n); }
            }
            print(r.join(" "));"#
        ),
        lines(&[
            "TypeError RangeError1 TypeError RangeError1 TypeError RangeError1 TypeError RangeError1 TypeError RangeError1"
        ])
    );
}

/// ToTemporalDate, ToTemporalDateTime and ToTemporalYearMonth of a string check the calendar
/// (CanonicalizeCalendar: `islamic` is not in AvailableCalendars, INTL-I1-D2, though temporal_rs
/// accepts it) before GetOptionsObject and the
/// overflow Get, also when the date is out of the ISO limits: a RangeError, and the overflow
/// getter does not run. An in-limits date gives the same result (spec-temporal Codex r2).
#[test]
fn string_calendar_is_checked_before_the_options() {
    assert_eq!(
        run_both(
            r#"var r = [];
            var cases = [[Temporal.PlainDate, "+999999-01-01[u-ca=islamic]"], [Temporal.PlainDate, "+275760-09-14[u-ca=islamic]"], [Temporal.PlainDate, "2020-01-01[u-ca=islamic]"], [Temporal.PlainDateTime, "+999999-01-01T00:00[u-ca=islamic]"], [Temporal.PlainDateTime, "-271821-04-19T00:00[u-ca=islamic]"], [Temporal.PlainDateTime, "2020-01-01T00:00[u-ca=islamic]"], [Temporal.PlainYearMonth, "+999999-01-01[u-ca=islamic]"], [Temporal.PlainYearMonth, "-271821-03-01[u-ca=islamic]"], [Temporal.PlainYearMonth, "2020-01-01[u-ca=islamic]"]];
            for (var [C, s] of cases) {
              try { C.from(s, null); r.push("ok"); } catch (e) { r.push(e.constructor.name); }
              var n = 0;
              try { C.from(s, { get overflow() { n++; return "constrain"; } }); r.push("ok"); } catch (e) { r.push(e.constructor.name + n); }
            }
            print(r.join(" "));"#
        ),
        lines(&[
            "RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0 RangeError RangeError0"
        ])
    );
}
