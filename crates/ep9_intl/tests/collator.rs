//! Intl.Collator (ECMA-402 §10; INTL W1 COLLATOR): the constructor, supportedLocalesOf,
//! compare, resolvedOptions and the locale data, with and without GC stress. The expected
//! lines are node v26.13.0's output for the same script (`print` = `console.log` of the
//! arguments, each through `String`, joined by a space), except the lines marked "ep9:",
//! where ECMA-402 or the ep9 data differ from V8 (each says why).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const C = Intl.Collator;
print(typeof C, C.length, C.name, Object.getPrototypeOf(C) === Function.prototype);
const pd = Object.getOwnPropertyDescriptor(C, "prototype");
print(pd.writable, pd.enumerable, pd.configurable);
print(Object.getOwnPropertyNames(C).join(), Object.getOwnPropertyNames(C.prototype).join());
const g = Object.getOwnPropertyDescriptor(C.prototype, "compare");
print(typeof g.get, g.set, g.enumerable, g.configurable, g.get.name, g.get.length);
print(C.supportedLocalesOf.length, C.supportedLocalesOf.name, C.prototype.resolvedOptions.length, C.prototype.resolvedOptions.name);
const c = new C("de");
const f = c.compare;
print(f === c.compare, f.length, JSON.stringify(f.name), Object.getOwnPropertyNames(f).join(), Object.getPrototypeOf(f) === Function.prototype, "prototype" in f);
print(Object.getPrototypeOf(c) === C.prototype, Object.isExtensible(c), Object.prototype.toString.call(c));
const c2 = C();
print(c2 instanceof C, C.call({}) instanceof C);
print(JSON.stringify(C.supportedLocalesOf(["de-CH", "zz", "en-u-co-phonebk", "fr-x-priv"])), JSON.stringify(C.supportedLocalesOf("ja", {localeMatcher: "lookup"})));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => C.prototype.compare);
t(() => C.prototype.resolvedOptions());
t(() => g.get.call({}));
t(() => new C("en", {usage: "find"}));
t(() => new C("en", {collation: "ab"}));
t(() => new C("en", {collation: "abc_def"}));
t(() => new C("en", {sensitivity: "none"}));
t(() => new C("en", {caseFirst: "yes"}));
t(() => new C("en", {localeMatcher: "fast"}));
t(() => new C("en_US"));
t(() => C.supportedLocalesOf("x-private"));
t(() => new C("en", null));
t(() => C.supportedLocalesOf("en", {localeMatcher: "x"}));
t(() => new C.prototype.compare);
t(() => new f("a", "b"));
print(r.join(" "));
"#;

/// The shape of %Intl.Collator% and its prototype, the bound compare function, and the errors of
/// the constructor and the methods.
#[test]
fn the_constructor_and_prototype_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 Collator true",
            "false false false",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,compare",
            "function undefined false true get compare 0",
            "1 supportedLocalesOf 0 resolvedOptions",
            "true 2 \"\" length,name true false",
            "true true [object Intl.Collator]",
            "true true",
            "[\"de-CH\",\"en-u-co-phonebk\",\"fr-x-priv\"] [\"ja\"]",
            "TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError TypeError RangeError TypeError TypeError",
        ])
    );
}

const RESOLVE: &str = r#"const ro = (l, o) => { const r = new Intl.Collator(l, o).resolvedOptions(); return Object.keys(r).map(k => r[k]).join(","); };
print(Object.keys(new Intl.Collator("en").resolvedOptions()).join());
print(ro("en"));
print(ro("th"));
print(ro("th", {ignorePunctuation: false}));
print(ro("da"));
print(ro("da", {caseFirst: "false"}));
print(ro("mt-MT"));
print(ro("de-u-co-phonebk"));
print(ro("de", {collation: "phonebk"}));
print(ro("de-u-co-phonebk", {collation: "eor"}));
print(ro("de-u-co-phonebk", {collation: "pinyin"}));
print(ro("en-u-co-phonebk"));
print(ro("de-DE-u-co-standard"));
print(ro("de-DE-u-co-search"));
print(ro("es-u-co-trad"));
print(ro("zh-u-co-stroke"));
print(ro("en-u-kn-kf-upper"));
print(ro("en-u-kn-true", {numeric: true}));
print(ro("en-u-kn-false", {numeric: true}));
print(ro("en-u-kn", {numeric: false}));
print(ro("en-u-kf-lower", {caseFirst: "upper"}));
print(ro("en", {usage: "search", sensitivity: "base"}));
print(ro("en", {sensitivity: "accent", ignorePunctuation: true, numeric: 1, caseFirst: "lower"}));
print(ro("de-CH-x-foo"));
print(ro("en-US-u-ca-gregory-co-emoji"));
print(ro(["zz", "fr-CA"]));
print(ro("en", {collation: "EMOJI"}));
"#;

/// resolvedOptions for locales, Unicode extension keywords and options: the ResolveLocale rules
/// of the relevant keys co, kf and kn, and the locale data defaults (th ignorePunctuation, da
/// and mt caseFirst).
#[test]
fn resolved_options_give_the_node_lines() {
    assert_eq!(
        run_both(RESOLVE),
        lines(&[
            "locale,usage,sensitivity,ignorePunctuation,collation,numeric,caseFirst",
            "en,sort,variant,false,default,false,false",
            "th,sort,variant,true,default,false,false",
            "th,sort,variant,false,default,false,false",
            "da,sort,variant,false,default,false,upper",
            "da,sort,variant,false,default,false,false",
            // ep9: the available locales hold the default-content children (mt-MT, de-DE,
            // de-CH; INTL W1 CORE `available::locales`); the V8 collator list has only "mt",
            // "de".
            "mt-MT,sort,variant,false,default,false,upper",
            "de-u-co-phonebk,sort,variant,false,phonebk,false,false",
            // ep9: an option value (not a keyword of the tag) is not put in [[Locale]]
            // (ResolveLocale step 9.i.iv: supportedKeyword is empty); V8 adds -u-co-phonebk.
            "de,sort,variant,false,phonebk,false,false",
            "de,sort,variant,false,eor,false,false",
            "de-u-co-phonebk,sort,variant,false,phonebk,false,false",
            "en,sort,variant,false,default,false,false",
            "de-DE,sort,variant,false,default,false,false",
            "de-DE,sort,variant,false,default,false,false",
            "es-u-co-trad,sort,variant,false,trad,false,false",
            "zh-u-co-stroke,sort,variant,false,stroke,false,false",
            "en-u-kf-upper-kn,sort,variant,false,default,true,upper",
            "en-u-kn,sort,variant,false,default,true,false",
            "en,sort,variant,false,default,true,false",
            "en,sort,variant,false,default,false,false",
            "en,sort,variant,false,default,false,upper",
            "en,search,base,false,default,false,false",
            "en,sort,accent,true,default,true,lower",
            "de-CH,sort,variant,false,default,false,false",
            "en-US-u-co-emoji,sort,variant,false,emoji,false,false",
            "fr-CA,sort,variant,false,default,false,false",
            // ep9: as the phonebk option line above.
            "en,sort,variant,false,emoji,false,false",
        ])
    );
}

const COMPARE: &str = r#"const cmp = (l, o, pairs) => { const c = new Intl.Collator(l, o).compare; return pairs.map(([a, b]) => c(a, b)).join(","); };
const P = [["a", "b"], ["b", "a"], ["a", "A"], ["a", "\u00e1"], ["\u00e4", "a\u0308"], ["", " "], ["", "*"], ["2", "10"], ["a1", "a01"], ["\u00c5", "A\u030a"]];
print(cmp("en", undefined, P));
print(cmp("en", {sensitivity: "base"}, P));
print(cmp("en", {sensitivity: "accent"}, P));
print(cmp("en", {sensitivity: "case"}, P));
print(cmp("en", {ignorePunctuation: true}, P));
print(cmp("th", undefined, P));
print(cmp("en", {numeric: true}, P));
print(cmp("en", {caseFirst: "upper"}, P));
print(cmp("da", undefined, P));
print(["A", "b", "Af", "Ab", "od", "off", "\u00c4", "\u00f6"].sort(new Intl.Collator("de-u-co-phonebk").compare).join());
print(["A", "b", "Af", "Ab", "od", "off", "\u00c4", "\u00f6"].sort(new Intl.Collator("de").compare).join());
print(["ch", "cz", "d", "ca"].sort(new Intl.Collator("es-u-co-trad").compare).join(), ["ch", "cz", "d", "ca"].sort(new Intl.Collator("es").compare).join());
print(["z", "\u00e4", "a", "\u00f6", "o"].sort(new Intl.Collator("sv").compare).join());
const c = new Intl.Collator();
print(c.compare(), c.compare("undefined"), c.compare(1, "1"), c.compare({toString() { return "b"; }}, "a"));
const log = [];
c.compare({toString() { log.push("x"); return "a"; }}, {toString() { log.push("y"); return "b"; }});
print(log.join());
print(["b", "a", "C"].sort(c.compare).join(), ["\ud800", "a", "\udfff", "\u{1f600}"].sort(c.compare).map(s => s.codePointAt(0).toString(16)).join());
"#;

/// compare with each sensitivity and option, the collation types phonebk and trad, Swedish,
/// the argument conversion order and lone surrogates.
#[test]
fn compare_gives_the_node_lines() {
    assert_eq!(
        run_both(COMPARE),
        lines(&[
            "-1,1,-1,-1,0,-1,-1,1,1,0",
            "-1,1,0,0,0,-1,-1,1,1,0",
            "-1,1,0,-1,0,-1,-1,1,1,0",
            "-1,1,-1,0,0,-1,-1,1,1,0",
            "-1,1,-1,-1,0,0,0,1,1,0",
            "-1,1,-1,-1,0,0,0,1,1,0",
            "-1,1,-1,-1,0,-1,-1,-1,0,0",
            "-1,1,1,-1,0,-1,-1,1,1,0",
            "-1,1,1,-1,0,-1,-1,1,1,0",
            "A,Ab,\u{c4},Af,b,od,\u{f6},off",
            "A,\u{c4},Ab,Af,b,\u{f6},od,off",
            "ca,cz,ch,d ca,ch,cz,d",
            "a,o,z,\u{e4},\u{f6}",
            "0 0 0 1",
            "x,y",
            "a,b,C 1f600,61,d800,dfff",
        ])
    );
}

/// Long runs of combining marks (Codex r1 COLLATOR-1): the ICU4X 2.3 collator shifts a list
/// once per mark of a run, and once per code point when it goes back over an identical
/// prefix, so 'a' + 320,000 U+0301 took 14.9 s (release) and a termination was not honored.
/// The UAX #15 stream-safe step (two CGJs after each 30 non-starters of NFD) makes each
/// compare linear: each case of 320,000 marks (a difference at the first mark, an identical
/// prefix of marks, Tibetan vowel signs that decompose to two marks, astral marks) ends in
/// well under 1 s in a release build; the 5 s bound covers a slow debug build. The results
/// are node v26.13.0's (node takes about 25 s for the Tibetan case at 40,000).
#[test]
fn long_combining_runs_compare_in_linear_time() {
    let (took, r) = common::timed(
        r#"var c = new Intl.Collator("en"), n = 320000, out = [];
var a = "a" + "́".repeat(n), b = "a" + "̀".repeat(n);
out.push(c.compare(a, b), a.localeCompare(b), c.compare(a + "b", a + "c"), (a + "c").localeCompare(a + "b"));
out.push(c.compare("a" + "ཱི".repeat(n / 2), "a" + "ཱུ".repeat(n / 2)));
out.push(c.compare("a" + "\u{1D165}".repeat(n / 2), "a" + "\u{1D166}".repeat(n / 2)));
print(out.join());"#,
    );
    assert_eq!(r, lines(&["-1,-1,-1,1,-1,0"]));
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

/// A long common prefix with no safe boundary ('a' + U+0301 repeated: no two starters in a
/// row) made the ICU4X identical-prefix step O(n^2): 64k pairs took 17.5 s in a release
/// build (4.1 s for the compare alone). Past a common prefix of 1,024 code units the compare
/// uses sort keys (linear; the same order, UTS #10), so this takes well under 1 s in a
/// release build; the 5 s bound covers a slow debug build. The Czech rows check a
/// contraction ("ch") right after a long prefix. The results are node v26.13.0's.
#[test]
fn a_long_common_prefix_compares_in_linear_time() {
    let (took, r) = common::timed(
        r#"var c = new Intl.Collator("en"), n = 65536, out = [];
var p = "a\u0301".repeat(n);
out.push(c.compare(p + "b", p + "c"), (p + "c").localeCompare(p + "b"), c.compare(p, p), c.compare(p + "b", p));
var q = "q\u0301".repeat(n / 2), s = "a\u0316".repeat(n / 2);
out.push(c.compare(q + "b", q + "c"), c.compare(s + "c", s + "b"));
var cs = new Intl.Collator("cs"), h = "a\u0301".repeat(2000);
out.push(cs.compare(h + "ach", h + "aci"), cs.compare(h + "ch", h + "h"), new Intl.Collator("en", {sensitivity: "base"}).compare(p + "B", p + "b"));
print(out.join());"#,
    );
    assert_eq!(r, lines(&["-1,1,0,1,-1,1,1,1,0"]));
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

/// The stream-safe step works on NFD and only on text whose NFD has a run of more than 30
/// non-starters, so canonically equivalent strings still compare equal (ECMA-262
/// `localeCompare`), with runs of 20 to 2,000 marks in another order or with a precomposed
/// first letter. A CGJ in the input is ignorable. node v26.13.0 gives the same lines.
#[test]
fn the_stream_safe_step_keeps_canonical_equivalence() {
    assert_eq!(
        run_both(
            r#"var c = new Intl.Collator("en"), eq = [];
for (var k of [10, 15, 16, 30, 31, 40, 1000]) {
  var x = "a" + "̖́".repeat(k), y = "a" + "̖".repeat(k) + "́".repeat(k);
  var z = "á" + "̖".repeat(k) + "́".repeat(k - 1);
  eq.push(c.compare(x, y) + "," + c.compare(x, z) + "," + x.localeCompare(z) + "," + c.compare(x, "a" + "́".repeat(2 * k)));
}
print(eq.join(" "));
print(c.compare("a͏b", "ab"), c.compare("e" + "́".repeat(31), "é" + "́".repeat(30)), "ṩ".repeat(40).localeCompare("ṩ".repeat(40)));"#
        ),
        lines(&[
            "0,0,0,1 0,0,0,1 0,0,0,1 0,0,0,1 0,0,0,1 0,0,0,1 0,0,0,1",
            "0 0 0",
        ])
    );
}
