//! The tag length limit (`tag::MAX_TAG_LEN`, 65,536 code units; H23 "Language tag length"):
//! it applies to the strings that JS gives, a tag and the free-form Intl.Locale options
//! (variants, calendar, collation, numberingSystem, firstDayOfWeek). At the limit they work,
//! one code unit more is a RangeError, on every path that gives a tag to ICU4X
//! (`Locale::try_from_str`): Intl.getCanonicalLocales, the Intl.Locale tag and its options,
//! the Intl.Collator locale list and Intl.Collator.supportedLocalesOf. A tag that the engine
//! made from such inputs (an `[[Locale]]`, the tags of UpdateLanguageId and MakeLocaleRecord,
//! maximize and minimize) is not checked again (Codex r2 F3). Before the limit, a tag of
//! 320,000 subtags ran for more than 120 s inside one ICU4X call (the `ShortBoxSlice` lists of
//! icu_locale_core grow by one reallocation per subtag), with no poll.

mod common;

use common::{lines, run};

/// The script prefix: `t` (65,536 code units: "en" and 8,192 distinct variants), `t1` (one
/// unit more), `o`/`o1` (a variants value of 65,536 / 65,537 units), `c`/`c1` (a type value
/// of 65,536 / 65,537 units, for calendar, collation, numberingSystem and firstDayOfWeek),
/// and `f(g)`, which prints "ok", or the error name and whether its message names the limit.
const PRELUDE: &str = r#"
const parts = [];
for (let i = 0; i < 8191; i++) parts.push('a' + String(i).padStart(6, '0'));
const v = parts.join('-') + '-abcde', v1 = parts.join('-') + '-abcdef';
const t = 'en-' + v, t1 = 'en-' + v1;
const o = parts.join('-') + '-abcdefgh', o1 = parts.join('-') + '-1abc-1abd';
const cal = [];
for (let i = 0; i < 8191; i++) cal.push('abcdefg');
const c = cal.join('-') + '-abcdefgh', c1 = cal.join('-') + '-abcd-abcd';
function f(g) {
  try { g(); return 'ok'; }
  catch (e) { return e.constructor.name + (e.message.includes('limit of 65536') ? ' limit' : ''); }
}
"#;

#[test]
fn a_tag_at_the_limit_works_and_one_unit_more_is_a_range_error() {
    let src = format!(
        r#"{PRELUDE}
print(t.length, t1.length, o.length, o1.length, c.length, c1.length);
print(f(() => Intl.getCanonicalLocales(t)), f(() => Intl.getCanonicalLocales(t1)));
print(f(() => Intl.getCanonicalLocales(['en', t1])));
print(f(() => new Intl.Locale(t)), f(() => new Intl.Locale(t1)));
print(f(() => new Intl.Locale('en', {{ variants: o }})), f(() => new Intl.Locale('en', {{ variants: o1 }})));
for (const k of ['calendar', 'collation', 'numberingSystem', 'firstDayOfWeek'])
  print(k, f(() => new Intl.Locale('en', {{ [k]: c }})), f(() => new Intl.Locale('en', {{ [k]: c1 }})));
print(f(() => new Intl.Collator(t)), f(() => new Intl.Collator([t1])));
print(f(() => Intl.Collator.supportedLocalesOf(t)), f(() => Intl.Collator.supportedLocalesOf([t1])));
print(Intl.getCanonicalLocales(t)[0].length, new Intl.Locale('en', {{ calendar: c }}).calendar.length);
print(f(() => Intl.getCanonicalLocales('en-' + 'x'.repeat(9))), f(() => Intl.getCanonicalLocales('en-' + 'x'.repeat(70000))));
// The options limit is on the value: a value under it that makes a longer tag works.
print(new Intl.Locale('en', {{ variants: v1 }}).toString().length, new Intl.Locale(t, {{ calendar: c }}).toString().length);
"#
    );
    assert_eq!(
        run(&src, false),
        lines(&[
            "65536 65537 65536 65537 65536 65537",
            "ok RangeError limit",
            "RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "calendar ok RangeError limit",
            "collation ok RangeError limit",
            "numberingSystem ok RangeError limit",
            "firstDayOfWeek ok RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "65536 65536",
            // A short invalid tag: the plain RangeError. A long one: the limit comes first
            // (it is checked before the subtag split).
            "RangeError RangeError limit",
            "65537 131078",
        ])
    );
}

/// Codex r2 F3: the limit is on input strings only. `t` has 65,536 code units; its canonical
/// form has 65,541 ("islamicc" -> "islamic-civil"). Every step that takes the tag the engine
/// made works, as in node v26.13.0: Intl.Locale (UpdateLanguageId re-reads the new tag), with
/// a region or a calendar option, maximize and minimize (the spec uses `! Construct`: they
/// cannot throw), an Intl.Locale as the tag or as a locale list element. A canonical string
/// passed back in is a new input: over the limit, a RangeError (node takes it; no spec text
/// asks for a limit either way). Then the made-tag bound (H23): a tag at the limit with the 5
/// free-form options at the limit gives about 5 x 65,536 code units, and the steps on it work.
#[test]
fn tags_that_the_engine_made_have_no_limit() {
    let src = format!(
        r#"{PRELUDE}
const u = 'en-u-ca-islamicc-x-' + 'a-'.repeat(32758) + 'a';
const g = Intl.getCanonicalLocales(u)[0];
print(u.length, g.length, g.slice(0, 23));
const l = new Intl.Locale(u);
print(l.toString().length, l.calendar, new Intl.Locale(u, {{}}).toString().length);
print(new Intl.Locale(u, {{ region: 'US' }}).toString().length, new Intl.Locale(u, {{ calendar: 'gregory' }}).toString().length);
print(l.maximize().toString().length, l.minimize().toString().length, l.maximize().baseName);
print(new Intl.Locale(l).toString().length, Intl.getCanonicalLocales([l])[0].length, Intl.getCanonicalLocales(l)[0].length);
print(f(() => new Intl.Collator(l)), f(() => Intl.Collator.supportedLocalesOf([l])));
print(f(() => Intl.getCanonicalLocales(g)), f(() => new Intl.Locale(g)));
const all = {{ variants: o, calendar: c, collation: c, numberingSystem: c, firstDayOfWeek: c }};
const big = new Intl.Locale(t, all);
print(big.toString().length, big.maximize().toString().length, big.minimize().toString().length);
print(new Intl.Locale(big).toString().length, new Intl.Locale(big, all).toString().length, Intl.getCanonicalLocales([big])[0].length);
"#
    );
    assert_eq!(
        run(&src, false),
        lines(&[
            "65536 65541 en-u-ca-islamic-civil-x",
            "65541 islamic-civil 65541",
            "65544 65535",
            "65549 65541 en-Latn-US",
            "65541 65541 65541",
            "ok ok",
            "RangeError limit RangeError limit",
            "327701 327709 327701",
            "327701 327701 327701",
        ])
    );
}

/// 320,000 distinct variants (2.6 MB of tag) and 320,000 calendar subtags, as a string
/// literal, so no JS runs before the call. The termination is requested before the script
/// runs; the call must end at once with the RangeError (not terminated: no poll is needed).
/// Before the limit the first call ran for about 30 s (debug build) inside ICU4X with the
/// termination pending, and ended with no error. The time check is the growth from 40,000 to
/// 160,000 subtags ([`common::growth`]: the limit check comes before any work that grows
/// faster than the tag, so about 1; the ICU4X lists grow as n^2 past about 120,000 subtags),
/// not a wall time (M6-ORCH-226).
#[test]
fn a_320k_subtag_tag_through_the_options_path_ends_at_once() {
    let calls = |n: usize| {
        let variants: Vec<String> = (0..n).map(|i| format!("a{i:06}")).collect();
        let calendar = vec!["abcde"; n].join("-");
        [
            format!(
                "new Intl.Locale('en', {{ variants: '{}' }})",
                variants.join("-")
            ),
            format!("new Intl.Locale('en', {{ calendar: '{calendar}' }})"),
            format!("Intl.getCanonicalLocales('en-u-ca-{calendar}')"),
        ]
    };
    for which in 0..3 {
        let ratio = common::growth(40_000, 160_000, |n| {
            let call = &calls(n)[which];
            assert_eq!(
                common::terminate_during(call),
                Some((false, Some("before".to_string()), Some("again".to_string()))),
                "{}",
                &call[..40]
            );
        });
        assert!(ratio < common::LINEAR, "call {which}: growth {ratio:.2}");
    }
    let variants: Vec<String> = (0..320_000).map(|i| format!("a{i:06}")).collect();
    let variants = variants.join("-");
    let src = format!(
        "try {{ new Intl.Locale('en', {{ variants: '{variants}' }}); }} \
         catch (e) {{ print(e.constructor.name, e.message.includes('Value of 2559999 code units')); }}"
    );
    assert_eq!(run(&src, false), lines(&["RangeError true"]));
}
