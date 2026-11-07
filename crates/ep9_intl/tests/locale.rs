//! Intl.Locale (ECMA-402 §14; INTL W1 LOCALE): the constructor, the getters, maximize,
//! minimize, toString and the Locale info methods, with and without GC stress. The expected
//! lines are node v26.13.0's output for the same script (`print` = `console.log` of the
//! arguments joined by a space), except where node does not follow ECMA-402 and test262
//! (`intl402/Locale`): node 26.13.0 has no `variants` getter or option (line 1 `1996-fonipa`,
//! line 4 `xx-1234-12345678-1xyz-abcde`, the RangeError of `fonipa-FONIPA` on line 7 and
//! `variants` in the option order of line 8), and it gives `["yes"]` for "en-u-nu"
//! (`getNumberingSystems/preferred-from-unicode-extension-true-empty.js` wants `[""]`).
//! node's own calendar id "islamic" is filtered out on line 9 (not in AvailableCalendars,
//! INTL-I1-D2).

mod common;
use common::{lines, run_both};

const SCRIPT: &str = r#"const l = new Intl.Locale("de-latn-de-fonipa-1996-u-ca-gregory-co-phonebk-hc-h23-kf-true-kn-false-nu-latn");
print(l.toString(), l.baseName, l.language, l.script, l.region, l.variants);
print(l.calendar, l.collation, l.hourCycle, JSON.stringify(l.caseFirst), l.numeric, l.numberingSystem, l.firstDayOfWeek);
const o = new Intl.Locale("en-GB-oxendict", { region: "US", calendar: "gregory", numeric: true, firstDayOfWeek: 0, hourCycle: "h11" });
print(o.toString(), o.numeric, o.firstDayOfWeek, o.hourCycle);
print(new Intl.Locale("und-Armn-SU", { language: "ru" }).toString(), new Intl.Locale("xx", { variants: "1xyz-1234-abcde-12345678" }).toString());
print(new Intl.Locale(l).toString() === l.toString(), Object.getPrototypeOf(l) === Intl.Locale.prototype, String(new Intl.Locale("EN-us")));
const m = new Intl.Locale("zh-Hant-u-nu-hanidec");
print(m.maximize().toString(), m.minimize().toString(), new Intl.Locale("und").maximize().toString(), new Intl.Locale("und-AT").minimize().toString());
const r = [];
function t(f) { try { f(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => Intl.Locale("en"));
t(() => new Intl.Locale());
t(() => new Intl.Locale(5));
t(() => new Intl.Locale("en_US"));
t(() => new Intl.Locale("en", { language: "abcd" }));
t(() => new Intl.Locale("en", { variants: "fonipa-FONIPA" }));
t(() => new Intl.Locale("en", { calendar: "ab" }));
t(() => new Intl.Locale("en", { hourCycle: "h25" }));
t(() => new Intl.Locale("en", null));
t(() => Intl.Locale.prototype.baseName);
t(() => Intl.Locale.prototype.getCalendars.call({}));
print(r.join(" "));
const order = [];
new Intl.Locale({ toString() { order.push("tag"); return "en"; } }, new Proxy({}, { get(_, k) { order.push(String(k)); } }));
print(order.join(","));
const ja = new Intl.Locale("ja-JP");
print(ja.getCalendars().join(), new Intl.Locale("fa").getCalendars().filter(c => c !== "islamic").join(), new Intl.Locale("en-u-ca-roc").getCalendars().join());
print(new Intl.Locale("de").getCollations().join(), new Intl.Locale("und").getCollations().join(), new Intl.Locale("de-u-co-phonebk").getCollations().join());
print(new Intl.Locale("en").getHourCycles().join(), new Intl.Locale("en-GB").getHourCycles().join(), new Intl.Locale("fr-CA").getHourCycles().join(), new Intl.Locale("en-u-hc-h11").getHourCycles().join());
print(new Intl.Locale("fa").getNumberingSystems().join(), new Intl.Locale("ar-EG").getNumberingSystems().join(), new Intl.Locale("en").getNumberingSystems().join(), JSON.stringify(new Intl.Locale("en-u-nu").getNumberingSystems()));
print(new Intl.Locale("de-DE").getTimeZones().join(), new Intl.Locale("en").getTimeZones(), new Intl.Locale("fr-FR").getTimeZones().join());
print(JSON.stringify(new Intl.Locale("ar").getTextInfo()), JSON.stringify(new Intl.Locale("en").getTextInfo()), JSON.stringify(new Intl.Locale("pa-PK").getTextInfo()));
print(JSON.stringify(new Intl.Locale("en-US").getWeekInfo()), JSON.stringify(new Intl.Locale("fa-AF").getWeekInfo()), JSON.stringify(new Intl.Locale("en-u-fw-mon").getWeekInfo()), JSON.stringify(new Intl.Locale("en-GB").getWeekInfo()));
print(JSON.stringify(Intl.getCanonicalLocales([new Intl.Locale("en-us"), "de"])));
"#;

#[test]
fn locale_gives_the_node_lines() {
    assert_eq!(
        run_both(SCRIPT),
        lines(&[
            "de-Latn-DE-1996-fonipa-u-ca-gregory-co-phonebk-hc-h23-kf-kn-false-nu-latn de-Latn-DE-1996-fonipa de Latn DE 1996-fonipa",
            r#"gregory phonebk h23 "" false latn undefined"#,
            "en-US-oxendict-u-ca-gregory-fw-sun-hc-h11-kn true sun h11",
            "ru-Armn-AM xx-1234-12345678-1xyz-abcde",
            "true true en-US",
            "zh-Hant-TW-u-nu-hanidec zh-TW-u-nu-hanidec en-Latn-US de-AT",
            "TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError TypeError TypeError TypeError",
            "tag,language,script,region,variants,calendar,collation,firstDayOfWeek,hourCycle,caseFirst,numeric,numberingSystem",
            "gregory,japanese persian,gregory,islamic-civil,islamic-tbla roc",
            "emoji,eor,phonebk emoji,eor phonebk",
            "h12 h23 h23 h11",
            r#"arabext arab latn [""]"#,
            "Europe/Berlin,Europe/Busingen undefined Europe/Paris",
            r#"{"direction":"rtl"} {"direction":"ltr"} {"direction":"rtl"}"#,
            r#"{"firstDay":7,"weekend":[6,7]} {"firstDay":6,"weekend":[4,5]} {"firstDay":1,"weekend":[6,7]} {"firstDay":1,"weekend":[6,7]}"#,
            r#"["en-US","de"]"#,
        ])
    );
}

/// The property shapes of test262 `intl402/Locale/prototype/*/prop-desc.js` and `name.js`:
/// accessors with a getter only, methods with length 0, and the result objects of
/// getTextInfo and getWeekInfo. node v26.13.0 gives the same lines, except two places where it
/// does not follow test262: it has 11 accessors (no `variants`), and "en-Zyyy" gives "ltr"
/// (`getTextInfo/script-metadata-rtl-is-unknown.js` wants undefined).
#[test]
fn prototype_properties() {
    assert_eq!(
        run_both(
            r#"const p = Intl.Locale.prototype;
const g = [];
for (const k of ["baseName", "calendar", "caseFirst", "collation", "firstDayOfWeek", "hourCycle", "numeric", "numberingSystem", "language", "script", "region", "variants"]) {
  const d = Object.getOwnPropertyDescriptor(p, k);
  g.push(typeof d.get === "function" && d.set === undefined && !d.enumerable && d.configurable && d.get.name === "get " + k);
}
print(g.every(x => x), g.length);
const m = [];
for (const k of ["maximize", "minimize", "toString", "getCalendars", "getCollations", "getHourCycles", "getNumberingSystems", "getTimeZones", "getTextInfo", "getWeekInfo"]) {
  const d = Object.getOwnPropertyDescriptor(p, k);
  m.push(d.writable && !d.enumerable && d.configurable && d.value.length === 0 && d.value.name === k);
}
print(m.every(x => x), m.length);
const w = new Intl.Locale("en").getWeekInfo();
print(Reflect.ownKeys(w).join(), Object.getPrototypeOf(w) === Object.prototype, Reflect.ownKeys(new Intl.Locale("en-Zyyy").getTextInfo()).join(), new Intl.Locale("en-Zyyy").getTextInfo().direction);
class L extends Intl.Locale {}
const s = new L("en");
print(s instanceof L, s.maximize() instanceof L, Object.getPrototypeOf(s.maximize()) === Intl.Locale.prototype);"#
        ),
        lines(&[
            "true 12",
            "true 10",
            "firstDay,weekend true direction undefined",
            "true false true",
        ])
    );
}
