//! Intl.NumberFormat notations "scientific" and "engineering" and the number ranges
//! (`formatRange`, `formatRangeToParts`) (ECMA-402 §16.3.4, §16.3.5, §16.5.3, §16.5.12,
//! §16.5.13, §16.5.18-§16.5.23; INTL W3 NF-SCIRANGE), with and without GC stress. The
//! expected lines are node v26.13.0's output for the same script (`print` = `console.log` of
//! the arguments, each through `String`, joined by a space; ICU 78.3).

mod common;
use common::{lines, run_both};

const SCIENTIFIC: &str = r#"const fmt = (l, o, ...xs) => xs.map(x => new Intl.NumberFormat(l, o).format(x)).join(" | ");
const parts = (l, o, x) => new Intl.NumberFormat(l, o).formatToParts(x).map(q => q.type + ":" + q.value).join(",");
const sci = {notation: "scientific"}, eng = {notation: "engineering"};
print(fmt("en", sci, 0.000345, 0.345, 3.45, 34.5, 543, 5430, 543000, 543211.1), "/", fmt("en", eng, 0.000345, 0.345, 3.45, 34.5, 543, 5430, 543000, 543211.1));
print(fmt("de-DE", sci, 0.000345, 543211.1, -Infinity, NaN), "/", fmt("zh-TW", eng, 0.000345, NaN));
print(fmt("en", sci, 999999, 9.9999, 0, -0, 1e21, 5e-324, -0.00012, 123456789n * 10n ** 30n), "/", fmt("en", {...eng, maximumSignificantDigits: 2}, 999999, 0.000999, -1234));
print(fmt("en", {...sci, minimumIntegerDigits: 5}, 12345, 0.5), "/", fmt("en", {...sci, signDisplay: "always"}, 1234, 0, -0.001), "/", fmt("en", {...sci, signDisplay: "exceptZero"}, 0, 0.0001));
print(fmt("en", {...sci, roundingIncrement: 5, maximumFractionDigits: 2, minimumFractionDigits: 2}, 1234), "/", fmt("en", {...sci, trailingZeroDisplay: "stripIfInteger", minimumFractionDigits: 2}, 1000, 1234), "/", fmt("en", {...sci, roundingPriority: "lessPrecision", maximumFractionDigits: 2, maximumSignificantDigits: 2}, 1234));
print(parts("ar", sci, 0.00012), "/", parts("ar-EG", sci, 0.00012), "/", parts("fa", sci, -12345), "/", parts("sv", sci, 0.00012), "/", parts("ps", sci, 0.00012));
print(fmt("en", {...sci, style: "percent"}, 0.5, -12), "/", fmt("de", {...sci, style: "percent"}, 0.5), "/", fmt("en", {...sci, style: "currency", currency: "USD"}, 1234, -1234), "/", fmt("en", {...sci, style: "currency", currency: "USD", currencySign: "accounting"}, -1234), "/", fmt("de", {...sci, style: "currency", currency: "EUR"}, 1234));
print(fmt("en", {...sci, style: "currency", currency: "USD", currencyDisplay: "name"}, 1, 1000, 2), "/", fmt("en", {...sci, style: "unit", unit: "meter", unitDisplay: "long"}, 1, 1000, 1.5), "/", fmt("fr", {...sci, style: "unit", unit: "meter", unitDisplay: "long"}, 1000000, 1, 1.5));
print(parts("en", {...eng, style: "unit", unit: "kilometer-per-hour"}, -12345), "/", (0.000345).toLocaleString("en", eng), (12345n).toLocaleString("de", sci));"#;

/// The exponent of scientific and engineering notation from the magnitude of the Decimal
/// (INTL-V2-FP3): the test262 values, a rounding that moves the magnitude (999,999 is "1E6"),
/// zero and negative zero, the Number and BigInt limits, the digit options, the exponent
/// parts of the locales (ar bidi marks, ar-EG and fa digits and separators, sv minus sign),
/// the exponent inside the percent, currency and unit patterns, and the plural form of the
/// value (en "1E3 meters", fr "1,5E0 mètre"). Old code: the decimal style in standard
/// notation ("0.000345" is "0").
#[test]
fn scientific_and_engineering_give_the_node_lines() {
    assert_eq!(
        run_both(SCIENTIFIC),
        lines(&[
            "3.45E-4 | 3.45E-1 | 3.45E0 | 3.45E1 | 5.43E2 | 5.43E3 | 5.43E5 | 5.432E5 / 345E-6 | 345E-3 | 3.45E0 | 34.5E0 | 543E0 | 5.43E3 | 543E3 | 543.211E3",
            "3,45E-4 | 5,432E5 | -∞ | NaN / 345E-6 | 非數值",
            "1E6 | 1E1 | 0E0 | -0E0 | 1E21 | 5E-324 | -1.2E-4 | 1.235E38 / 1E6 | 1E-3 | -1.2E3",
            "00,001.235E4 | 00,005E-1 / +1.234E3 | +0E0 | -1E-3 / 0E0 | +1E-4",
            "1.25E3 / 1E3 | 1.234E3 / 1.2E3",
            "integer:1,decimal:.,fraction:2,exponentSeparator:E,literal:\u{200e},exponentMinusSign:-,exponentInteger:4 / integer:١,decimal:٫,fraction:٢,exponentSeparator:أس,literal:\u{61c},exponentMinusSign:-,exponentInteger:٤ / literal:\u{200e},minusSign:−,integer:۱,decimal:٫,fraction:۲۳۵,exponentSeparator:×۱۰^,exponentInteger:۴ / integer:1,decimal:,,fraction:2,exponentSeparator:×10^,exponentMinusSign:−,exponentInteger:4 / integer:۱,decimal:٫,fraction:۲,exponentSeparator:×۱۰^,literal:\u{200e},exponentMinusSign:-,literal:\u{200e},exponentInteger:۴",
            "5E1% | -1E3% / 5E1\u{a0}% / $1.234E3 | -$1.234E3 / ($1.234E3) / 1,234E3\u{a0}€",
            "1E0 US dollar | 1E3 US dollars | 2E0 US dollars / 1E0 meter | 1E3 meters | 1.5E0 meters / 1E6\u{a0}mètres | 1E0\u{a0}mètre | 1,5E0\u{a0}mètre",
            "minusSign:-,integer:12,decimal:.,fraction:345,exponentSeparator:E,exponentInteger:3,literal: ,unit:km/h / 345E-6 1,235E4",
        ])
    );
}

const RANGES: &str = r#"const fr = (l, o, ...ps) => { const nf = new Intl.NumberFormat(l, o); return ps.map(([a, b]) => nf.formatRange(a, b)).join(" | "); };
const frp = (l, o, a, b) => new Intl.NumberFormat(l, o).formatRangeToParts(a, b).map(q => q.type + ":" + q.value + ":" + q.source).join(",");
const usd0 = {style: "currency", currency: "USD", maximumFractionDigits: 0}, eur0 = {style: "currency", currency: "EUR", maximumFractionDigits: 0};
print(fr("en-US", usd0, [3, 5], [2.9, 3.1], [-1, -1], [-3, -5]), "/", fr("en-US", {style: "currency", currency: "USD", signDisplay: "always"}, [2.9, 3.1]), "/", fr("en-US", {}, ["987654321987654321", "987654321987654322"]));
print(fr("pt-PT", eur0, [3, 5], [2.9, 3.1], [-3, -3]), "/", fr("pt-PT", {style: "currency", currency: "EUR", signDisplay: "always"}, [2.9, 3.1]), "/", fr("pt-PT", {}, ["987654321987654321", "987654321987654322"]));
print(fr("en", {}, [3, 5], [3, -5], [-3, 5], [-5, -3], [1, 1], [-1, -1], [0, 0], [-0, -0], [2.9, 3.1], [Infinity, Infinity], [-Infinity, Infinity], [1000, 2000], [23, 12], [5n, 3n]));
print(fr("en", {signDisplay: "always"}, [3, 5], [-1, 1], [1, 1], [0, 0]), "/", fr("en", {signDisplay: "exceptZero"}, [0, 1], [-0, 0]));
print(fr("en", {notation: "scientific"}, [1000, 2000], [1000, 1000], [0.001, 1000], [-1000, -2000]), "/", fr("en", {notation: "engineering"}, [1000, 20000]), "/", fr("en", {notation: "compact"}, [1000, 2000], [1000, 2000000], [1500, 1500], [-1000, -2000]), "/", fr("en", {notation: "compact", compactDisplay: "long"}, [1000, 2000], [1, 1000]));
print(fr("en", {style: "percent"}, [0.03, 0.05], [-0.03, 0.05], [0.5, 0.5]), "/", fr("de", {style: "percent"}, [0.03, 0.05], [0.5, 0.5]), "/", fr("tr", {style: "percent"}, [0.03, 0.03]));
print(fr("en", {style: "currency", currency: "EUR"}, [3, 5], [-3, -5], [-3, 5]), "/", fr("en", {style: "currency", currency: "USD", currencySign: "accounting"}, [-3, -5], [-3, 5], [-3, -3], [3, 3]), "/", fr("de", {style: "currency", currency: "EUR"}, [3, 5], [-3, 5], [3, 3], [-3, -3]));
print(fr("en", {style: "currency", currency: "USD", currencyDisplay: "name"}, [1, 5], [1, 1], [-3, 5]), "/", fr("en", {style: "unit", unit: "kilometer-per-hour"}, [3, 5], [-3, 5], [1, 1]), "/", fr("en", {style: "unit", unit: "kilometer-per-hour", unitDisplay: "long"}, [1, 5], [0, 1], [1, 1]));
print(fr("ru", {style: "unit", unit: "kilometer", unitDisplay: "long"}, [1, 2], [1, 5], [2, 5]), "/", fr("fr", {style: "unit", unit: "kilometer", unitDisplay: "long"}, [1, 2]), "/", fr("en", {style: "unit", unit: "meter", unitDisplay: "narrow"}, [3, 5]), "/", fr("en", {style: "unit", unit: "percent"}, [3, 5]));
print(fr("ja", {style: "currency", currency: "JPY"}, [3, 5]), "/", fr("ja", {}, [3, 5], [1, 1]), "/", fr("de", {}, [1, 1]), "/", fr("fr", {}, [1, 1], [-3, -1]), "/", fr("es", {}, [3, 5]), "/", fr("zh", {}, [3, 5]));
print(fr("ar", {}, [3, 5], [-3, -1], [-3, -3]), "/", fr("he", {}, [-3, 5], [-3, -3]), "/", fr("ar", {style: "currency", currency: "EUR"}, [3, 3], [-3, -3]), "/", fr("he", {style: "currency", currency: "USD"}, [3, 3], [-3, -3]), "/", fr("fa", {style: "currency", currency: "EUR"}, [3, 3], [-3, -3]), "/", fr("nl", {style: "currency", currency: "EUR"}, [3, 3], [-3, -3]), "/", fr("de-CH", {style: "currency", currency: "CHF"}, [3, 3]));
print(frp("en-US", usd0, 3, 5), "/", frp("en-US", usd0, 1, 1), "/", frp("en-US", usd0, 2.999, 3.001));
print(frp("en-US", {style: "currency", currency: "USD", signDisplay: "always"}, 2.9, 3.1), "/", frp("pt-PT", eur0, 3, 5));
print(frp("en", {notation: "scientific"}, 0.001, 1000), "/", frp("en", {style: "unit", unit: "kilometer-per-hour", unitDisplay: "long"}, -1, 5), "/", frp("en", {style: "currency", currency: "USD", currencySign: "accounting"}, -3, -5));
print(frp("ar", {}, -3, -3), "/", frp("he", {style: "currency", currency: "USD"}, -3, -3));"#;

/// formatRange and formatRangeToParts: the test262 strings (en-US, pt-PT), the
/// approximately sign and its place (before the sign, at the sign place of the pattern: nl
/// "€ ~3,00", ar, he), the collapse of the middle layer (more than one code point: "-$3–5",
/// "+$2.90–3.10", not "$3 – $5") and of the unit or currency name (with the plural form of
/// the range: ru), the spaces around the separator, the exponent, the compact affixes and the
/// sources of the parts. Old code: a TypeError.
#[test]
fn ranges_give_the_node_lines() {
    assert_eq!(
        run_both(RANGES),
        lines(&[
            "$3 – $5 | ~$3 | ~-$1 | -$3–5 / +$2.90–3.10 / 987,654,321,987,654,321–987,654,321,987,654,322",
            "3 - 5\u{a0}€ | ~3\u{a0}€ | ~-3\u{a0}€ / +2,90 - 3,10\u{a0}€ / 987\u{a0}654\u{a0}321\u{a0}987\u{a0}654\u{a0}321 - 987\u{a0}654\u{a0}321\u{a0}987\u{a0}654\u{a0}322",
            "3–5 | 3–-5 | -3 – 5 | -5 – -3 | ~1 | ~-1 | ~0 | ~-0 | 2.9–3.1 | ~∞ | -∞ – ∞ | 1,000–2,000 | 23–12 | 5–3",
            "+3 – +5 | -1 – +1 | ~+1 | ~+0 / 0–+1 | ~0",
            "1E3 – 2E3 | ~1E3 | 1E-3 – 1E3 | -1E3 – -2E3 / 1E3 – 20E3 / 1K – 2K | 1K – 2M | ~1.5K | -1–2K / 1–2 thousand | 1–1 thousand",
            "3% – 5% | -3% – 5% | ~50% / 3–5\u{a0}% | ≈50\u{a0}% / ~%3",
            "€3.00 – €5.00 | -€3.00–5.00 | -€3.00 – €5.00 / ($3.00–5.00) | ($3.00) – $5.00 | ($3.00) | ~$3.00 / 3,00–5,00\u{a0}€ | -3,00\u{a0}€ – 5,00\u{a0}€ | ≈3,00\u{a0}€ | ≈-3,00\u{a0}€",
            "1.00–5.00 US dollars | ~1.00 US dollars | -3.00 – 5.00 US dollars / 3–5 km/h | -3 – 5 km/h | ~1 km/h / 1–5 kilometers per hour | 0–1 kilometers per hour | ~1 kilometer per hour",
            "1–2 километра | 1–5 километров | 2–5 километров / 1–2\u{a0}kilomètres / 3–5m / 3% – 5%",
            "￥3 ～ ￥5 / 3～5 | 約1 / ≈1 / ≃1 | -3 – -1 / 3-5 / 3-5",
            "3–5 | \u{200e}-3–1 | ~\u{200e}-3 / \u{200e}-3 – 5 | ~\u{200e}-3 / \u{200f}~3.00\u{a0}€ | \u{200f}~\u{200e}-3.00\u{a0}€ / \u{200f}~3.00\u{a0}\u{200f}$ | \u{200f}~\u{200e}-3.00\u{a0}\u{200f}$ / ~\u{200e}€۳٫۰۰ | ~\u{200e}−\u{200e}€۳٫۰۰ / €\u{a0}~3,00 | €\u{a0}~-3,00 / CHF≈3.00",
            "currency:$:startRange,integer:3:startRange,literal: – :shared,currency:$:endRange,integer:5:endRange / approximatelySign:~:shared,currency:$:shared,integer:1:shared / approximatelySign:~:shared,currency:$:shared,integer:3:shared",
            "plusSign:+:shared,currency:$:shared,integer:2:startRange,decimal:.:startRange,fraction:90:startRange,literal:–:shared,integer:3:endRange,decimal:.:endRange,fraction:10:endRange / integer:3:startRange,literal: - :shared,integer:5:endRange,literal:\u{a0}:shared,currency:€:shared",
            "integer:1:startRange,exponentSeparator:E:startRange,exponentMinusSign:-:startRange,exponentInteger:3:startRange,literal: – :shared,integer:1:endRange,exponentSeparator:E:endRange,exponentInteger:3:endRange / minusSign:-:startRange,integer:1:startRange,literal: – :shared,integer:5:endRange,literal: :shared,unit:kilometers per hour:shared / literal:(:shared,currency:$:shared,integer:3:startRange,decimal:.:startRange,fraction:00:startRange,literal:–:shared,integer:5:endRange,decimal:.:endRange,fraction:00:endRange,literal:):shared",
            "approximatelySign:~:shared,literal:\u{200e}:shared,minusSign:-:shared,integer:3:shared / literal:\u{200f}:shared,approximatelySign:~:shared,literal:\u{200e}:shared,minusSign:-:shared,integer:3:shared,decimal:.:shared,fraction:00:shared,literal:\u{a0}\u{200f}:shared,currency:$:shared",
        ])
    );
}

const ERRORS: &str = r#"const nf = new Intl.NumberFormat();
const err = f => { try { return String(f()); } catch (e) { return e.name; } };
const log = [];
const v = (name, x) => ({ valueOf() { log.push(name); return x; } });
print(err(() => nf.formatRange(undefined, 23)), err(() => nf.formatRange(1, undefined)), err(() => nf.formatRangeToParts(undefined, undefined)), err(() => nf.formatRange(Symbol(), 1)), err(() => nf.formatRange(1, Symbol())));
print(err(() => nf.formatRange(NaN, 23)), err(() => nf.formatRange(12, NaN)), err(() => nf.formatRangeToParts(NaN, NaN)), err(() => nf.formatRange("abc", 1)), err(() => nf.formatRange(1, 2n)));
print(err(() => nf.formatRange(undefined, v("y", 1))), log.join(), err(() => nf.formatRange(v("x", NaN), v("y", 1))), log.join());
print(err(() => Intl.NumberFormat.prototype.formatRange.call({}, 1, 2)), err(() => Intl.NumberFormat.prototype.formatRangeToParts.call(new Intl.PluralRules(), 1, 2)), err(() => Intl.NumberFormat.prototype.formatRange.call(nf, undefined, 2)));
print(typeof nf.formatRange(23, 12), typeof nf.formatRange(23n, 12n), typeof nf.formatRange(Infinity, -0), nf.formatRange(0, -0), nf.formatRange(-0, -Infinity), nf.formatRangeToParts(23, -Infinity).length);
const p = nf.formatRangeToParts(1, 2)[0];
print(Object.getPrototypeOf(p) === Object.prototype, Object.keys(p).join(), JSON.stringify(Object.getOwnPropertyDescriptor(p, "source")));
print(nf.formatRange.length, nf.formatRangeToParts.length, nf.formatRange.name, Object.keys(Intl.NumberFormat.prototype).length);"#;

/// The steps before PartitionNumberRangePattern: a TypeError for an undefined end (before
/// ToIntlMathematicalValue), for a Symbol and for a receiver that is not a NumberFormat; a
/// RangeError for NaN (after both values); x > y does not throw; the part objects and the
/// method properties. Old code: a TypeError for every call.
#[test]
fn range_errors_and_parts_give_the_node_lines() {
    assert_eq!(
        run_both(ERRORS),
        lines(&[
            "TypeError TypeError TypeError TypeError TypeError",
            "RangeError RangeError RangeError RangeError 1–2",
            "TypeError  RangeError x,y",
            "TypeError TypeError TypeError",
            "string string string 0–-0 -0 – -∞ 4",
            "true type,value,source {\"value\":\"startRange\",\"writable\":true,\"enumerable\":true,\"configurable\":true}",
            "2 2 formatRange 0",
        ])
    );
}

/// formatRangeToParts takes one budget unit for each part that it makes into an object (H23).
/// 10n ** 40000n gives 26,667 parts on each side, more than the budget of 10,000 units. Old
/// code: a TypeError, no termination.
#[test]
fn a_termination_stops_the_range_parts_loop() {
    let call = "new Intl.NumberFormat('en').formatRangeToParts(10n ** 40000n, 10n ** 40000n + 1n)";
    assert_eq!(
        common::terminate_during(call),
        Some((true, Some("before".to_string()), Some("again".to_string())))
    );
}
