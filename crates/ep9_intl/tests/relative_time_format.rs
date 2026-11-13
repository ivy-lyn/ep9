//! Intl.RelativeTimeFormat (ECMA-402 §18): INTL W3 RTF, with and without GC stress. The
//! expected lines are node v26.13.0's output for the same script (`print` = `console.log` of
//! the arguments, each through `String`, joined by a space; ICU 78.3, CLDR 48: the strings of
//! these cases are the same in the CLDR 49 data of ICU4X 2.4).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const R = Intl.RelativeTimeFormat;
print(typeof R, R.length, R.name, Object.getPrototypeOf(R) === Function.prototype);
print(Object.getOwnPropertyNames(R).join(), Object.getOwnPropertyNames(R.prototype).join());
print(R.supportedLocalesOf.length, R.prototype.format.length, R.prototype.formatToParts.length, R.prototype.resolvedOptions.length);
const rtf = new R("en");
print(Object.prototype.toString.call(rtf), Object.isExtensible(rtf), Object.getPrototypeOf(rtf) === R.prototype);
print(JSON.stringify(R.supportedLocalesOf(["de-CH", "zz", "en-u-nu-thai"])));
print(JSON.stringify(new R().resolvedOptions()), JSON.stringify(new R("de-u-nu-arab", {style: "narrow", numeric: "auto"}).resolvedOptions()));
print(JSON.stringify(new R("en-u-nu-latn", {numberingSystem: "arab"}).resolvedOptions()), JSON.stringify(new R("en-u-nu-xyz", {numberingSystem: "abc"}).resolvedOptions()));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => R());
t(() => new R("en", null));
t(() => new R("en", "x"));
t(() => new R("en", {style: "x"}));
t(() => new R("en", {numeric: "x"}));
t(() => new R("en", {localeMatcher: "x"}));
t(() => new R("en", {numberingSystem: "a"}));
t(() => new R("x-private"));
t(() => R.prototype.format(1, "day"));
t(() => R.prototype.resolvedOptions.call({}));
t(() => rtf.format(NaN, "day"));
t(() => rtf.format(-Infinity, "day"));
t(() => rtf.format(1, "decade"));
t(() => rtf.format(1, "DAY"));
t(() => rtf.format(Symbol(), "day"));
t(() => rtf.format(1n, "day"));
t(() => rtf.formatToParts(1, Symbol()));
print(r.join(" "));
const log = [];
new R([], { get localeMatcher() { log.push("lm"); }, get numberingSystem() { log.push("nu"); }, get style() { log.push("style"); }, get numeric() { log.push("numeric"); } });
const v = { valueOf() { log.push("value"); return 1; } };
const u = { toString() { log.push("unit"); return "days"; } };
rtf.formatToParts(v, u);
print(log.join());
class S extends R {}
print(new S("de") instanceof S, new S("de").format(2, "hours"));
"#;

/// The shape of %Intl.RelativeTimeFormat% and its prototype, resolvedOptions (the "nu"
/// keyword and option), the errors of the constructor and the methods, and the order of the
/// observable reads (the options; ToNumber(value) before ToString(unit), each once).
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 RelativeTimeFormat true",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,format,formatToParts",
            "1 2 2 0",
            "[object Intl.RelativeTimeFormat] true true",
            "[\"de-CH\",\"en-u-nu-thai\"]",
            "{\"locale\":\"en-US\",\"style\":\"long\",\"numeric\":\"always\",\"numberingSystem\":\"latn\"} {\"locale\":\"de-u-nu-arab\",\"style\":\"narrow\",\"numeric\":\"auto\",\"numberingSystem\":\"arab\"}",
            "{\"locale\":\"en\",\"style\":\"long\",\"numeric\":\"always\",\"numberingSystem\":\"arab\"} {\"locale\":\"en\",\"style\":\"long\",\"numeric\":\"always\",\"numberingSystem\":\"latn\"}",
            "TypeError TypeError ok RangeError RangeError RangeError RangeError RangeError TypeError TypeError RangeError RangeError RangeError RangeError TypeError TypeError TypeError",
            "lm,nu,style,numeric,value,unit",
            "true in 2 Stunden",
        ])
    );
}

const FORMAT: &str = r#"const f = (l, o, ...xs) => xs.map(([v, u]) => new Intl.RelativeTimeFormat(l, o).format(v, u)).join(" | ");
const p = (l, o, v, u) => new Intl.RelativeTimeFormat(l, o).formatToParts(v, u).map(q => q.type + ":" + q.value + (q.unit === undefined ? "" : "@" + q.unit)).join("/");
print(f("en", {}, [1, "day"], [-1, "day"], [0, "day"], [-0, "day"], [2, "days"], [-2.5, "hours"], [1000, "seconds"], [1.0004, "minute"], [123456.789, "year"]));
print(f("en", {numeric: "auto"}, [1, "day"], [-1, "day"], [0, "day"], [-0, "day"], [2, "day"], [-1, "hour"], [0, "second"], [-0, "quarter"], [1.0, "week"], [0.5, "day"]));
print(f("en", {style: "short"}, [3, "month"], [-3, "quarter"], [1, "year"]), f("en", {style: "narrow"}, [3, "month"], [-1, "minute"]));
print(f("pl", {}, [1, "year"], [2, "year"], [5, "year"], [22, "year"], [1.5, "year"], [-0, "day"], [1000, "minute"]));
print(f("de", {numeric: "auto"}, [-2, "day"], [2, "day"], [-1, "year"]), f("fr", {}, [1.5, "hour"], [-1234.5, "second"]));
print(f("ar", {}, [3, "day"], [-11, "month"]), f("en-u-nu-arab", {}, [12, "day"]), f("en", {numberingSystem: "deva"}, [-56, "hour"]));
print(f("ja", {}, [3, "week"], [-3, "week"]), f("zh", {numeric: "auto"}, [-1, "day"], [1, "day"]));
print(p("en", {}, 1000.5, "seconds"), "|", p("en", {numeric: "auto"}, -1, "days"), "|", p("en", {}, -0, "day"));
print(p("pl", {style: "narrow"}, -1234, "hour"), "|", p("en-u-nu-thai", {}, 12, "months"));
print(JSON.stringify(Object.keys(new Intl.RelativeTimeFormat("en").formatToParts(5, "day")[1])));
print(f("en", {}, [1e21, "day"], [5e-324, "day"], [-1e-7, "day"], [0.9995, "day"]));
print(f("sr-Latn", {}, [1, "year"], [2, "year"], [5, "year"]), f("pt-PT", {}, [1, "day"], [2, "day"]));
"#;

/// format and formatToParts: the past and future patterns by the plural category of the
/// rounded value (pl 1.5 "roku", fr 1.5 "heure"), "auto" (-0 is "today": ToString(-0) is
/// "0"), the 3 styles, the NumberFormat digits, grouping and numbering systems, the `unit`
/// field of the number parts, and a script variant with the plural rules of its language.
#[test]
fn format_and_format_to_parts_give_the_node_lines() {
    assert_eq!(
        run_both(FORMAT),
        lines(&[
            "in 1 day | 1 day ago | in 0 days | 0 days ago | in 2 days | 2.5 hours ago | in 1,000 seconds | in 1 minute | in 123,456.789 years",
            "tomorrow | yesterday | today | today | in 2 days | 1 hour ago | now | this quarter | next week | in 0.5 days",
            "in 3 mo. | 3 qtrs. ago | in 1 yr. in 3mo | 1m ago",
            "za 1 rok | za 2 lata | za 5 lat | za 22 lata | za 1,5 roku | 0 dni temu | za 1000 minut",
            "vorgestern | übermorgen | letztes Jahr dans 1,5 heure | il y a 1 234,5 secondes",
            "خلال 3 أيام | قبل 11 شهرًا in ١٢ days ५६ hours ago",
            "3 週間後 | 3 週間前 昨天 | 明天",
            "literal:in /integer:1@second/group:,@second/integer:000@second/decimal:.@second/fraction:5@second/literal: seconds | literal:yesterday | integer:0@day/literal: days ago",
            "integer:1234@hour/literal: g. temu | literal:in /integer:๑๒@month/literal: months",
            "[\"type\",\"value\",\"unit\"]",
            "in 1,000,000,000,000,000,000,000 days | in 0 days | 0 days ago | in 1 day",
            "za 1 godinu | za 2 godine | za 5 godina dentro de 1 dia | dentro de 2 dias",
        ])
    );
}
