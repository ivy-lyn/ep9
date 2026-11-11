//! Intl.NumberFormat style "unit" (ECMA-402 §16.5.3, §16.5.4; INTL W3 NF-UNIT): the unit
//! patterns of the CLDR T2 table in the 3 widths, the "unit" and "literal" parts, the units
//! "X-per-Y", the plural forms, and the other options with the style "unit", with and without
//! GC stress. The expected lines are node v26.13.0's output for the same script (`print` =
//! `console.log` of the arguments, each through `String`, joined by a space; ICU 78.3, CLDR
//! 48: the strings of these cases are the same in the CLDR 49 data of T2, except one, named
//! at its line).

mod common;
use common::{lines, run_both};

const KMH: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).formatToParts(x).map(p => p.type + ":" + p.value).join(" ");
for (const d of ["short", "narrow", "long"]) {
  print(d, fmt("en-US", {unit: "kilometer-per-hour", unitDisplay: d}, -987, -0.001, -0, 0, 0.001, 987));
  print(d, fmt("de-DE", {unit: "kilometer-per-hour", unitDisplay: d}, -987, 1));
  print(d, fmt("ja-JP", {unit: "kilometer-per-hour", unitDisplay: d}, -987, 1));
  print(d, fmt("ko-KR", {unit: "kilometer-per-hour", unitDisplay: d}, -987, 1));
  print(d, fmt("zh-TW", {unit: "kilometer-per-hour", unitDisplay: d}, -987, 1));
}
print(parts("zh-TW", {unit: "kilometer-per-hour", unitDisplay: "long"}, -987));
print(parts("ja-JP", {unit: "kilometer-per-hour", unitDisplay: "long"}, 0.001));
print(parts("en-US", {unit: "percent"}, -123), "|", parts("de", {unit: "percent"}, 5));
"#;

/// The CLDR compound unit `kilometer-per-hour` in the 5 locales of the test262 `unit-*.js`
/// files and the 3 widths, and its parts: the unit text on both sides of the number (zh-TW,
/// ja long), the spaces next to the number as literal parts.
#[test]
fn kilometer_per_hour_gives_the_node_lines() {
    assert_eq!(
        run_both(KMH),
        lines(&[
            "short -987 km/h | -0.001 km/h | -0 km/h | 0 km/h | 0.001 km/h | 987 km/h",
            "short -987 km/h | 1 km/h",
            "short -987 km/h | 1 km/h",
            "short -987km/h | 1km/h",
            "short -987 公里/小時 | 1 公里/小時",
            "narrow -987km/h | -0.001km/h | -0km/h | 0km/h | 0.001km/h | 987km/h",
            "narrow -987 km/h | 1 km/h",
            "narrow -987km/h | 1km/h",
            "narrow -987km/h | 1km/h",
            "narrow -987公里/小時 | 1公里/小時",
            "long -987 kilometers per hour | -0.001 kilometers per hour | -0 kilometers per hour | 0 kilometers per hour | 0.001 kilometers per hour | 987 kilometers per hour",
            "long -987 Kilometer pro Stunde | 1 Kilometer pro Stunde",
            "long 時速 -987 キロメートル | 時速 1 キロメートル",
            "long 시속 -987킬로미터 | 시속 1킬로미터",
            "long 每小時 -987 公里 | 每小時 1 公里",
            "unit:每小時 literal:  minusSign:- integer:987 literal:  unit:公里",
            "unit:時速 literal:  integer:0 decimal:. fraction:001 literal:  unit:キロメートル",
            // CLDR 49 de percent "{0} %" has U+0020 (CLDR 48, node: U+00A0).
            "minusSign:- integer:123 unit:% | integer:5 literal:  unit:%",
        ])
    );
}

const FORMS: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).formatToParts(x).map(p => p.type + ":" + p.value).join(" ");
print(fmt("en", {unit: "meter-per-hour"}, 2), "|", fmt("en", {unit: "meter-per-stone"}, 2), "|", fmt("en", {unit: "stone-per-meter", unitDisplay: "long"}, 1, 2), "|", fmt("ja", {unit: "stone-per-acre", unitDisplay: "long"}, 2));
print(fmt("ar", {unit: "hour", unitDisplay: "long"}, 1, 2, 3, 11, 100), "|", parts("ar", {unit: "hour", unitDisplay: "long"}, 1));
print(fmt("he", {unit: "degree", unitDisplay: "long"}, -1, 2), "|", fmt("ru", {unit: "hour", unitDisplay: "long"}, 1, 2, 5, 1.5), "|", fmt("pl", {unit: "liter", unitDisplay: "long"}, 1, 2, 5, 22));
print(fmt("fr", {unit: "kilometer", unitDisplay: "long"}, 1.5, 2, 1000000), "|", fmt("en", {unit: "kilometer", unitDisplay: "long"}, NaN, -Infinity, 1));
print(fmt("en", {unit: "liter-per-kilometer", unitDisplay: "narrow"}, 3), "|", fmt("en", {unit: "mile-per-gallon", unitDisplay: "long"}, 30), "|", fmt("en", {unit: "meter-per-second"}, 9.81));
"#;

/// The units "X-per-Y" that T2 does not list (X in the per-unit pattern of Y, or in the
/// "per" pattern with the name of Y), the plural form of the rounded value (ar, he, ru, pl,
/// fr), a pattern with no number (ar one and two hours, he one and two degrees), NaN and
/// the infinities ("other").
#[test]
fn per_units_and_plural_forms_give_the_node_lines() {
    assert_eq!(
        run_both(FORMS),
        lines(&[
            "2 m/h | 2 m/st | 1 stone per meter | 2 stones per meter | 2 ストーン毎エーカー",
            "ساعة | ساعتان | 3 ساعات | 11 ساعة | 100 ساعة | unit:ساعة",
            "מעלה אחת | שתי מעלות | 1 час | 2 часа | 5 часов | 1,5 часа | 1 litr | 2 litry | 5 litrów | 22 litry",
            "1,5\u{a0}kilomètre | 2\u{a0}kilomètres | 1\u{202f}000\u{202f}000\u{a0}kilomètres | NaN kilometers | -∞ kilometers | 1 kilometer",
            "3L/km | 30 miles per gallon | 9.81 m/s",
        ])
    );
}

const OPTIONS: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, Object.assign({style: "unit"}, o)).formatToParts(x).map(p => p.type + ":" + p.value).join(" ");
print(fmt("en", {unit: "megabyte", notation: "compact"}, 1234567), "|", fmt("en", {unit: "liter", notation: "compact", compactDisplay: "long", unitDisplay: "long"}, 2500), "|", fmt("en", {unit: "celsius", signDisplay: "always"}, 0, 21));
print(fmt("en-u-nu-thai", {unit: "fahrenheit"}, 98.6), "|", fmt("fa", {unit: "mile-per-hour"}, -60), "|", fmt("de", {unit: "byte-per-bit", unitDisplay: "long"}, -1));
print((123).toLocaleString("en", {style: "unit", unit: "acre"}), "|", (12345678901234567890n).toLocaleString("en", {style: "unit", unit: "byte", unitDisplay: "long"}));
print(parts("sv", {unit: "kilometer"}, -5), "|", parts("fa", {unit: "kilometer"}, -5));
print(JSON.stringify(new Intl.NumberFormat("en", {style: "unit", unit: "gigabyte", unitDisplay: "narrow"}).resolvedOptions()));
"#;

/// The style "unit" with compact notation, signDisplay, a numbering system, a BigInt,
/// toLocaleString, the sign affixes with bidi marks (fa), and resolvedOptions.
#[test]
fn unit_options_give_the_node_lines() {
    assert_eq!(
        run_both(OPTIONS),
        lines(&[
            "1.2M MB | 2.5 thousand liters | +0°C | +21°C",
            "๙๘.๖°F | \u{200e}−۶۰\u{200e} mph | -1\u{a0}Byte pro Bit",
            "123 ac | 12,345,678,901,234,567,890 bytes",
            "minusSign:− integer:5 literal:  unit:km | literal:\u{200e} minusSign:− integer:۵ literal:  unit:کیلومتر",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"unit\",\"unit\":\"gigabyte\",\"unitDisplay\":\"narrow\",\"minimumIntegerDigits\":1,\"minimumFractionDigits\":0,\"maximumFractionDigits\":3,\"useGrouping\":\"auto\",\"notation\":\"standard\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
        ])
    );
}
