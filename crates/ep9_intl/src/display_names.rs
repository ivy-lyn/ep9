//! Intl.DisplayNames (ECMA-402 §12; block 37): `%Intl.DisplayNames%` and
//! `%Intl.DisplayNames.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.DisplayNames {w, c}     %Intl.DisplayNames%  prototype {}, supportedLocalesOf
//! Intl.DisplayNames.prototype  %Intl.DisplayNames.prototype%  constructor {w, c},
//!                                resolvedOptions {w, c}, of {w, c},
//!                                [@@toStringTag] "Intl.DisplayNames" {c}
//! slots (ObjectClass::IntlDisplayNames, SLOTS = 2):
//!   0 [[Locale]]  a String ("de-CH")
//!   1 options     an int32 (Packed): [[Type]], [[Style]], [[Fallback]], [[LanguageDisplay]]
//! of(code) ─ ToString ─ CanonicalCodeForDisplayNames (RangeError) ─ name(locale, packed, code)
//!   language, region, script  cache::get_or_make(Service::DisplayNames, locale, packed)
//!                               -> Rc<Names> (icu_experimental::displaynames::multi)
//!   currency                  the CurrencyDisplaynameV1 data through the provider (no object)
//!   calendar, dateTimeField   no ICU4X data: the code is the name (the workaround below)
//!   no name: fallback "code" -> the code, "none" -> undefined
//! ```
//!
//! `[[Fields]]` (12.2.3) is not copied into the object: `of` looks the code up in the ICU4X
//! data of `[[Locale]]` with the style (long: the plain names; short and narrow: the CLDR
//! "short" names, as V8) and, for "language", the languageDisplay. No ICU4X object is in a
//! slot (INTL-I1-D7, plan rule 8).
//!
//! Locale data rules (each one a test of this module):
//! - currency: a name only for the codes of AvailableCanonicalCurrencies
//!   (`available::CURRENCIES`, `Intl.supportedValuesOf("currency")`): ECMA-402 defines that
//!   list as the currencies that DisplayNames supports. The CLDR data also names old
//!   currencies ("DEM"); they get the fallback.
//! - calendar and dateTimeField (plan v2 §2.2 row DISPLAYNAMES; ICU4X 2.4 has no data for
//!   them): a calendar of AvailableCalendars (`available::CALENDARS`) and each of the 12
//!   dateTimeField codes have the code as their name; any other calendar gets the fallback.
//!   Real names need an ep9 CLDR table (production work, not test262).
//! - language with fallback "none": a name only when each subtag (language, script, region,
//!   variants) has one; with fallback "code" the ICU4X locale display name (it puts the code
//!   of a subtag with no name), or the code when ICU4X cannot read the tag.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-display_names.txt`.

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};
use icu_experimental::displaynames::multi::{
    LanguageDisplayNames, LocaleDisplayNamesFormatter, RegionDisplayNames, ScriptDisplayNames,
    VariantDisplayNames,
};
use icu_experimental::displaynames::{
    DisplayNamesOptions, DisplayNamesPreferences, LanguageDisplay, Style,
};
use icu_locale_core::Locale;
use icu_locale_core::subtags::{Region, Script, Variant};

use crate::available;
use crate::cache::{self, Service};
use crate::options::{get_locale_matcher, get_options_object, get_string_option};
use crate::resolve;
use crate::tag;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 37;

/// The class of an Intl.DisplayNames object.
pub const CLASS: ObjectClass = ObjectClass::IntlDisplayNames;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the [`Packed`] options (an int32).
pub const SLOT_OPTIONS: usize = 1;
/// The slot count of an Intl.DisplayNames object.
pub const SLOTS: usize = 2;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const OF: u16 = 2;
    pub const RESOLVED_OPTIONS: u16 = 3;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("DisplayNames", 2, constructor), // 0 (12.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (12.2.2)
    BuiltinDef::method("of", 1, of),                         // 2 (12.3.3)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 3 (12.3.2)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlDisplayNames,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlDisplayNamesPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlDisplayNames,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlDisplayNamesPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlDisplayNamesPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlDisplayNames,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, then of.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("of"), def::OF),
            // Intl.DisplayNames.prototype [ @@toStringTag ] ({c}, 12.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.DisplayNames"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("DisplayNames"),
            IntrinsicId::IntlDisplayNames,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "display_names",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the options --------------------------------------------------------------------------------

/// The `type` values (12.1.1 step 12), in [`Packed`] order.
pub const TYPES: [&str; 6] = [
    "language",
    "region",
    "script",
    "currency",
    "calendar",
    "dateTimeField",
];
/// The `style` values (step 10), in [`Packed`] order.
pub const STYLES: [&str; 3] = ["narrow", "short", "long"];
/// The `fallback` values (step 15), in [`Packed`] order.
pub const FALLBACKS: [&str; 2] = ["code", "none"];
/// The `languageDisplay` values (step 21), in [`Packed`] order.
pub const LANGUAGE_DISPLAYS: [&str; 2] = ["dialect", "standard"];

/// The valid dateTimeField codes (the ECMA-402 table "Codes For Date Time Field of
/// DisplayNames").
pub const DATE_TIME_FIELDS: [&str; 12] = [
    "era",
    "year",
    "quarter",
    "month",
    "weekOfYear",
    "weekday",
    "day",
    "dayPeriod",
    "hour",
    "minute",
    "second",
    "timeZoneName",
];

/// The type of a DisplayNames object (`[[Type]]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Language,
    Region,
    Script,
    Currency,
    Calendar,
    DateTimeField,
}

impl Type {
    const ALL: [Type; 6] = [
        Type::Language,
        Type::Region,
        Type::Script,
        Type::Currency,
        Type::Calendar,
        Type::DateTimeField,
    ];
}

/// The options of a DisplayNames object, packed in an int32 slot: bits 0-2 `[[Type]]` (index
/// of [`TYPES`]), 3-4 `[[Style]]` ([`STYLES`]), 5 `[[Fallback]]` ([`FALLBACKS`]), 6
/// `[[LanguageDisplay]]` ([`LANGUAGE_DISPLAYS`]; only read for the type "language").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed(pub u32);

impl Packed {
    /// The packed form of the option indices.
    pub fn new(ty: Type, style: u32, fallback_none: bool, standard: bool) -> Packed {
        let t = Type::ALL.iter().position(|t| *t == ty).unwrap_or(0) as u32;
        Packed(t | (style & 3) << 3 | u32::from(fallback_none) << 5 | u32::from(standard) << 6)
    }
    /// `[[Type]]`.
    pub fn ty(self) -> Type {
        Type::ALL[((self.0 & 7) as usize).min(5)]
    }
    fn style_index(self) -> usize {
        (((self.0 >> 3) & 3) as usize).min(2)
    }
    /// `[[Style]]`.
    pub fn style(self) -> &'static str {
        STYLES[self.style_index()]
    }
    /// Whether `[[Fallback]]` is "none".
    pub fn fallback_none(self) -> bool {
        self.0 & (1 << 5) != 0
    }
    /// `[[LanguageDisplay]]` is "standard".
    pub fn standard(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    /// The ICU4X options: long gives the plain CLDR names, short and narrow the "short" names
    /// (V8 gives the same names for both: "UK" for GB).
    fn icu_options(self) -> DisplayNamesOptions {
        let mut o = DisplayNamesOptions::default();
        o.style = match self.style_index() {
            2 => None,
            _ => Some(Style::Short),
        };
        o.language_display = if self.standard() {
            LanguageDisplay::Standard
        } else {
            LanguageDisplay::Dialect
        };
        o
    }
}

/// The checked options of the constructor (12.1.1 steps 3-27), with the resolved locale.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub locale: String,
    pub options: Packed,
}

/// The option index of `value` in `list` (the value passed GetOption, so it is in the list).
fn index_of(list: &[&str], value: Option<&str>) -> u32 {
    list.iter().position(|v| Some(*v) == value).unwrap_or(0) as u32
}

/// Steps 3-27 of Intl.DisplayNames ( locales, options ): the requested locales, the options
/// in spec order and ResolveLocale (no relevant extension keys).
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 3. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 4. If options is undefined, throw a TypeError exception.
    if options.is_undefined() {
        return Err(cx.throw_type_error("Intl.DisplayNames needs an options object"));
    }
    // 5. Set options to ? GetOptionsObject(options).
    let options = get_options_object(cx, options)?;
    // 6-8. Let matcher be ? GetOption(options, "localeMatcher", string, « "lookup", "best
    //      fit" », "best fit").
    get_locale_matcher(cx, options)?;
    // 9. Let r be ResolveLocale(%Intl.DisplayNames%.[[AvailableLocales]], requestedLocales,
    //    opt, %Intl.DisplayNames%.[[RelevantExtensionKeys]] (empty), localeData).
    let r = resolve::resolve_locale(cx, &requested, &[], &[], &|_, _| Vec::new())?;
    // 10-11. Let style be ? GetOption(options, "style", string, « "narrow", "short", "long" »,
    //        "long").
    let style = get_string_option(cx, options, "style", &STYLES, Some("long"))?;
    // 12. Let type be ? GetOption(options, "type", string, « "language", "region", "script",
    //     "currency", "calendar", "dateTimeField" », undefined).
    let ty = get_string_option(cx, options, "type", &TYPES, None)?;
    // 13. If type is undefined, throw a TypeError exception.
    let Some(ty) = ty else {
        return Err(cx.throw_type_error("Intl.DisplayNames needs the option type"));
    };
    // 15. Let fallback be ? GetOption(options, "fallback", string, « "code", "none" », "code").
    let fallback = get_string_option(cx, options, "fallback", &FALLBACKS, Some("code"))?;
    // 21. Let languageDisplay be ? GetOption(options, "languageDisplay", string, « "dialect",
    //     "standard" », "dialect").
    let language_display = get_string_option(
        cx,
        options,
        "languageDisplay",
        &LANGUAGE_DISPLAYS,
        Some("dialect"),
    )?;
    let ty = Type::ALL[index_of(&TYPES, Some(&ty)) as usize];
    Ok(Resolved {
        // 17. Set displayNames.[[Locale]] to r.[[Locale]].
        locale: r.locale,
        options: Packed::new(
            ty,
            index_of(&STYLES, style.as_deref()),
            fallback.as_deref() == Some("none"),
            // 24.a. Only the type "language" keeps [[LanguageDisplay]].
            ty == Type::Language && language_display.as_deref() == Some("standard"),
        ),
    })
}

// ---- CanonicalCodeForDisplayNames (12.5.1) -----------------------------------------------------

/// Whether `s` matches the `type` nonterminal of UTS 35: `alphanum{3,8} (sep alphanum{3,8})*`
/// with "-" as the separator.
fn is_unicode_type(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|p| (3..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// CanonicalCodeForDisplayNames ( type, code ) (12.5.1): the canonical code, or `None` for
/// the RangeError.
pub fn canonical_code(ty: Type, code: &str) -> Option<String> {
    match ty {
        Type::Language => {
            // 1.a-b. code matches unicode_language_id and IsStructurallyValidLanguageTag
            //        (no extension and no private use; "root" and a script first fail the
            //        ECMA-402 grammar).
            let p = tag::parse(code).ok()?;
            if !p.extensions.is_empty() || !p.private.is_empty() {
                return None;
            }
            // 1.c. Return CanonicalizeUnicodeLocaleId(code).
            Some(tag::canonicalize_parsed(&p))
        }
        Type::Region => {
            // 2.a. code matches unicode_region_subtag; 2.b. its ASCII upper case.
            tag::is_region_subtag(code).then(|| code.to_ascii_uppercase())
        }
        Type::Script => {
            // 3.a. code matches unicode_script_subtag; 3.c-e. the title case form.
            if !tag::is_script_subtag(code) {
                return None;
            }
            let (first, rest) = code.split_at(1);
            Some(first.to_ascii_uppercase() + &rest.to_ascii_lowercase())
        }
        Type::Calendar => {
            // 4.a. code matches the type nonterminal; 4.b. no backwards compatibility syntax
            //      ("_" is not a separator of the type nonterminal).
            if !is_unicode_type(code) {
                return None;
            }
            // 4.c-d. Return CanonicalizeUValue("ca", the ASCII lower case of code).
            Some(tag::canonicalize_unicode_value(
                "ca",
                &code.to_ascii_lowercase(),
            ))
        }
        Type::DateTimeField => {
            // 5.a. code is a valid dateTimeField code; 5.b. return code.
            DATE_TIME_FIELDS.contains(&code).then(|| code.to_string())
        }
        Type::Currency => {
            // 7. IsWellFormedCurrencyCode(code); 8. its ASCII upper case.
            (code.len() == 3 && code.bytes().all(|b| b.is_ascii_alphabetic()))
                .then(|| code.to_ascii_uppercase())
        }
    }
}

// ---- the names ---------------------------------------------------------------------------------

/// The ICU4X display names objects of one (locale, options) pair, in the cache.
enum Names {
    Language(Box<LanguageNames>),
    Region(RegionDisplayNames),
    Script(ScriptDisplayNames),
}

/// The objects of the type "language": the locale display name formatter and the subtag
/// names (for the fallback "none" check).
struct LanguageNames {
    locale: LocaleDisplayNamesFormatter,
    language: LanguageDisplayNames,
    script: ScriptDisplayNames,
    region: RegionDisplayNames,
    variant: VariantDisplayNames,
}

/// The ICU4X locale of `locale` (canonical; "und" if ICU4X cannot read it).
fn icu_locale(locale: &str) -> Locale {
    Locale::try_from_str(locale).unwrap_or(Locale::UNKNOWN)
}

/// New ICU4X objects for `locale` and `p` (`None` if the data cannot load; not reached with the
/// compiled data, which falls back to the root locale).
fn make_names(locale: &str, p: Packed) -> Option<Names> {
    let prefs = DisplayNamesPreferences::from(&icu_locale(locale));
    let o = p.icu_options();
    Some(match p.ty() {
        Type::Language => Names::Language(Box::new(LanguageNames {
            locale: LocaleDisplayNamesFormatter::try_new(prefs, o).ok()?,
            language: LanguageDisplayNames::try_new(prefs, o).ok()?,
            script: ScriptDisplayNames::try_new(prefs, o).ok()?,
            region: RegionDisplayNames::try_new(prefs, o).ok()?,
            variant: VariantDisplayNames::try_new(prefs, o).ok()?,
        })),
        Type::Region => Names::Region(RegionDisplayNames::try_new(prefs, o).ok()?),
        Type::Script => Names::Script(ScriptDisplayNames::try_new(prefs, o).ok()?),
        Type::Currency | Type::Calendar | Type::DateTimeField => return None,
    })
}

/// The cached names of (`locale`, `p`).
fn names_for(locale: &str, p: Packed) -> Option<Rc<Names>> {
    cache::get_or_make(Service::DisplayNames, locale, u64::from(p.0), || {
        make_names(locale, p)
    })
}

/// The CLDR display name of the currency `code` (upper case) in `locale`, from the
/// `CurrencyDisplaynameV1` data through the locale fallback (no ICU4X formatter has it).
pub fn currency_name(locale: &str, code: &str) -> Option<String> {
    use icu_experimental::dimension::provider::currency::displayname::CurrencyDisplaynameV1;
    use icu_experimental::provider::Baked;
    use icu_provider::prelude::*;
    let attributes = DataMarkerAttributes::try_from_str(code).ok()?;
    let prefs = DisplayNamesPreferences::from(&icu_locale(locale));
    let data_locale = CurrencyDisplaynameV1::make_locale(prefs.locale_preferences);
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &data_locale),
        metadata: {
            let mut m = DataRequestMetadata::default();
            m.silent = true;
            m
        },
    };
    let response = DataProvider::<CurrencyDisplaynameV1>::load(&Baked, req).ok()?;
    let name: &str = response.payload.get();
    (!name.is_empty()).then(|| name.to_string())
}

/// The name of the language id `code` (canonical) with `names`, or `None` when `fallback_none`
/// and a subtag has no name. With fallback "code" the ICU4X text, which puts the code of a
/// subtag with no name ("English (XX)"); `None` only when ICU4X cannot read the tag (a
/// language subtag of 5-8 letters).
fn language_name(names: &Names, code: &str, fallback_none: bool) -> Option<String> {
    let Names::Language(n) = names else {
        return None;
    };
    let LanguageNames {
        locale,
        language,
        script,
        region,
        variant,
    } = &**n;
    let id = Locale::try_from_str(code).ok()?;
    if fallback_none {
        let has = |s: &str| !s.is_empty();
        let lang_ok = language.of(id.id.language).is_some_and(has);
        let script_ok = id
            .id
            .script
            .is_none_or(|s: Script| script.of(s).is_some_and(has));
        let region_ok = id
            .id
            .region
            .is_none_or(|r: Region| region.of(r).is_some_and(has));
        let variants_ok = id
            .id
            .variants
            .iter()
            .all(|v: &Variant| variant.of(*v).is_some_and(has));
        if !(lang_ok && script_ok && region_ok && variants_ok) {
            return None;
        }
    }
    Some(locale.of(&id).into_owned())
}

/// The name of `code` (the result of [`canonical_code`]) for the locale `locale` and the
/// options `p`: `fields.[[<code>]]` of 12.3.3 step 5, `None` if the locale data has none.
pub fn name(locale: &str, p: Packed, code: &str) -> Option<String> {
    match p.ty() {
        Type::Language => language_name(&*names_for(locale, p)?, code, p.fallback_none()),
        Type::Region => {
            let r = Region::try_from_str(code).ok()?;
            match &*names_for(locale, p)? {
                Names::Region(n) => n.of(r).map(str::to_string),
                _ => None,
            }
        }
        Type::Script => {
            let s = Script::try_from_str(code).ok()?;
            match &*names_for(locale, p)? {
                Names::Script(n) => n.of(s).map(str::to_string),
                _ => None,
            }
        }
        Type::Currency => {
            if available::CURRENCIES.binary_search(&code).is_err() {
                return None;
            }
            currency_name(locale, code)
        }
        // The workaround of plan v2 §2.2 (no ICU4X data): the code is the name.
        Type::Calendar => available::CALENDARS
            .contains(&code)
            .then(|| code.to_string()),
        Type::DateTimeField => Some(code.to_string()),
    }
}

// ---- the builtins -------------------------------------------------------------------------------

/// Intl.DisplayNames ( locales, options ) (12.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.DisplayNames requires 'new'"));
    };
    // 2. Let displayNames be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%Intl.DisplayNames.prototype%", « [[InitializedDisplayNames]], [[Locale]], [[Style]],
    //    [[Type]], [[Fallback]], [[LanguageDisplay]], [[Fields]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlDisplayNamesPrototype)?;
    let display_names = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3-27.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(display_names, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(
        display_names,
        CLASS,
        SLOT_OPTIONS,
        Local::int32(r.options.0 as i32),
    )?;
    // 28. Return displayNames.
    Ok(display_names.into())
}

/// Intl.DisplayNames.supportedLocalesOf ( locales [ , options ] ) (12.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.DisplayNames%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The `[[Locale]]` text and the packed options of `o` (TypeError if `o` is not an
/// Intl.DisplayNames: RequireInternalSlot(o, `[[InitializedDisplayNames]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<(String, Packed)> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let options = cx.slot(o, CLASS, SLOT_OPTIONS)?;
    let locale = cx.to_rust_string(locale)?;
    Ok((locale, Packed(options.as_int32().unwrap_or(0) as u32)))
}

/// Intl.DisplayNames.prototype.of ( code ) (12.3.3).
fn of<'s>(cx: &mut Cx<'s>, this: Local<'s>, args: Args<'s>, _nt: Local<'s>) -> JsResult<Local<'s>> {
    // 1-2. Let displayNames be the this value; ? RequireInternalSlot(displayNames,
    //      [[InitializedDisplayNames]]).
    let (locale, p) = read(cx, this)?;
    // 3. Let code be ? ToString(code).
    let code = cx.to_rust_string(args.get(0))?;
    // 4. Let code be ? CanonicalCodeForDisplayNames(displayNames.[[Type]], code).
    let Some(code) = canonical_code(p.ty(), &code) else {
        return Err(cx.throw_range_error(&format!(
            "invalid code for Intl.DisplayNames.prototype.of: {code}"
        )));
    };
    // 5-6. If fields has a field [[<code>]], return fields.[[<code>]].
    if let Some(n) = name(&locale, p, &code) {
        return Ok(cx.new_string(&n)?.into());
    }
    // 7. If displayNames.[[Fallback]] is "code", return code.
    if !p.fallback_none() {
        return Ok(cx.new_string(&code)?.into());
    }
    // 8. Return undefined.
    Ok(Local::undefined())
}

/// Intl.DisplayNames.prototype.resolvedOptions ( ) (12.3.2).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let displayNames be the this value; ? RequireInternalSlot(displayNames,
    //      [[InitializedDisplayNames]]).
    let (locale, p) = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of the resolved options table (locale, style, type, fallback,
    //    languageDisplay), in table order: if v is not undefined, CreateDataPropertyOrThrow(options, p, v).
    let ty = TYPES[Type::ALL.iter().position(|t| *t == p.ty()).unwrap_or(0)];
    let fallback = FALLBACKS[usize::from(p.fallback_none())];
    for (key, value) in [
        ("locale", locale.as_str()),
        ("style", p.style()),
        ("type", ty),
        ("fallback", fallback),
    ] {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)?;
    }
    if p.ty() == Type::Language {
        let v = cx.new_string(LANGUAGE_DISPLAYS[usize::from(p.standard())])?;
        cx.create_data_property_or_throw(o, "languageDisplay", v)?;
    }
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(ty: Type, style: &str, none: bool, standard: bool) -> Packed {
        Packed::new(ty, index_of(&STYLES, Some(style)), none, standard)
    }

    #[test]
    fn packed_round_trips() {
        for ty in Type::ALL {
            for style in STYLES {
                for none in [false, true] {
                    for standard in [false, true] {
                        let p = packed(ty, style, none, standard);
                        assert_eq!(
                            (p.ty(), p.style(), p.fallback_none(), p.standard()),
                            (ty, style, none, standard)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn canonical_codes() {
        let c = |ty, s: &str| canonical_code(ty, s);
        assert_eq!(
            c(Type::Language, "EN-latn-us").as_deref(),
            Some("en-Latn-US")
        );
        assert_eq!(c(Type::Language, "sh").as_deref(), Some("sr-Latn"));
        for bad in [
            "",
            "root",
            "abcd",
            "abcd-GB",
            "en_GB",
            "en-u-ca-gregory",
            "en-x-priv",
        ] {
            assert_eq!(c(Type::Language, bad), None, "{bad}");
        }
        assert_eq!(c(Type::Region, "us").as_deref(), Some("US"));
        assert_eq!(c(Type::Region, "419").as_deref(), Some("419"));
        assert_eq!(c(Type::Region, "a1"), None);
        assert_eq!(c(Type::Script, "LATN").as_deref(), Some("Latn"));
        assert_eq!(c(Type::Script, "Lat"), None);
        assert_eq!(
            c(Type::Calendar, "ISLAMICC").as_deref(),
            Some("islamic-civil")
        );
        assert_eq!(
            c(Type::Calendar, "ethiopic-amete-alem").as_deref(),
            Some("ethioaa")
        );
        assert_eq!(c(Type::Calendar, "123_abc"), None);
        assert_eq!(
            c(Type::DateTimeField, "weekOfYear").as_deref(),
            Some("weekOfYear")
        );
        assert_eq!(c(Type::DateTimeField, "weekofyear"), None);
        assert_eq!(c(Type::Currency, "usd").as_deref(), Some("USD"));
        assert_eq!(c(Type::Currency, "US1"), None);
    }

    /// Every currency of AvailableCanonicalCurrencies has an English name in the data, and the
    /// names are the CLDR ones (node v26.13.0 gives the same strings).
    #[test]
    fn every_available_currency_has_a_name() {
        let missing: Vec<&str> = available::CURRENCIES
            .iter()
            .copied()
            .filter(|c| currency_name("en", c).is_none())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(currency_name("en", "USD").as_deref(), Some("US Dollar"));
        assert_eq!(currency_name("de", "USD").as_deref(), Some("US-Dollar"));
        assert_eq!(currency_name("ja", "JPY").as_deref(), Some("日本円"));
        assert_eq!(currency_name("en", "QQQ"), None);
    }

    #[test]
    fn names_follow_the_options() {
        let n = |locale: &str, p, code: &str| name(locale, p, code);
        let lang = packed(Type::Language, "long", false, false);
        assert_eq!(n("en", lang, "en-GB").as_deref(), Some("British English"));
        assert_eq!(n("fr", lang, "en-US").as_deref(), Some("anglais américain"));
        let std = packed(Type::Language, "long", false, true);
        assert_eq!(
            n("en", std, "en-GB").as_deref(),
            Some("English (United Kingdom)")
        );
        let short = packed(Type::Language, "short", false, false);
        assert_eq!(n("en", short, "en-GB").as_deref(), Some("UK English"));
        assert_eq!(n("en", lang, "en-XX").as_deref(), Some("English (XX)"));
        assert_eq!(n("en", lang, "zzzzzzzz"), None);
        let none = packed(Type::Language, "long", true, false);
        assert_eq!(n("en", none, "en-XX"), None);
        assert_eq!(n("en", none, "xx"), None);
        assert_eq!(
            n("en", none, "zh-Hant").as_deref(),
            Some("Traditional Chinese")
        );
        let region = packed(Type::Region, "long", true, false);
        assert_eq!(n("de", region, "DE").as_deref(), Some("Deutschland"));
        assert_eq!(n("en", region, "AA"), None);
        let region_short = packed(Type::Region, "narrow", true, false);
        assert_eq!(n("en", region_short, "GB").as_deref(), Some("UK"));
        let script = packed(Type::Script, "long", true, false);
        assert_eq!(n("zh", script, "Hans").as_deref(), Some("简体"));
        assert_eq!(n("en", script, "Aaaa"), None);
        let currency = packed(Type::Currency, "long", true, false);
        assert_eq!(n("en", currency, "EUR").as_deref(), Some("Euro"));
        // Not in AvailableCanonicalCurrencies: the fallback.
        assert_eq!(n("en", currency, "DEM"), None);
        let calendar = packed(Type::Calendar, "long", true, false);
        assert_eq!(n("en", calendar, "gregory").as_deref(), Some("gregory"));
        assert_eq!(n("en", calendar, "islamic"), None);
        let field = packed(Type::DateTimeField, "long", true, false);
        assert_eq!(n("en", field, "era").as_deref(), Some("era"));
    }
}
