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

const H24: &str = r#"const f = (l, o, x = 0) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).format(x);
const p = (l, o, x = 0) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).formatToParts(x).map(q => q.type + ":" + q.value).join("/");
print(f("en", {hourCycle: "h24", hour: "2-digit", minute: "2-digit"}), "|", p("en", {hourCycle: "h24", hour: "2-digit", minute: "2-digit"}), "|", f("en", {hourCycle: "h24", timeStyle: "short"}), "|", f("en-u-hc-h24", {hour: "numeric"}), "|", f("en-u-nu-arab", {hourCycle: "h24", hour: "numeric", minute: "numeric"}), "|", f("en", {hourCycle: "h24", hour: "numeric", minute: "numeric"}, 3600000), "|", new Date(0).toLocaleTimeString("en-u-hc-h24", {timeZone: "UTC"}));
print(f("en", {hourCycle: "h24", hour: "numeric"}), "|", f("de", {hourCycle: "h24", dateStyle: "short", timeStyle: "short"}), "|", f("en", {hourCycle: "h24", hour: "numeric", dayPeriod: "short"}), "|", new Intl.DateTimeFormat("en", {hourCycle: "h24", hour: "numeric", timeZone: "UTC"}).resolvedOptions().hourCycle);
"#;

/// hourCycle "h24" (W4 Codex r1 F3): ICU4X has no h24, so the formatter takes h23; at hour 0
/// the hour field is "24" (FormatDateTimePattern 11.5.5, hour: h24 and v = 0 gives 24), in
/// the digits of the numbering system, for the components and the styles; hour 1 stays "01".
#[test]
fn h24_writes_24_at_midnight() {
    assert_eq!(
        run_both(H24),
        lines(&[
            "24:00 | hour:24/literal::/minute:00 | 24:00 | 24 | ٢٤:٠٠ | 01:00 | 24:00:00",
            "24 | 01.01.70, 24:00 | 24 | h24",
        ])
    );
}

const NARROW: &str = r#"const d = Date.UTC(2024, 0, 1, 2, 35, 6, 789);
const f = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).format(x);
const p = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).formatToParts(x).map(q => q.type + ":" + q.value).join("/");
print(f("en", {month: "narrow"}), "|", f("en", {weekday: "narrow"}), "|", f("en", {weekday: "narrow", month: "long", day: "numeric"}), "|", p("en", {month: "narrow", day: "numeric"}), "|", f("de", {month: "narrow"}), "|", f("ja", {weekday: "narrow"}));
print(f("en", {weekday: "narrow", month: "narrow", day: "numeric"}), "|", f("en", {weekday: "long", month: "narrow", day: "numeric"}), "|", f("en", {weekday: "narrow", year: "numeric", month: "short", day: "numeric"}), "|", f("ru", {month: "narrow"}), "|", f("ru", {month: "narrow", day: "numeric"}), "|", f("zh", {weekday: "narrow"}));
print(f("en-u-ca-buddhist", {month: "narrow", year: "numeric"}), "|", f("en-u-ca-japanese", {month: "narrow"}), "|", f("en-u-ca-roc", {month: "narrow"}), "|", f("fr-u-ca-iso8601", {month: "narrow"}));
print(["hebrew", "chinese", "dangi", "islamic-umalqura", "persian", "indian", "coptic", "ethioaa"].map(ca => f("en-u-ca-" + ca, {month: "narrow"})).join(" "), "|", f("fr-u-ca-hebrew", {month: "narrow"}));
const ro = new Intl.DateTimeFormat("en", {month: "narrow", weekday: "narrow"}).resolvedOptions();
print(ro.month, ro.weekday);
"#;

/// month and weekday "narrow" (W4 Codex r1 F4): the ICU4X field bag maps narrow to the
/// abbreviated length (or wide when another field is long), so the narrow name comes from
/// the ICU4X names data per field (format or stand-alone), in the calendar of the format (a
/// number where the data is numeric). The Hebrew month numbers are in
/// `hebrew_narrow_months_use_the_cldr_numbers`.
#[test]
fn narrow_weekday_and_month_names() {
    assert_eq!(
        run_both(NARROW),
        lines(&[
            "J | M | M, January 1 | month:J/literal: /day:1 | J | 月",
            "M, J 1 | Monday, J 1 | M, Jan 1, 2024 | Я | 1 Я | 一",
            "J 2567 BE | J | J | J",
            "4 11 11 6 10 10 4 4 | 4",
            "narrow narrow",
        ])
    );
}

const HEBREW: &str = r#"const f = (l, o, x) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).format(x);
const leap = [Date.UTC(2023, 8, 20), Date.UTC(2024, 0, 20), Date.UTC(2024, 1, 20), Date.UTC(2024, 2, 20), Date.UTC(2024, 3, 15), Date.UTC(2024, 7, 15), Date.UTC(2024, 8, 10)];
const plain = [Date.UTC(2024, 9, 10), Date.UTC(2025, 1, 10), Date.UTC(2025, 2, 15), Date.UTC(2025, 3, 15), Date.UTC(2025, 8, 1)];
const m = (l, o, xs) => xs.map(x => f(l + "-u-ca-hebrew", o, x)).join(" ");
print(m("en", {month: "narrow"}, leap), "|", m("en", {month: "narrow"}, plain));
print(m("bn", {month: "narrow"}, leap), "|", m("bn", {month: "narrow"}, plain));
print(m("mr", {month: "narrow"}, leap), "|", m("mr", {month: "narrow"}, plain));
print(m("en", {month: "narrow", day: "numeric"}, [leap[3], leap[4], plain[2]]), "|", m("fr", {month: "narrow"}, [leap[2], leap[3], plain[2]]), "|", m("ff-Adlm", {month: "narrow"}, [leap[2], leap[3], leap[6]]), "|", f("bn-u-ca-hebrew-nu-latn", {month: "narrow"}, leap[4]));
"#;

/// Hebrew narrow months where the CLDR names are digits (root, so en and fr; bn, mr,
/// ff-Adlm): the CLDR number, as in node. CLDR gives Adar I 6, Adar and Adar II 7, and the
/// months after Adar the ICU4X number plus 1 (Nisan 8, Elul 13). ICU4X numbers the months by
/// code (Nisan M07). The digits are the digits of the names (bn-u-nu-latn stays Bengali).
/// The dates: Tishrei, Shevat, Adar I, Adar II, Nisan, Av, Elul of the leap year 5784, and
/// Tishrei, Shevat, Adar, Nisan, Elul of 5785.
#[test]
fn hebrew_narrow_months_use_the_cldr_numbers() {
    assert_eq!(
        run_both(HEBREW),
        lines(&[
            "1 5 6 7 8 12 13 | 1 5 7 8 13",
            "১ ৫ ৬ ৭ ৮ ১২ ১৩ | ১ ৫ ৭ ৮ ১৩",
            "१ ५ ६ ७ ८ १२ १३ | १ ५ ७ ८ १३",
            "10 7 7 8 15 7 | 6 7 7 | 𞥖 𞥗 𞥑𞥓 | ৮",
        ])
    );
}

const WIDTHS: &str = r#"const ro = (l, o) => { const r = new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).resolvedOptions(); return ["year", "month", "day", "hour", "minute", "second"].filter(k => r[k]).map(k => k + ":" + r[k]).join(" "); };
print(ro("en-GB", {}), "|", ro("en", {hour: "numeric", minute: "numeric"}), "|", ro("en-GB", {hour: "numeric"}));
print(ro("en", {minute: "numeric", second: "numeric"}), "|", ro("de", {}), "|", ro("en", {year: "2-digit", month: "numeric", day: "numeric"}));
print(ro("fr", {}), "|", ro("en", {hour: "numeric", minute: "numeric", second: "numeric"}), "|", ro("en", {hour: "numeric", minute: "numeric", hourCycle: "h23"}), "|", ro("de", {hour: "numeric", hourCycle: "h11", minute: "numeric"}));
print(ro("en", {minute: "numeric"}), "|", ro("en", {hour: "numeric"}), "|", ro("en", {month: "2-digit", day: "numeric"}), "|", ro("en", {month: "numeric", day: "2-digit"}), "|", ro("en", {month: "short", day: "numeric"}), "|", ro("ja", {}));
"#;

/// resolvedOptions reports the widths of the pattern that format uses (W4 Codex r1 F5;
/// ECMA-402 11.3.7 reads [[DateTimeFormat]], the selected format, as V8 does): en-GB
/// "dd/MM/y" gives month and day "2-digit", en "h:mm a" gives minute "2-digit". A text field,
/// the year and a field the request did not have stay as they are.
#[test]
fn resolved_options_report_the_selected_widths() {
    assert_eq!(
        run_both(WIDTHS),
        lines(&[
            "year:numeric month:2-digit day:2-digit | hour:numeric minute:2-digit | hour:2-digit",
            "minute:2-digit second:2-digit | year:numeric month:numeric day:numeric | year:2-digit month:numeric day:numeric",
            "year:numeric month:2-digit day:2-digit | hour:numeric minute:2-digit second:2-digit | hour:2-digit minute:2-digit | hour:numeric minute:2-digit",
            "minute:numeric | hour:numeric | month:2-digit day:numeric | month:numeric day:2-digit | month:short day:numeric | year:numeric month:numeric day:numeric",
        ])
    );
}
