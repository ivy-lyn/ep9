//! Intl.PluralRules (ECMA-402 §17; INTL W2 PLURAL): the constructor and its option read
//! order, supportedLocalesOf, resolvedOptions, select (with the notations) and selectRange,
//! with and without GC stress. The expected lines are node v26.13.0's output for the same
//! script (`print` = `console.log` of the arguments, each through `String`, joined by a
//! space), except the lines marked "ep9:", where ECMA-402 and V8 differ (each says why).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const P = Intl.PluralRules;
print(typeof P, P.length, P.name, Object.getPrototypeOf(P) === Function.prototype);
const pd = Object.getOwnPropertyDescriptor(P, "prototype");
print(pd.writable, pd.enumerable, pd.configurable);
print(Object.getOwnPropertyNames(P).join(), Object.getOwnPropertyNames(P.prototype).join());
print(P.supportedLocalesOf.length, P.prototype.resolvedOptions.length, P.prototype.select.length, P.prototype.selectRange.length, P.prototype.selectRange.name);
const p = new P("de");
print(Object.getPrototypeOf(p) === P.prototype, Object.isExtensible(p), Object.prototype.toString.call(p));
class My extends P {}
print(new My("en").select(1), new My("en") instanceof P);
print(JSON.stringify(P.supportedLocalesOf(["de-CH", "zz", "en-u-co-phonebk", "fr-x-priv"])));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => P());
t(() => P.prototype.select(1));
t(() => P.prototype.resolvedOptions.call({}));
t(() => new P("en", {type: "dual"}));
t(() => new P("en", {notation: "COMPACT"}));
t(() => new P("en", {compactDisplay: "tiny"}));
t(() => new P("en", {minimumIntegerDigits: 0}));
t(() => new P("en", {roundingIncrement: 3}));
t(() => new P("en", {roundingIncrement: 5, maximumSignificantDigits: 2}));
t(() => new P("en", null));
t(() => new P("en_US"));
t(() => p.selectRange(undefined, 1));
t(() => p.selectRange(NaN, 1));
t(() => p.selectRange(1, Symbol()));
t(() => p.select(Symbol()));
t(() => new P.prototype.select());
print(r.join(" "));
const reads = [];
const handler = { get(t, k) { reads.push(String(k)); return undefined; } };
new P(undefined, new Proxy({}, handler));
print(reads.join());
"#;

/// The shape of %Intl.PluralRules% and its prototype, a subclass, the errors of the
/// constructor and the methods, and the order of the option reads.
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 PluralRules true",
            "false false false",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,select,selectRange",
            "1 0 1 2 selectRange",
            "true true [object Intl.PluralRules]",
            "one true",
            "[\"de-CH\",\"en-u-co-phonebk\",\"fr-x-priv\"]",
            "TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError TypeError TypeError RangeError TypeError RangeError TypeError TypeError TypeError",
            "localeMatcher,type,notation,compactDisplay,minimumIntegerDigits,minimumFractionDigits,maximumFractionDigits,minimumSignificantDigits,maximumSignificantDigits,roundingIncrement,roundingMode,roundingPriority,trailingZeroDisplay",
        ])
    );
}

const SELECT: &str = r#"const ro = (l, o) => { const r = new Intl.PluralRules(l, o).resolvedOptions(); return Object.keys(r).map(k => k + "=" + r[k]).join(" "); };
print(ro("en"));
print(ro("de-u-nu-arab-co-phonebk", {type: "ordinal"}));
print(ro("en", {minimumSignificantDigits: 3}));
print(ro("en", {minimumFractionDigits: 2, roundingMode: "floor", trailingZeroDisplay: "stripIfInteger"}));
print(ro("ar").split(" ").filter(s => s.startsWith("pluralCategories")).join());
print(ro("fr").split(" ").filter(s => s.startsWith("pluralCategories")).join());
print(ro("sl").split(" ").filter(s => s.startsWith("pluralCategories")).join());
const pr = new Intl.PluralRules("en");
print([1, "1", 0, -1, 1.5, 1e21, -0, NaN, Infinity, -Infinity, "1.0", 0.9999, 1.0001].map(x => pr.select(x)).join());
print(new Intl.PluralRules("en", {minimumFractionDigits: 1}).select(1), new Intl.PluralRules("en", {maximumSignificantDigits: 1}).select(1.4));
const o = new Intl.PluralRules("en", {type: "ordinal"});
print([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111].map(x => o.select(x)).join());
const ru = new Intl.PluralRules("ru");
print([0, 1, 2, 5, 11, 21, 22, 25, 1.5].map(x => ru.select(x)).join());
for (const n of ["compact", "scientific", "engineering"]) {
  const fr = new Intl.PluralRules("fr", {notation: n});
  print(n, [1e6, 1.5e6, 1e3, 1500, 2e9, 0, 1, 2, 1.5, 1e5, 1e7].map(x => fr.select(x)).join());
}
const frl = new Intl.PluralRules("fr", {notation: "compact", compactDisplay: "long"});
print([1e6, 1.5e6, 1e3, 1500, 2e9].map(x => frl.select(x)).join());
const sl = new Intl.PluralRules("sl");
print(sl.selectRange(102, 201), sl.selectRange(1, 2), ru.selectRange(1, 2), ru.selectRange(1, 5), ru.selectRange(2, 1), ru.selectRange(0, 1.5), pr.selectRange(5, 1), pr.selectRange(-Infinity, Infinity));
"#;

/// resolvedOptions (key order, pluralCategories order), select with the digit options, the
/// ordinal rules and the three notations, and selectRange on the CLDR range data.
#[test]
fn resolved_options_select_and_ranges_give_the_node_lines() {
    assert_eq!(
        run_both(SELECT),
        lines(&[
            "locale=en type=cardinal notation=standard minimumIntegerDigits=1 minimumFractionDigits=0 maximumFractionDigits=3 pluralCategories=one,other roundingIncrement=1 roundingMode=halfExpand roundingPriority=auto trailingZeroDisplay=auto",
            "locale=de type=ordinal notation=standard minimumIntegerDigits=1 minimumFractionDigits=0 maximumFractionDigits=3 pluralCategories=other roundingIncrement=1 roundingMode=halfExpand roundingPriority=auto trailingZeroDisplay=auto",
            "locale=en type=cardinal notation=standard minimumIntegerDigits=1 minimumSignificantDigits=3 maximumSignificantDigits=21 pluralCategories=one,other roundingIncrement=1 roundingMode=halfExpand roundingPriority=auto trailingZeroDisplay=auto",
            "locale=en type=cardinal notation=standard minimumIntegerDigits=1 minimumFractionDigits=2 maximumFractionDigits=3 pluralCategories=one,other roundingIncrement=1 roundingMode=floor roundingPriority=auto trailingZeroDisplay=stripIfInteger",
            "pluralCategories=zero,one,two,few,many,other",
            "pluralCategories=one,many,other",
            "pluralCategories=one,two,few,other",
            "one,one,other,one,other,other,other,other,other,other,one,one,one",
            "other one",
            "one,two,few,other,other,other,other,one,two,few,one,other",
            "many,one,few,many,many,one,few,many,other",
            "compact many,many,other,other,many,one,one,other,one,other,many",
            "scientific many,many,other,other,many,one,one,other,one,other,many",
            "engineering many,many,other,other,many,one,one,other,one,other,many",
            "many,many,other,other,many",
            "few two few many one other other other",
        ])
    );
}

const SPEC: &str = r#"const pr = new Intl.PluralRules("en");
print(pr.select(1n), pr.select(2n), new Intl.PluralRules("en", {maximumFractionDigits: 20}).select("1.00000000000000000001"));
print(new Intl.PluralRules("sl").selectRange(1, 1), pr.selectRange(1, "1.0"), pr.selectRange(1n, 2n));
const fa = new Intl.PluralRules("fa");
print(new Intl.PluralRules("sl").selectRange(-1, 1), fa.selectRange(-1, 1), fa.selectRange(-0, 0), fa.selectRange(-0.0001, 0));
const c = new Intl.PluralRules("en", {notation: "compact"}).resolvedOptions();
print(Object.keys(c).join());
const l = new Intl.PluralRules("en", {roundingPriority: "lessPrecision"}).resolvedOptions();
print(Object.keys(l).join());
const a = new Intl.PluralRules().resolvedOptions();
print(a.pluralCategories !== new Intl.PluralRules().resolvedOptions().pluralCategories);
"#;

/// The lines where ECMA-402 and V8 differ.
#[test]
fn the_spec_lines_where_v8_differs() {
    assert_eq!(
        run_both(SPEC),
        lines(&[
            // ep9: select reads its argument with ToIntlMathematicalValue (17.3.3 step 3), so
            // a BigInt is a value and a String keeps all its digits (V8: ToNumber, so a
            // TypeError for 1n and "one" for the string).
            "one other other",
            // ep9: ResolvePluralRange step 4: the same formatted text gives the start category
            // (V8 gives the range category: "few", "other"; and a TypeError for 1n).
            "one one other",
            // ep9: the [[FormattedString]] of FormatNumericToString has no sign (§16.5.2), so
            // -1 and 1 (and -0 and 0) have the same text and step 4 gives the start category
            // (V8: "few other other other", the range data).
            "one one one one",
            // ep9: SetNumberFormatDigitOptions step 25 sets the fraction and the significant
            // digits for the compact default, and lessPrecision sets both (V8 shows only the
            // significant digits).
            "locale,type,notation,compactDisplay,minimumIntegerDigits,minimumFractionDigits,maximumFractionDigits,minimumSignificantDigits,maximumSignificantDigits,pluralCategories,roundingIncrement,roundingMode,roundingPriority,trailingZeroDisplay",
            "locale,type,notation,minimumIntegerDigits,minimumFractionDigits,maximumFractionDigits,minimumSignificantDigits,maximumSignificantDigits,pluralCategories,roundingIncrement,roundingMode,roundingPriority,trailingZeroDisplay",
            "true",
        ])
    );
}

const EXTRA: &str = r#"const xs = [0, 1, 2, 3, 5, 11, 12, 20, 21, 22, 40, 100, 1000, 1.5, 0.5, "1.0", 101, 1000000];
for (const l of ["gv", "kw", "lag", "shi", "tzm", "ff", "yi", "naq", "ln"]) {
  const p = new Intl.PluralRules(l);
  print(l, p.resolvedOptions().pluralCategories.join(), xs.map(x => p.select(x)).join());
}
const k = new Intl.PluralRules("kw", {type: "ordinal"});
print(k.resolvedOptions().pluralCategories.join(), [1, 4, 5, 6, 21, 25, 105, 1000].map(x => k.select(x)).join());
const kc = new Intl.PluralRules("kw");
print([2, 22, 1000, 21000, 40000, 100000, 1100000, 3, 81, 1, 0].map(x => kc.select(x)).join());
const g = new Intl.PluralRules("gsw");
print(g.selectRange(1, 2), g.selectRange(2, 1), g.selectRange(0, 1));
"#;

/// The available languages that the ICU4X compiled data has no plural rules for take the CLDR
/// rules (cardinal and ordinal); the "gsw" ranges give the end category.
#[test]
fn languages_without_icu4x_rules_give_the_node_lines() {
    assert_eq!(
        run_both(EXTRA),
        lines(&[
            "gv one,two,few,many,other few,one,two,other,other,one,two,few,one,two,few,few,few,many,many,one,one,few",
            "kw zero,one,two,few,many,other zero,one,two,few,other,other,other,other,many,two,other,other,two,other,other,one,many,other",
            "lag zero,one,other zero,one,other,other,other,other,other,other,other,other,other,other,other,one,one,one,other,other",
            "shi one,few,other one,one,few,few,few,other,other,other,other,other,other,other,other,other,one,one,other,other",
            "tzm one,other one,one,other,other,other,one,one,one,one,one,one,other,other,other,other,one,other,other",
            "ff one,other one,one,other,other,other,other,other,other,other,other,other,other,other,one,one,one,other,other",
            "yi one,other other,one,other,other,other,other,other,other,other,other,other,other,other,other,other,one,other,other",
            "naq one,two,other other,one,two,other,other,other,other,other,other,other,other,other,other,other,other,one,other,other",
            "ln one,other one,one,other,other,other,other,other,other,other,other,other,other,other,other,other,one,other,other",
            "one,many,other one,one,many,other,one,other,many,other",
            "two,two,two,other,two,two,two,few,many,one,zero",
            "other one one",
        ])
    );
}
