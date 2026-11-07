//! Intl.Collator (ECMA-402 §10; block 31): `%Intl.Collator%` and `%Intl.Collator.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics. Also the body of `String.prototype.localeCompare` (§19.1.1,
//! [`locale_compare`]), which `ep9_builtins` calls (INTL-I1 §1.5).
//!
//! ```text
//! Intl.Collator {w, c}            %Intl.Collator%            prototype {}, supportedLocalesOf
//! Intl.Collator.prototype         %Intl.Collator.prototype%  constructor {w, c},
//!                                                            resolvedOptions {w, c},
//!                                                            compare (get, {c}),
//!                                                            [@@toStringTag] "Intl.Collator" {c}
//! slots (ObjectClass::IntlCollator, SLOTS = 3):
//!   0 [[Locale]]        a String ("de-u-co-phonebk")
//!   1 options           an int32 (Packed): [[Usage]], [[Sensitivity]], [[IgnorePunctuation]],
//!                       [[Numeric]], [[CaseFirst]], [[Collation]]
//!   2 [[BoundCompare]]  undefined, or the compare function (made by the first `compare` get)
//! compare(x, y) ─ ToString both ─ cache::get_or_make(Service::Collator, data locale,
//!                                  packed) ─> Rc<CollatorBorrowed>  ─ compare_utf16
//! ```
//!
//! Locale data (§10.2.3): `co` comes from the ICU4X tailoring data (a collation type is
//! supported for a locale if the data has a tailoring for it, through the locale fallback);
//! `kf` and `ignorePunctuation` defaults come from [`UPPER_FIRST`] and [`SHIFTED`], which a
//! test checks against the ICU4X data for every available locale. No ICU4X object is in a
//! slot (INTL-I1-D7, plan rule 8): the constructor checks the options and resolves the locale
//! with no ICU4X object; the first compare makes one through `crate::cache`.
//!
//! Known gap (INTL-I1 §5.3 row 12): the baked data has no `search` tailoring, so `usage:
//! "search"` compares with the sort tailoring (resolvedOptions keeps `usage`), and its `co`
//! list is « null » (the spec's `[[SearchLocaleData]]` then has no collation type).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-collator.txt`.

use core::cmp::Ordering;
use std::borrow::Cow;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};
use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering, CollationType};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_locale_core::Locale;
use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_normalizer::properties::{
    CanonicalCombiningClassMap, CanonicalCombiningClassMapBorrowed, CanonicalDecompositionBorrowed,
    Decomposed,
};

use crate::cache::{self, Service};
use crate::options::{
    coerce_options_to_object, get_boolean_option, get_locale_matcher, get_string_option,
};
use crate::resolve::{self, KeyData};
use crate::tag;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 31;

/// The class of an Intl.Collator object.
pub const CLASS: ObjectClass = ObjectClass::IntlCollator;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the [`Packed`] options (an int32).
pub const SLOT_OPTIONS: usize = 1;
/// Slot 2: `[[BoundCompare]]` (undefined until the first `compare` get).
pub const SLOT_BOUND_COMPARE: usize = 2;
/// The slot count of an Intl.Collator object.
pub const SLOTS: usize = 3;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const GET_COMPARE: u16 = 2;
    pub const RESOLVED_OPTIONS: u16 = 3;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Collator", 0, constructor), // 0 (10.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (10.2.2)
    BuiltinDef::getter("compare", get_compare),          // 2 (10.3.3)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 3 (10.3.4)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlCollator,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlCollatorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlCollator,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlCollatorPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlCollatorPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlCollator,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, then compare.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            // get Intl.Collator.prototype.compare ({c}, 10.3.3).
            PropDef::accessor(PropKey::Str("compare"), Some(def::GET_COMPARE), None),
            // Intl.Collator.prototype [ @@toStringTag ] ({c}, 10.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Collator"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Collator"),
            IntrinsicId::IntlCollator,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "collator",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Sensitivity]]` values, in their packed order.
const SENSITIVITIES: [&str; 4] = ["base", "accent", "case", "variant"];
/// `[[CaseFirst]]` values, in their packed order.
const CASE_FIRSTS: [&str; 3] = ["false", "lower", "upper"];
/// The CLDR collation types (`common/bcp47/collation.xml` without "standard" and "search"),
/// in their packed order (append-only; packed index i + 1).
const COLLATION_TYPES: [&str; 13] = [
    "compat", "dict", "emoji", "eor", "phonebk", "phonetic", "pinyin", "searchjl", "stroke",
    "trad", "unihan", "zhuyin", "ducet",
];

/// The resolved options of a collator, packed in one int32 (slot 1; the cache key without
/// the usage bit):
///
/// ```text
/// bit 0      [[Usage]]              0 "sort", 1 "search"
/// bits 1-2   [[Sensitivity]]        index in SENSITIVITIES
/// bit 3      [[IgnorePunctuation]]
/// bit 4      [[Numeric]]
/// bits 5-6   [[CaseFirst]]          index in CASE_FIRSTS
/// bits 7-10  [[Collation]]          0 "default", i + 1 COLLATION_TYPES[i]
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed(pub u32);

impl Packed {
    const SEARCH: u32 = 1;

    /// Packs the resolved options. `sensitivity` is an index in SENSITIVITIES, `case_first`
    /// in CASE_FIRSTS, `collation` 0 or 1 + an index in COLLATION_TYPES.
    fn new(
        search: bool,
        sensitivity: u32,
        ignore_punctuation: bool,
        numeric: bool,
        case_first: u32,
        collation: u32,
    ) -> Packed {
        Packed(
            u32::from(search)
                | (sensitivity & 3) << 1
                | u32::from(ignore_punctuation) << 3
                | u32::from(numeric) << 4
                | (case_first & 3) << 5
                | (collation & 15) << 7,
        )
    }

    /// `[[Usage]]`.
    pub fn usage(self) -> &'static str {
        if self.0 & Self::SEARCH != 0 {
            "search"
        } else {
            "sort"
        }
    }

    /// `[[Sensitivity]]`.
    pub fn sensitivity(self) -> &'static str {
        SENSITIVITIES[(self.0 >> 1 & 3) as usize]
    }

    /// `[[IgnorePunctuation]]`.
    pub fn ignore_punctuation(self) -> bool {
        self.0 >> 3 & 1 != 0
    }

    /// `[[Numeric]]`.
    pub fn numeric(self) -> bool {
        self.0 >> 4 & 1 != 0
    }

    /// `[[CaseFirst]]`.
    pub fn case_first(self) -> &'static str {
        CASE_FIRSTS
            .get((self.0 >> 5 & 3) as usize)
            .copied()
            .unwrap_or("false")
    }

    /// `[[Collation]]`: "default" or a CLDR collation type.
    pub fn collation(self) -> &'static str {
        match self.0 >> 7 & 15 {
            0 => "default",
            i => COLLATION_TYPES
                .get(i as usize - 1)
                .copied()
                .unwrap_or("default"),
        }
    }

    /// The cache key: the options that change the ICU4X collator (all but the usage).
    fn key(self) -> u64 {
        u64::from(self.0 & !Self::SEARCH)
    }
}

/// The result of InitializeCollator: `[[Locale]]` and the packed options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// The other resolved options.
    pub options: Packed,
}

// ---- the locale data (§10.2.3) ----------------------------------------------------------------

/// The data locales whose default `kf` is "upper" (the CLDR collation data; a test checks the
/// list against the ICU4X data for every available locale). A locale gets the value of its
/// longest listed prefix.
pub const UPPER_FIRST: &[&str] = &["da", "mt"];

/// The data locales whose default alternate handling is "shifted", so `ignorePunctuation` is
/// true by default (checked as [`UPPER_FIRST`] is).
pub const SHIFTED: &[&str] = &["th"];

/// Whether `data_locale` (canonical, no extension) or one of its prefixes is in `list`.
fn in_locale_list(list: &[&str], data_locale: &str) -> bool {
    let mut prefix = data_locale;
    loop {
        if list.contains(&prefix) {
            return true;
        }
        match prefix.rfind('-') {
            Some(i) => prefix = &prefix[..i],
            None => return false,
        }
    }
}

/// The default `[[CaseFirst]]` of `data_locale` (the first element of its `kf` list).
pub fn default_case_first(data_locale: &str) -> &'static str {
    if in_locale_list(UPPER_FIRST, data_locale) {
        "upper"
    } else {
        "false"
    }
}

/// The default `[[IgnorePunctuation]]` of `data_locale` (`resolvedLocaleData.[[ignorePunctuation]]`).
pub fn default_ignore_punctuation(data_locale: &str) -> bool {
    in_locale_list(SHIFTED, data_locale)
}

/// The ICU4X locale of `data_locale` (canonical, no extension; "und" if ICU4X cannot read it).
fn icu_locale(data_locale: &str) -> Locale {
    Locale::try_from_str(data_locale).unwrap_or(Locale::UNKNOWN)
}

/// Whether the ICU4X data has the collation type `co` for `data_locale` (through the locale
/// fallback: "de-CH" finds the "de" phonebk tailoring). A trie lookup; no ICU4X object.
pub fn has_collation(data_locale: &str, co: &str) -> bool {
    use icu_collator::provider::{Baked, CollationMetadataV1};
    use icu_provider::prelude::*;
    let Ok(attributes) = DataMarkerAttributes::try_from_str(co) else {
        return false;
    };
    let prefs = CollatorPreferences::from(&icu_locale(data_locale));
    let locale = CollationMetadataV1::make_locale(prefs.locale_preferences);
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale),
        metadata: {
            let mut m = DataRequestMetadata::default();
            m.silent = true;
            m
        },
    };
    DataProvider::<CollationMetadataV1>::load(&Baked, req).is_ok()
}

/// The collation types of `data_locale` (`localeData.[[<locale>]].[[co]]` without the
/// leading null; also `Intl.Locale.prototype.getCollations`), sorted.
pub fn collations_of(data_locale: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = COLLATION_TYPES
        .iter()
        .copied()
        .filter(|co| has_collation(data_locale, co))
        .collect();
    v.sort_unstable();
    v
}

/// `localeData.[[<locale>]].[[<key>]]` (§10.2.3) for the relevant keys "co", "kf" and "kn",
/// default first. The null of "co" is "". `search`: the `[[SearchLocaleData]]` (its "co" is
/// « null »; INTL-I1 §5.3 row 12).
fn key_data(data_locale: &str, key: &str, search: bool) -> KeyData {
    match key {
        "co" => {
            let mut v = vec![String::new()];
            if !search {
                v.extend(collations_of(data_locale).into_iter().map(str::to_string));
            }
            v
        }
        "kf" => {
            let first = default_case_first(data_locale);
            let mut v = vec![first.to_string()];
            v.extend(
                ["false", "lower", "upper"]
                    .iter()
                    .filter(|s| **s != first)
                    .map(|s| (*s).to_string()),
            );
            v
        }
        "kn" => vec!["false".to_string(), "true".to_string()],
        _ => Vec::new(),
    }
}

/// Whether `s` matches the Unicode locale identifier `type` nonterminal: alphanum{3,8}
/// subtags joined by "-" (UTS 35 §3.2).
fn is_unicode_type(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|t| (3..=8).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_alphanumeric()))
}

// ---- InitializeCollator (10.1.2) --------------------------------------------------------------

/// InitializeCollator steps 4-32 (10.1.1; the object creation of step 3 is the caller's): the
/// `[[Locale]]` and the packed options of a collator for `locales` and `options`, in the spec
/// order of the observable steps. No ICU4X object is made.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 4. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 5. Set options to ? CoerceOptionsToObject(options).
    let options = coerce_options_to_object(cx, options)?;
    // 6. Let usage be ? GetOption(options, "usage", string, « "sort", "search" », "sort").
    let usage = get_string_option(cx, options, "usage", &["sort", "search"], Some("sort"))?;
    let search = usage.as_deref() == Some("search");
    // 9-11. Let matcher be ? GetOption(options, "localeMatcher", ...).
    get_locale_matcher(cx, options)?;
    // 12. Let collation be ? GetOption(options, "collation", string, empty, undefined).
    let collation = get_string_option(cx, options, "collation", &[], None)?;
    // 13. If collation is not undefined and does not match the type nonterminal, throw a
    //     RangeError exception.
    if let Some(c) = &collation
        && !is_unicode_type(c)
    {
        return Err(cx.throw_range_error(&format!("Invalid collation : {c}")));
    }
    // 15-16. Let numeric be ? GetOption(options, "numeric", boolean, empty, undefined); if
    //        it is not undefined, set it to ! ToString(numeric).
    let numeric = get_boolean_option(cx, options, "numeric", None)?.map(|b| b.to_string());
    // 18. Let caseFirst be ? GetOption(options, "caseFirst", string, « "upper", "lower",
    //     "false" », undefined).
    let case_first =
        get_string_option(cx, options, "caseFirst", &["upper", "lower", "false"], None)?;
    // 20-21. Let r be ResolveLocale(%Intl.Collator%.[[AvailableLocales]], requestedLocales,
    //        opt, « "co", "kf", "kn" », localeData).
    let opt = [("co", collation), ("kn", numeric), ("kf", case_first)];
    let r = resolve::resolve_locale(cx, &requested, &opt, &["co", "kf", "kn"], &|l, k| {
        key_data(l, k, search)
    })?;
    let keyword = |k: &str| {
        r.keywords
            .iter()
            .find(|(key, _)| key == k)
            .map_or("", |(_, v)| v.as_str())
    };
    // 23-24. Let collation be r.[[co]]; null is "default".
    let collation = COLLATION_TYPES
        .iter()
        .position(|c| *c == keyword("co"))
        .map_or(0, |i| i as u32 + 1);
    // 25. [[Numeric]] is SameValue(r.[[kn]], "true").
    let numeric = keyword("kn") == "true";
    // 26. [[CaseFirst]] is r.[[kf]].
    let case_first = CASE_FIRSTS
        .iter()
        .position(|c| *c == keyword("kf"))
        .unwrap_or(0) as u32;
    // 28-29. [[Sensitivity]]: ? GetOption(options, "sensitivity", string, « "base",
    //        "accent", "case", "variant" », "variant") (the search default is "variant"
    //        too).
    let sensitivity =
        get_string_option(cx, options, "sensitivity", &SENSITIVITIES, Some("variant"))?;
    let sensitivity = SENSITIVITIES
        .iter()
        .position(|s| Some(*s) == sensitivity.as_deref())
        .unwrap_or(3) as u32;
    // 30-32. [[IgnorePunctuation]]: ? GetOption(options, "ignorePunctuation", boolean,
    //        empty, undefined), else the locale data.
    let ignore_punctuation = get_boolean_option(cx, options, "ignorePunctuation", None)?
        .unwrap_or_else(|| default_ignore_punctuation(&r.data_locale));
    Ok(Resolved {
        locale: r.locale,
        options: Packed::new(
            search,
            sensitivity,
            ignore_punctuation,
            numeric,
            case_first,
            collation,
        ),
    })
}

// ---- the ICU4X collator and CompareStrings (10.3.3.2) -----------------------------------------

/// A new ICU4X collator for `data_locale` and `options` (`None` if the data has none).
fn make_collator(data_locale: &str, options: Packed) -> Option<CollatorBorrowed<'static>> {
    let mut prefs = CollatorPreferences::from(&icu_locale(data_locale));
    prefs.collation_type = match options.collation() {
        "default" => None,
        co => CollationType::try_from(
            &icu_locale_core::extensions::unicode::Value::try_from_str(co).ok()?,
        )
        .ok(),
    };
    prefs.case_first = Some(match options.case_first() {
        "upper" => CollationCaseFirst::Upper,
        "lower" => CollationCaseFirst::Lower,
        _ => CollationCaseFirst::False,
    });
    prefs.numeric_ordering = Some(if options.numeric() {
        CollationNumericOrdering::True
    } else {
        CollationNumericOrdering::False
    });
    let mut o = CollatorOptions::default();
    // "base": primary; "accent": secondary; "case": primary with the case level; "variant":
    // tertiary.
    let (strength, case_level) = match options.sensitivity() {
        "base" => (Strength::Primary, CaseLevel::Off),
        "accent" => (Strength::Secondary, CaseLevel::Off),
        "case" => (Strength::Primary, CaseLevel::On),
        _ => (Strength::Tertiary, CaseLevel::Off),
    };
    o.strength = Some(strength);
    o.case_level = Some(case_level);
    o.alternate_handling = Some(if options.ignore_punctuation() {
        AlternateHandling::Shifted
    } else {
        AlternateHandling::NonIgnorable
    });
    icu_collator::Collator::try_new(prefs, o).ok()
}

/// The cached ICU4X collator of `locale` (a `[[Locale]]`) and `options` (crate::cache; the
/// "und" data if the locale has none).
fn collator_for(locale: &str, options: Packed) -> Option<std::rc::Rc<CollatorBorrowed<'static>>> {
    let data_locale = tag::remove_unicode_extension(locale);
    cache::get_or_make(Service::Collator, &data_locale, options.key(), || {
        make_collator(&data_locale, options).or_else(|| make_collator("und", options))
    })
}

/// The most non-starters in a row that the collator gets (UAX #15 §13, Stream-Safe Text
/// Format).
const MAX_NON_STARTERS: usize = 30;

/// U+034F COMBINING GRAPHEME JOINER: a starter with no collation weight (completely
/// ignorable), so it ends a run of non-starters and changes no order by itself. The step
/// puts in two: the collator's identical-prefix step goes back from the first difference
/// until it finds two starters in a row, one code point at a time, each with a shift of its
/// list (`prepend_upcoming_before_init`, icu_collator `elements.rs`), so with one CGJ a long
/// identical prefix of runs still cost O(n^2).
const CGJ: u16 = 0x034F;

/// The canonical combining class, the one-step canonical decomposition and NFD of the
/// ICU4X normalizer (compiled data: the NFD tables that the collator reads too).
const CCC: CanonicalCombiningClassMapBorrowed<'static> = CanonicalCombiningClassMap::new();
const DECOMPOSITION: CanonicalDecompositionBorrowed<'static> =
    CanonicalDecompositionBorrowed::new();
const NFD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfd();

/// Adds the code points of the full canonical decomposition of `c` to `run`, the count of
/// non-starters in a row (a starter sets it to 0); true once `run` is above
/// [`MAX_NON_STARTERS`]. The canonical reorder of NFD moves no starter, so this counts the
/// runs of NFD.
fn nfd_run(c: char, run: &mut usize) -> bool {
    match DECOMPOSITION.decompose(c) {
        Decomposed::Default => {
            if CCC.get_u8(c) == 0 {
                *run = 0;
            } else {
                *run += 1;
            }
            *run > MAX_NON_STARTERS
        }
        Decomposed::Singleton(d) => nfd_run(d, run),
        Decomposed::Expansion(a, b) => nfd_run(a, run) || nfd_run(b, run),
    }
}

/// The UAX #15 stream-safe form of `s` for the collator. The ICU4X 2.3 collator gathers a
/// run of non-starters in a list that it shifts once per character (`upcoming.remove(0)`,
/// icu_collator `elements.rs`), so a run of n costs O(n^2) ('a' + 320,000 U+0301 took
/// 14.9 s, Codex r1 COLLATOR-1). If NFD(`s`) has a run of more than 30 non-starters, the
/// result is NFD(`s`) with two CGJs before each non-starter that has 30 non-starters right
/// before it, so the collator sees runs of at most 30 and the cost is linear. Else `s` is
/// returned as it is (no copy), so such text gets the same order as before. The test and
/// the result depend only on NFD(`s`), so canonically equivalent strings still compare
/// equal (ECMA-262 `localeCompare`); text with a longer run gets the order of its
/// stream-safe form. Code units below U+00C0 are starters with no decomposition (no
/// lookup); a lone surrogate counts as U+FFFD, as in the collator.
fn stream_safe(s: &[u16]) -> Cow<'_, [u16]> {
    let mut run = 0usize;
    let long = char::decode_utf16(s.iter().copied()).any(|c| match c {
        Ok(c) if u32::from(c) >= 0xC0 => nfd_run(c, &mut run),
        _ => {
            run = 0;
            false
        }
    });
    if !long {
        return Cow::Borrowed(s);
    }
    let nfd = NFD.normalize_utf16(s);
    let mut out: Vec<u16> = Vec::with_capacity(nfd.len() + nfd.len() / MAX_NON_STARTERS + 1);
    let mut run = 0usize;
    let mut buf = [0u16; 2];
    for c in char::decode_utf16(nfd.iter().copied()) {
        let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
        if CCC.get_u8(c) == 0 {
            run = 0;
        } else {
            if run == MAX_NON_STARTERS {
                out.extend_from_slice(&[CGJ, CGJ]);
                run = 0;
            }
            run += 1;
        }
        out.extend_from_slice(c.encode_utf16(&mut buf));
    }
    Cow::Owned(out)
}

/// The ICU4X compare of the stream-safe forms of `x` and `y` ([`stream_safe`]).
fn icu_compare(c: &CollatorBorrowed<'static>, x: &[u16], y: &[u16]) -> Ordering {
    c.compare_utf16(&stream_safe(x), &stream_safe(y))
}

/// CompareStrings ( collator, x, y ) (10.3.3.2) of the UTF-16 strings `x` and `y`.
/// Canonically equivalent strings compare as equal (the ICU4X collator normalizes); text
/// whose NFD has a run of more than 30 non-starters is compared in its stream-safe form
/// (`stream_safe`).
pub fn compare_strings(locale: &str, options: Packed, x: &[u16], y: &[u16]) -> Ordering {
    match collator_for(locale, options) {
        Some(c) => icu_compare(&c, x, y),
        // No data at all (not the case with the compiled data): the code-unit order.
        None => x.cmp(y),
    }
}

/// -1, 0 or 1 for `o` (the result of a compare function).
fn ordering_value(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// String.prototype.localeCompare ( that [ , locales [ , options ] ] ) (§19.1.1) steps 4-5
/// for `s` and `that` (the code units of steps 2-3): CompareStrings with a collator made as
/// Construct(%Intl.Collator%, « locales, options ») makes it. The construct is not
/// observable, so no JS object is made; with `locales` and `options` undefined the
/// resolution is the same each time (its option reads see a new null-prototype object), and
/// the default collator of the cache serves it.
pub fn locale_compare<'s>(
    cx: &mut Cx<'s>,
    s: &[u16],
    that: &[u16],
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Ordering> {
    if locales.is_undefined() && options.is_undefined() {
        let c = cache::get_or_make_default(Service::Collator, || {
            let r = default_resolved(&resolve::default_locale(cx));
            make_collator(&r.locale, r.options).or_else(|| make_collator("und", r.options))
        });
        if let Some(c) = c {
            return Ok(icu_compare(&c, s, that));
        }
    }
    let r = initialize(cx, locales, options)?;
    Ok(compare_strings(&r.locale, r.options, s, that))
}

/// The resolution of InitializeCollator with `locales` and `options` undefined and the
/// default locale `locale` (an available locale, no extension).
fn default_resolved(locale: &str) -> Resolved {
    let case_first = CASE_FIRSTS
        .iter()
        .position(|c| *c == default_case_first(locale))
        .unwrap_or(0) as u32;
    Resolved {
        locale: locale.to_string(),
        options: Packed::new(
            false,
            3,
            default_ignore_punctuation(locale),
            false,
            case_first,
            0,
        ),
    }
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.Collator ( [ locales [ , options ] ] ) (10.1.1). NewTarget undefined is allowed (step
/// 1 uses the active function object).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, let newTarget be the active function object, else let
    //    newTarget be NewTarget.
    let nt = match new_target.as_object() {
        Some(nt) => nt,
        None => args.callee().into(),
    };
    // 2-3. Let collator be ? OrdinaryCreateFromConstructor(newTarget,
    //      "%Intl.Collator.prototype%", internalSlotsList).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlCollatorPrototype)?;
    let collator = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 4-32. InitializeCollator.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(collator, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(
        collator,
        CLASS,
        SLOT_OPTIONS,
        Local::int32(r.options.0 as i32),
    )?;
    // 33. Return collator.
    Ok(collator.into())
}

/// Intl.Collator.supportedLocalesOf ( locales [ , options ] ) (10.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.Collator%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The `[[Locale]]` text and the packed options of the collator `o` (TypeError if `o` is not
/// an Intl.Collator: RequireInternalSlot(collator, `[[InitializedCollator]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<(String, Packed)> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let options = cx.slot(o, CLASS, SLOT_OPTIONS)?;
    let locale = cx.to_rust_string(locale)?;
    Ok((locale, Packed(options.as_int32().unwrap_or(0) as u32)))
}

/// get Intl.Collator.prototype.compare (10.3.3).
fn get_compare<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let collator be the this value; ? RequireInternalSlot(collator,
    //      [[InitializedCollator]]).
    let bound = cx.slot(this, CLASS, SLOT_BOUND_COMPARE)?;
    // 3. If collator.[[BoundCompare]] is undefined, then
    if bound.is_undefined() {
        // a-b. Let F be a new built-in function object as defined in 10.3.3.1 (length 2,
        //      name ""), with F.[[Collator]] collator (the closure data).
        let f = cx.new_builtin_closure(compare_function, "", 2, this);
        // c. Set collator.[[BoundCompare]] to F.
        cx.set_slot(this, CLASS, SLOT_BOUND_COMPARE, f)?;
        return Ok(f.into());
    }
    // 4. Return collator.[[BoundCompare]].
    Ok(bound)
}

/// A Collator compare function (10.3.3.1).
fn compare_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let collator be F.[[Collator]] (an Intl.Collator).
    let collator = cx.closure_data(args.callee());
    // 3-6. Let X be ? ToString(x); let Y be ? ToString(y).
    let x = cx.to_string(args.get(0))?;
    let y = cx.to_string(args.get(1))?;
    let x = units(cx, x)?;
    let y = units(cx, y)?;
    // 7. Return CompareStrings(collator, X, Y).
    let (locale, options) = read(cx, collator)?;
    Ok(Local::int32(ordering_value(compare_strings(
        &locale, options, &x, &y,
    ))))
}

/// The code units of the string `s`, copied out (a view cannot live across a GC point).
fn units<'s>(cx: &mut Cx<'s>, s: Local<'s, ep9_vm::JsString>) -> JsResult<Vec<u16>> {
    let len = cx.string_len(s);
    let flat = cx.flatten(s)?;
    Ok((0..len)
        .map(|i| cx.code_unit_at(flat, i).unwrap_or(0))
        .collect())
}

/// Intl.Collator.prototype.resolvedOptions ( ) (10.3.4).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let collator be the this value; ? RequireInternalSlot(collator,
    //      [[InitializedCollator]]).
    let (locale, p) = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 4, in table order: CreateDataPropertyOrThrow(options, p, v).
    let v = cx.new_string(&locale)?;
    cx.create_data_property_or_throw(o, "locale", v)?;
    for (key, value) in [("usage", p.usage()), ("sensitivity", p.sensitivity())] {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)?;
    }
    cx.create_data_property_or_throw(
        o,
        "ignorePunctuation",
        Local::boolean(p.ignore_punctuation()),
    )?;
    let v = cx.new_string(p.collation())?;
    cx.create_data_property_or_throw(o, "collation", v)?;
    cx.create_data_property_or_throw(o, "numeric", Local::boolean(p.numeric()))?;
    let v = cx.new_string(p.case_first())?;
    cx.create_data_property_or_throw(o, "caseFirst", v)?;
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::available;
    use icu_collator::options::{AlternateHandling, CaseLevel, Strength};

    /// The `kf` and `ignorePunctuation` defaults ([`UPPER_FIRST`], [`SHIFTED`]) equal the
    /// ICU4X data defaults for every available locale, and no locale has another default
    /// strength or case level (the constructor sets both from `sensitivity`).
    #[test]
    fn locale_defaults_match_the_icu4x_data() {
        let mut bad = Vec::new();
        for l in available::locales() {
            let prefs = CollatorPreferences::from(&icu_locale(l));
            let c = icu_collator::Collator::try_new(prefs, CollatorOptions::default()).unwrap();
            let r = c.resolved_options();
            let case_first = match r.case_first {
                CollationCaseFirst::Upper => "upper",
                CollationCaseFirst::Lower => "lower",
                _ => "false",
            };
            let shifted = r.alternate_handling == AlternateHandling::Shifted;
            if case_first != default_case_first(l)
                || shifted != default_ignore_punctuation(l)
                || r.strength != Strength::Tertiary
                || r.case_level != CaseLevel::Off
            {
                bad.push(l.as_str());
            }
        }
        assert!(bad.is_empty(), "{bad:?}");
        assert!(available::locales().len() > 700);
    }

    /// The collation types per locale equal node v26.13.0's
    /// `new Intl.Locale(l).getCollations()`, except "searchjl" of "ko" (the ICU4X baked data
    /// has no search tailorings, INTL-I1 §5.3 row 12) and "phonetic" of "ln" (the compiled
    /// data is CLDR "recommended", which has no "ln" tailoring; INTL-I1-D4).
    #[test]
    fn collations_per_locale_match_node() {
        for (l, want) in [
            ("en", "emoji,eor"),
            ("und", "emoji,eor"),
            ("de", "emoji,eor,phonebk"),
            ("de-CH", "emoji,eor,phonebk"),
            ("de-AT", "emoji,eor,phonebk"),
            ("zh", "emoji,eor,pinyin,stroke,unihan,zhuyin"),
            ("zh-Hant", "emoji,eor,pinyin,stroke,unihan,zhuyin"),
            ("ja", "emoji,eor,unihan"),
            ("ko", "emoji,eor,unihan"),
            ("es", "emoji,eor,trad"),
            ("ar", "compat,emoji,eor"),
            ("si", "dict,emoji,eor"),
            ("sv", "emoji,eor,trad"),
            ("fi", "emoji,eor,trad"),
            ("ln", "emoji,eor"),
            ("bn", "emoji,eor,trad"),
            ("hi", "emoji,eor"),
            ("fr", "emoji,eor"),
        ] {
            assert_eq!(collations_of(l).join(","), want, "{l}");
        }
    }

    /// Each value of `supportedValuesOf("collation")` (`available::COLLATIONS`) is a
    /// collation type of the data for a locale of the test262 list
    /// (`collations-accepted-by-Collator.js`), and no other type is (the INTL W1 fixer
    /// dropped "phonetic" and "searchjl" from the list: the compiled data has no tailoring
    /// for them; see `collations_per_locale_match_node`).
    #[test]
    fn supported_collations_are_the_data_types() {
        let locales = ["en", "ar", "de", "es", "hi", "ko", "ln", "si", "sv", "zh"];
        let mut in_data: Vec<&str> = locales.iter().flat_map(|l| collations_of(l)).collect();
        in_data.sort_unstable();
        in_data.dedup();
        assert_eq!(in_data, available::COLLATIONS);
    }

    #[test]
    fn packed_options_round_trip() {
        for search in [false, true] {
            for s in 0..4 {
                for ip in [false, true] {
                    for n in [false, true] {
                        for cf in 0..3 {
                            for co in 0..=COLLATION_TYPES.len() as u32 {
                                let p = Packed::new(search, s, ip, n, cf, co);
                                assert_eq!(p.usage(), if search { "search" } else { "sort" });
                                assert_eq!(p.sensitivity(), SENSITIVITIES[s as usize]);
                                assert_eq!(p.ignore_punctuation(), ip);
                                assert_eq!(p.numeric(), n);
                                assert_eq!(p.case_first(), CASE_FIRSTS[cf as usize]);
                                let want = if co == 0 {
                                    "default"
                                } else {
                                    COLLATION_TYPES[co as usize - 1]
                                };
                                assert_eq!(p.collation(), want);
                                assert_eq!(p.key() & 1, 0);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unicode_type_values() {
        for ok in ["phonebk", "abc", "a1b2c3d4", "abc-defgh", "PHONEBK"] {
            assert!(is_unicode_type(ok), "{ok}");
        }
        for bad in [
            "",
            "ab",
            "abcdefghi",
            "abc-",
            "-abc",
            "abc--def",
            "ab_cd",
            "a\u{e9}cd",
        ] {
            assert!(!is_unicode_type(bad), "{bad}");
        }
    }

    /// The ICU4X collator follows the options: sensitivity, ignorePunctuation, numeric,
    /// caseFirst and a collation type (the node v26.13.0 results of the same compares).
    #[test]
    fn compare_strings_follows_the_options() {
        let u = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let cmp = |locale: &str, p: Packed, a: &str, b: &str| {
            ordering_value(compare_strings(locale, p, &u(a), &u(b)))
        };
        let variant = Packed::new(false, 3, false, false, 0, 0);
        assert_eq!(cmp("en", variant, "a", "b"), -1);
        assert_eq!(cmp("en", variant, "a", "A"), -1);
        assert_eq!(cmp("en", variant, "\u{e4}", "a\u{308}"), 0);
        assert_eq!(
            cmp(
                "en",
                Packed::new(false, 0, false, false, 0, 0),
                "a",
                "\u{c1}"
            ),
            0
        );
        assert_eq!(
            cmp("en", Packed::new(false, 1, false, false, 0, 0), "a", "A"),
            0
        );
        assert_eq!(
            cmp(
                "en",
                Packed::new(false, 1, false, false, 0, 0),
                "a",
                "\u{e1}"
            ),
            -1
        );
        assert_eq!(
            cmp(
                "en",
                Packed::new(false, 2, false, false, 0, 0),
                "a",
                "\u{e1}"
            ),
            0
        );
        assert_eq!(
            cmp("en", Packed::new(false, 2, false, false, 0, 0), "a", "A"),
            -1
        );
        assert_eq!(cmp("en", variant, "", " "), -1);
        assert_eq!(
            cmp("en", Packed::new(false, 3, true, false, 0, 0), "", " "),
            0
        );
        assert_eq!(cmp("en", variant, "2", "10"), 1);
        assert_eq!(
            cmp("en", Packed::new(false, 3, false, true, 0, 0), "2", "10"),
            -1
        );
        assert_eq!(
            cmp("en", Packed::new(false, 3, false, false, 2, 0), "a", "A"),
            1
        );
        // de phonebk (co index 5): "\u{e4}" sorts as "ae", so it comes before "af".
        let phonebk = COLLATION_TYPES
            .iter()
            .position(|c| *c == "phonebk")
            .unwrap() as u32
            + 1;
        assert_eq!(
            cmp(
                "de",
                Packed::new(false, 3, false, false, 0, phonebk),
                "\u{e4}",
                "ab"
            ),
            1
        );
        assert_eq!(cmp("de", variant, "\u{e4}", "ab"), -1);
        assert_eq!(
            cmp(
                "de-u-co-phonebk",
                Packed::new(false, 3, false, false, 0, phonebk),
                "\u{e4}",
                "ab"
            ),
            1
        );
        assert!(cache::len(Service::Collator) > 0);
    }
}
