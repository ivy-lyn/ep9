//! Intl.DateTimeFormat pattern work (INTL W4 DTF-PATTERN): the flexible day periods (T3),
//! the time fields that ICU4X adds, the text width per field, the numeric widths of the CLDR
//! availableFormats, the 2-digit year, the decimal sign of the fractional seconds and U+0020
//! before the day period, with and without GC stress. The expected lines are node
//! v26.13.1's output for the same script (`print` = `console.log` of the arguments, each
//! through `String`, joined by a space).

mod common;
use common::{lines, run_both};

const PATTERN: &str = r#"const d = Date.UTC(2024, 0, 1, 2, 35, 6, 789);
const f = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).format(x);
const p = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).formatToParts(x).map(q => q.type + ":" + q.value).join("/");
const h = (k) => Date.UTC(2017, 11, 12, k);
print([0, 6, 12, 13, 18, 21].map(k => f("en", {dayPeriod: "long"}, h(k))).join(), "|", f("en", {dayPeriod: "narrow"}, h(12)), "|", f("en", {dayPeriod: "short"}, h(12) + 60000));
print(f("en", {hour: "numeric", dayPeriod: "short"}, h(0)), "|", f("en", {hour: "numeric", minute: "numeric", dayPeriod: "long"}, h(12) + 60000), "|", f("de", {dayPeriod: "long"}, h(3)), "|", f("en", {hour: "numeric", dayPeriod: "long", hourCycle: "h23"}, h(3)));
print(p("en", {hour: "numeric", dayPeriod: "narrow"}, h(12)), "|", p("en", {dayPeriod: "long"}, h(21)));
print(f("en", {year: "numeric", month: "short", day: "numeric", hour: "numeric", dayPeriod: "long"}));
print(f("en", {minute: "numeric", second: "numeric"}), "|", p("en", {minute: "numeric", second: "numeric", fractionalSecondDigits: 2}), "|", f("en", {second: "numeric"}), "|", f("en", {minute: "2-digit"}));
print(f("de", {weekday: "short", day: "numeric", month: "long"}), "|", f("de", {weekday: "long", day: "numeric", month: "short"}), "|", f("en", {weekday: "short", year: "numeric", month: "long", day: "numeric"}));
print(f("de", {}), "|", f("de", {month: "numeric", day: "numeric"}), "|", f("en-GB", {}), "|", f("ja", {}), "|", f("de", {year: "numeric", month: "2-digit", day: "2-digit"}));
print(f("en-US", {dateStyle: "short"}, Date.UTC(1886, 4, 1)), "|", f("en", {year: "2-digit", month: "numeric", day: "numeric"}, Date.UTC(1886, 4, 1)), "|", f("de", {dateStyle: "short"}, Date.UTC(1886, 4, 1)));
print(f("en", {timeStyle: "short"}), "|", f("en-US-u-nu-arab", {hour: "numeric", minute: "numeric", second: "numeric", fractionalSecondDigits: 3}), "|", f("de", {minute: "numeric", second: "numeric", fractionalSecondDigits: 1}));
print(new Date(d).toLocaleString("de", {timeZone: "UTC"}), "|", new Date(d).toLocaleTimeString("en", {timeZone: "UTC"}));
"#;

/// dayPeriod (`dayPeriod-{long,short,narrow}-en.js`: "noon" at 12:00 when the pattern shows no
/// minute or the minute is 0, no "midnight", the hour before it with a space), {minute,
/// second} (`fractionalSecondDigits.js`), one width per field (`german-weekday-no-year.js`),
/// the numeric widths ("1.1.2024" in German: `staging/sm/String/internalUsage.js`), the
/// 2-digit year far from the present (`timedatestyle-en.js`) and the decimal sign of arab
/// (`numbering-system.js`).
#[test]
fn patterns_give_the_node_lines() {
    assert_eq!(
        run_both(PATTERN),
        lines(&[
            "in the morning,in the morning,noon,in the afternoon,in the evening,at night | n | noon",
            "12 in the morning | 12:01 in the afternoon | nachts | 03",
            "hour:12/literal: /dayPeriod:n | dayPeriod:at night",
            "Jan 1, 2024, 2 in the morning",
            "35:06 | minute:35/literal::/second:06/literal:./fractionalSecond:78 | 6 | 35",
            "Mo., 1. Januar | Montag, 1. Jan. | Mon, January 1, 2024",
            "1.1.2024 | 1.1. | 01/01/2024 | 2024/1/1 | 01.01.2024",
            "5/1/86 | 5/1/86 | 01.05.86",
            "2:35 AM | ٢:٣٥:٠٦٫٧٨٩ AM | 35:06,7",
            "1.1.2024, 02:35:06 | 2:35:06 AM",
        ])
    );
}
