//! Intl.Locale (ECMA-402 §14; block 30): `%Intl.Locale%` and `%Intl.Locale.prototype%`, a
//! module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and its
//! own intrinsics.
//!
//! ```text
//! Intl.Locale {w, c}              %Intl.Locale%            prototype {}
//! Intl.Locale.prototype           %Intl.Locale.prototype%  constructor {w, c},
//!                                                          [@@toStringTag] "Intl.Locale" {c},
//!                                                          the getters and methods of §14.3
//! slots (ObjectClass::IntlLocale, 8; each a String or undefined, [[Numeric]] a Boolean):
//!   0 [[Locale]]  1 [[Calendar]]  2 [[Collation]]  3 [[FirstDayOfWeek]]  4 [[HourCycle]]
//!   5 [[CaseFirst]]  6 [[Numeric]]  7 [[NumberingSystem]]
//!
//! new Intl.Locale("EN-latn-us", { region: "GB", calendar: "gregory" })
//!   tag ── IsStructurallyValidLanguageTag, CanonicalizeUnicodeLocaleId ─> "en-Latn-US"
//!       ── UpdateLanguageId (language, script, region, variants) ───────> "en-Latn-GB"
//!       ── the -u- options (ca co fw hc kf kn nu), MakeLocaleRecord ─────> "en-Latn-GB-u-ca-gregory"
//! ```
//!
//! The Locale info methods (`getCalendars` .. `getWeekInfo`) are in `locale_info.rs`; their ids
//! are in this block. Ids are append-only: definition `i` of [`DEFS`] is
//! `BuiltinId::new(BLOCK, i)`, pinned by `crates/ep9_builtins/tests/golden/ids-locale.txt`.

use std::collections::HashSet;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};
use icu_locale::LocaleExpander;
use icu_locale_core::LanguageIdentifier;

use crate::locale_info;
use crate::options::{coerce_options_to_object, get_boolean_option, get_string_option};
use crate::tag::{self, TagError, canonicalize_tag, canonicalize_unicode_value};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 30;

/// The class of an Intl.Locale object.
pub const CLASS: ObjectClass = ObjectClass::IntlLocale;

/// Slot 0: `[[Locale]]`, the canonical language tag (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slots 1-5 and 7: `[[Calendar]]`, `[[Collation]]`, `[[FirstDayOfWeek]]`, `[[HourCycle]]`,
/// `[[CaseFirst]]` and `[[NumberingSystem]]` (a String or undefined), in [`KEYS`] order with
/// `kn` at slot 6.
pub const SLOT_CALENDAR: usize = 1;
pub const SLOT_COLLATION: usize = 2;
pub const SLOT_FIRST_DAY_OF_WEEK: usize = 3;
pub const SLOT_HOUR_CYCLE: usize = 4;
pub const SLOT_CASE_FIRST: usize = 5;
/// Slot 6: `[[Numeric]]` (a Boolean).
pub const SLOT_NUMERIC: usize = 6;
pub const SLOT_NUMBERING_SYSTEM: usize = 7;
const SLOTS: usize = 8;

/// `%Intl.Locale%.[[LocaleExtensionKeys]]` (14.2.2, with "fw" of the Locale info proposal), in
/// the order of the slots 1-7.
pub const KEYS: [&str; 7] = ["ca", "co", "fw", "hc", "kf", "kn", "nu"];

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Locale", 1, constructor), // 0 (14.1.1)
    BuiltinDef::getter("baseName", get_base_name),     // 1 (14.3.3)
    BuiltinDef::getter("calendar", get_calendar),      // 2 (14.3.4)
    BuiltinDef::getter("caseFirst", get_case_first),   // 3 (14.3.5)
    BuiltinDef::getter("collation", get_collation),    // 4 (14.3.6)
    BuiltinDef::getter("firstDayOfWeek", get_first_day_of_week), // 5 (Locale info)
    BuiltinDef::getter("hourCycle", get_hour_cycle),   // 6 (14.3.7)
    BuiltinDef::getter("numeric", get_numeric),        // 7 (14.3.12)
    BuiltinDef::getter("numberingSystem", get_numbering_system), // 8 (14.3.11)
    BuiltinDef::getter("language", get_language),      // 9 (14.3.8)
    BuiltinDef::getter("script", get_script),          // 10 (14.3.14)
    BuiltinDef::getter("region", get_region),          // 11 (14.3.13)
    BuiltinDef::getter("variants", get_variants),      // 12 (14.3.17)
    BuiltinDef::method("maximize", 0, maximize),       // 13 (14.3.9)
    BuiltinDef::method("minimize", 0, minimize),       // 14 (14.3.10)
    BuiltinDef::method("toString", 0, to_string),      // 15 (14.3.15)
    BuiltinDef::method("getCalendars", 0, locale_info::get_calendars), // 16
    BuiltinDef::method("getCollations", 0, locale_info::get_collations), // 17
    BuiltinDef::method("getHourCycles", 0, locale_info::get_hour_cycles), // 18
    BuiltinDef::method("getNumberingSystems", 0, locale_info::get_numbering_systems), // 19
    BuiltinDef::method("getTimeZones", 0, locale_info::get_time_zones), // 20
    BuiltinDef::method("getTextInfo", 0, locale_info::get_text_info), // 21
    BuiltinDef::method("getWeekInfo", 0, locale_info::get_week_info), // 22
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlLocale,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlLocalePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlLocale,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::IntlLocalePrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::IntlLocalePrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlLocale,
                PropAttrs::METHOD,
            ),
            // Intl.Locale.prototype [ @@toStringTag ] ({c}, 14.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Locale"),
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::accessor(PropKey::Str("baseName"), Some(1), None),
            PropDef::accessor(PropKey::Str("calendar"), Some(2), None),
            PropDef::accessor(PropKey::Str("caseFirst"), Some(3), None),
            PropDef::accessor(PropKey::Str("collation"), Some(4), None),
            PropDef::accessor(PropKey::Str("firstDayOfWeek"), Some(5), None),
            PropDef::accessor(PropKey::Str("hourCycle"), Some(6), None),
            PropDef::accessor(PropKey::Str("numeric"), Some(7), None),
            PropDef::accessor(PropKey::Str("numberingSystem"), Some(8), None),
            PropDef::accessor(PropKey::Str("language"), Some(9), None),
            PropDef::accessor(PropKey::Str("script"), Some(10), None),
            PropDef::accessor(PropKey::Str("region"), Some(11), None),
            PropDef::accessor(PropKey::Str("variants"), Some(12), None),
            PropDef::method(PropKey::Str("maximize"), 13),
            PropDef::method(PropKey::Str("minimize"), 14),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 15),
            PropDef::method(PropKey::Str("getCalendars"), 16),
            PropDef::method(PropKey::Str("getCollations"), 17),
            PropDef::method(PropKey::Str("getHourCycles"), 18),
            PropDef::method(PropKey::Str("getNumberingSystems"), 19),
            PropDef::method(PropKey::Str("getTimeZones"), 20),
            PropDef::method(PropKey::Str("getTextInfo"), 21),
            PropDef::method(PropKey::Str("getWeekInfo"), 22),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Locale"),
            IntrinsicId::IntlLocale,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "locale",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- tags (no Cx) ---------------------------------------------------------------------------

/// The `unicode_language_id` parts of a canonical tag, with their case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseName {
    pub language: String,
    pub script: Option<String>,
    pub region: Option<String>,
    pub variants: Vec<String>,
}

impl BaseName {
    /// The base name of the structurally valid tag `tag` (GetLocaleBaseName and its parts:
    /// GetLocaleLanguage, GetLocaleScript, GetLocaleRegion, GetLocaleVariants), and the
    /// suffix after it (the extensions and the private use part, with the leading "-").
    pub fn split(tag: &str) -> (BaseName, &str) {
        let mut b = BaseName::default();
        let mut end = 0;
        for (i, part) in tag.split('-').enumerate() {
            if i == 0 {
                b.language = part.to_string();
            } else if part.len() == 1 {
                break;
            } else if i == 1 && tag::is_script_subtag(part) {
                b.script = Some(part.to_string());
            } else if b.region.is_none() && b.variants.is_empty() && tag::is_region_subtag(part) {
                b.region = Some(part.to_string());
            } else {
                b.variants.push(part.to_string());
            }
            end += part.len() + usize::from(i > 0);
        }
        (b, &tag[end..])
    }

    /// The tag text: `language[-Script][-REGION][-variant]...`, with the canonical case of the
    /// script (title) and the region (upper). The tag code keeps a language that ICU4X cannot
    /// read (5-8 letters) in lower case, so this sets the case again.
    pub fn text(&self) -> String {
        let mut s = self.language.to_ascii_lowercase();
        if let Some(script) = &self.script {
            s.push('-');
            let mut chars = script.chars();
            if let Some(c) = chars.next() {
                s.push(c.to_ascii_uppercase());
                s.extend(chars.map(|c| c.to_ascii_lowercase()));
            }
        }
        if let Some(region) = &self.region {
            s.push('-');
            s.push_str(&region.to_ascii_uppercase());
        }
        for v in &self.variants {
            s.push('-');
            s.push_str(&v.to_ascii_lowercase());
        }
        s
    }
}

/// The canonical form of the structurally valid tag `tag` with the case of its base name set
/// (see [`BaseName::text`]), or the [`TagError`] of `tag::parse` (not structurally valid, or
/// over the tag length limit).
pub fn canonical_locale(tag: &str) -> Result<String, TagError> {
    let c = canonicalize_tag(tag)?;
    let (base, rest) = BaseName::split(&c);
    Ok(format!("{}{rest}", base.text()))
}

/// Matches the `type` nonterminal of UTS 35 (`alphanum{3,8} (sep alphanum{3,8})*`).
pub fn is_unicode_type(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|p| (3..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// WeekdayToString ( fw ): "0" and "7" give "sun", "1" .. "6" give "mon" .. "sat"; any other
/// value stays.
pub fn weekday_to_string(fw: &str) -> String {
    const DAYS: [&str; 8] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    match fw.as_bytes() {
        [d @ b'0'..=b'7'] => DAYS[usize::from(d - b'0')].to_string(),
        _ => fw.to_string(),
    }
}

/// The attributes and the keywords (key, value; value "" for none) of the Unicode locale
/// extension of the canonical tag `tag`, in tag order (UnicodeExtensionComponents).
fn unicode_components(tag: &str) -> (Vec<String>, Vec<(String, String)>) {
    let mut attributes = Vec::new();
    let mut keywords: Vec<(String, String)> = Vec::new();
    let Some(p) = tag::parse_canonical(tag) else {
        return (attributes, keywords);
    };
    let Some((_, subtags)) = p.extensions.iter().find(|(s, _)| *s == b'u') else {
        return (attributes, keywords);
    };
    let mut i = 0;
    while i < subtags.len() && subtags[i].len() != 2 {
        attributes.push(subtags[i].clone());
        i += 1;
    }
    while i < subtags.len() {
        let key = subtags[i].clone();
        i += 1;
        let start = i;
        while i < subtags.len() && subtags[i].len() != 2 {
            i += 1;
        }
        keywords.push((key, subtags[start..i].join("-")));
    }
    (attributes, keywords)
}

/// The result of MakeLocaleRecord: `[[Locale]]` and the value of each key of [`KEYS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleRecord {
    pub locale: String,
    pub keys: [Option<String>; 7],
}

/// MakeLocaleRecord ( tag, options, localeExtensionKeys ) (14.1.2) for the canonical tag
/// `tag` and the option value of each key of [`KEYS`] (`None`: undefined), then the canonical
/// form of the result (InsertUnicodeExtensionAndCanonicalize). An error if the result is not
/// structurally valid (it does not happen for values that match the `type` nonterminal) or
/// is over the tag length limit (a long option value).
pub fn make_locale_record(
    tag: &str,
    options: &[Option<String>; 7],
) -> Result<LocaleRecord, TagError> {
    let (attributes, mut keywords) = unicode_components(tag);
    for (i, key) in KEYS.iter().enumerate() {
        // a-b. The value of the keyword `key` in the tag, if there is one (read back from the
        //      canonical tag below).
        // d-e. An option value overrides it: CanonicalizeUValue(key, overrideValue), the UTS 35
        //      canonical form, where "true" is the empty value.
        if let Some(ov) = &options[i] {
            let mut v = canonicalize_unicode_value(key, &ov.to_ascii_lowercase());
            if v == "true" {
                v.clear();
            }
            match keywords.iter_mut().find(|(k, _)| k == key) {
                Some(entry) => entry.1 = v,
                None => keywords.push(((*key).to_string(), v)),
            }
        }
    }
    // 5. The tag without its Unicode locale extension sequence.
    let mut locale = tag::remove_unicode_extension(tag);
    // 6. InsertUnicodeExtensionAndCanonicalize(locale, attributes, keywords).
    if !attributes.is_empty() || !keywords.is_empty() {
        let mut ext = String::from("-u");
        for a in &attributes {
            ext.push('-');
            ext.push_str(a);
        }
        for (k, v) in &keywords {
            ext.push('-');
            ext.push_str(k);
            if !v.is_empty() {
                ext.push('-');
                ext.push_str(v);
            }
        }
        locale = match locale.find("-x-") {
            Some(i) => format!("{}{ext}{}", &locale[..i], &locale[i..]),
            None => format!("{locale}{ext}"),
        };
    }
    let locale = canonical_locale(&locale)?;
    // The canonical form can differ from the keywords above: ICU4X drops each "true" subtag
    // of a type value ("gregory-true" -> "gregory"; icu_locale_core extensions/unicode/
    // value.rs). So the value of each key comes from the canonical tag, and toString() and
    // the getters agree (Codex r1 LOCALE-2; node keeps "gregory-true" in both: route
    // INTL-LOCALE-TRUE).
    let (_, canonical) = unicode_components(&locale);
    let keys = KEYS.map(|key| {
        canonical
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    });
    Ok(LocaleRecord { locale, keys })
}

/// The ICU4X language id of `base` (language, script, region; no variants), or `None` if
/// ICU4X cannot read the language (a 5-8 letter language subtag).
fn icu_language_id(base: &BaseName) -> Option<LanguageIdentifier> {
    let mut s = base.language.clone();
    for part in base.script.iter().chain(&base.region) {
        s.push('-');
        s.push_str(part);
    }
    LanguageIdentifier::try_from_str(&s).ok()
}

/// The likely-subtags data (CLDR, compiled; extended set: every language of the data).
const EXPANDER: LocaleExpander = LocaleExpander::new_extended();

/// Add Likely Subtags (UTS 35 §4.4) on the language, script and region of `base`; the variants
/// stay. ICU4X does not use the "und" -> "en-Latn-US" row of the CLDR table at the end of its
/// lookups (it leaves "und", "und-150" as they are), so that row is applied here: an
/// undetermined language becomes "en", a missing script "Latn", a missing region "US". A
/// language with no data (or one that ICU4X cannot read) stays as it is.
pub fn add_likely_subtags(base: &BaseName) -> BaseName {
    let Some(mut id) = icu_language_id(base) else {
        return base.clone();
    };
    EXPANDER.maximize(&mut id);
    let mut out = BaseName {
        language: id.language.as_str().to_string(),
        script: id.script.map(|s| s.as_str().to_string()),
        region: id.region.map(|r| r.as_str().to_string()),
        variants: base.variants.clone(),
    };
    if out.language == "und" {
        out.language = "en".to_string();
        out.script.get_or_insert_with(|| "Latn".to_string());
        out.region.get_or_insert_with(|| "US".to_string());
    }
    out
}

/// Remove Likely Subtags (UTS 35 §4.4, favoring the region): the shortest of `language`,
/// `language-region`, `language-script` whose maximized form is the maximized form of `base`;
/// else that maximized form. The variants stay.
pub fn remove_likely_subtags(base: &BaseName) -> BaseName {
    let max = add_likely_subtags(base);
    let bare = |script: bool, region: bool| BaseName {
        language: max.language.clone(),
        script: if script { max.script.clone() } else { None },
        region: if region { max.region.clone() } else { None },
        variants: Vec::new(),
    };
    for (script, region) in [(false, false), (false, true), (true, false)] {
        let trial = bare(script, region);
        let m = add_likely_subtags(&trial);
        if m.language == max.language && m.script == max.script && m.region == max.region {
            return BaseName {
                variants: base.variants.clone(),
                ..trial
            };
        }
    }
    max
}

// ---- objects --------------------------------------------------------------------------------

/// Makes an Intl.Locale object with prototype `proto` from a record (steps 35-43 of 14.1.1).
fn create<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    r: &LocaleRecord,
) -> JsResult<Local<'s>> {
    let o = cx.new_object_with_slots(proto, CLASS, SLOTS);
    let s = cx.new_string(&r.locale)?;
    cx.set_slot(o, CLASS, SLOT_LOCALE, s)?;
    for (i, value) in r.keys.iter().enumerate() {
        let slot = i + 1;
        let v: Local<'s> = if slot == SLOT_NUMERIC {
            // [[Numeric]]: SameValue(r.[[kn]], "true") or r.[[kn]] is the empty String.
            Local::boolean(matches!(value.as_deref(), Some("true" | "")))
        } else {
            match value {
                Some(v) => cx.new_string(v)?.into(),
                None => Local::undefined(),
            }
        };
        cx.set_slot(o, CLASS, slot, v)?;
    }
    Ok(o.value())
}

/// A new Intl.Locale object (prototype `%Intl.Locale.prototype%` of the current realm) for the
/// canonical tag `tag`: Construct(%Intl.Locale%, « tag ») with no options.
fn create_from_tag<'s>(cx: &mut Cx<'s>, tag: &str) -> JsResult<Local<'s>> {
    let r = make_locale_record(tag, &Default::default())
        .map_err(|e| cx.throw_range_error(&e.message(tag)))?;
    let proto = cx.intrinsic(IntrinsicId::IntlLocalePrototype);
    create(cx, proto, &r)
}

/// `this.[[Locale]]` (`RequireInternalSlot(loc, [[InitializedLocale]])`: a TypeError for any
/// other value).
pub fn this_locale<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<String> {
    let v = cx.slot(this, CLASS, SLOT_LOCALE)?;
    cx.to_rust_string(v)
}

/// The base name of `this.[[Locale]]` (after RequireInternalSlot).
pub fn this_base_name<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<BaseName> {
    let tag = this_locale(cx, this)?;
    Ok(BaseName::split(&tag).0)
}

/// A RangeError for the option `property` with the value `value`.
fn option_error(cx: &mut Cx<'_>, property: &str, value: &str) -> ep9_vm::Throw {
    cx.throw_range_error(&format!(
        "Value {value} out of range for Intl.Locale options property {property}"
    ))
}

/// Reads the string option `property` and checks it with `valid` (a RangeError if it does not
/// match).
fn subtag_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
    valid: fn(&str) -> bool,
) -> JsResult<Option<String>> {
    let v = get_string_option(cx, options, property, &[], None)?;
    if let Some(s) = &v
        && !valid(s)
    {
        return Err(option_error(cx, property, s));
    }
    Ok(v)
}

/// The variants option check (UpdateLanguageId step 9): not empty, each subtag a
/// `unicode_variant_subtag`, no repeat (after ASCII lower case).
/// The repeat check is a set, so n subtags cost O(n), not O(n^2).
fn is_variants_option(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let mut seen: HashSet<&str> = HashSet::new();
    !s.is_empty()
        && lower
            .split('-')
            .all(|v| tag::is_variant_subtag(v) && seen.insert(v))
}

/// UpdateLanguageId ( tag, options ) (14.1.3) on the canonical tag `tag`, then the canonical
/// form of the new tag.
fn update_language_id<'s>(
    cx: &mut Cx<'s>,
    tag: &str,
    options: Local<'s, JsObject>,
) -> JsResult<String> {
    // 1. Let baseName be GetLocaleBaseName(tag).
    let (base, extensions) = BaseName::split(tag);
    // 2-3. language: a unicode_language_subtag.
    let language = subtag_option(cx, options, "language", tag::is_language_subtag)?;
    // 4-5. script: a unicode_script_subtag.
    let script = subtag_option(cx, options, "script", tag::is_script_subtag)?;
    // 6-7. region: a unicode_region_subtag.
    let region = subtag_option(cx, options, "region", tag::is_region_subtag)?;
    // 8-9. variants: unicode_variant_subtags, no repeat.
    let variants = subtag_option(cx, options, "variants", is_variants_option)?;
    // 10-15. The new tag: the parts, each from the option or else from baseName, then the
    //        extensions of tag.
    let new_base = BaseName {
        language: language.unwrap_or(base.language),
        script: script.or(base.script),
        region: region.or(base.region),
        variants: match variants {
            Some(v) => v.split('-').map(str::to_string).collect(),
            None => base.variants,
        },
    };
    let new_tag = format!("{}{extensions}", new_base.text());
    canonical_locale(&new_tag).map_err(|e| cx.throw_range_error(&e.message(&new_tag)))
}

/// A Unicode extension option (GetOption string, then the `type` check): calendar,
/// collation, numberingSystem.
fn type_option<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    property: &str,
) -> JsResult<Option<String>> {
    subtag_option(cx, options, property, is_unicode_type)
}

/// Intl.Locale ( tag [ , options ] ) (14.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.Locale requires 'new'"));
    };
    // 6. Let locale be ? OrdinaryCreateFromConstructor(NewTarget, "%Intl.Locale.prototype%",
    //    internalSlotsList): the prototype now, the object at the end.
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlLocalePrototype)?;
    let tag = args.get(0);
    // 7. If tag is not a String and tag is not an Object, throw a TypeError exception.
    if !tag.is_string() && tag.as_object().is_none() {
        return Err(cx.throw_type_error(
            "First argument to Intl.Locale constructor can't be empty or missing",
        ));
    }
    // 8-9. An Intl.Locale gives its [[Locale]]; else ? ToString(tag).
    let tag = if cx.object_class(tag) == Some(CLASS) {
        let t = cx.slot(tag, CLASS, SLOT_LOCALE)?;
        cx.to_rust_string(t)?
    } else {
        cx.to_rust_string(tag)?
    };
    // 10. Set options to ? CoerceOptionsToObject(options).
    let options = coerce_options_to_object(cx, args.get(1))?;
    // 11-12. IsStructurallyValidLanguageTag, CanonicalizeUnicodeLocaleId.
    let tag = canonical_locale(&tag).map_err(|e| cx.throw_range_error(&e.message(&tag)))?;
    // 13. Set tag to ? UpdateLanguageId(tag, options).
    let tag = update_language_id(cx, &tag, options)?;
    // 14-33. The Unicode extension options, in KEYS order of reading:
    //        calendar, collation, firstDayOfWeek, hourCycle, caseFirst, numeric,
    //        numberingSystem.
    let mut opt: [Option<String>; 7] = Default::default();
    opt[0] = type_option(cx, options, "calendar")?;
    opt[1] = type_option(cx, options, "collation")?;
    // firstDayOfWeek: WeekdayToString, then the type check.
    if let Some(fw) = get_string_option(cx, options, "firstDayOfWeek", &[], None)? {
        let fw = weekday_to_string(&fw);
        if !is_unicode_type(&fw) {
            return Err(option_error(cx, "firstDayOfWeek", &fw));
        }
        opt[2] = Some(fw);
    }
    opt[3] = get_string_option(
        cx,
        options,
        "hourCycle",
        &["h11", "h12", "h23", "h24"],
        None,
    )?;
    opt[4] = get_string_option(cx, options, "caseFirst", &["upper", "lower", "false"], None)?;
    // numeric: GetOption boolean; not undefined: ! ToString(kn).
    opt[5] = get_boolean_option(cx, options, "numeric", None)?.map(|b| b.to_string());
    opt[6] = type_option(cx, options, "numberingSystem")?;
    // 34. Let r be MakeLocaleRecord(tag, opt, localeExtensionKeys).
    let r = make_locale_record(&tag, &opt).map_err(|e| cx.throw_range_error(&e.message(&tag)))?;
    // 35-44. The slots; return locale.
    create(cx, proto, &r)
}

// ---- getters and methods (§14.3) ------------------------------------------------------------

/// A String or undefined.
fn opt_string<'s>(cx: &mut Cx<'s>, s: Option<&str>) -> JsResult<Local<'s>> {
    match s {
        Some(s) => Ok(cx.new_string(s)?.into()),
        None => Ok(Local::undefined()),
    }
}

/// get Intl.Locale.prototype.baseName (14.3.3).
fn get_base_name<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = this_base_name(cx, this)?;
    Ok(cx.new_string(&base.text())?.into())
}

/// A getter that returns slot `slot` (14.3.4-14.3.7, 14.3.11, firstDayOfWeek).
fn slot_getter<'s>(cx: &mut Cx<'s>, this: Local<'s>, slot: usize) -> JsResult<Local<'s>> {
    cx.slot(this, CLASS, slot)
}

/// get Intl.Locale.prototype.calendar (14.3.4).
fn get_calendar<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_CALENDAR)
}

/// get Intl.Locale.prototype.caseFirst (14.3.5).
fn get_case_first<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_CASE_FIRST)
}

/// get Intl.Locale.prototype.collation (14.3.6).
fn get_collation<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_COLLATION)
}

/// get Intl.Locale.prototype.firstDayOfWeek (Intl Locale Info).
fn get_first_day_of_week<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_FIRST_DAY_OF_WEEK)
}

/// get Intl.Locale.prototype.hourCycle (14.3.7).
fn get_hour_cycle<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_HOUR_CYCLE)
}

/// get Intl.Locale.prototype.numeric (14.3.12).
fn get_numeric<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_NUMERIC)
}

/// get Intl.Locale.prototype.numberingSystem (14.3.11).
fn get_numbering_system<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    slot_getter(cx, this, SLOT_NUMBERING_SYSTEM)
}

/// get Intl.Locale.prototype.language (14.3.8).
fn get_language<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = this_base_name(cx, this)?;
    Ok(cx.new_string(&base.language)?.into())
}

/// get Intl.Locale.prototype.script (14.3.14).
fn get_script<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = this_base_name(cx, this)?;
    opt_string(cx, base.script.as_deref())
}

/// get Intl.Locale.prototype.region (14.3.13).
fn get_region<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = this_base_name(cx, this)?;
    opt_string(cx, base.region.as_deref())
}

/// get Intl.Locale.prototype.variants (14.3.17): the variants joined by "-", or undefined.
fn get_variants<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let base = this_base_name(cx, this)?;
    let v = (!base.variants.is_empty()).then(|| base.variants.join("-"));
    opt_string(cx, v.as_deref())
}

/// `this.[[Locale]]` with its base name replaced by `f(base name)`.
fn with_base<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: fn(&BaseName) -> BaseName,
) -> JsResult<Local<'s>> {
    let tag = this_locale(cx, this)?;
    let (base, rest) = BaseName::split(&tag);
    let new_tag = format!("{}{rest}", f(&base).text());
    create_from_tag(cx, &new_tag)
}

/// Intl.Locale.prototype.maximize ( ) (14.3.9): Add Likely Subtags on the base name, then
/// Construct(%Intl.Locale%, maximal).
fn maximize<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    with_base(cx, this, add_likely_subtags)
}

/// Intl.Locale.prototype.minimize ( ) (14.3.10): Remove Likely Subtags on the base name, then
/// Construct(%Intl.Locale%, minimal).
fn minimize<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    with_base(cx, this, remove_likely_subtags)
}

/// Intl.Locale.prototype.toString ( ) (14.3.15): `this.[[Locale]]`.
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.slot(this, CLASS, SLOT_LOCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tag: &str, opts: &[(usize, &str)]) -> String {
        let mut o: [Option<String>; 7] = Default::default();
        for (i, v) in opts {
            o[*i] = Some((*v).to_string());
        }
        make_locale_record(&canonical_locale(tag).unwrap(), &o)
            .unwrap()
            .locale
    }

    /// The variants option check is linear: 100,000 subtags of 8 characters (900 KB) with and
    /// without a repeat at the end, under 5 s (Codex r1 LOCALE-1: the scan of the subtags
    /// before each one was O(n^2), 27.8 s in a release build, with no poll).
    #[test]
    fn a_long_variants_option_is_checked_in_linear_time() {
        let v: Vec<String> = (0..100_000)
            .map(|i| format!("v{}", 1_000_000 + i))
            .collect();
        let option = v.join("-");
        let t = std::time::Instant::now();
        assert!(is_variants_option(&option));
        assert!(!is_variants_option(&format!("{option}-V1000000")));
        let took = t.elapsed();
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
        assert!(is_variants_option("fonipa-1996"));
        assert!(!is_variants_option("fonipa-FONIPA"));
        assert!(!is_variants_option(""));
        assert!(!is_variants_option("fonipa-"));
        assert!(!is_variants_option("abc"));
    }

    #[test]
    fn base_name_split_keeps_the_case() {
        let (b, rest) = BaseName::split("de-Latn-DE-1996-fonipa-u-ca-gregory-x-a");
        assert_eq!(b.language, "de");
        assert_eq!(b.script.as_deref(), Some("Latn"));
        assert_eq!(b.region.as_deref(), Some("DE"));
        assert_eq!(b.variants, ["1996", "fonipa"]);
        assert_eq!(rest, "-u-ca-gregory-x-a");
        let (b, rest) = BaseName::split("und-419");
        assert_eq!((b.region.as_deref(), rest), (Some("419"), ""));
        let (b, _) = BaseName::split("sl-1994-biske");
        assert_eq!((b.region, b.variants.len()), (None, 2));
        assert_eq!(
            canonical_locale("abcdefgh-latn-us").as_deref(),
            Ok("abcdefgh-Latn-US")
        );
        // new Intl.Locale("iw-Latn-t-abcdefgh").language is "he" (Claude reviewer R3; node).
        assert_eq!(
            canonical_locale("iw-Latn-t-abcdefgh").map(|t| BaseName::split(&t).0.language),
            Ok("he".to_string())
        );
    }

    /// node v26.13.0 `new Intl.Locale(tag, options).toString()` results.
    #[test]
    fn records_give_the_node_tags() {
        assert_eq!(
            record(
                "de-latn-de-fonipa-1996-u-ca-gregory-co-phonebk-hc-h23-kf-true-kn-false-nu-latn",
                &[]
            ),
            "de-Latn-DE-1996-fonipa-u-ca-gregory-co-phonebk-hc-h23-kf-kn-false-nu-latn"
        );
        assert_eq!(record("en", &[(5, "true")]), "en-u-kn");
        assert_eq!(record("en", &[(0, "GREGORY")]), "en-u-ca-gregory");
        assert_eq!(record("en", &[(0, "islamicc")]), "en-u-ca-islamic-civil");
        assert_eq!(
            record("en-u-ca-buddhist-x-p", &[(0, "japanese")]),
            "en-u-ca-japanese-x-p"
        );
        assert_eq!(
            record("pt-u-attr2-attr1-ca-gregory", &[]),
            "pt-u-attr1-attr2-ca-gregory"
        );
        assert_eq!(record("en-u-fw-mon", &[(2, "sun")]), "en-u-fw-sun");
        let mut o: [Option<String>; 7] = Default::default();
        o[2] = Some("true".to_string());
        o[5] = Some("true".to_string());
        let r = make_locale_record("en-u-fw-wed", &o).unwrap();
        assert_eq!(r.locale, "en-u-fw-kn");
        assert_eq!(
            (r.keys[2].as_deref(), r.keys[5].as_deref()),
            (Some(""), Some(""))
        );
    }

    #[test]
    fn option_checks() {
        for t in ["gregory", "ethiopic-amete-alem", "100", "11111111"] {
            assert!(is_unicode_type(t), "{t}");
        }
        for t in ["", "ab", "abc-", "-abc", "abcdefghi", "abc--def", "a_bc"] {
            assert!(!is_unicode_type(t), "{t}");
        }
        assert_eq!(weekday_to_string("0"), "sun");
        assert_eq!(weekday_to_string("1"), "mon");
        assert_eq!(weekday_to_string("7"), "sun");
        assert_eq!(weekday_to_string("8"), "8");
        assert_eq!(weekday_to_string("100"), "100");
        assert!(is_variants_option("1xyz-1234-abcde-12345678"));
        for v in [
            "",
            "-",
            "GB-scouse",
            "fonipa-valencia-Fonipa",
            "spanglis--oxendict",
            "abcd",
        ] {
            assert!(!is_variants_option(v), "{v}");
        }
    }

    /// The test262 `minimize/removing-likely-subtags-first-adds-likely-subtags.js` and
    /// `constructor-non-iana-canon.js` pairs, and node v26.13.0 results.
    #[test]
    fn likely_subtags() {
        let max = |t: &str| add_likely_subtags(&BaseName::split(t).0).text();
        let min = |t: &str| remove_likely_subtags(&BaseName::split(t).0).text();
        for (t, want) in [
            ("und", "en-Latn-US"),
            ("und-150", "en-Latn-150"),
            ("und-Thai-150", "th-Thai-150"),
            ("zh-Hani", "zh-Hani-CN"),
            ("aar", "aar"),
            ("abcdefgh", "abcdefgh"),
            ("es-ES-preeuro", "es-Latn-ES-preeuro"),
            ("pa-PK", "pa-Arab-PK"),
        ] {
            assert_eq!(max(t), want, "{t}");
        }
        for (t, want) in [
            ("und", "en"),
            ("und-Thai", "th"),
            ("und-419", "es-419"),
            ("und-150", "en-150"),
            ("und-AT", "de-AT"),
            ("aae-Latn-IT", "aae"),
            ("aae-Thai-CO", "aae-Thai-CO"),
            ("und-CW", "pap"),
            ("und-US", "en"),
            ("zh-Hant", "zh-TW"),
            ("zh-Hani", "zh-Hani"),
            ("uz-Latn-UZ-cyrillic", "uz-cyrillic"),
        ] {
            assert_eq!(min(t), want, "{t}");
        }
    }
}
