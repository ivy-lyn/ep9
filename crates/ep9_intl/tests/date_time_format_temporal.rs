//! Intl.DateTimeFormat on Temporal objects and the Temporal `toLocaleString` methods
//! (ECMA-402 with Temporal; INTL W5 DTF-TEMPORAL), with and without GC stress. The expected
//! lines are node v26.13.1's output for the same script (`print` = `console.log` of the
//! arguments, each through `String`, joined by a space), except where test262 asks for
//! another result than V8 gives (each named at its test).

mod common;
use common::{lines, run_both};

const TEMPORAL: &str = r#"const dtf = new Intl.DateTimeFormat("en-US", {timeZone: "Pacific/Apia"});
const pd = new Temporal.PlainDate(2021, 8, 4);
const pdt = new Temporal.PlainDateTime(2021, 8, 4, 0, 30, 45, 123, 456, 789);
const pt = new Temporal.PlainTime(23, 30, 45, 123);
const pym = new Temporal.PlainYearMonth(2021, 8, "gregory");
const pmd = new Temporal.PlainMonthDay(8, 4, "gregory");
const inst = new Temporal.Instant(1628039445123456789n);
print(dtf.format(pd), "|", dtf.format(pdt), "|", dtf.format(pt), "|", dtf.format(pym), "|", dtf.format(pmd), "|", dtf.format(inst));
print(dtf.formatToParts(pdt).map(p => p.type + ":" + p.value).join(","));
const iso = new Intl.DateTimeFormat("en", {calendar: "iso8601", timeZone: "UTC"});
print(iso.format(new Temporal.PlainDate(275760, 9, 13)), "|", iso.format(new Temporal.PlainDateTime(-271821, 4, 19, 0, 0, 0, 0, 0, 1)));
const ds = new Intl.DateTimeFormat("en", {dateStyle: "long", timeStyle: "long", timeZone: "UTC"});
print(ds.format(pd), "|", ds.format(pdt), "|", ds.format(pt), "|", ds.format(pym), "|", ds.format(pmd), "|", ds.format(inst));
const mh = new Intl.DateTimeFormat("en", {month: "long", hour: "2-digit", timeZone: "UTC"});
print(mh.format(pd), "|", mh.format(pt), "|", mh.format(pdt) === mh.format(Date.UTC(2021, 7, 4, 0, 30, 45)), "|", mh.format(pmd));
print(pd.toLocaleString("en"), "|", pdt.toLocaleString("en"), "|", pt.toLocaleString("en"), "|", pym.toLocaleString("en-u-ca-gregory"), "|", pmd.toLocaleString("en-u-ca-gregory"));
print(inst.toLocaleString("en", {timeZone: "UTC"}), "|", inst.toLocaleString("de", {timeZone: "Europe/Berlin"}));
for (const tz of ["+00:00", "+01:00", "-01:00", "UTC", "Europe/Vienna", "Asia/Calcutta", "Asia/Kolkata"]) {
  print(tz, new Temporal.ZonedDateTime(0n, tz).toLocaleString("en"));
}
print(new Temporal.ZonedDateTime(0n, "Europe/Vienna").toLocaleString("en", {timeZoneName: "long", dateStyle: undefined}));
print(new Temporal.PlainTime(0, 0).toLocaleString("en", {hourCycle: "h24"}), "|", new Temporal.PlainTime(0, 0).toLocaleString("en", {hour12: false}), "|", new Temporal.PlainTime(13, 5).toLocaleString("ja"));
print(pd.toLocaleString("en", {era: "short"}), "|", pym.toLocaleString("en-u-ca-gregory", {era: "short"}), "|", pmd.toLocaleString("en-u-ca-gregory", {weekday: "long", month: "long"}));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => dtf.format(new Temporal.ZonedDateTime(0n, "UTC")));
t(() => dtf.formatToParts(new Temporal.ZonedDateTime(0n, "UTC")));
t(() => dtf.format(new Temporal.PlainDate(2021, 8, 4, "hebrew")));
t(() => dtf.format(new Temporal.PlainYearMonth(2021, 8)));
t(() => dtf.format(new Temporal.PlainDate(2021, 8, 4, "iso8601")));
t(() => new Intl.DateTimeFormat("en", {year: "numeric"}).format(pmd));
t(() => new Intl.DateTimeFormat("en", {hour: "numeric"}).format(pd));
t(() => new Intl.DateTimeFormat("en", {timeStyle: "short"}).format(pd));
t(() => new Intl.DateTimeFormat("en", {dateStyle: "short"}).format(pt));
t(() => pd.toLocaleString("en", {timeStyle: "short"}));
t(() => pt.toLocaleString("en", {dateStyle: "short"}));
t(() => pd.toLocaleString("en", {hour: "numeric"}));
t(() => new Temporal.ZonedDateTime(0n, "UTC").toLocaleString("en", {timeZone: "UTC"}));
t(() => new Temporal.ZonedDateTime(0n, "UTC", "hebrew").toLocaleString("en"));
t(() => Temporal.PlainDate.prototype.toLocaleString.call(pt));
t(() => dtf.format(Temporal.Duration.from({days: 1})));
print(r.join(" "));
"#;

/// A DateTimeFormat formats each plain Temporal kind by its ISO fields in no time zone (the
/// format's zone is not used) and an Instant in the format's zone, with the per-kind format:
/// the default (year, month, day for a date kind, hour, minute, second for PlainTime), a style
/// format without the fields that the kind does not have, and a components format with only
/// the relevant fields (month and hour: the month for a date kind, the hour for PlainTime;
/// the PlainDateTime text is the text of the Date of the same fields). The 8 toLocaleString
/// methods (the zone of a ZonedDateTime: "GMT" for +00:00, "GMT+1" for +01:00, one text for
/// Asia/Kolkata and Asia/Calcutta); the TypeErrors (a ZonedDateTime in format and
/// formatToParts, no field of the kind, a style of the other kind, a timeZone option in
/// ZonedDateTime toLocaleString, a wrong this, a Duration) and the RangeErrors (a calendar
/// that is not the format's calendar or iso8601; PlainYearMonth needs the format's calendar).
///
/// Not node's lines (node v26.13.1 differs from test262): line 3, the dates past the TimeClip
/// range (node throws a RangeError; `format/temporal-objects-no-time-clip.js`); line 16, the
/// hour cycle of PlainTime toLocaleString (node keeps h12; `PlainTime/prototype/
/// toLocaleString/hourcycle.js`); line 17, the era option alone adds the default fields
/// (node writes only the era; `PlainDate/prototype/toLocaleString/era.js`).
#[test]
fn temporal_values_format_by_their_fields() {
    assert_eq!(
        run_both(TEMPORAL),
        lines(&[
            "8/4/2021 | 8/4/2021, 12:30:45 AM | 11:30:45 PM | 8/2021 | 8/4 | 8/4/2021, 2:10:45 PM",
            "month:8,literal:/,day:4,literal:/,year:2021,literal:, ,hour:12,literal::,minute:30,literal::,second:45,literal: ,dayPeriod:AM",
            "9/13/275760 | 4/19/271822 BC, 12:00:00 AM",
            "August 4, 2021 | August 4, 2021 at 12:30:45 AM | 11:30:45 PM | August 2021 | August 4 | August 4, 2021 at 1:10:45 AM UTC",
            "August | 11 PM | true | August",
            "8/4/2021 | 8/4/2021, 12:30:45 AM | 11:30:45 PM | 8/2021 | 8/4",
            "8/4/2021, 1:10:45 AM | 4.8.2021, 03:10:45",
            "+00:00 1/1/1970, 12:00:00 AM GMT",
            "+01:00 1/1/1970, 1:00:00 AM GMT+1",
            "-01:00 12/31/1969, 11:00:00 PM GMT-1",
            "UTC 1/1/1970, 12:00:00 AM UTC",
            "Europe/Vienna 1/1/1970, 1:00:00 AM GMT+1",
            "Asia/Calcutta 1/1/1970, 5:30:00 AM GMT+5:30",
            "Asia/Kolkata 1/1/1970, 5:30:00 AM GMT+5:30",
            "1/1/1970, 1:00:00 AM Central European Standard Time",
            "24:00:00 | 00:00:00 | 13:05:00",
            "8/4/2021 AD | 8/2021 AD | August",
            "TypeError TypeError RangeError RangeError ok TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError RangeError TypeError TypeError",
        ])
    );
}
