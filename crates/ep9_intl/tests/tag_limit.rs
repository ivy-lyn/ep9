//! The tag length limit (`tag::MAX_TAG_LEN`, 65,536 code units; H23 "Size limits"): a tag
//! at the limit works, one code unit more is a RangeError, on every path that gives a tag to
//! ICU4X (`Locale::try_from_str`): Intl.getCanonicalLocales, the Intl.Locale tag and its
//! options, the Intl.Collator locale list and Intl.Collator.supportedLocalesOf. Before the
//! limit, a tag of 320,000 subtags ran for more than 120 s inside one ICU4X call (the
//! `ShortBoxSlice` lists of icu_locale_core grow by one reallocation per subtag), with no
//! poll.

mod common;

use common::{lines, run};

/// The script prefix: `t` (65,536 code units: "en" and 8,192 distinct variants), `t1` (one
/// unit more), `v`/`v1` (the variants of `t`/`t1`, for the variants option), `c`/`c1` (a
/// calendar value that gives a tag of 65,536 / 65,537 units with "en-u-ca-"), and `f(g)`,
/// which prints "ok", or the error name and whether its message names the limit.
const PRELUDE: &str = r#"
const parts = [];
for (let i = 0; i < 8191; i++) parts.push('a' + String(i).padStart(6, '0'));
const v = parts.join('-') + '-abcde', v1 = parts.join('-') + '-abcdef';
const t = 'en-' + v, t1 = 'en-' + v1;
const cal = [];
for (let i = 0; i < 8190; i++) cal.push('abcdefg');
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
print(t.length, t1.length, ('en-u-ca-' + c).length, ('en-u-ca-' + c1).length);
print(f(() => Intl.getCanonicalLocales(t)), f(() => Intl.getCanonicalLocales(t1)));
print(f(() => Intl.getCanonicalLocales(['en', t1])));
print(f(() => new Intl.Locale(t)), f(() => new Intl.Locale(t1)));
print(f(() => new Intl.Locale('en', {{ variants: v }})), f(() => new Intl.Locale('en', {{ variants: v1 }})));
print(f(() => new Intl.Locale('en', {{ calendar: c }})), f(() => new Intl.Locale('en', {{ calendar: c1 }})));
print(f(() => new Intl.Collator(t)), f(() => new Intl.Collator([t1])));
print(f(() => Intl.Collator.supportedLocalesOf(t)), f(() => Intl.Collator.supportedLocalesOf([t1])));
print(Intl.getCanonicalLocales(t)[0].length, new Intl.Locale('en', {{ calendar: c }}).calendar.length);
print(f(() => Intl.getCanonicalLocales('en-' + 'x'.repeat(9))), f(() => Intl.getCanonicalLocales('en-' + 'x'.repeat(70000))));
"#
    );
    assert_eq!(
        run(&src, false),
        lines(&[
            "65536 65537 65536 65537",
            "ok RangeError limit",
            "RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "ok RangeError limit",
            "65536 65528",
            // A short invalid tag: the plain RangeError. A long one: the limit comes first
            // (it is checked before the subtag split).
            "RangeError RangeError limit",
        ])
    );
}

/// 320,000 distinct variants (2.6 MB of tag) and 320,000 calendar subtags, as a string
/// literal, so no JS runs before the call. The termination is requested before the script
/// runs; the call must end at once with the RangeError (not terminated: no poll is needed).
/// Before the limit the first call ran for about 30 s (debug build) inside ICU4X with the
/// termination pending, and ended with no error.
#[test]
fn a_320k_subtag_tag_through_the_options_path_ends_at_once() {
    let variants: Vec<String> = (0..320_000).map(|i| format!("a{i:06}")).collect();
    let variants = variants.join("-");
    let calendar = vec!["abcde"; 320_000].join("-");
    for call in [
        format!("new Intl.Locale('en', {{ variants: '{variants}' }})"),
        format!("new Intl.Locale('en', {{ calendar: '{calendar}' }})"),
        format!("Intl.getCanonicalLocales('en-u-ca-{calendar}')"),
    ] {
        let t = std::time::Instant::now();
        assert_eq!(
            common::terminate_during(&call),
            Some((false, Some("before".to_string()), Some("again".to_string()))),
            "{}",
            &call[..40]
        );
        // Far above the time of a debug build (about 0.5 s), far below the old one.
        let took = t.elapsed();
        assert!(took < std::time::Duration::from_secs(10), "{took:?}");
    }
    let src = format!(
        "try {{ new Intl.Locale('en', {{ variants: '{variants}' }}); }} \
         catch (e) {{ print(e.constructor.name, e.message.includes('of 2560002 code units')); }}"
    );
    assert_eq!(run(&src, false), lines(&["RangeError true"]));
}
