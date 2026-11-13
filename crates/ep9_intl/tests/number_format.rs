//! Intl.NumberFormat (ECMA-402 §16; INTL W2 NF-CORE): the constructor, supportedLocalesOf,
//! format, formatToParts, resolvedOptions, the decimal style in standard and compact
//! notation, and Number and BigInt toLocaleString, with and without GC stress. The expected
//! lines are node v26.13.0's output for the same script (`print` = `console.log` of the
//! arguments, each through `String`, joined by a space; ICU 78.3, CLDR 48: the strings of
//! these cases are the same in the CLDR 49 data of ICU4X 2.4).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const N = Intl.NumberFormat;
print(typeof N, N.length, N.name, Object.getPrototypeOf(N) === Function.prototype);
print(Object.getOwnPropertyNames(N).join(), Object.getOwnPropertyNames(N.prototype).filter(k => !k.startsWith("formatRange")).join());
const g = Object.getOwnPropertyDescriptor(N.prototype, "format");
print(typeof g.get, g.set, g.enumerable, g.configurable, g.get.name, g.get.length);
print(N.supportedLocalesOf.length, N.prototype.formatToParts.length, N.prototype.resolvedOptions.length);
const nf = new N("en");
const f = nf.format;
print(f === nf.format, f.length, JSON.stringify(f.name), Object.prototype.toString.call(nf), N() instanceof N);
print(JSON.stringify(N.supportedLocalesOf(["de-CH", "zz", "en-u-nu-thai"])));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => N.prototype.format);
t(() => N.prototype.resolvedOptions());
t(() => N.prototype.formatToParts.call({}, 1));
t(() => new N("en", {style: "currency"}));
t(() => new N("en", {style: "unit"}));
t(() => new N("en", {currency: "EU"}));
t(() => new N("en", {unit: "km"}));
t(() => new N("en", {numberingSystem: "ab"}));
t(() => new N("en", {roundingIncrement: 3}));
t(() => new N("en", {roundingIncrement: 5, maximumSignificantDigits: 2}));
t(() => new N("en", {minimumFractionDigits: 3, maximumFractionDigits: 1}));
t(() => new N("en", {notation: "x"}));
t(() => new N("en", {signDisplay: "x"}));
t(() => new N("en", {useGrouping: "x"}));
t(() => f(Symbol()));
print(r.join(" "));
"#;

/// The shape of %Intl.NumberFormat% and its prototype, the bound format function, and the
/// errors of the constructor and the methods.
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 NumberFormat true",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,formatToParts,format",
            "function undefined false true get format 0",
            "1 1 0",
            "true 1 \"\" [object Intl.NumberFormat] true",
            "[\"de-CH\",\"en-u-nu-thai\"]",
            "TypeError TypeError TypeError TypeError TypeError RangeError RangeError RangeError RangeError TypeError RangeError RangeError RangeError RangeError TypeError",
        ])
    );
}

const FORMAT: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, o).format(x)).join(" | ");
print(fmt("en", {}, 1234567.891, -0.0001, -0, 1e21, 5e-324, 0.1 + 0.2, NaN, -Infinity));
print(fmt("de", {}, 1234567.891, "1.500", 12345678901234567890n, "0x1F", " 42 ", ""));
print(fmt("en-IN", {}, 123456789), fmt("es", {}, 1000, 10000), fmt("es", {useGrouping: "always"}, 1000));
print(fmt("en", {useGrouping: "min2"}, 1000, 10000), fmt("en", {useGrouping: false}, 1234567));
print(fmt("en", {notation: "compact"}, 987654321, 9876, 159, 0.159, 999999), fmt("en", {notation: "compact", compactDisplay: "long"}, 98765, -1500));
print(fmt("ja", {notation: "compact"}, 98765432), fmt("de", {notation: "compact", compactDisplay: "long"}, 1234567));
print(fmt("en", {signDisplay: "always"}, 1, 0, -0, NaN), fmt("en", {signDisplay: "exceptZero"}, 1, 0, -0.0001, -1), fmt("en", {signDisplay: "negative"}, -0.0001, -1));
print(fmt("en", {maximumFractionDigits: 2, maximumSignificantDigits: 2, roundingPriority: "lessPrecision"}, "1.500", "1.625"));
print(fmt("en", {minimumFractionDigits: 2, maximumFractionDigits: 2, roundingIncrement: 5, roundingMode: "ceil"}, 1.231, -1.239));
print(fmt("en", {maximumFractionDigits: 0, roundingMode: "halfEven"}, 2.5, 3.5, -2.5), fmt("en", {minimumIntegerDigits: 3, trailingZeroDisplay: "stripIfInteger", minimumFractionDigits: 2}, 5, 5.1));
print(fmt("en-u-nu-thai", {}, 1234.5), fmt("zh", {numberingSystem: "hanidec"}, 1234), fmt("ar-EG", {}, 1234.5), fmt("zh-TW", {}, NaN));
print(JSON.stringify(new Intl.NumberFormat("en", {notation: "compact", compactDisplay: "long"}).formatToParts(-9876)));
print(JSON.stringify(new Intl.NumberFormat("de").formatToParts(-1234.5)));
const ro = (l, o) => JSON.stringify(new Intl.NumberFormat(l, o).resolvedOptions());
print(ro("en"));
print(ro("de-u-nu-arab", {maximumSignificantDigits: 3, notation: "compact", signDisplay: "never"}));
print(ro("en", {style: "currency", currency: "eur", roundingPriority: "morePrecision", useGrouping: true}));
print(ro("en", {style: "unit", unit: "kilometer-per-hour", unitDisplay: "long", useGrouping: "false"}));
print((1234.5).toLocaleString(), (1234.5).toLocaleString("de"), (-12345678901234567890n).toLocaleString("en", {notation: "compact"}), (5).toLocaleString("en", {minimumFractionDigits: 2}));
"#;

/// format, formatToParts and resolvedOptions: grouping, signs, rounding modes and priority,
/// increments, String and BigInt values (the INTL-V2-D20 trailing-zero rule: "1.500" is 1.5),
/// numbering systems, compact notation, NaN from the CLDR T1 table, toLocaleString.
#[test]
fn format_gives_the_node_lines() {
    assert_eq!(
        run_both(FORMAT),
        lines(&[
            "1,234,567.891 | -0 | -0 | 1,000,000,000,000,000,000,000 | 0 | 0.3 | NaN | -∞",
            "1.234.567,891 | 1,5 | 12.345.678.901.234.567.890 | 31 | 42 | 0",
            "12,34,56,789 1000 | 10.000 1.000",
            "1000 | 10,000 1234567",
            "988M | 9.9K | 159 | 0.16 | 1M 99 thousand | -1.5 thousand",
            "9877万 1,2 Millionen",
            "+1 | +0 | -0 | +NaN +1 | 0 | 0 | -1 0 | -1",
            "1.5 | 1.6",
            "1.25 | -1.20",
            "2 | 4 | -2 005 | 005.10",
            "๑,๒๓๔.๕ 一,二三四 ١٬٢٣٤٫٥ 非數值",
            "[{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"9\"},{\"type\":\"decimal\",\"value\":\".\"},{\"type\":\"fraction\",\"value\":\"9\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"compact\",\"value\":\"thousand\"}]",
            "[{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"1\"},{\"type\":\"group\",\"value\":\".\"},{\"type\":\"integer\",\"value\":\"234\"},{\"type\":\"decimal\",\"value\":\",\"},{\"type\":\"fraction\",\"value\":\"5\"}]",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"decimal\",\"minimumIntegerDigits\":1,\"minimumFractionDigits\":0,\"maximumFractionDigits\":3,\"useGrouping\":\"auto\",\"notation\":\"standard\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
            "{\"locale\":\"de-u-nu-arab\",\"numberingSystem\":\"arab\",\"style\":\"decimal\",\"minimumIntegerDigits\":1,\"minimumSignificantDigits\":1,\"maximumSignificantDigits\":3,\"useGrouping\":\"min2\",\"notation\":\"compact\",\"compactDisplay\":\"short\",\"signDisplay\":\"never\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"currency\",\"currency\":\"EUR\",\"currencyDisplay\":\"symbol\",\"currencySign\":\"standard\",\"minimumIntegerDigits\":1,\"minimumFractionDigits\":2,\"maximumFractionDigits\":2,\"minimumSignificantDigits\":1,\"maximumSignificantDigits\":21,\"useGrouping\":\"always\",\"notation\":\"standard\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"morePrecision\",\"trailingZeroDisplay\":\"auto\"}",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"unit\",\"unit\":\"kilometer-per-hour\",\"unitDisplay\":\"long\",\"minimumIntegerDigits\":1,\"minimumFractionDigits\":0,\"maximumFractionDigits\":3,\"useGrouping\":\"auto\",\"notation\":\"standard\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
            "1,234.5 1.234,5 -12,345,679T 5.00",
        ])
    );
}

const BIG: &str = r#"const big = 10n ** 40000n;
const s = new Intl.NumberFormat("en").format(-big);
print((10n ** 32768n).toLocaleString("en").length, s.slice(0, 8), s.length, s.slice(-4), (10n ** 32767n).toLocaleString("en").length);
const one = new Intl.NumberFormat("en", {maximumSignificantDigits: 1}).format(10n ** 32768n - 1n);
print(one.length, one.slice(0, 6), one.slice(-4));
print(new Intl.NumberFormat("en", {notation: "compact"}).format(big).slice(-6), new Intl.NumberFormat("en", {notation: "compact", maximumFractionDigits: 2}).format(big + 5n * 10n ** 9n).slice(-8), new Intl.NumberFormat("de", {notation: "compact", compactDisplay: "long"}).format(big).slice(-10));
print(new Intl.NumberFormat("de", {maximumSignificantDigits: 3, roundingMode: "ceil"}).format(big + 1n).slice(0, 8), new Intl.NumberFormat("en", {maximumSignificantDigits: 2, roundingMode: "floor"}).format(-(big + 1n)).slice(0, 6));
print(new Intl.NumberFormat("en", {minimumFractionDigits: 2}).format(big).slice(-5), new Intl.NumberFormat("en-IN").format(big).slice(0, 9));
const p = new Intl.NumberFormat("en", {useGrouping: false}).formatToParts(-big);
print(p.length, p[0].type, p[1].type, p[1].value.length, p[1].value.slice(0, 3));
const t = 10n ** 39998n;
const f = (m, x) => new Intl.NumberFormat("en", {maximumSignificantDigits: 2, roundingMode: m}).format(x).slice(0, 5);
print(f("halfEven", 225n * t), f("halfEven", 235n * t), f("halfExpand", -225n * t), f("halfTrunc", 225n * t), f("halfTrunc", 225n * t + 1n), f("halfCeil", -225n * t), f("halfFloor", -225n * t), f("expand", 221n * t), f("trunc", -229n * t), f("ceil", -221n * t), f("floor", 229n * t), f("halfEven", 995n * t));
"#;

/// A BigInt of 32,768 digits or more (fixed_decimal holds at most 32,768 digits, and a
/// rounding carry past magnitude 32,767 gives zero): the exact digits with the grouping, sign
/// and digits of the locale, the rounding modes, compact notation and formatToParts (with no
/// grouping: 2 parts, so the GC stress run stays short). Old code: "NaN" (Codex r1 finding
/// 2), and "0" for the carry.
#[test]
fn large_bigints_give_the_node_lines() {
    assert_eq!(
        run_both(BIG),
        lines(&[
            "43691 -10,000, 53335 ,000 43690",
            "43691 100,00 ,000",
            "0,000T ,000.01T  Billionen",
            "10.100.0 -11,00",
            "00.00 10,00,00,",
            "2 minusSign integer 40001 100",
            "22,00 24,00 -23,0 22,00 23,00 -22,0 -23,0 23,00 -22,0 -22,0 22,00 100,0",
        ])
    );
}

const BIDI: &str = r#"const p = (l, o, x) => JSON.stringify(new Intl.NumberFormat(l, o).formatToParts(x).filter(q => q.type !== "integer").map(q => [q.type, [...q.value].map(c => c.codePointAt(0).toString(16)).join("+")]));
for (const [l, o, x] of [["fa", {}, -1], ["he", {}, -1], ["ar-EG", {}, -1], ["ps", {}, -1], ["fa", {signDisplay: "always"}, 1], ["ur", {}, -1]]) print(l, p(l, o, x), new Intl.NumberFormat(l, o).format(x).length);
"#;

/// The bidi marks of a sign affix (U+200E, U+200F, U+061C at its start or end) are literal
/// parts, as in node (review B2). Old code: one minusSign or plusSign part with the mark.
#[test]
fn sign_bidi_marks_are_literal_parts() {
    assert_eq!(
        run_both(BIDI),
        lines(&[
            "fa [[\"literal\",\"200e\"],[\"minusSign\",\"2212\"]] 3",
            "he [[\"literal\",\"200e\"],[\"minusSign\",\"2d\"]] 3",
            "ar-EG [[\"literal\",\"61c\"],[\"minusSign\",\"2d\"]] 3",
            "ps [[\"literal\",\"200e\"],[\"minusSign\",\"2d\"],[\"literal\",\"200e\"]] 4",
            "fa [[\"literal\",\"200e\"],[\"plusSign\",\"2b\"]] 3",
            "ur [[\"literal\",\"200e\"],[\"minusSign\",\"2d\"]] 3",
        ])
    );
}

const FP2: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, o).format(x)).join(" | ");
print(fmt("en", {maximumSignificantDigits: 21}, 0.1 + 0.2, 1e21, 1.005, 123456789.12345678, 2 ** 53 + 2));
print(fmt("en", {maximumSignificantDigits: 3}, 5e-324));
"#;

/// INTL-V2-FP2 (plan §2.7): the digits of a Number are the digits of Number::toString, read
/// into a Decimal. With 21 significant digits the exact binary value would show more digits
/// (0.1 + 0.2 is 0.3000000000000000444089... in binary; 1.005 is 1.00499999999999989...),
/// so these node lines hold only for the Number::toString digits (review B3).
#[test]
fn number_digits_are_the_number_to_string_digits() {
    let tiny = format!("0.{}5", "0".repeat(323));
    assert_eq!(
        run_both(FP2),
        lines(&[
            "0.30000000000000004 | 1,000,000,000,000,000,000,000 | 1.005 | 123,456,789.12345678 | 9,007,199,254,740,994",
            &tiny,
        ])
    );
}

const NU: &str = r#"for (const l of ["en-u-nu-arab", "de-u-nu-arab", "en-u-nu-arabext", "fr-u-nu-arab"]) print(l, JSON.stringify(new Intl.NumberFormat(l).format(-1234.5)));
"#;

/// A numbering system that the locale's data does not have: the digits of the system with
/// the locale's own separators and sign (review A5). The ICU4X 2.4 compiled data has decimal
/// symbols for a numbering system only under the locales that use it ("ar" has "arab"; not
/// "und", not "en"; a probe of DecimalSymbolsV1 with the marker attribute gave "missing data"
/// for en/arab, und/arab, und/arabext), so an "und" fallback finds nothing.
#[test]
fn numbering_system_symbols_are_the_locale_symbols() {
    assert_eq!(
        run_both(NU),
        lines(&[
            // ep9: node (ICU4C) takes the CLDR root symbols of the system: "\u{61c}-١٬٢٣٤٫٥"
            // for en, de and fr with "arab", "\u{200e}-\u{200e}۱٬۲۳۴٫۵" for en with
            // "arabext". The ICU4X data has no root symbols per system (an open point: a
            // CLDR side table of the root symbols per numbering system).
            "en-u-nu-arab \"-١,٢٣٤.٥\"",
            "de-u-nu-arab \"-١.٢٣٤,٥\"",
            "en-u-nu-arabext \"-۱,۲۳۴.۵\"",
            "fr-u-nu-arab \"-١\u{202f}٢٣٤,٥\"",
        ])
    );
}

/// Number and BigInt toLocaleString with no arguments use the default NumberFormat of the
/// cache (design INTL-I1 §4.5 "default formatters"; review A6): after a call, the default
/// slot of the service holds the resolved options, and an argument call does not change it.
#[test]
fn to_locale_string_with_no_arguments_uses_the_default_formatter() {
    use ep9_intl::cache::{Service, get_or_make_default};
    use ep9_intl::number_format::Resolved;
    let cached = || get_or_make_default::<Resolved>(Service::NumberFormat, || None);
    assert!(cached().is_none());
    assert_eq!(
        common::run("print((1234.5).toLocaleString(\"de\"))", false),
        lines(&["1.234,5"])
    );
    assert!(cached().is_none());
    let out = common::run(
        "const nf = new Intl.NumberFormat(); print((1234.5).toLocaleString() === nf.format(1234.5), (12345n).toLocaleString() === nf.format(12345n), (5).toLocaleString(undefined, undefined) === nf.format(5)); print(nf.resolvedOptions().locale)",
        false,
    )
    .expect("runs");
    assert_eq!(out[0], "true true true");
    let r = cached().expect("the default formatter");
    assert_eq!(r.locale, out[1]);
}

/// The passes over the digits of a WideDecimal poll: one budget unit per 4,096 digits (Codex
/// r2 finding 2). 10n ** 1500000n + 1n has 1,500,001 digits. Its BigInt::toString (in
/// ToIntlMathematicalValue) polls, but stays under the budget of 10,000 units (the
/// toString of 10n ** 1600000n + 1n uses it up). The format then reads, copies, rounds (the
/// increment 5: a division and two products) and groups all the digits, about 10 passes, so
/// the budget runs out in the format. Old code: no poll in these passes, so the call ends and
/// `stage` is "reached".
#[test]
fn a_termination_stops_the_format_of_a_huge_bigint() {
    let call = "new Intl.NumberFormat('en', {roundingIncrement: 5, maximumFractionDigits: 0}).format(10n ** 1500000n + 1n)";
    assert_eq!(
        common::terminate_during(call),
        Some((true, Some("before".to_string()), Some("again".to_string())))
    );
}

/// formatToParts takes one budget unit for each part that it makes into an object (H23: one
/// unit per pass of a builtin loop). 10n ** 40000n gives 13,334 integer parts and 13,333
/// group parts, more than the budget of 10,000 units. Old code: no poll in the loop, so the
/// call ends and `stage` is "reached".
#[test]
fn a_termination_stops_the_parts_loop() {
    let call = "new Intl.NumberFormat('en').formatToParts(10n ** 40000n)";
    assert_eq!(
        common::terminate_during(call),
        Some((true, Some("before".to_string()), Some("again".to_string())))
    );
}

const CURRENCY: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, o).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, o).formatToParts(x).map(q => q.type + ":" + q.value).join(",");
const usd = {style: "currency", currency: "USD"};
const acc = {style: "currency", currency: "USD", currencySign: "accounting"};
print(fmt("en-US", acc, -987, -0.0001, -0, 0, 987), fmt("en-US", {...acc, signDisplay: "always"}, -987, 0, 987), fmt("en-US", {...acc, signDisplay: "negative"}, -0.0001, -987));
print(fmt("ko-KR", acc, -987, 987), fmt("de-DE", acc, -987, 987), fmt("ja-JP", {...acc, signDisplay: "exceptZero"}, -0, -987, 987));
print(fmt("en", usd, -1234.5, NaN, -Infinity), fmt("en", acc, -Infinity), fmt("en", {...usd, currencyDisplay: "code"}, -1), fmt("en", {...usd, currencyDisplay: "narrowSymbol"}, 1));
print(fmt("ko", usd, 1), fmt("ko", {...usd, currencyDisplay: "narrowSymbol"}, 1), fmt("fr", usd, 1), fmt("en", {style: "currency", currency: "XYZ"}, 1));
print(fmt("en", {...usd, currencyDisplay: "name"}, 1, -1, 1234.5), fmt("en", {style: "currency", currency: "XYZ", currencyDisplay: "name"}, 1));
print(fmt("ru", {style: "currency", currency: "RUB", currencyDisplay: "name"}, 1, 2, 5), fmt("ja", {...usd, currencyDisplay: "name"}, 1));
print(fmt("en", {style: "currency", currency: "JPY"}, 1234.5), fmt("en", {style: "currency", currency: "KWD"}, 1.23456), fmt("de", {style: "currency", currency: "EUR"}, 1234.5));
print(fmt("nl", {style: "currency", currency: "EUR"}, -1), fmt("de-CH", {style: "currency", currency: "CHF"}, -1), fmt("en", {...usd, notation: "compact"}, -12345), fmt("de", {style: "currency", currency: "EUR", notation: "compact"}, 1234567));
print(["JPY", "KWD", "USD", "HUF", "CLF", "ABC"].map(c => new Intl.NumberFormat("en", {style: "currency", currency: c}).resolvedOptions().maximumFractionDigits).join());
print(parts("ko-KR", acc, -987));
print(parts("de-DE", acc, -987));
print(parts("en", {...usd, currencyDisplay: "name"}, -1));
print(parts("nl", {style: "currency", currency: "EUR"}, -1));
print(parts("he", {style: "currency", currency: "ILS"}, -1).replace(/[‎‏]/g, c => "<" + c.charCodeAt(0).toString(16) + ">"));
"#;

/// INTL W3 NF-CURRENCY: the style "currency" (ECMA-402 §16.5.1 CurrencyDigits, §16.5.3,
/// §16.5.4): `currencySign` "accounting" with each `signDisplay` (the negative pattern of the
/// data, "($987.00)", and the sign before the pattern where the data has none, "-987,00 $"),
/// `currencyDisplay` symbol, narrowSymbol, code (the "alpha next to number" space) and name
/// (by the plural category of the rounded value; the ISO code where the data has no name),
/// an unknown code, NaN and the infinities, compact notation, the CLDR currency digits
/// (JPY 0, KWD 3, HUF 0, CLF 4), a "-" in a negative pattern of the data (nl, de-CH, he), and
/// the currency, literal and minusSign parts. Old code (the W3 skeleton stub): the decimal
/// style with 2 fraction digits ("-987.00", no currency).
#[test]
fn currency_gives_the_node_lines() {
    assert_eq!(
        run_both(CURRENCY),
        lines(&[
            "($987.00) | ($0.00) | ($0.00) | $0.00 | $987.00 ($987.00) | +$0.00 | +$987.00 $0.00 | ($987.00)",
            "(US$987.00) | US$987.00 -987,00\u{a0}$ | 987,00\u{a0}$ $0.00 | ($987.00) | +$987.00",
            "-$1,234.50 | $NaN | -$∞ ($∞) -USD\u{a0}1.00 $1.00",
            "US$1.00 $1.00 1,00\u{a0}$US XYZ\u{a0}1.00",
            "1.00 US dollars | -1.00 US dollars | 1,234.50 US dollars 1.00 XYZ",
            "1,00 российского рубля | 2,00 российского рубля | 5,00 российского рубля 1.00米ドル",
            "¥1,235 KWD\u{a0}1.235 1.234,50\u{a0}€",
            "€\u{a0}-1,00 CHF-1.00 -$12K 1,2\u{a0}Mio.\u{a0}€",
            "0,3,2,0,4,2",
            "literal:(,currency:US$,integer:987,decimal:.,fraction:00,literal:)",
            "minusSign:-,integer:987,decimal:,,fraction:00,literal:\u{a0},currency:$",
            "minusSign:-,integer:1,decimal:.,fraction:00,literal: ,currency:US dollars",
            "currency:€,literal:\u{a0},minusSign:-,integer:1,decimal:,,fraction:00",
            "literal:<200f><200e>,minusSign:-,integer:1,decimal:.,fraction:00,literal:\u{a0}<200f>,currency:₪",
        ])
    );
}

const PERCENT: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, o).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, o).formatToParts(x).map(q => q.type + ":" + q.value).join(",");
const pct = {style: "percent"};
print(fmt("en", pct, 0.2, 0.011, -1.23, -0, 1234.5678), fmt("en", {...pct, signDisplay: "always"}, 0.5, 0), fmt("en", {...pct, maximumFractionDigits: 2}, 0.12345));
print(fmt("de", pct, -0.5), fmt("tr", pct, -0.5), fmt("fr", pct, 0.5), fmt("en", pct, "0.125", 5n, NaN, Infinity));
print(parts("en-US", pct, -123));
print(parts("de", pct, -0.5));
print(parts("tr", pct, -0.5));
print(parts("ar-u-nu-latn", pct, -0.5).replace(/[‎‏]/g, c => "<" + c.charCodeAt(0).toString(16) + ">"));
const big = new Intl.NumberFormat("en", {...pct, useGrouping: false}).format(10n ** 32766n);
print(big.length, big.slice(0, 3), big.slice(-3), new Intl.NumberFormat("en", {...pct, useGrouping: false}).format(-(10n ** 40000n)).length);
"#;

/// INTL W3 NF-CURRENCY: the style "percent" (§16.5.3: x × 100 first; the percent pattern of
/// the locale with the sign before it, or in its place where the data puts it: tr "-%50"),
/// the percentSign and literal parts (a space and the bidi marks of the pattern are
/// literals), and a BigInt whose highest digit x × 100 moves past the i16 magnitudes of
/// fixed_decimal (10n ** 32766n: 32,767 digits; a Decimal would overflow to zero). Old code:
/// the decimal style ("0.2", no percent sign).
#[test]
fn percent_gives_the_node_lines() {
    assert_eq!(
        run_both(PERCENT),
        lines(&[
            "20% | 1% | -123% | -0% | 123,457% +50% | +0% 12.35%",
            "-50\u{a0}% -%50 50\u{a0}% 13% | 500% | NaN% | ∞%",
            "minusSign:-,integer:12,group:,,integer:300,percentSign:%",
            "minusSign:-,integer:50,literal:\u{a0},percentSign:%",
            "minusSign:-,percentSign:%,integer:50",
            "literal:<200e>,minusSign:-,integer:50,literal:<200e>,percentSign:%,literal:<200e>",
            "32770 100 00% 40005",
        ])
    );
}

const PERCENT_CARRY: &str = r#"const nf = new Intl.NumberFormat("en", {style: "percent", maximumSignificantDigits: 1, useGrouping: false});
for (const k of [32765, 32766, 32767]) {
  const v = 10n ** BigInt(k) - 1n, want = "1" + "0".repeat(k + 2) + "%";
  print(k, nf.format(v).length, nf.format(v) === want, nf.format(-v) === "-" + want, nf.formatRange(v, 10n ** BigInt(k)) === "~" + want);
}"#;

/// The style "percent" with a rounding carry at the i16 limit of fixed_decimal: the BigInt
/// 10n ** 32766n - 1n (32,766 nines) times 100 has its highest digit at magnitude 32,767
/// (i16::MAX), and one significant digit carries it to 32,768. x × 100 is a WideDecimal from
/// that magnitude on, so the carry is exact ("1" and 32,768 zeros). The all-nines BigInts one
/// digit shorter and one digit longer, the negative values and the range ("~", both ends
/// round to the same text) too. Old code: "0%" (the Decimal overflow gives zero).
#[test]
fn percent_rounding_carry_at_the_i16_limit() {
    assert_eq!(
        run_both(PERCENT_CARRY),
        lines(&[
            "32765 32769 true true true",
            "32766 32770 true true true",
            "32767 32771 true true true",
        ])
    );
}
