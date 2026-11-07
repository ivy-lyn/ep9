//! The Intl namespace (ECMA-402 §8; INTL W1 CORE): %Intl%, Intl.getCanonicalLocales,
//! Intl.supportedValuesOf, and the W1 constructor stubs on %Intl%, with and without GC
//! stress. The expected lines are node v26.13.0's output for the same script (`print` =
//! `console.log` of the arguments joined by a space), except one value: node 26.13.0 leaves
//! "UTC" out of `supportedValuesOf("timeZone")`, and ECMA-402 (AvailablePrimaryTimeZone
//! Identifiers; test262 `timeZones-include-non-continental.js`) puts it in.

mod common;
use common::{lines, run_both};

const SCRIPT: &str = r#"print(typeof Intl, Object.prototype.toString.call(Intl), Object.getPrototypeOf(Intl) === Object.prototype);
const d = Object.getOwnPropertyDescriptor(globalThis, "Intl");
print(d.writable, d.enumerable, d.configurable, d.value === Intl);
const t = Object.getOwnPropertyDescriptor(Intl, Symbol.toStringTag);
print(t.value, t.writable, t.enumerable, t.configurable);
print(Intl.getCanonicalLocales.length, Intl.getCanonicalLocales.name, Intl.supportedValuesOf.length, Intl.supportedValuesOf.name);
print(JSON.stringify(Intl.getCanonicalLocales(["EN-us", "en-US", "de-u-kn-true-co-phonebk", "art-lojban", "und-u-ca-ethiopic-amete-alem"])));
print(JSON.stringify(Intl.getCanonicalLocales()), JSON.stringify(Intl.getCanonicalLocales("ZH-hant-tw")), JSON.stringify(Intl.getCanonicalLocales({length: 2, 0: "fr", 1: "fr-FR"})));
const r = [];
function t2(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t2(() => Intl.getCanonicalLocales("en_US"));
t2(() => Intl.getCanonicalLocales([1]));
t2(() => Intl.getCanonicalLocales(null));
t2(() => Intl.getCanonicalLocales(["x-private"]));
t2(() => Intl.supportedValuesOf("calendars"));
t2(() => Intl.supportedValuesOf(Symbol()));
t2(() => new Intl.getCanonicalLocales());
print(r.join(" "));
print(Intl.supportedValuesOf("unit").length, Intl.supportedValuesOf("calendar").includes("gregory"), Intl.supportedValuesOf("timeZone").includes("UTC"), Intl.supportedValuesOf("timeZone").includes("Etc/UTC"), Intl.supportedValuesOf("currency").includes("EUR"));
const a = Intl.supportedValuesOf("collation"); print(Array.isArray(a), a !== Intl.supportedValuesOf("collation"), a.includes("phonebk"));
print(typeof Intl.Locale, Intl.Locale.length, Intl.Locale.name, Object.prototype.toString.call(Intl.Locale.prototype), Intl.Locale.prototype.constructor === Intl.Locale);
print(typeof Intl.Collator, Intl.Collator.length, Intl.Collator.name, Object.prototype.toString.call(Intl.Collator.prototype));
"#;

#[test]
fn intl_namespace_gives_the_node_lines() {
    assert_eq!(
        run_both(SCRIPT),
        lines(&[
            "object [object Intl] true",
            "true false true true",
            "Intl false false true",
            "1 getCanonicalLocales 1 supportedValuesOf",
            r#"["en-US","de-u-co-phonebk-kn","jbo","und-u-ca-ethioaa"]"#,
            r#"[] ["zh-Hant-TW"] ["fr","fr-FR"]"#,
            "RangeError TypeError TypeError RangeError RangeError TypeError TypeError",
            "45 true true false true",
            "true true true",
            "function 1 Locale [object Intl.Locale] true",
            "function 0 Collator [object Intl.Collator]",
        ])
    );
}

/// An element getter that throws stops CanonicalizeLocaleList; the stub Locale constructor
/// needs `new`; supportedValuesOf takes ToString of its key.
#[test]
fn locale_stub_and_list_errors() {
    assert_eq!(
        run_both(
            r#"const r = [];
function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => Intl.Locale("en"));
t(() => Intl.getCanonicalLocales({ length: 1, get 0() { throw new SyntaxError(); } }));
t(() => Intl.supportedValuesOf({ toString() { return "unit"; } }));
print(r.join(" "));"#
        ),
        lines(&["TypeError SyntaxError ok"])
    );
}
