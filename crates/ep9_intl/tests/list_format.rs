//! Intl.ListFormat (ECMA-402 §13) and toLocaleLowerCase / toLocaleUpperCase (§19.1.2-3):
//! INTL W2 LIST+CASEMAP, with and without GC stress. The expected lines are node v26.13.0's
//! output for the same script (`print` = `console.log` of the arguments, each through
//! `String`, joined by a space; ICU 78.3, CLDR 48: the strings of these cases are the same in
//! the CLDR 49 data of ICU4X 2.4), with two differences, each named at its line.

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const L = Intl.ListFormat;
print(typeof L, L.length, L.name, Object.getPrototypeOf(L) === Function.prototype);
print(Object.getOwnPropertyNames(L).join(), Object.getOwnPropertyNames(L.prototype).join());
print(L.supportedLocalesOf.length, L.prototype.format.length, L.prototype.formatToParts.length, L.prototype.resolvedOptions.length);
const lf = new L("en");
print(Object.prototype.toString.call(lf), Object.isExtensible(lf), Object.getPrototypeOf(lf) === L.prototype);
print(JSON.stringify(L.supportedLocalesOf(["de-CH", "zz", "en-u-co-phonebk"])));
print(JSON.stringify(new L().resolvedOptions()), JSON.stringify(new L("ES-es-u-nu-arab", {type: "unit", style: "narrow"}).resolvedOptions()));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => L());
t(() => new L("en", null));
t(() => new L("en", "x"));
t(() => new L("en", {type: "x"}));
t(() => new L("en", {style: "x"}));
t(() => new L("en", {localeMatcher: "x"}));
t(() => new L("x-private"));
t(() => L.prototype.format([]));
t(() => L.prototype.resolvedOptions.call({}));
t(() => lf.format(5));
t(() => lf.format(["a", 1]));
t(() => lf.formatToParts([null]));
t(() => lf.format(null));
print(r.join(" "));
const log = [];
new L([], { get localeMatcher() { log.push("lm"); }, get type() { log.push("type"); }, get style() { log.push("style"); } });
print(log.join());
const it = { [Symbol.iterator]() { return this; }, n: 0, next() { this.n++; return this.n < 3 ? {done: false, value: "s" + this.n} : this.n === 3 ? {done: false, value: 3} : {done: true}; }, return() { log.push("return " + this.n); return {}; } };
t(() => lf.format(it));
print(r.pop(), log.pop());
class S extends L {}
print(new S("de") instanceof S, new S("de").format(["a", "b"]));
"#;

/// The shape of %Intl.ListFormat% and its prototype, resolvedOptions, the errors of the
/// constructor and the methods, the option read order and the iterator close.
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 ListFormat true",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,format,formatToParts",
            "1 1 1 0",
            "[object Intl.ListFormat] true true",
            "[\"de-CH\",\"en-u-co-phonebk\"]",
            "{\"locale\":\"en-US\",\"type\":\"conjunction\",\"style\":\"long\"} {\"locale\":\"es-ES\",\"type\":\"unit\",\"style\":\"narrow\"}",
            "TypeError TypeError TypeError RangeError RangeError RangeError RangeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "lm,type,style",
            "TypeError return 3",
            "true a und b",
        ])
    );
}

const FORMAT: &str = r#"const f = (l, o, ...xs) => xs.map(x => new Intl.ListFormat(l, o).format(x)).join(" | ");
const p = (l, o, x) => new Intl.ListFormat(l, o).formatToParts(x).map(q => q.type[0] + ":" + q.value).join("/");
print(f("en", {}, [], ["a"], ["a", "b"], ["a", "b", "c"], "abcd"));
print(f("en", {type: "disjunction"}, ["a", "b"], ["a", "b", "c"]), f("en", {type: "unit", style: "short"}, ["1h", "2m", "3s"]), f("en", {type: "unit", style: "narrow"}, ["1h", "2m", "3s"]));
print(f("en", {style: "short"}, ["a", "b", "c"]), f("en", {style: "narrow"}, ["a", "b", "c"]), f("en-GB", {}, ["a", "b", "c"]));
print(f("es", {}, ["agua", "hielo"], ["agua", "iglesia"], ["a", "b", "hijo"]), f("es", {type: "disjunction"}, ["siete", "ocho"], ["uno", "dos", "once"]));
print(f("de", {}, ["a", "b", "c"]), f("fr", {type: "disjunction"}, ["a", "b", "c"]), f("ja", {}, ["a", "b", "c"]), f("zh", {}, ["甲", "乙", "丙"]));
print(f("he", {}, ["א", "ב"], ["א", "B"]), f("ar", {}, ["أ", "ب", "ج"]), f("th", {type: "disjunction"}, ["ก", "ข"]));
print(p("en", {}, ["a", "b", "c"]), p("es", {style: "narrow", type: "unit"}, "foo"), p("en", {}, ["and", "and"]));
print(f("en", {}, ["\uD800", "x\uDC00"]).length, escape(f("en", {}, ["\uD800", "x\uDC00"])));
print(p("en", {}, new Set(["x", "y"])), f("en", {}, ["", "", ""]));
print(new Intl.ListFormat("en").format(undefined) === "", JSON.stringify(new Intl.ListFormat("en").formatToParts()));
"#;

/// format and formatToParts: the three types and styles, the conditional literals of es and
/// he, lone surrogates, an iterable that is not an Array.
#[test]
fn format_and_format_to_parts_give_the_node_lines() {
    assert_eq!(
        run_both(FORMAT),
        lines(&[
            " | a | a and b | a, b, and c | a, b, c, and d",
            "a or b | a, b, or c 1h, 2m, 3s 1h 2m 3s",
            "a, b, & c a, b, c a, b and c",
            "agua y hielo | agua e iglesia | a, b e hijo siete u ocho | uno, dos u once",
            "a, b und c a, b ou c a、b、c 甲、乙和丙",
            // node (CLDR 48) gives "ו-B" with U+002D; the CLDR 49 data of ICU4X 2.4 has U+2011.
            "א וב | א ו\u{2011}B أ وب وج ก หรือ ข",
            "e:a/l:, /e:b/l:, and /e:c e:f/l: /e:o/l: /e:o e:and/l: and /e:and",
            "8 %uD800%20and%20x%uDC00",
            "e:x/l: and /e:y , , and ",
            "true []",
        ])
    );
}

const CASEMAP: &str = r#"const U = (...c) => String.fromCharCode(...c);
const e = s => escape(s);
print(e("istanbul".toLocaleUpperCase("tr")), e("ISTANBUL".toLocaleLowerCase("tr")), e("I".toLocaleLowerCase("az-Latn")), e("i".toLocaleUpperCase(["tr", "en"])), e("i".toLocaleUpperCase(["en", "tr"])));
print(e(("I" + U(0x307)).toLocaleLowerCase("tr")), e(("I" + U(0x307)).toLocaleLowerCase("en")), e(U(0xCC).toLocaleLowerCase("lt")), e(("i" + U(0x307)).toLocaleUpperCase("lt")), e(("j" + U(0x307)).toLocaleUpperCase("lt-LT")));
print(e(U(0x3AC, 0x3BB, 0x3C6, 0x3B1).toLocaleUpperCase("el")), e(U(0x391, 0x3A3).toLocaleLowerCase("el")), e(U(0x587).toLocaleUpperCase("hy")), e("ijs".toLocaleUpperCase("nl")));
print(e(("I" + U(0xD800, 0x307)).toLocaleLowerCase("tr")), e(U(0xDC00) + "i".toLocaleUpperCase("tr")), e("i".toLocaleUpperCase("tr-u-co-trad")), e("i".toLocaleUpperCase("TR")), e("i".toLocaleUpperCase(new Intl.Locale("tr"))));
print(e("Abc".toLocaleUpperCase()), e("ABC".toLocaleLowerCase(undefined)), e(U(0xDF).toLocaleUpperCase("de")), "a".toLocaleUpperCase.length, "A".toLocaleLowerCase.length);
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (x) { r.push(x.constructor.name); } }
t(() => "a".toLocaleLowerCase(["en-US", "this is not a valid locale"]));
t(() => "".toLocaleUpperCase("x"));
t(() => "".toLocaleUpperCase([1]));
t(() => "".toLocaleUpperCase(null));
t(() => String.prototype.toLocaleUpperCase.call(null, "tr"));
const log = [];
t(() => String.prototype.toLocaleLowerCase.call({ toString() { log.push("this"); return "I"; } }, { get length() { log.push("len"); return 1; }, get 0() { log.push("0"); return "tr"; } }));
print(r.join(" "), log.join());
"#;

/// The language-sensitive case mappings (tr, az, lt, el; hy and nl map as the root), lone
/// surrogates, the locale checks and the read order.
#[test]
fn case_mapping_gives_the_node_lines() {
    assert_eq!(
        run_both(CASEMAP),
        lines(&[
            "%u0130STANBUL %u0131stanbul %u0131 %u0130 I",
            "i i%u0307 i%u0307%u0300 I J",
            "%u0391%u039B%u03A6%u0391 %u03B1%u03C2 %u0535%u0552 IJS",
            "%u0131%uD800%u0307 %uDC00%u0130 %u0130 %u0130 %u0130",
            "ABC abc SS 0 0",
            // node gives "ok" here: V8 checks only the first tag; CanonicalizeLocaleList checks
            // every tag (test262 validates-all-locale-identifiers.js).
            "RangeError RangeError TypeError TypeError TypeError ok this,len,0",
        ])
    );
}
