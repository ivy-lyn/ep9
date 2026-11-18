//! Intl.DateTimeFormat.prototype.formatRange and formatRangeToParts (ECMA-402 §11.3.5,
//! §11.3.6; INTL W5 DTF-RANGE), with and without GC stress. The expected lines are node
//! v26.13.1's output for the same script (`print` = `console.log` of the arguments, each
//! through `String`, joined by a space), except where named at the test.

mod common;
use common::{lines, run_both};

const RANGES: &str = r#"const U = (y, mo, d, h = 0, mi = 0, s = 0, ms = 0) => Date.UTC(y, mo - 1, d, h, mi, s, ms);
const P = (parts) => parts.map(p => p.type + ":" + p.value + ":" + p.source).join("|");
const en = new Intl.DateTimeFormat("en-US", {timeZone: "UTC"});
const hm = new Intl.DateTimeFormat("en-US", {hour: "numeric", minute: "numeric", timeZone: "UTC"});
print(en.formatRange(U(2019, 1, 3), U(2019, 1, 5)), "|", P(en.formatRangeToParts(U(2019, 1, 3), U(2019, 1, 5))));
print(en.formatRange(U(2019, 1, 3, 1), U(2019, 1, 3, 23)) === en.format(U(2019, 1, 3)), "|", P(en.formatRangeToParts(U(2019, 1, 3, 1), U(2019, 1, 3, 23))));
print(hm.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)), "|", P(hm.formatRangeToParts(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30))));
print(hm.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 13, 30)), "|", hm.formatRange(U(2019, 1, 3, 10), U(2019, 1, 3, 15, 30)), "|", hm.formatRange(U(2019, 1, 3, 13, 5, 1), U(2019, 1, 3, 13, 5, 59)));
const ymd = new Intl.DateTimeFormat("en-US", {year: "numeric", month: "short", day: "numeric", timeZone: "UTC"});
print(ymd.formatRange(U(2019, 1, 3), U(2020, 3, 4)), "|", ymd.formatRange(U(2020, 3, 4), U(2019, 1, 3)));
const dt = new Intl.DateTimeFormat("en-US", {dateStyle: "medium", timeStyle: "short", timeZone: "UTC"});
print(dt.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)), "|", P(dt.formatRangeToParts(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30))));
print(dt.formatRange(U(2019, 1, 3, 13), U(2019, 1, 5, 15, 30)));
print(new Intl.DateTimeFormat("de", {dateStyle: "medium", timeStyle: "short", timeZone: "UTC"}).formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)));
print(new Intl.DateTimeFormat("ja", {hour: "numeric", minute: "numeric", timeZone: "UTC"}).formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)));
print(new Intl.DateTimeFormat("en-US", {hour: "numeric", minute: "numeric", timeZoneName: "short", timeZone: "UTC"}).formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)));
print(new Intl.DateTimeFormat("en-US", {hour: "numeric", minute: "numeric", hourCycle: "h23", timeZone: "UTC"}).formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15, 30)));
const fs = new Intl.DateTimeFormat("en", {minute: "numeric", second: "numeric", fractionalSecondDigits: 2, timeZone: "UTC"});
print(fs.formatRange(U(2019, 1, 3, 1, 2, 3, 234), U(2019, 1, 3, 1, 2, 3, 567)), "|", fs.formatRange(U(2019, 1, 3, 1, 2, 3, 234), U(2019, 1, 3, 1, 2, 3, 239)));
print(new Intl.DateTimeFormat("en", {minute: "numeric", second: "numeric", timeZone: "UTC"}).formatRange(U(2019, 1, 3, 1, 2, 3), U(2019, 1, 3, 1, 2, 13)));
const T = Temporal;
print(en.formatRange(new T.PlainDate(2019, 1, 3), new T.PlainDate(2019, 1, 5)), "|", hm.formatRange(new T.PlainTime(13), new T.PlainTime(15, 30)), "|", en.formatRange(new T.PlainDateTime(2019, 1, 3, 13), new T.PlainDateTime(2019, 1, 3, 15, 30)));
print(en.formatRange(new T.PlainYearMonth(2019, 1, "gregory"), new T.PlainYearMonth(2019, 3, "gregory")), "|", en.formatRange(new T.PlainMonthDay(1, 3, "gregory"), new T.PlainMonthDay(1, 5, "gregory")), "|", en.formatRange(new T.Instant(0n), new T.Instant(3600000000000n)));
print(new Intl.DateTimeFormat("en", {calendar: "iso8601", timeZone: "UTC"}).formatRange(new T.PlainDate(-271821, 4, 19), new T.PlainDate(275760, 9, 13)));
print(Intl.DateTimeFormat.prototype.formatRange.length, Intl.DateTimeFormat.prototype.formatRangeToParts.length, Intl.DateTimeFormat.prototype.formatRange.name, Intl.DateTimeFormat.prototype.formatRangeToParts.name);
const log = [];
const v = (name, value) => ({valueOf() { log.push(name); return value; }});
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => en.formatRange(v("a", 0)));
t(() => en.formatRange(undefined, v("b", 0)));
t(() => en.formatRange(v("c", NaN), v("d", 0)));
t(() => en.formatRange(v("e", 0), v("f", Infinity)));
t(() => en.formatRange(v("g", 0), new T.PlainDate(2019, 1, 3)));
t(() => en.formatRange(new T.PlainDate(2019, 1, 3), new T.PlainDateTime(2019, 1, 3)));
t(() => en.formatRange(new T.PlainDate(2019, 1, 3), new T.PlainDate(2019, 1, 3, "japanese")));
t(() => en.formatRange(new T.ZonedDateTime(0n, "UTC"), new T.ZonedDateTime(0n, "UTC")));
t(() => en.formatRange(new T.PlainTime(1), new T.PlainTime(2)) === en.format(new T.PlainTime(1)));
t(() => new Intl.DateTimeFormat("en", {year: "numeric"}).formatRange(new T.PlainTime(1), new T.PlainTime(2)));
t(() => en.formatRange(T.Duration.from({days: 1}), T.Duration.from({days: 2})));
t(() => Intl.DateTimeFormat.prototype.formatRange.call({}, 0, 1));
t(() => Intl.DateTimeFormat.prototype.formatRangeToParts.call(Intl.DateTimeFormat.prototype, 0, 1));
t(() => en.formatRangeToParts(Symbol(), 0));
print(r.join(" "), "|", log.join(""));
"#;

/// Time values and Temporal objects: one format for practically equal values (every part
/// "shared"); the date and the time range for one date ("Jan 3, 2019, 1:00 – 3:30 PM"); the
/// time collapse of the ICU4X interval pattern with its literals (ja "13時00分～15時30分") and a
/// time zone name kept once; else the fallback "{0} – {1}"; the sources by the ICU4C rule
/// (the fields that occur twice); the TypeError for undefined before any ToNumber, the
/// ToNumber order, the RangeError of TimeClip and of a calendar, the TypeError of two
/// Temporal types, of a ZonedDateTime, of a null format and of a receiver that is not a
/// DateTimeFormat. Not V8's output: (1) de "03.01.2019, 13:00–15:30": V8 writes " Uhr" after
/// the range (ICU4C's de interval pattern), the ICU4X 2.4 de pattern has none; (2)
/// "02:03.23" for 234 ms and 239 ms with fractionalSecondDigits 2: the spec compares
/// floor(ms / 10) (23 and 23: practically equal), V8 gives "02:03.23 – 02:03.23"; (3) the
/// PlainDate range at the ends of the Temporal range: V8 throws a RangeError, test262
/// `formatRange/temporal-objects-no-time-clip.js` asks for a string.
#[test]
fn format_range_and_to_parts() {
    assert_eq!(
        run_both(RANGES),
        lines(&[
            "1/3/2019 – 1/5/2019 | month:1:startRange|literal:/:startRange|day:3:startRange|literal:/:startRange|year:2019:startRange|literal: – :shared|month:1:endRange|literal:/:endRange|day:5:endRange|literal:/:endRange|year:2019:endRange",
            "true | month:1:shared|literal:/:shared|day:3:shared|literal:/:shared|year:2019:shared",
            "1:00 – 3:30 PM | hour:1:startRange|literal:::startRange|minute:00:startRange|literal: – :shared|hour:3:endRange|literal:::endRange|minute:30:endRange|literal: :shared|dayPeriod:PM:shared",
            "1:00 – 1:30 PM | 10:00 AM – 3:30 PM | 1:05 PM",
            "Jan 3, 2019 – Mar 4, 2020 | Mar 4, 2020 – Jan 3, 2019",
            "Jan 3, 2019, 1:00 – 3:30 PM | month:Jan:shared|literal: :shared|day:3:shared|literal:, :shared|year:2019:shared|literal:, :shared|hour:1:startRange|literal:::startRange|minute:00:startRange|literal: – :shared|hour:3:endRange|literal:::endRange|minute:30:endRange|literal: :shared|dayPeriod:PM:shared",
            "Jan 3, 2019, 1:00 PM – Jan 5, 2019, 3:30 PM",
            "03.01.2019, 13:00–15:30",
            "13時00分～15時30分",
            "1:00 – 3:30 PM UTC",
            "13:00 – 15:30",
            "02:03.23 – 02:03.56 | 02:03.23",
            "02:03 – 02:13",
            "1/3/2019 – 1/5/2019 | 1:00 – 3:30 PM | 1/3/2019, 1:00:00 PM – 3:30:00 PM",
            "1/2019 – 3/2019 | 1/3 – 1/5 | 1/1/1970, 12:00:00 AM – 1:00:00 AM",
            "4/19/271822 BC – 9/13/275760",
            "2 2 formatRange formatRangeToParts",
            "TypeError TypeError RangeError RangeError TypeError TypeError RangeError TypeError ok TypeError TypeError TypeError TypeError TypeError | cdefg",
        ])
    );
}

const FIELD_VALUES: &str = r#"const U = (y, mo, d, h = 0, mi = 0) => Date.UTC(y, mo - 1, d, h, mi);
const P = (parts) => parts.map(p => p.type + ":" + p.value + ":" + p.source).join("|");
const f = (o) => new Intl.DateTimeFormat("en", Object.assign({timeZone: "UTC"}, o));
const y2 = f({year: "2-digit"});
print(y2.formatRange(U(1920, 1, 1), U(2020, 1, 1)), "|", P(y2.formatRangeToParts(U(1920, 1, 1), U(2020, 1, 1))));
const narrow = f({month: "narrow"});
print(narrow.formatRange(U(2019, 6, 1), U(2019, 7, 1)), "|", P(narrow.formatRangeToParts(U(2019, 6, 1), U(2019, 7, 1))));
print(f({month: "short", day: "numeric"}).formatRange(U(2019, 1, 3), U(2020, 1, 3)));
const hm = f({hour: "numeric", minute: "numeric"});
print(hm.formatRange(U(2019, 1, 3, 13), U(2019, 1, 5, 13)), "|", hm.formatRange(U(2019, 1, 3, 13), U(2019, 1, 5, 15, 30)));
print(f({year: "numeric"}).formatRange(U(2019, 1, 3), U(2019, 5, 3)), "|", f({month: "long", day: "numeric"}).formatRange(U(2019, 1, 3, 1), U(2019, 1, 3, 23)));
const he = f({year: "numeric", calendar: "hebrew"});
print(he.formatRange(U(2019, 12, 1), U(2020, 1, 1)), "|", he.formatRange(U(2019, 1, 1), U(2019, 12, 1)) !== he.format(U(2019, 1, 1)));
const dp = f({hour: "numeric", minute: "numeric", dayPeriod: "long"});
print(dp.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 18)), "|", P(dp.formatRangeToParts(U(2019, 1, 3, 13), U(2019, 1, 3, 18))));
print(dp.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 15)), "|", P(dp.formatRangeToParts(U(2019, 1, 3, 13), U(2019, 1, 3, 15))));
"#;

/// relevantFieldsEqual (PartitionDateTimeRangePattern step 11) compares the local calendar
/// field values, not the formatted text: the years 1920 and 2020 with year "2-digit" ("20")
/// and June and July with month "narrow" ("J") are a range (Codex W5 r1 finding 1); a field
/// of the format that is not shown counts too (the year of {month, day}, the day of a time
/// format), a field below the smallest field of the format does not ({year}: January and May
/// 2019; {month, day}: 1:00 and 23:00), the field values are those of the calendar of the
/// format (Hebrew 5780 from December 2019 to January 2020). A collapse template that writes a
/// field once needs that field the same at both ends (finding 2): "in the afternoon" and "in
/// the evening" are both shown. Not V8's output: (1) "Jan 3 – Jan 3" and "1:00 PM – 1:00 PM",
/// "1:00 PM – 3:30 PM": V8 (ICU4C) adds the year and the dates; the spec takes the
/// [[Default]] range pattern of the format; (2) V8 gives "1:00 – 6:00 in the evening" (ICU4C
/// picks the interval pattern by the AM/PM field).
#[test]
fn format_range_compares_the_field_values() {
    assert_eq!(
        run_both(FIELD_VALUES),
        lines(&[
            "20 – 20 | year:20:startRange|literal: – :shared|year:20:endRange",
            "J – J | month:J:startRange|literal: – :shared|month:J:endRange",
            "Jan 3 – Jan 3",
            "1:00 PM – 1:00 PM | 1:00 PM – 3:30 PM",
            "2019 | January 3",
            "5780 | true",
            "1:00 in the afternoon – 6:00 in the evening | hour:1:startRange|literal:::startRange|minute:00:startRange|literal: :startRange|dayPeriod:in the afternoon:startRange|literal: – :shared|hour:6:endRange|literal:::endRange|minute:00:endRange|literal: :endRange|dayPeriod:in the evening:endRange",
            "1:00 – 3:00 in the afternoon | hour:1:startRange|literal:::startRange|minute:00:startRange|literal: – :shared|hour:3:endRange|literal:::endRange|minute:00:endRange|literal: :shared|dayPeriod:in the afternoon:shared",
        ])
    );
}

const AM_PM: &str = r#"const U = (y, mo, d, h = 0, mi = 0) => Date.UTC(y, mo - 1, d, h, mi);
const P = (parts) => parts.map(p => p.type + ":" + p.value + ":" + p.source).join("|");
const f = (l, o) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o));
const id = f("id", {dayPeriod: "long"});
print(id.format(U(2019, 1, 3, 11)), id.format(U(2019, 1, 3, 13)));
print(id.formatRange(U(2019, 1, 3, 11), U(2019, 1, 3, 13)), "|", P(id.formatRangeToParts(U(2019, 1, 3, 11), U(2019, 1, 3, 13))));
print(id.formatRange(U(2019, 1, 3, 10), U(2019, 1, 3, 11, 30)), "|", P(id.formatRangeToParts(U(2019, 1, 3, 10), U(2019, 1, 3, 11, 30))));
const en = f("en", {dayPeriod: "long"});
print(en.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 14)), "|", en.formatRange(U(2019, 1, 3, 11), U(2019, 1, 3, 13)));
const h12 = f("id", {hour: "numeric", minute: "numeric", dayPeriod: "long", hour12: true});
print(h12.formatRange(U(2019, 1, 3, 11), U(2019, 1, 3, 13)), "|", h12.formatRange(U(2019, 1, 3, 13), U(2019, 1, 3, 14, 30)));
"#;

/// relevantFieldsEqual compares [[AmPm]] (the hour below 12 or not) as its own Table 7 row
/// before [[DayPeriod]] (Codex W5 r2 finding): in id the flexible day period "siang" covers
/// 10:00 to 15:00, so 11:00 and 13:00 have the same day period text but not the same AmPm:
/// a range, in both methods. Both rows count when the smallest field of the format is a day
/// period or below it (the loop of step 11 stops only at a row, and AmPm is the row before
/// DayPeriod). The same AmPm and the same text (10:00 and 11:30, en 13:00 and 14:00): one
/// value, every part "shared". The collapse needs no change: with an hour, ICU4X picks the
/// template by AM/PM, and its AM/PM template writes the day period at both ends ("11.00
/// siang – 1.00 siang"). Not V8's output: V8 (ICU4C) gives "siang" for 11:00 to 13:00 and "in
/// the morning" for en 11:00 to 13:00 (it does not compare the day period of a format
/// without an hour).
#[test]
fn format_range_compares_am_pm_before_the_day_period() {
    assert_eq!(
        run_both(AM_PM),
        lines(&[
            "siang siang",
            "siang–siang | dayPeriod:siang:startRange|literal:–:shared|dayPeriod:siang:endRange",
            "siang | dayPeriod:siang:shared",
            "in the afternoon | in the morning\u{2009}–\u{2009}in the afternoon",
            "11.00 siang\u{2009}–\u{2009}1.00 siang | 1.00–2.30 siang",
        ])
    );
}
