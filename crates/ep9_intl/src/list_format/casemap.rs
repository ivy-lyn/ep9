//! TransformCase ( S, locales, targetCase ) (ECMA-402 §20.1.2.1): the body of
//! `String.prototype.toLocaleLowerCase` and `toLocaleUpperCase` (INTL W2 LIST+CASEMAP), which
//! `ep9_builtins` calls (INTL-I1 §1.5). The language-sensitive mappings come from
//! `icu_casemap` (Unicode 18 data in ICU4X 2.4).
//!
//! ```text
//! locales ─ CanonicalizeLocaleList ─ first tag, or DefaultLocale() ─ no extension
//!   ─ LookupMatchingLocaleByPrefix(« az, el, lt, tr ») ─ none: None (the caller maps with
//!                                                        the Default Case Conversion)
//!                                                     └ a match: CaseMapper with its language
//! code units ─> runs split at each lone surrogate (copied as it is) ─> lowercase/uppercase
//! ```
//!
//! The available locales are the languages whose mappings differ from the root ones in the
//! Unicode data (SpecialCasing.txt: tr, az, lt; and the Greek uppercase of CLDR), as V8 and
//! SpiderMonkey list them. icu_casemap also has Armenian ("hy": U+0587 uppercases to
//! "ԵՎ") and Dutch (titlecase only); node v26.13.0 gives the root mapping for both
//! ("և".toLocaleUpperCase("hy") is "ԵՒ"), so they are not listed. A lone surrogate is neither
//! cased, case-ignorable nor a combining mark (General_Category Cs, ccc 0), so no context of
//! SpecialCasing.txt (Final_Sigma, After_Soft_Dotted, More_Above, Before_Dot, After_I) passes
//! one: each run maps alone.

use ep9_vm::{Cx, JsResult, Local};
use icu_casemap::CaseMapper;
use icu_locale_core::LanguageIdentifier;

use crate::{resolve, tag};

/// The languages with a language-sensitive case mapping (the availableLocales of
/// TransformCase step 4), sorted.
pub const LOCALES: [&str; 4] = ["az", "el", "lt", "tr"];

/// Which way TransformCase maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetCase {
    /// toLocaleLowerCase.
    Lower,
    /// toLocaleUpperCase.
    Upper,
}

/// TransformCase steps 2-6 for the canonical locale list `requested`: the matched available
/// locale (one of [`LOCALES`]), or `None` for "und". `default` gives DefaultLocale().
pub fn case_locale(requested: &[String], default: impl FnOnce() -> String) -> Option<&'static str> {
    // 2. If requestedLocales is not empty, let requestedLocale be requestedLocales[0]; else
    //    let requestedLocale be DefaultLocale().
    let requested = match requested.first() {
        Some(r) => r.clone(),
        None => default(),
    };
    // 3. Let noExtensionsLocale be requestedLocale with any Unicode locale extension
    //    sequences removed.
    let no_extensions = tag::remove_unicode_extension(&requested);
    // 5. Let match be LookupMatchingLocaleByPrefix(availableLocales, noExtensionsLocale). Each
    //    available locale is a language subtag alone, so the prefix walk ends at the language
    //    subtag of the tag (a canonical tag starts with it, in lower case).
    let language = no_extensions.split('-').next().unwrap_or("");
    // 6. If match is not undefined, let locale be match.[[locale]]; else "und".
    LOCALES.iter().copied().find(|l| *l == language)
}

/// The case mapping of the code units `s` with the ICU4X rules of `language` (one of
/// [`LOCALES`]).
pub fn map_units(s: &[u16], language: &str, target: TargetCase) -> Vec<u16> {
    let langid = LanguageIdentifier::try_from_str(language).unwrap_or(LanguageIdentifier::UNKNOWN);
    let cm = CaseMapper::new();
    let mut out = Vec::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut Vec<u16>| {
        if run.is_empty() {
            return;
        }
        let mapped = match target {
            TargetCase::Lower => cm.lowercase_to_string(run, &langid),
            TargetCase::Upper => cm.uppercase_to_string(run, &langid),
        };
        out.extend(mapped.encode_utf16());
        run.clear();
    };
    for c in char::decode_utf16(s.iter().copied()) {
        match c {
            Ok(c) => run.push(c),
            Err(e) => {
                flush(&mut run, &mut out);
                out.push(e.unpaired_surrogate());
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

/// TransformCase ( S, locales, targetCase ) (20.1.2.1) for the code units `s` (steps 1-2 of
/// the String methods, RequireObjectCoercible and ToString, are the caller's): the mapped code
/// units, or `None` when the matched locale is "und", so the caller maps `s` with the Unicode
/// Default Case Conversion (the mapping of toLowerCase and toUpperCase, step 8.a/9.a "or the
/// Unicode Default Case Conversion algorithm"). A RangeError or TypeError comes from
/// CanonicalizeLocaleList.
pub fn transform_case<'s>(
    cx: &mut Cx<'s>,
    s: &[u16],
    locales: Local<'s>,
    target: TargetCase,
) -> JsResult<Option<Vec<u16>>> {
    // 1. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 2-6.
    let Some(language) = case_locale(&requested, || resolve::default_locale(cx)) else {
        return Ok(None);
    };
    // 7-10. The mapping of the code points with the rules of the locale.
    Ok(Some(map_units(s, language, target)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(s: &str, l: &str, t: TargetCase) -> String {
        String::from_utf16_lossy(&map_units(&s.encode_utf16().collect::<Vec<_>>(), l, t))
    }

    #[test]
    fn the_locale_is_the_language_of_the_first_tag() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let en = || "en-US".to_string();
        assert_eq!(case_locale(&v(&["tr"]), en), Some("tr"));
        assert_eq!(case_locale(&v(&["az-Latn-AZ"]), en), Some("az"));
        assert_eq!(case_locale(&v(&["lt-u-co-trad"]), en), Some("lt"));
        assert_eq!(case_locale(&v(&["en", "tr"]), en), None);
        assert_eq!(case_locale(&v(&["hy"]), en), None);
        assert_eq!(case_locale(&v(&["trv"]), en), None);
        assert_eq!(case_locale(&[], || "el".to_string()), Some("el"));
        assert_eq!(case_locale(&[], en), None);
    }

    #[test]
    fn special_casing_of_the_four_languages() {
        use TargetCase::{Lower, Upper};
        assert_eq!(map("i", "tr", Upper), "\u{130}");
        assert_eq!(map("I\u{307}", "tr", Lower), "i");
        assert_eq!(map("I", "az", Lower), "\u{131}");
        assert_eq!(map("i\u{307}", "lt", Upper), "I");
        assert_eq!(map("\u{cc}", "lt", Lower), "i\u{307}\u{300}");
        assert_eq!(
            map("\u{3ac}\u{3bb}\u{3c6}\u{3b1}", "el", Upper),
            "\u{391}\u{39b}\u{3a6}\u{391}"
        );
        assert_eq!(map("\u{391}\u{3a3}", "el", Lower), "\u{3b1}\u{3c2}");
    }

    #[test]
    fn a_lone_surrogate_stays_and_ends_the_context() {
        let s: Vec<u16> = vec![u16::from(b'I'), 0xD800, 0x0307];
        assert_eq!(
            map_units(&s, "tr", TargetCase::Lower),
            vec![0x0131, 0xD800, 0x0307]
        );
        let s: Vec<u16> = vec![0xDC00, u16::from(b'i')];
        assert_eq!(map_units(&s, "tr", TargetCase::Upper), vec![0xDC00, 0x0130]);
    }
}
