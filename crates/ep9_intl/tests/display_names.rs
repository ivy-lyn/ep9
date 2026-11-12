//! Intl.DisplayNames (ECMA-402 §12; INTL W2 DISPLAYNAMES): the constructor, supportedLocalesOf,
//! of, resolvedOptions and the locale data, with and without GC stress. The expected lines are
//! node v26.13.0's output for the same script (`print` = `console.log` of the arguments, each
//! through `String`, joined by a space), except the lines marked "ep9:", where ECMA-402 or the
//! ep9 data differ from V8 (each says why).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"
const D = Intl.DisplayNames;
print(typeof D, D.length, D.name, Object.getPrototypeOf(D) === Function.prototype);
const pd = Object.getOwnPropertyDescriptor(D, "prototype");
print(pd.writable, pd.enumerable, pd.configurable);
print(Object.getOwnPropertyNames(D).join(), Object.getOwnPropertyNames(D.prototype).join());
print(D.supportedLocalesOf.length, D.prototype.of.length, D.prototype.of.name, D.prototype.resolvedOptions.length);
const d = new D("de", {type: "region"});
print(Object.getPrototypeOf(d) === D.prototype, Object.isExtensible(d), Object.prototype.toString.call(d));
print(JSON.stringify(D.supportedLocalesOf(["de-CH", "zz", "fr-x-priv"])), JSON.stringify(D.supportedLocalesOf("ja", {localeMatcher: "lookup"})));
print(JSON.stringify(d.resolvedOptions()), JSON.stringify(new D("en-u-ca-gregory", {type: "language", style: "short", fallback: "none", languageDisplay: "standard"}).resolvedOptions()));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => D("en", {type: "region"}));
t(() => new D("en"));
t(() => new D("en", {}));
t(() => new D("en", null));
t(() => new D("en", 5));
t(() => new D("en_US", {type: "region"}));
t(() => new D("en", {type: "lang"}));
t(() => new D("en", {type: "region", style: "full"}));
t(() => new D("en", {type: "region", fallback: "undefined"}));
t(() => new D("en", {type: "region", languageDisplay: "x"}));
t(() => new D("en", {type: "region", localeMatcher: "x"}));
t(() => D.prototype.of("US"));
t(() => D.prototype.resolvedOptions.call({}));
t(() => d.of(Symbol()));
t(() => d.of("USA"));
t(() => new D("en", {type: "language"}).of("en_GB"));
t(() => new D("en", {type: "calendar"}).of("abc_def"));
t(() => new D("en", {type: "dateTimeField"}).of("Year"));
t(() => new D("en", {type: "currency"}).of("US"));
t(() => new D("en", {type: "script"}).of("Lat"));
print(r.join(" "));
const log = [];
new D({get length() { log.push("locales"); return 0; }}, new Proxy({}, {get(o, k) { log.push(String(k)); return k === "type" ? "language" : undefined; }}));
print(log.join());
"#;

/// The shape of %Intl.DisplayNames% and its prototype, resolvedOptions, the errors of the
/// constructor and of `of`, and the order of the option reads.
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 2 DisplayNames true",
            "false false false",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,of",
            "1 1 of 0",
            "true true [object Intl.DisplayNames]",
            "[\"de-CH\",\"fr-x-priv\"] [\"ja\"]",
            "{\"locale\":\"de\",\"style\":\"long\",\"type\":\"region\",\"fallback\":\"code\"} {\"locale\":\"en\",\"style\":\"short\",\"type\":\"language\",\"fallback\":\"none\",\"languageDisplay\":\"standard\"}",
            "TypeError TypeError TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError RangeError TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError RangeError",
            "locales,localeMatcher,style,type,fallback,languageDisplay",
        ])
    );
}

const NAMES: &str = r#"
const D = Intl.DisplayNames;
const l = new D("en", {type: "language"});
print(l.of("en-GB"), "|", l.of("EN-latn-us"), "|", l.of("sh"), "|", l.of("en-XX"), "|", l.of("zh-Hant"));
print(new D("en", {type: "language", languageDisplay: "standard"}).of("en-GB"), "|", new D("en", {type: "language", style: "short"}).of("en-GB"), "|", new D("fr", {type: "language"}).of("en-US"));
const ln = new D("en", {type: "language", fallback: "none"});
print(ln.of("xx"), ln.of("en-XX"), ln.of("en-GB"));
const rg = new D("en", {type: "region"});
print(rg.of("us"), "|", rg.of("419"), "|", new D("de", {type: "region"}).of("DE"), "|", new D("en", {type: "region", style: "narrow"}).of("GB"), "|", new D("en", {type: "region", style: "short"}).of("BA"));
print(new D("en", {type: "region", fallback: "none"}).of("AA"), new D("en", {type: "region"}).of("AA"));
print(new D("en", {type: "script"}).of("latn"), "|", new D("zh", {type: "script"}).of("Hans"), "|", new D("en", {type: "script", fallback: "none"}).of("Aaaa"));
const cu = new D("en", {type: "currency"});
print(cu.of("usd"), "|", cu.of("EUR"), "|", new D("de", {type: "currency"}).of("USD"), "|", new D("ja", {type: "currency"}).of("JPY"), "|", new D("en", {type: "currency", fallback: "none"}).of("QQQ"));
const sv = Intl.supportedValuesOf("currency");
const cn = new D("en", {type: "currency", fallback: "none"});
print(sv.every(c => typeof cn.of(c) === "string"));
"#;

/// The names of each type from the ICU4X data, the styles, languageDisplay and the fallback.
#[test]
fn of_gives_the_cldr_names() {
    assert_eq!(
        run_both(NAMES),
        lines(&[
            // ep9: "American English (Latin)" (V8 gives "English (Latin, United States)"): the
            // ICU4X dialect rule of UTS 35 (language+script, then language+region) finds
            // "en-US".
            "British English | American English (Latin) | Serbian (Latin) | English (XX) | Traditional Chinese",
            "English (United Kingdom) | UK English | anglais américain",
            "undefined undefined British English",
            // ep9: "United States" (V8 gives "us"): CanonicalCodeForDisplayNames makes the
            // region code upper case (12.5.1 step 2.b) before the lookup.
            "United States | Latin America | Deutschland | UK | Bosnia",
            "undefined AA",
            // ep9: "Latin" (V8 gives "latn"): the script code is made title case (step 3).
            "Latin | 简体 | undefined",
            "US Dollar | Euro | US-Dollar | 日本円 | undefined",
            "true",
        ])
    );
}

/// The workaround for calendar and dateTimeField (ICU4X 2.4 has no data for them; plan v2
/// §2.2): a calendar of `Intl.supportedValuesOf("calendar")` and each dateTimeField code give
/// the code; another valid calendar gives the fallback. The calendar code is canonical (an
/// alias gives its value). The list of currencies with a name is the supportedValuesOf list
/// (ECMA-402 AvailableCanonicalCurrencies): an old currency gets the fallback.
#[test]
fn calendar_date_time_field_and_currency_follow_the_supported_lists() {
    assert_eq!(
        run_both(
            r#"const D = Intl.DisplayNames;
const c = new D("en", {type: "calendar", fallback: "none"});
print(Intl.supportedValuesOf("calendar").every(x => c.of(x) === x), c.of("islamic"), c.of("ISLAMICC"), c.of("ethiopic-amete-alem"));
print(new D("en", {type: "calendar"}).of("Abc-DEF"));
const f = new D("en", {type: "dateTimeField", fallback: "none"});
print(["era", "year", "quarter", "month", "weekOfYear", "weekday", "day", "dayPeriod", "hour", "minute", "second", "timeZoneName"].every(x => f.of(x) === x));
print(new D("en", {type: "currency", fallback: "none"}).of("DEM"), new D("en", {type: "currency"}).of("dem"));"#
        ),
        lines(&[
            // ep9: node gives "Gregorian Calendar" style names; ICU4X has no calendar names.
            "true undefined islamic-civil ethioaa",
            "abc-def",
            "true",
            // ep9: node gives "German Mark" for both (V8 names currencies outside its
            // supportedValuesOf list).
            "undefined DEM",
        ])
    );
}
