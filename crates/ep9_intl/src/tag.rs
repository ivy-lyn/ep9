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
//!   -> the -u- and -t- value aliases of the CLDR bcp47 data ([`U_VALUE_ALIASES`], generated:
//!      crate::cldr; "yes" -> "true" for the boolean collation keys is one of them)
//!   -> "en-u-ca-ethioaa-kn"
//! ```
//!
//! No `Cx` in this module except [`canonicalize_locale_list`], so the tag code has unit tests
//! with no isolate.

use std::collections::HashSet;

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
    subtags: &[&str],
    i: &mut usize,
) -> Option<(String, Option<String>, Option<String>, Vec<String>)> {
    let lang = subtags
        .get(*i)
        .filter(|s| is_language_subtag(s))?
        .to_string();
    *i += 1;
    let mut script = None;
    let mut region = None;
    let mut variants: Vec<String> = Vec::new();
    // The repeat check: a set, so a tag of n variants costs O(n), not O(n^2).
    let mut seen: HashSet<&str> = HashSet::new();
    if let Some(s) = subtags.get(*i).filter(|s| is_script_subtag(s)) {
        script = Some(s.to_string());
        *i += 1;
    }
    if let Some(s) = subtags.get(*i).filter(|s| is_region_subtag(s)) {
        region = Some(s.to_string());
        *i += 1;
    }
    while let Some(s) = subtags.get(*i).filter(|s| is_variant_subtag(s)) {
        if !seen.insert(s) {
            return None;
        }
        variants.push(s.to_string());
        *i += 1;
    }
    Some((lang, script, region, variants))
}

/// The subtags `s` as owned strings.
fn owned(s: &[&str]) -> Vec<String> {
    s.iter().map(|t| (*t).to_string()).collect()
}

/// The engine limit on the length of a language tag string that JS gives, in code units (H23
/// "Language tag length"): a longer input tag is a RangeError ([`TagError::TooLong`]), and so
/// is a longer value of one of the free-form Intl.Locale options (variants, calendar,
/// collation, numberingSystem, firstDayOfWeek). No ECMA-402 text gives a limit; it bounds the
/// time of the ICU4X steps, whose subtag lists grow by one reallocation per subtag
/// (icu_locale_core `ShortBoxSlice`, O(n^2) past about 120,000 subtags). At this length the
/// worst list takes about 67 ms; the longest test262 tag has 80 characters. A tag that the
/// engine made from such inputs (an `[[Locale]]`, the tags of UpdateLanguageId and
/// MakeLocaleRecord) is not checked again ([`canonicalize_made_tag`]): it can be a little
/// longer (an alias value: "islamicc" -> "islamic-civil") or up to about 5 times longer
/// (the options), and each of its subtag lists stays below the knee (Codex r2 F3).
pub const MAX_TAG_LEN: usize = 65_536;

/// Why a language tag gives a RangeError.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagError {
    /// It is not structurally valid (IsStructurallyValidLanguageTag, 6.2.1).
    Invalid,
    /// It is longer than [`MAX_TAG_LEN`] code units (an engine limit): its length.
    TooLong(usize),
}

impl TagError {
    /// The RangeError message for the tag `tag` (the input of the step that failed). A tag
    /// that is too long is not put in the message.
    pub fn message(self, tag: &str) -> String {
        match self {
            TagError::Invalid => format!("Incorrect locale information provided: {tag}"),
            TagError::TooLong(n) => format!(
                "Incorrect locale information provided: a language tag of {n} code units is \
                 longer than the limit of {MAX_TAG_LEN}"
            ),
        }
    }
}

/// Parses `tag` by the `unicode_locale_id` grammar of UTS 35 with the ECMA-402 limits
/// (IsStructurallyValidLanguageTag, 6.2.1): [`TagError::Invalid`] if it is not structurally
/// valid, [`TagError::TooLong`] if it is longer than [`MAX_TAG_LEN`] code units (checked
/// first, before the subtag split). The subtags of the result are lower case.
pub fn parse(tag: &str) -> Result<Parsed, TagError> {
    if tag.is_empty() || !tag.is_ascii() {
        return Err(TagError::Invalid);
    }
    // ASCII: one byte is one code unit.
    if tag.len() > MAX_TAG_LEN {
        return Err(TagError::TooLong(tag.len()));
    }
    parse_grammar(tag).ok_or(TagError::Invalid)
}

/// [`parse`] with no length limit, for a tag that this crate made from inputs that passed the
/// limit ([`MAX_TAG_LEN`]; a canonical tag can be longer than its input: an alias value, the
/// Intl.Locale options).
pub fn parse_canonical(tag: &str) -> Option<Parsed> {
    parse_grammar(tag)
}

/// The grammar of [`parse`], with no length limit.
fn parse_grammar(tag: &str) -> Option<Parsed> {
    if tag.is_empty() || !tag.is_ascii() {
        return None;
    }
    // Each subtag is 1-8 ASCII letters or digits: checked on the input before any subtag list
    // is made, so a long invalid tag ("-".repeat(n)) is rejected with no allocation.
    if tag
        .split('-')
        .any(|s| s.is_empty() || s.len() > 8 || !is_alnum(s))
    {
        return None;
    }
    let lower = tag.to_ascii_lowercase();
    let subtags: Vec<&str> = lower.split('-').collect();
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
                out.private = owned(&subtags[i..]);
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
        out.extensions.push((singleton, owned(&subtags[start..i])));
    }
    Some(out)
}

/// IsStructurallyValidLanguageTag ( locale ) (6.2.1), with no length limit.
pub fn is_structurally_valid_language_tag(tag: &str) -> bool {
    parse_grammar(tag).is_some()
}

/// The -u- value aliases of the CLDR bcp47 data (`common/bcp47/*.xml`, the `alias` and
/// `deprecated`/`preferred` attributes) whose alias matches `alphanum{3,8}`, as (key, alias,
/// value); "yes" -> "true" of the boolean collation keys (kb, kc, kh, kk, kn) is one of them.
/// The rg and sd subdivision aliases come from ICU4X (`LocaleCanonicalizer`). Generated by
/// `tools/cldr-tables` from the CLDR release of the ICU4X compiled data
/// (`cldr/gen/aliases.rs`).
pub const U_VALUE_ALIASES: &[(&str, &str, &str)] = &crate::cldr::aliases::U_VALUE_ALIASES;

/// The -t- tvalue aliases (CLDR `transform*.xml`): (tkey, alias, value). Generated in the same
/// file.
const T_VALUE_ALIASES: &[(&str, &str, &str)] = &crate::cldr::aliases::T_VALUE_ALIASES;

/// The canonical value of the Unicode extension type `value` (lower case, subtags joined by
/// "-") of key `key`: the CLDR alias table (with "yes" -> "true" for the boolean collation
/// keys). "true" stays "true" here (the canonical tag leaves it out).
pub fn canonicalize_unicode_value(key: &str, value: &str) -> String {
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
/// ECMA-402 also allows 5-8 letters, in the main language and in the -t- tlang: such a
/// language ("abcdefgh-latn-us", "iw-t-abcdefgh") takes "und" in its place for the ICU4X
/// steps and gets its own place back at the end ("abcdefgh-Latn-US", "he-t-abcdefgh", as node
/// v26.13.0). If ICU4X still cannot read the tag, or an alias rule changes a placeholder, the
/// lower-case rebuilt tag is the result.
pub fn canonicalize_parsed(p: &Parsed) -> String {
    let rebuilt = rebuild(p);
    let long = p.language.len() >= 5;
    // The first subtag of a -t- extension is a tlang language (alpha) or a tkey (2 units).
    let tlang = p
        .extensions
        .iter()
        .position(|(s, t)| *s == b't' && t.first().is_some_and(|l| l.len() >= 5));
    let input = if long || tlang.is_some() {
        let mut q = p.clone();
        if long {
            q.language = "und".to_string();
        }
        if let Some(i) = tlang {
            q.extensions[i].1[0] = "und".to_string();
        }
        rebuild(&q)
    } else {
        rebuilt.clone()
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
    let mut out = loc.to_string();
    // "und" is a language-free alias match, so an alias rule could give another language;
    // then the rebuilt tag is the result, as before.
    if long {
        match out.strip_prefix("und") {
            Some(tail) if tail.is_empty() || tail.starts_with('-') => {
                out = format!("{}{tail}", p.language);
            }
            _ => return rebuilt,
        }
    }
    if let Some(i) = tlang {
        // The first "-t-" of the result is the -t- extension: the subtags of the base name
        // and of the other extensions have 2 or more units, and ICU4X writes the private use
        // part (the only other place of a 1-unit subtag) last.
        let Some(at) = out.find("-t-").map(|at| at + 3) else {
            return rebuilt;
        };
        let rest = &out[at..];
        match rest.strip_prefix("und") {
            Some(tail) if tail.is_empty() || tail.starts_with('-') => {
                out = format!("{}{}{tail}", &out[..at], p.extensions[i].1[0]);
            }
            _ => return rebuilt,
        }
    }
    out
}

/// The canonical form of `tag` (6.2.2), or the [`TagError`] of [`parse`]: for a tag string
/// that JS gives (the limit applies).
pub fn canonicalize_tag(tag: &str) -> Result<String, TagError> {
    parse(tag).map(|p| canonicalize_parsed(&p))
}

/// The canonical form of `tag`, a tag that the engine made (an Intl.Locale `[[Locale]]`, the
/// new tag of UpdateLanguageId, MakeLocaleRecord or ResolveLocale), with no length limit
/// ([`parse_canonical`]): the spec steps that take such a tag do not fail on it
/// (MakeLocaleRecord returns a Record; maximize and minimize use `! Construct`), so the limit
/// of the inputs must not reject it (Codex r2 F3). [`TagError::Invalid`] only if it is not
/// structurally valid (not reached: the inputs were checked).
pub fn canonicalize_made_tag(tag: &str) -> Result<String, TagError> {
    parse_canonical(tag)
        .map(|p| canonicalize_parsed(&p))
        .ok_or(TagError::Invalid)
}

/// The ICU4X locale of the data locale `data_locale` (canonical, no extension; "und" if ICU4X
/// cannot read it).
pub fn icu_locale(data_locale: &str) -> Locale {
    Locale::try_from_str(data_locale).unwrap_or(Locale::UNKNOWN)
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
    let Some(p) = parse_canonical(tag) else {
        return out;
    };
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
    // 6-7. For each k < len: if ? HasProperty(O, Pk), read and check the element. A hole runs
    //      no JS, so the walk to len (up to 2^53 - 1) takes one interrupt budget unit per
    //      pass (H23 "Interrupt poll").
    let mut k: u64 = 0;
    while k < len {
        cx.poll_interrupt()?;
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
/// its tag, checked and canonicalized. The `[[Locale]]` of an Intl.Locale is a tag that the
/// engine made, so it has no length limit ([`canonicalize_made_tag`]); a String has it.
fn element_tag<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<String> {
    let (tag, made) = if cx.object_class(v) == Some(ObjectClass::IntlLocale) {
        let t = cx.slot(v, ObjectClass::IntlLocale, 0)?;
        (cx.to_rust_string(t)?, true)
    } else {
        (cx.to_rust_string(v)?, false)
    };
    let c = if made {
        canonicalize_made_tag(&tag)
    } else {
        canonicalize_tag(&tag)
    };
    c.map_err(|e| cx.throw_range_error(&e.message(&tag)))
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

    /// The variant repeat check is a set: 12,500 and 50,000 variants parse in linear time,
    /// with and without a repeat at the end (Codex r1 CORE-3: the list scan was O(n^2), 8.2 s
    /// for 100,000 variants in a release build, and it has no poll). The check is the growth
    /// of the time ([`crate::testing::growth`]: about 1 for a linear cost; the old scan about
    /// 4), not a wall time (M6-ORCH-226).
    #[test]
    fn many_variants_parse_in_linear_time() {
        let ratio = crate::testing::growth(12_500, 50_000, |n| {
            let variants: Vec<String> = (0..n).map(|i| format!("v{}", 1_000_000 + i)).collect();
            let tag = format!("en-{}", variants.join("-"));
            let repeat = format!("{tag}-v1000000");
            assert_eq!(parse_canonical(&tag).map(|p| p.variants.len()), Some(n));
            assert!(!is_structurally_valid_language_tag(&repeat));
        });
        assert!(ratio < crate::testing::LINEAR, "growth {ratio:.2}");
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
            // CLDR bcp47 aliases that node v26.13.0 also applies (INTL W2-0: the table is
            // generated from the CLDR 49 data; the CLDR 48 hand table had none of these).
            ("und-u-tz-cet", "und-u-tz-bebru"),
            ("und-u-tz-factory", "und-u-tz-unk"),
            ("und-u-tz-caffs", "und-u-tz-cawnp"),
            ("en-t-d0-name", "en-t-d0-charname"),
            ("en-t-m0-ies-jes", "en-t-m0-iesjes"),
            // CLDR 49 no longer has these as aliases (node v26.13.0, CLDR 48: the same).
            ("und-u-tz-est5edt", "und-u-tz-est5edt"),
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
            // A 5-8 letter tlang language (Claude reviewer R3; node v26.13.0).
            ("iw-t-abcdefgh", "he-t-abcdefgh"),
            ("de-DD-t-abcdefgh", "de-DE-t-abcdefgh"),
            ("en-latn-us-t-abcdefgh", "en-Latn-US-t-abcdefgh"),
            (
                "sl-rozaj-biske-1994-t-abcdefgh",
                "sl-1994-biske-rozaj-t-abcdefgh",
            ),
            ("en-u-ca-gregory-t-abcdefgh", "en-t-abcdefgh-u-ca-gregory"),
            ("en-u-kn-yes-t-abcdefgh", "en-t-abcdefgh-u-kn"),
            ("iw-Latn-t-abcdefgh", "he-Latn-t-abcdefgh"),
            // The control rows: a 2-3 letter tlang; both languages long; a tlang with its
            // own script, region and variants; a "-t-und" in the private use part.
            ("en-t-iw-abcdefgh", "en-t-he-abcdefgh"),
            ("abcde-t-abcdefgh-latn-us", "abcde-t-abcdefgh-latn-us"),
            ("abcde-t-iw", "abcde-t-he"),
            ("abcdefgh-t-abcdefgh", "abcdefgh-t-abcdefgh"),
            (
                "en-t-abcdefgh-latn-us-m0-names-x-t-und",
                "en-t-abcdefgh-latn-us-m0-prprname-x-t-und",
            ),
        ] {
            assert_eq!(canonicalize_tag(t).as_deref(), Ok(want), "{t}");
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
