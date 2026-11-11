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
