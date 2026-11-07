//! Locale resolution (ECMA-402 §9.2): DefaultLocale (6.2.4), LookupMatchingLocaleByPrefix,
//! ResolveLocale, LookupSupportedLocales and SupportedLocales. "best fit" runs the lookup
//! matcher too, as V8 and SpiderMonkey do by default (INTL-I1 §5.3 row 15).
//!
//! ```text
//! requested (CanonicalizeLocaleList) ──> lookup_matching_locale ──> { locale, extension }
//!   ("de-CH-u-co-phonebk")               prefix walk over available::locales()
//!                                        ("de-CH" -> "de"); none: DefaultLocale
//!   ──> resolve_locale(relevant keys, locale data) ──> ResolvedLocale { locale, data_locale,
//!       keywords }   ("de-u-co-phonebk", "de", [("co", "phonebk")])
//! ```

use std::sync::OnceLock;

use ep9_vm::{Cx, JsResult, Local};

use crate::available;
use crate::options::{coerce_options_to_object, get_locale_matcher};
use crate::tag;

/// The locale when the host gives none, or one with no data (and the last fallback of every
/// service).
pub const FALLBACK_LOCALE: &str = "en-US";

/// DefaultLocale ( ) (6.2.4): the host locale (`Cx::default_locale`), canonical, without
/// extensions, cut to its longest available prefix; [`FALLBACK_LOCALE`] if the host gives
/// none or no prefix is available. So the result is always an available locale.
///
/// The host locale is read once per process, at the first use (as node: V8 sets the ICU
/// default locale at start-up), so a later change of LC_ALL, LC_MESSAGES or LANG changes
/// nothing. Every cached object made for the default locale (the default collator of
/// `localeCompare`, a thread-local `crate::cache` slot) then stays right for every isolate
/// (Codex r1 COLLATOR-2).
pub fn default_locale(cx: &mut Cx<'_>) -> String {
    static DEFAULT: OnceLock<String> = OnceLock::new();
    DEFAULT
        .get_or_init(|| {
            cx.default_locale()
                .and_then(|t| tag::canonicalize_tag(&t).ok())
                .map(|t| tag::remove_unicode_extension(&t))
                .and_then(|t| best_available_locale(&t))
                .unwrap_or_else(|| FALLBACK_LOCALE.to_string())
        })
        .clone()
}

/// The prefix walk of LookupMatchingLocaleByPrefix (and BestAvailableLocale): the longest
/// available prefix of `locale` (canonical, no Unicode extension). A cut also drops a
/// singleton before the last subtag ("de-x-foo" -> "de").
pub fn best_available_locale(locale: &str) -> Option<String> {
    let mut prefix = locale;
    loop {
        if available::is_available(prefix) {
            return Some(prefix.to_string());
        }
        // b. If prefix contains "-": let pos be the index of the last "-"; if pos >= 2 and
        //    prefix[pos - 2] is "-", set pos to pos - 2.
        let mut pos = prefix.rfind('-')?;
        if pos >= 2 && prefix.as_bytes()[pos - 2] == b'-' {
            pos -= 2;
        }
        prefix = &prefix[..pos];
    }
}

/// The result of LookupMatchingLocaleByPrefix: the available locale and the Unicode locale
/// extension sequence of the requested tag it came from (the `-u-...` text, or empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub locale: String,
    pub extension: Vec<(String, String)>,
}

/// LookupMatchingLocaleByPrefix ( availableLocales, requestedLocales ) (9.2.2): the first
/// requested locale (canonical) whose prefix is available.
pub fn lookup_matching_locale(requested: &[String]) -> Option<Match> {
    requested.iter().find_map(|r| {
        let no_ext = tag::remove_unicode_extension(r);
        best_available_locale(&no_ext).map(|locale| Match {
            locale,
            extension: tag::unicode_keywords(r),
        })
    })
}

/// The data of one relevant extension key for a found locale: the allowed values, the
/// default first (the `localeData.[[<locale>]].[[<key>]]` list of §9.1). "true" is a
/// boolean key's value spelled "true".
pub type KeyData = Vec<String>;

/// The result of ResolveLocale (9.2.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLocale {
    /// `[[Locale]]`: the found locale with the supported extension keywords put back.
    pub locale: String,
    /// `[[DataLocale]]`: the found locale, no extension.
    pub data_locale: String,
    /// The value of each relevant key, in `relevant_keys` order.
    pub keywords: Vec<(String, String)>,
}

/// ResolveLocale ( availableLocales, requestedLocales, options, relevantExtensionKeys,
/// localeData ) (9.2.7). `options` holds the option value of each key that the constructor
/// read (`None`: undefined). `locale_data(found_locale, key)` gives the key's data list,
/// default first. A RangeError only if the result tag is over the tag length limit
/// (`tag::MAX_TAG_LEN`; it does not happen: an available locale and supported values).
pub fn resolve_locale(
    cx: &mut Cx<'_>,
    requested: &[String],
    options: &[(&str, Option<String>)],
    relevant_keys: &[&str],
    locale_data: &dyn Fn(&str, &str) -> KeyData,
) -> JsResult<ResolvedLocale> {
    // 1-3. matcher: both values run the lookup matcher; none matched: DefaultLocale.
    let r = lookup_matching_locale(requested).unwrap_or_else(|| Match {
        locale: default_locale(cx),
        extension: Vec::new(),
    });
    // 4. Let foundLocale be r.[[locale]].
    let found = r.locale;
    let mut keywords: Vec<(String, String)> = Vec::new();
    let mut supported: Vec<(String, String)> = Vec::new();
    // 8. For each key of relevantExtensionKeys:
    for key in relevant_keys {
        let data = locale_data(&found, key);
        // c. Let value be keyLocaleData[0].
        let mut value = data.first().cloned().unwrap_or_default();
        let mut supported_keyword: Option<(String, String)> = None;
        // e-f. The requested keyword of this key, if any.
        if let Some((_, requested_value)) = r.extension.iter().find(|(k, _)| k == key) {
            if !requested_value.is_empty() {
                if data.contains(requested_value) {
                    value = requested_value.clone();
                    supported_keyword = Some(((*key).to_string(), value.clone()));
                }
            } else if data.iter().any(|v| v == "true") {
                value = "true".to_string();
                supported_keyword = Some(((*key).to_string(), String::new()));
            }
        }
        // g-i. The option value, canonicalized as a Unicode extension value.
        if let Some((_, Some(option))) = options.iter().find(|(k, _)| k == key) {
            let lower = option.to_ascii_lowercase();
            let mut v = tag::canonicalize_unicode_value(key, &lower);
            if v.is_empty() {
                v = "true".to_string();
            }
            // j. If SameValue(optionsValue, value) is false and keyLocaleData contains
            //    optionsValue, the option wins and the keyword goes.
            if v != value && data.contains(&v) {
                value = v;
                supported_keyword = None;
            }
        }
        if let Some(kw) = supported_keyword {
            supported.push(kw);
        }
        keywords.push(((*key).to_string(), value));
    }
    // 9. InsertUnicodeExtensionAndCanonicalize(foundLocale, « », supportedKeywords).
    let locale = if supported.is_empty() {
        found.clone()
    } else {
        match insert_unicode_extension(&found, &supported) {
            Ok(l) => l,
            Err(e) => return Err(cx.throw_range_error(&e.message(&found))),
        }
    };
    Ok(ResolvedLocale {
        locale,
        data_locale: found,
        keywords,
    })
}

/// InsertUnicodeExtensionAndCanonicalize ( locale, attributes, keywords ) (6.2.3) with no
/// attributes: `locale` (no Unicode extension) with `-u-<key>[-<value>]...` put before any
/// private-use part, then canonicalized. The error of the canonicalization (a result over
/// the tag length limit) is kept, not hidden.
pub fn insert_unicode_extension(
    locale: &str,
    keywords: &[(String, String)],
) -> Result<String, tag::TagError> {
    let mut ext = String::from("-u");
    for (k, v) in keywords {
        ext.push('-');
        ext.push_str(k);
        if !v.is_empty() {
            ext.push('-');
            ext.push_str(v);
        }
    }
    let joined = match locale.find("-x-") {
        Some(i) => format!("{}{ext}{}", &locale[..i], &locale[i..]),
        None => format!("{locale}{ext}"),
    };
    tag::canonicalize_tag(&joined)
}

/// LookupSupportedLocales ( availableLocales, requestedLocales ) (9.2.8): the requested
/// locales (with their extensions) whose prefix without the extension is available.
pub fn lookup_supported_locales(requested: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|r| best_available_locale(&tag::remove_unicode_extension(r)).is_some())
        .cloned()
        .collect()
}

/// SupportedLocales ( availableLocales, requestedLocales, options ) (9.2.10): the
/// `supportedLocalesOf` result as an Array.
pub fn supported_locales<'s>(
    cx: &mut Cx<'s>,
    requested: &[String],
    options: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set options to ? CoerceOptionsToObject(options).
    let options = coerce_options_to_object(cx, options)?;
    // 2. Let matcher be ? GetOption(options, "localeMatcher", ...).
    get_locale_matcher(cx, options)?;
    // 3-4. Both matchers: LookupSupportedLocales.
    let list = lookup_supported_locales(requested);
    // 5. Return CreateArrayFromList(supportedLocales).
    crate::intl::string_array(cx, &list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_walk() {
        assert_eq!(best_available_locale("de-CH").as_deref(), Some("de-CH"));
        assert_eq!(best_available_locale("en-ZZ").as_deref(), Some("en"));
        assert_eq!(best_available_locale("de-x-foo").as_deref(), Some("de"));
        assert_eq!(best_available_locale("zz"), None);
        assert_eq!(
            best_available_locale("zh-Hant-TW").as_deref(),
            Some("zh-Hant-TW")
        );
    }

    #[test]
    fn lookup_keeps_the_extension() {
        let m = lookup_matching_locale(&["zz".into(), "de-CH-u-co-phonebk".into()]).unwrap();
        assert_eq!(m.locale, "de-CH");
        assert_eq!(m.extension, vec![("co".to_string(), "phonebk".to_string())]);
        assert_eq!(
            lookup_supported_locales(&["zz".into(), "de-u-co-phonebk".into(), "en-ZZ".into()]),
            vec!["de-u-co-phonebk".to_string(), "en-ZZ".to_string()]
        );
    }

    #[test]
    fn insert_extension() {
        assert_eq!(
            insert_unicode_extension(
                "de",
                &[
                    ("kn".into(), String::new()),
                    ("co".into(), "phonebk".into())
                ]
            ),
            Ok("de-u-co-phonebk-kn".to_string())
        );
        assert_eq!(
            insert_unicode_extension("de-x-priv", &[("co".into(), "phonebk".into())]),
            Ok("de-u-co-phonebk-x-priv".to_string())
        );
    }
}
