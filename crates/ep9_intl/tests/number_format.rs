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
