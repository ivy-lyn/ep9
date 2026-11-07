//! Language tags (ECMA-402 §6.2): IsStructurallyValidLanguageTag (6.2.1),
//! CanonicalizeUnicodeLocaleId (6.2.2) and CanonicalizeLocaleList (9.2.1).
//!
//! ```text
//! "EN-u-kn-yes-ca-ethiopic-amete-alem"
//!   -> parse (own grammar, UTS 35 unicode_locale_id; ECMA-402 limits: no "root", no
//!      leading script, no extlang, no duplicate variant or singleton)    else RangeError
//!   -> drop a repeated -u- key (the first one stays), build the lower-case string
//!   -> icu_locale_core::Locale, icu_locale::LocaleCanonicalizer (CLDR aliases: language,
//!      script, region, variant, complex region; tlang; rg and sd)
//!   -> the -u- and -t- value aliases of the CLDR bcp47 data ([`U_VALUE_ALIASES`]) and
//!      "yes" -> "true" for the boolean collation keys
//!   -> "en-u-ca-ethioaa-kn"
//! ```
//!
//! No `Cx` in this module except [`canonicalize_locale_list`], so the tag code has unit tests
//! with no isolate.

use ep9_vm::{Cx, JsResult, Local, ObjectClass};
use icu_locale::LocaleCanonicalizer;
use icu_locale_core::Locale;
use icu_locale_core::extensions::unicode::{Key, Value};

/// One parsed `unicode_locale_id`, all subtags lower case.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `unicode_language_id`: the language, then the script, region and variants (if any).
    pub language: String,
    pub script: Option<String>,
    pub region: Option<String>,
    pub variants: Vec<String>,
    /// The extensions other than `x`, in input order: the singleton and its subtags.
    pub extensions: Vec<(u8, Vec<String>)>,
    /// The private use subtags after `x` (empty: none).
    pub private: Vec<String>,
}

fn is_alpha(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_digit(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_alnum(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `unicode_language_subtag`: alpha{2,3} | alpha{5,8}.
pub fn is_language_subtag(s: &str) -> bool {
    is_alpha(s) && matches!(s.len(), 2 | 3 | 5..=8)
}

/// `unicode_script_subtag`: alpha{4}.
pub fn is_script_subtag(s: &str) -> bool {
    is_alpha(s) && s.len() == 4
}

/// `unicode_region_subtag`: alpha{2} | digit{3}.
pub fn is_region_subtag(s: &str) -> bool {
    (is_alpha(s) && s.len() == 2) || (is_digit(s) && s.len() == 3)
}

/// `unicode_variant_subtag`: alphanum{5,8} | digit alphanum{3}.
pub fn is_variant_subtag(s: &str) -> bool {
    is_alnum(s)
        && match s.len() {
            5..=8 => true,
            4 => s.as_bytes()[0].is_ascii_digit(),
            _ => false,
        }
}

/// `alphanum{3,8}`: a Unicode extension attribute or type subtag, or a tvalue subtag.
fn is_3_8(s: &str) -> bool {
    is_alnum(s) && (3..=8).contains(&s.len())
}

/// `key = alphanum alpha`.
fn is_ukey(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphanumeric() && b[1].is_ascii_alphabetic()
}

/// `tkey = alpha digit`.
fn is_tkey(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1].is_ascii_digit()
}

/// A `unicode_language_id` (or a `tlang`) from `subtags[*i..]`; moves `*i` past it. `None`
/// if the first subtag is no language subtag, or a variant repeats.
#[allow(clippy::type_complexity)]
fn language_id(
    subtags: &[String],
    i: &mut usize,
) -> Option<(String, Option<String>, Option<String>, Vec<String>)> {
    let lang = subtags.get(*i).filter(|s| is_language_subtag(s))?.clone();
    *i += 1;
    let mut script = None;
    let mut region = None;
    let mut variants: Vec<String> = Vec::new();
    if let Some(s) = subtags.get(*i).filter(|s| is_script_subtag(s)) {
        script = Some(s.clone());
        *i += 1;
    }
    if let Some(s) = subtags.get(*i).filter(|s| is_region_subtag(s)) {
        region = Some(s.clone());
        *i += 1;
    }
    while let Some(s) = subtags.get(*i).filter(|s| is_variant_subtag(s)) {
        if variants.contains(s) {
            return None;
        }
        variants.push(s.clone());
        *i += 1;
    }
    Some((lang, script, region, variants))
}

/// Parses `tag` by the `unicode_locale_id` grammar of UTS 35 with the ECMA-402 limits
/// (IsStructurallyValidLanguageTag, 6.2.1): `None` if it is not structurally valid. The
/// subtags of the result are lower case.
pub fn parse(tag: &str) -> Option<Parsed> {
    if tag.is_empty() || !tag.is_ascii() {
        return None;
    }
    let subtags: Vec<String> = tag.split('-').map(|s| s.to_ascii_lowercase()).collect();
    if subtags
        .iter()
        .any(|s| s.is_empty() || s.len() > 8 || !is_alnum(s))
    {
        return None;
    }
    let mut i = 0;
    let (language, script, region, variants) = language_id(&subtags, &mut i)?;
    let mut out = Parsed {
        language,
        script,
        region,
        variants,
        ..Parsed::default()
    };
    let mut singletons: Vec<u8> = Vec::new();
    while i < subtags.len() {
        let s = &subtags[i];
        if s.len() != 1 {
            return None;
        }
        let singleton = s.as_bytes()[0];
        if singletons.contains(&singleton) {
            return None;
        }
        singletons.push(singleton);
        i += 1;
        let start = i;
        match singleton {
            b'x' => {
                // pu_extensions = sep [xX] (sep alphanum{1,8})+
                if i == subtags.len() {
                    return None;
                }
                out.private = subtags[i..].to_vec();
                return Some(out);
            }
            b'u' => {
                // (sep keyword)+ | (sep attribute)+ (sep keyword)*
                while subtags.get(i).is_some_and(|s| is_3_8(s)) {
                    i += 1;
                }
                while subtags.get(i).is_some_and(|s| is_ukey(s)) {
                    i += 1;
                    while subtags.get(i).is_some_and(|s| is_3_8(s)) {
                        i += 1;
                    }
                }
            }
            b't' => {
                // (sep tlang (sep tfield)*) | (sep tfield)+
                if subtags.get(i).is_some_and(|s| is_language_subtag(s)) {
                    language_id(&subtags, &mut i)?;
                }
                while subtags.get(i).is_some_and(|s| is_tkey(s)) {
                    i += 1;
                    let values = i;
                    while subtags.get(i).is_some_and(|s| is_3_8(s)) {
                        i += 1;
                    }
                    if i == values {
                        return None;
                    }
                }
            }
            _ => {
                // other_extensions = sep [alphanum-[tTuUxX]] (sep alphanum{2,8})+
                while subtags.get(i).is_some_and(|s| s.len() >= 2) {
                    i += 1;
                }
            }
        }
        if i == start {
            return None;
        }
        out.extensions.push((singleton, subtags[start..i].to_vec()));
    }
    Some(out)
}

/// IsStructurallyValidLanguageTag ( locale ) (6.2.1).
pub fn is_structurally_valid_language_tag(tag: &str) -> bool {
    parse(tag).is_some()
}

/// The -u- and -t- value aliases of the CLDR bcp47 data (`common/bcp47/*.xml`, the `alias`
/// and `deprecated`/`preferred` attributes) whose alias matches `alphanum{3,8}`, as
/// (key, alias, value). The rg and sd subdivision aliases come from ICU4X
/// (`LocaleCanonicalizer`). CLDR 48 (the data of the ICU4X 2.3 compiled data); each line was
/// checked against node v26.13.0 (ICU) `Intl.getCanonicalLocales`.
pub const U_VALUE_ALIASES: &[(&str, &str, &str)] = &[
    // calendar.xml
    ("ca", "ethiopic-amete-alem", "ethioaa"),
    ("ca", "islamicc", "islamic-civil"),
    // collation.xml
    ("ks", "primary", "level1"),
    ("ks", "tertiary", "level3"),
    // measure.xml
    ("ms", "imperial", "uksystem"),
    // timezone.xml: deprecated ids and IANA aliases
    ("tz", "aqams", "aqmcm"),
    ("tz", "aukns", "auhba"),
    ("tz", "cnckg", "cnsha"),
    ("tz", "cnhrb", "cnsha"),
    ("tz", "cnkhg", "cnurc"),
    ("tz", "cuba", "cuhav"),
    ("tz", "egypt", "egcai"),
    ("tz", "eire", "iedub"),
    ("tz", "est", "papty"),
    ("tz", "gmt0", "gmt"),
    ("tz", "hongkong", "hkhkg"),
    ("tz", "hst", "ushnl"),
    ("tz", "iceland", "isrey"),
    ("tz", "iran", "irthr"),
    ("tz", "israel", "jeruslm"),
    ("tz", "jamaica", "jmkin"),
    ("tz", "japan", "jptyo"),
    ("tz", "libya", "lytip"),
    ("tz", "mst", "usphx"),
    ("tz", "mxstis", "mxtij"),
    ("tz", "navajo", "usden"),
    ("tz", "poland", "plwaw"),
    ("tz", "portugal", "ptlis"),
    ("tz", "prc", "cnsha"),
    ("tz", "roc", "twtpe"),
    ("tz", "rok", "krsel"),
    ("tz", "turkey", "trist"),
    ("tz", "uct", "utc"),
    ("tz", "usnavajo", "usden"),
    ("tz", "zulu", "utc"),
];

/// The -t- tvalue aliases (CLDR `transform*.xml`): (tkey, alias, value).
const T_VALUE_ALIASES: &[(&str, &str, &str)] = &[("m0", "names", "prprname")];

/// The Unicode extension keys whose "yes" is an alias of "true" (CLDR collation.xml).
const YES_IS_TRUE: &[&str] = &["kb", "kc", "kh", "kk", "kn"];

/// The canonical value of the Unicode extension type `value` (lower case, subtags joined by
/// "-") of key `key`: the CLDR alias table, and "yes" -> "true" for the boolean collation
/// keys. "true" stays "true" here (the canonical tag leaves it out).
pub fn canonicalize_unicode_value(key: &str, value: &str) -> String {
    if value == "yes" && YES_IS_TRUE.contains(&key) {
        return "true".to_string();
    }
    U_VALUE_ALIASES
        .iter()
        .find(|(k, a, _)| *k == key && *a == value)
        .map_or_else(|| value.to_string(), |(_, _, v)| (*v).to_string())
}

/// The canonicalizer (CLDR alias data, compiled; made at compile time: it holds only
/// references to the `'static` baked data).
const CANONICALIZER: LocaleCanonicalizer = LocaleCanonicalizer::new_extended();

/// The tag of `p` with a repeated -u- key dropped (the first one stays, UTS 35 §3.2.1) and
/// the -u- and -t- value aliases applied, lower case.
fn rebuild(p: &Parsed) -> String {
    let mut s = p.language.clone();
    for part in p.script.iter().chain(&p.region).chain(&p.variants) {
        s.push('-');
        s.push_str(part);
    }
    for (singleton, subtags) in &p.extensions {
        s.push('-');
        s.push(char::from(*singleton));
        match singleton {
            b'u' => {
                let mut i = 0;
                while i < subtags.len() && is_3_8(&subtags[i]) {
                    s.push('-');
                    s.push_str(&subtags[i]);
                    i += 1;
                }
                let mut seen: Vec<&str> = Vec::new();
                while i < subtags.len() {
                    let key = subtags[i].as_str();
                    i += 1;
                    let start = i;
                    while i < subtags.len() && is_3_8(&subtags[i]) {
                        i += 1;
                    }
                    if seen.contains(&key) {
                        continue;
                    }
                    seen.push(key);
                    s.push('-');
                    s.push_str(key);
                    let value = subtags[start..i].join("-");
                    let value = canonicalize_unicode_value(key, &value);
                    if !value.is_empty() {
                        s.push('-');
                        s.push_str(&value);
                    }
                }
            }
            b't' => {
                let mut i = 0;
                while i < subtags.len() {
                    let tag = subtags[i].as_str();
                    if is_tkey(tag) {
                        let start = i + 1;
                        let mut end = start;
                        while end < subtags.len() && is_3_8(&subtags[end]) {
                            end += 1;
                        }
                        let value = subtags[start..end].join("-");
                        let value = T_VALUE_ALIASES
                            .iter()
                            .find(|(k, a, _)| *k == tag && *a == value)
                            .map_or(value.clone(), |(_, _, v)| (*v).to_string());
                        s.push('-');
                        s.push_str(tag);
                        s.push('-');
                        s.push_str(&value);
                        i = end;
                    } else {
                        s.push('-');
                        s.push_str(tag);
                        i += 1;
                    }
                }
            }
            _ => {
                for t in subtags {
                    s.push('-');
                    s.push_str(t);
                }
            }
        }
    }
    if !p.private.is_empty() {
        s.push_str("-x");
        for t in &p.private {
            s.push('-');
            s.push_str(t);
        }
    }
    s
}

/// CanonicalizeUnicodeLocaleId ( locale ) (6.2.2) of a structurally valid tag `p`: the UTS 35
/// canonical form (case, subtag order, aliases). ICU4X reads only 2-3 letter languages, and
/// ECMA-402 also allows 5-8 letters: such a language ("abcdefgh-latn-us") takes "und" in its
/// place for the ICU4X steps and gets its own place back at the end ("abcdefgh-Latn-US", as
/// node v26.13.0). If ICU4X still cannot read the tag, the lower-case rebuilt tag is the
/// result.
pub fn canonicalize_parsed(p: &Parsed) -> String {
    let rebuilt = rebuild(p);
    let long = p.language.len() >= 5;
    let input = match rebuilt.strip_prefix(p.language.as_str()) {
        Some(rest) if long => format!("und{rest}"),
        _ => rebuilt.clone(),
    };
    let Ok(mut loc) = Locale::try_from_str(&input) else {
        return rebuilt;
    };
    CANONICALIZER.canonicalize(&mut loc);
    // A canonicalized value may be an alias again ("yes" from no alias; the rg/sd rules of
    // ICU4X give canonical values), so one pass of the value table is enough.
    let fix: Vec<(Key, Value)> = loc
        .extensions
        .unicode
        .keywords
        .iter()
        .filter_map(|(k, v)| {
            let text = v.to_string();
            let canonical = canonicalize_unicode_value(k.as_str(), &text);
            if canonical == text {
                return None;
            }
            Value::try_from_str(&canonical).ok().map(|nv| (*k, nv))
        })
        .collect();
    for (k, v) in fix {
        loc.extensions.unicode.keywords.set(k, v);
    }
    let out = loc.to_string();
    if !long {
        return out;
    }
    // "und" is a language-free alias match, so an alias rule could give another language;
    // then the rebuilt tag is the result, as before.
    match out.strip_prefix("und") {
        Some(tail) if tail.is_empty() || tail.starts_with('-') => format!("{}{tail}", p.language),
        _ => rebuilt,
    }
}

/// The canonical form of `tag` (6.2.2), or `None` if it is not structurally valid (6.2.1).
pub fn canonicalize_tag(tag: &str) -> Option<String> {
    parse(tag).map(|p| canonicalize_parsed(&p))
}

/// `tag` without its Unicode locale extension sequence (the `-u-...` part), as the
/// ResolveLocale and BestAvailableLocale steps need. `tag` is canonical; the other subtags
/// keep their case.
pub fn remove_unicode_extension(tag: &str) -> String {
    let parts: Vec<&str> = tag.split('-').collect();
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    let mut in_u = false;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 && p.len() == 1 {
            if *p == "x" || *p == "X" {
                out.extend_from_slice(&parts[i..]);
                break;
            }
            in_u = p.eq_ignore_ascii_case("u");
        }
        if !in_u {
            out.push(p);
        }
    }
    out.join("-")
}

/// The Unicode extension keywords of the canonical tag `tag`, as (key, value) in tag order
/// (value "" for "true"). Attributes are left out.
pub fn unicode_keywords(tag: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(p) = parse(tag) else { return out };
    let Some((_, subtags)) = p.extensions.iter().find(|(s, _)| *s == b'u') else {
        return out;
    };
    let mut i = 0;
    while i < subtags.len() && is_3_8(&subtags[i]) {
        i += 1;
    }
    while i < subtags.len() {
        let key = subtags[i].clone();
        i += 1;
        let start = i;
        while i < subtags.len() && is_3_8(&subtags[i]) {
            i += 1;
        }
        if out.iter().all(|(k, _)| *k != key) {
            let v = subtags[start..i].join("-");
            out.push((key, if v == "true" { String::new() } else { v }));
        }
    }
    out
}

/// CanonicalizeLocaleList ( locales ) (9.2.1): the canonical tags of `locales`, without
/// repeats, in order. An Intl.Locale object gives its `[[Locale]]` (slot 0 of
/// `ObjectClass::IntlLocale`).
pub fn canonicalize_locale_list<'s>(cx: &mut Cx<'s>, locales: Local<'s>) -> JsResult<Vec<String>> {
    // 1. If locales is undefined, return a new empty List.
    if locales.is_undefined() {
        return Ok(Vec::new());
    }
    // 2. Let seen be a new empty List.
    let mut seen: Vec<String> = Vec::new();
    // 3. If locales is a String or has an [[InitializedLocale]] internal slot, let O be
    //    CreateArrayFromList(« locales »): the one element, read directly.
    if locales.is_string() || cx.object_class(locales) == Some(ObjectClass::IntlLocale) {
        let tag = element_tag(cx, locales)?;
        seen.push(tag);
        return Ok(seen);
    }
    // 4. Else, let O be ? ToObject(locales).
    let o = cx.to_object(locales)?;
    // 5. Let len be ? LengthOfArrayLike(O).
    let len = cx.get(o, "length")?;
    let len = cx.to_length(len)?;
    // 6-7. For each k < len: if ? HasProperty(O, Pk), read and check the element.
    let mut k: u64 = 0;
    while k < len {
        if cx.has_index(o, k)? {
            let v = cx.get_index(o, k)?;
            // 7.c.ii. If kValue is not a String or an Object, throw a TypeError exception.
            if !v.is_string() && v.as_object().is_none() {
                return Err(cx.throw_type_error("a locale must be a String or an Object"));
            }
            let tag = element_tag(cx, v)?;
            if !seen.contains(&tag) {
                seen.push(tag);
            }
        }
        k += 1;
    }
    Ok(seen)
}

/// Steps 7.c.iii-vii of CanonicalizeLocaleList for one element `v` (a String or an Object):
/// its tag, checked and canonicalized.
fn element_tag<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<String> {
    let tag = if cx.object_class(v) == Some(ObjectClass::IntlLocale) {
        let t = cx.slot(v, ObjectClass::IntlLocale, 0)?;
        cx.to_rust_string(t)?
    } else {
        cx.to_rust_string(v)?
    };
    match canonicalize_tag(&tag) {
        Some(t) => Ok(t),
        None => Err(cx.throw_range_error(&format!("Incorrect locale information provided: {tag}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_and_invalid_tags() {
        for t in [
            "en",
            "en-US",
            "zh-Hant-TW",
            "sl-rozaj-biske-1994",
            "en-0abc",
            "de-u-co-phonebk",
            "en-u-attr-ca-gregory",
            "en-t-m0-true",
            "en-x-u-foo",
            "en-a-bar-x-u-foo",
            "art-lojban",
            "und",
            "en-u-ca-gregory-ca-buddhist",
        ] {
            assert!(is_structurally_valid_language_tag(t), "{t}");
        }
        for t in [
            "",
            "x-private",
            "i-klingon",
            "root",
            "en-a-x-b",
            "en-emodeng-emodeng",
            "en_US",
            "en-u-ca-gregory-u-nu-latn",
            "en-t",
            "en-t-root",
            "en-t-ar-aao",
            "en-t-en-latn-latn",
            "en-t-d0",
            "en-t-d0-x-private",
            "en-",
            "-en",
            "en--US",
            "abcd",
            "zh-yue",
            "Latn-en",
            "en-u",
            "en-x",
            "en-a",
            "en-a-b",
            "en-ü",
        ] {
            assert!(!is_structurally_valid_language_tag(t), "{t}");
        }
    }

    /// node v26.13.0 `Intl.getCanonicalLocales` results.
    #[test]
    fn canonical_forms() {
        for (t, want) in [
            ("en-u-ca-gregory-ca-buddhist", "en-u-ca-gregory"),
            ("sl-rozaj-biske-1994", "sl-1994-biske-rozaj"),
            ("EN-U-CA-GREGORY-X-A-B", "en-u-ca-gregory-x-a-b"),
            ("de-u-kn-true-co-phonebk", "de-u-co-phonebk-kn"),
            ("art-lojban", "jbo"),
            ("zh-guoyu", "zh"),
            ("cel-gaulish", "xtg"),
            (
                "und-Latn-t-und-hani-m0-names",
                "und-Latn-t-und-hani-m0-prprname",
            ),
            ("de-DD", "de-DE"),
            ("sgn-GR", "gss"),
            ("en-b-ccc-a-ddd", "en-a-ddd-b-ccc"),
            ("hy-arevela", "hy"),
            ("en-x-u-foo", "en-x-u-foo"),
            ("en-a-bar-u-baz-x-u-foo", "en-a-bar-u-baz-x-u-foo"),
            ("und-u-ca-ethiopic-amete-alem", "und-u-ca-ethioaa"),
            ("und-u-kn-yes", "und-u-kn"),
            ("und-u-kf-yes", "und-u-kf-yes"),
            ("und-u-tz-cnckg", "und-u-tz-cnsha"),
            ("und-u-ms-imperial", "und-u-ms-uksystem"),
            ("und-u-rg-no23", "und-u-rg-no50"),
            ("en-t-iw", "en-t-he"),
            ("DE-T-M0-DIN-K0-QWERTZ", "de-t-k0-qwertz-m0-din"),
            ("sl-t-sl-rozaj-biske-1994", "sl-t-sl-1994-biske-rozaj"),
            ("en-t-m0-true", "en-t-m0-true"),
            ("en-u-cu-usd-cu-eur", "en-u-cu-usd"),
            // 5-8 letter languages (node v26.13.0); the value alias of a long language too.
            ("abcdefgh-latn-us", "abcdefgh-Latn-US"),
            (
                "ABCDE-arab-001-u-nu-arab-ca-gregory",
                "abcde-Arab-001-u-ca-gregory-nu-arab",
            ),
            ("abcde-u-ca-islamicc", "abcde-u-ca-islamic-civil"),
            ("abcde-u-tz-japan", "abcde-u-tz-jptyo"),
            ("abcdefgh", "abcdefgh"),
            ("abcde-hepburn-heploc", "abcde-alalc97"),
        ] {
            assert_eq!(canonicalize_tag(t).as_deref(), Some(want), "{t}");
        }
    }

    #[test]
    fn keywords_and_extension_removal() {
        assert_eq!(
            remove_unicode_extension("de-u-co-phonebk-kn-x-priv"),
            "de-x-priv"
        );
        assert_eq!(
            remove_unicode_extension("de-CH-a-bc-u-co-phonebk"),
            "de-CH-a-bc"
        );
        assert_eq!(remove_unicode_extension("de-CH-x-u-foo"), "de-CH-x-u-foo");
        assert_eq!(
            unicode_keywords("de-u-attr-co-phonebk-kn"),
            vec![
                ("co".to_string(), "phonebk".to_string()),
                ("kn".to_string(), String::new())
            ]
        );
    }
}
