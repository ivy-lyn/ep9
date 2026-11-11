//! Intl.PluralRules (ECMA-402 §17; block 33): `%Intl.PluralRules%` and `%Intl.PluralRules.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics.
//!
//! ```text
//! Intl.PluralRules {w, c}      %Intl.PluralRules%  prototype {}, supportedLocalesOf
//! Intl.PluralRules.prototype   %Intl.PluralRules.prototype%  constructor {w, c},
//!                                resolvedOptions, select, selectRange {w, c},
//!                                [@@toStringTag] "Intl.PluralRules" {c}
//! slots (ObjectClass::IntlPluralRules, SLOTS = 4):
//!   0 [[Locale]]   a String (PluralRules has no relevant extension key: no "-u-")
//!   1 options      an int32: [[Type]], [[Notation]], [[CompactDisplay]], [[RoundingMode]],
//!                  [[RoundingType]], [[ComputedRoundingPriority]], [[TrailingZeroDisplay]]
//!   2 digits       an int32: [[MinimumIntegerDigits]] .. [[MaximumSignificantDigits]]
//!   3 increment    an int32: [[RoundingIncrement]]
//! select(v) ─ ToIntlMathematicalValue ─ ResolvePlural:
//!   FormatNumericToString (crate::digits, on Decimal) ─> [[FormattedString]]
//!   the notation exponent (ComputeExponent: compact from the ICU4X compact data, scientific,
//!   engineering) ─> PluralOperands (significand, exponent) ─ cache::get_or_make(
//!   Service::PluralRules, data locale, type + notation) ─> Rc<Icu> ─ category_for
//!   (the ICU4X rules of the locale or of its language; a language that the ICU4X data does
//!   not have takes the CLDR rules of `cldr_extra`)
//! selectRange(x, y) ─ two ResolvePlural ─ same text: x's category; else the ICU4X range data
//!   (`icu_plurals` `unstable`, PluralRulesWithRanges::resolve_range)
//! ```
//!
//! No ICU4X object is in a slot (INTL-I1-D7, plan rule 8): the constructor reads the options
//! and resolves the locale with no ICU4X object; `select`, `selectRange` and
//! `resolvedOptions` find the ICU4X rules through `crate::cache`. No f64 arithmetic: the
//! digits come from Number::toString and are rounded on `Decimal` (H36, INTL-V2-FP2).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-plural_rules.txt`.

mod cldr_extra;

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};
use fixed_decimal::CompactDecimal;
use icu_decimal::CompactDecimalFormatter;
use icu_decimal::options::CompactDecimalFormatterOptions;
use icu_decimal::preferences::CompactDecimalFormatterPreferences;
use icu_locale_core::Locale;
use icu_plurals::{
    PluralCategory, PluralOperands, PluralRuleType, PluralRules, PluralRulesOptions,
    PluralRulesPreferences, PluralRulesWithRanges, RawPluralOperands,
};

use crate::cache::{self, Service};
use crate::digits::{
    DecimalValue, DigitOptions, RoundingMode, RoundingType, format_numeric_to_string,
    set_number_format_digit_options,
};
use crate::number_format::decimal::{IntlMv, to_intl_mv};
use crate::options::index_of;
use crate::options::{coerce_options_to_object, get_locale_matcher, get_string_option};
use crate::resolve;
use crate::tag;
use crate::tag::icu_locale;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 33;

/// The class of an Intl.PluralRules object.
pub const CLASS: ObjectClass = ObjectClass::IntlPluralRules;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the packed options (an int32; the layout of `Resolved::pack_options`).
pub const SLOT_OPTIONS: usize = 1;
/// Slot 2: the packed digit counts (an int32; the layout of `Resolved::pack_digits`).
pub const SLOT_DIGITS: usize = 2;
/// Slot 3: `[[RoundingIncrement]]` (an int32).
pub const SLOT_INCREMENT: usize = 3;
/// The slot count of an Intl.PluralRules object.
pub const SLOTS: usize = 4;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const RESOLVED_OPTIONS: u16 = 2;
    pub const SELECT: u16 = 3;
    pub const SELECT_RANGE: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("PluralRules", 0, constructor), // 0 (17.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (17.2.2)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 2 (17.3.2)
    BuiltinDef::method("select", 1, select),                // 3 (17.3.3)
    BuiltinDef::method("selectRange", 2, select_range),     // 4 (17.3.4)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlPluralRules,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlPluralRulesPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlPluralRules,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlPluralRulesPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlPluralRulesPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlPluralRules,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, select, selectRange.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("select"), def::SELECT),
            PropDef::method(PropKey::Str("selectRange"), def::SELECT_RANGE),
            // Intl.PluralRules.prototype [ @@toStringTag ] ({c}, 17.3.5).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.PluralRules"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("PluralRules"),
            IntrinsicId::IntlPluralRules,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "plural_rules",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Type]]` values, in their packed order.
const TYPES: [&str; 2] = ["cardinal", "ordinal"];
/// `[[Notation]]` values, in their packed order.
const NOTATIONS: [&str; 4] = ["standard", "scientific", "engineering", "compact"];
/// `[[CompactDisplay]]` values, in their packed order.
const COMPACT_DISPLAYS: [&str; 2] = ["short", "long"];
/// `[[RoundingMode]]` values, in their packed order (the order of `RoundingMode::NAMES`).
const ROUNDING_MODES: [RoundingMode; 9] = [
    RoundingMode::Ceil,
    RoundingMode::Floor,
    RoundingMode::Expand,
    RoundingMode::Trunc,
    RoundingMode::HalfCeil,
    RoundingMode::HalfFloor,
    RoundingMode::HalfExpand,
    RoundingMode::HalfTrunc,
    RoundingMode::HalfEven,
];
/// `[[RoundingType]]` values, in their packed order.
const ROUNDING_TYPES: [RoundingType; 4] = [
    RoundingType::FractionDigits,
    RoundingType::SignificantDigits,
    RoundingType::MorePrecision,
    RoundingType::LessPrecision,
];
/// `[[ComputedRoundingPriority]]` values, in their packed order.
const PRIORITIES: [&str; 3] = ["auto", "morePrecision", "lessPrecision"];

/// The internal slots of a PluralRules object (§17.4), unpacked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[Type]]`: an index in `TYPES`.
    pub plural_type: u32,
    /// `[[Notation]]`: an index in `NOTATIONS`.
    pub notation: u32,
    /// `[[CompactDisplay]]`: an index in `COMPACT_DISPLAYS` (read only for "compact").
    pub compact_display: u32,
    /// The fields of SetNumberFormatDigitOptions.
    pub digits: DigitOptions,
}

impl Resolved {
    /// The packed options (slot 1):
    ///
    /// ```text
    /// bit 0      [[Type]]               bits 1-2  [[Notation]]     bit 3  [[CompactDisplay]]
    /// bits 4-7   [[RoundingMode]]       bits 8-9  [[RoundingType]]
    /// bits 10-11 [[ComputedRoundingPriority]]     bit 12 [[TrailingZeroDisplay]] "stripIfInteger"
    /// ```
    fn pack_options(&self) -> i32 {
        let d = &self.digits;
        (self.plural_type
            | self.notation << 1
            | self.compact_display << 3
            | index_of(&ROUNDING_MODES, &d.rounding_mode) << 4
            | index_of(&ROUNDING_TYPES, &d.rounding_type) << 8
            | index_of(&PRIORITIES, &d.computed_rounding_priority) << 10
            | u32::from(d.strip_if_integer) << 12) as i32
    }

    /// The packed digit counts (slot 2; the NumberFormat layout): `[[MinimumIntegerDigits]]`
    /// bits 0-4, `[[MinimumFractionDigits]]` 5-11, `[[MaximumFractionDigits]]` 12-18,
    /// `[[MinimumSignificantDigits]]` 19-23, `[[MaximumSignificantDigits]]` 24-28.
    fn pack_digits(&self) -> i32 {
        let d = &self.digits;
        (u32::from(d.minimum_integer_digits)
            | u32::from(d.minimum_fraction_digits) << 5
            | u32::from(d.maximum_fraction_digits) << 12
            | u32::from(d.minimum_significant_digits) << 19
            | u32::from(d.maximum_significant_digits) << 24) as i32
    }

    /// The options from the two packed slots and the increment.
    fn unpack(locale: String, o: u32, d: u32, increment: u16) -> Resolved {
        let field = |shift: u32, bits: u32| (o >> shift) & ((1 << bits) - 1);
        let dfield = |shift: u32, bits: u32| ((d >> shift) & ((1 << bits) - 1)) as u8;
        let pick = |i: u32, n: usize| (i as usize).min(n - 1);
        Resolved {
            locale,
            plural_type: field(0, 1),
            notation: field(1, 2),
            compact_display: field(3, 1),
            digits: DigitOptions {
                minimum_integer_digits: dfield(0, 5),
                minimum_fraction_digits: dfield(5, 7),
                maximum_fraction_digits: dfield(12, 7),
                minimum_significant_digits: dfield(19, 5),
                maximum_significant_digits: dfield(24, 5),
                rounding_increment: increment,
                rounding_mode: ROUNDING_MODES[pick(field(4, 4), 9)],
                rounding_type: ROUNDING_TYPES[pick(field(8, 2), 4)],
                computed_rounding_priority: PRIORITIES[pick(field(10, 2), 3)],
                strip_if_integer: field(12, 1) != 0,
            },
        }
    }

    /// `[[Notation]]`.
    pub fn notation(&self) -> &'static str {
        NOTATIONS[self.notation as usize]
    }

    /// `[[CompactDisplay]]`: `None` unless the notation is "compact".
    pub fn compact_display(&self) -> Option<&'static str> {
        (self.notation() == "compact").then(|| COMPACT_DISPLAYS[self.compact_display as usize])
    }

    /// The cache key of the ICU4X objects: type, notation and compact display.
    fn key(&self) -> u64 {
        u64::from(self.plural_type)
            | u64::from(self.notation) << 1
            | u64::from(self.compact_display) << 3
    }
}

// ---- the constructor (17.1.1) -----------------------------------------------------------------

/// Intl.PluralRules steps 3-13 (17.1.1; the object creation of step 2 is the caller's): the
/// resolved options for `locales` and `options`, in the spec order of the observable steps.
/// No ICU4X object is made.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 3. ResolveOptions(%Intl.PluralRules%, ..., locales, options, « coerce-options »):
    //    CanonicalizeLocaleList, CoerceOptionsToObject, localeMatcher, ResolveLocale with the
    //    relevant extension keys « ».
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    let options = coerce_options_to_object(cx, options)?;
    get_locale_matcher(cx, options)?;
    let r = resolve::resolve_locale(cx, &requested, &[], &[], &|_, _| Vec::new())?;
    // 7-8. Let t be ? GetOption(options, "type", string, « "cardinal", "ordinal" »,
    //      "cardinal").
    let t = get_string_option(cx, options, "type", &TYPES, Some("cardinal"))?;
    // 9-10. Let notation be ? GetOption(options, "notation", string, « "standard",
    //       "scientific", "engineering", "compact" », "standard").
    let notation = get_string_option(cx, options, "notation", &NOTATIONS, Some("standard"))?;
    // 11-12. Let compactDisplay be ? GetOption(options, "compactDisplay", string, « "short",
    //        "long" », "short"); kept only for "compact".
    let compact_display = get_string_option(
        cx,
        options,
        "compactDisplay",
        &COMPACT_DISPLAYS,
        Some("short"),
    )?;
    let notation = NOTATIONS
        .iter()
        .position(|n| Some(*n) == notation.as_deref())
        .unwrap_or(0) as u32;
    let compact = NOTATIONS[notation as usize] == "compact";
    // 13. Perform ? SetNumberFormatDigitOptions(pluralRules, options, 0, 3, notation).
    let digits = set_number_format_digit_options(cx, options, 0, 3, compact)?;
    Ok(Resolved {
        locale: r.locale,
        plural_type: u32::from(t.as_deref() == Some("ordinal")),
        notation,
        compact_display: if compact {
            u32::from(compact_display.as_deref() == Some("long"))
        } else {
            0
        },
        digits,
    })
}

/// Intl.PluralRules ( [ locales [ , options ] ] ) (17.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.PluralRules requires 'new'"));
    };
    // 2. Let pluralRules be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%Intl.PluralRules.prototype%", internalSlotsList).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlPluralRulesPrototype)?;
    let pr = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3-13.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(pr, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(pr, CLASS, SLOT_OPTIONS, Local::int32(r.pack_options()))?;
    cx.set_slot(pr, CLASS, SLOT_DIGITS, Local::int32(r.pack_digits()))?;
    cx.set_slot(
        pr,
        CLASS,
        SLOT_INCREMENT,
        Local::int32(i32::from(r.digits.rounding_increment)),
    )?;
    // 14. Return pluralRules.
    Ok(pr.into())
}

/// Intl.PluralRules.supportedLocalesOf ( locales [ , options ] ) (17.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.PluralRules%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The resolved options of the PluralRules `o` (TypeError if `o` is not one:
/// RequireInternalSlot(pr, `[[InitializedPluralRules]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<Resolved> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let options = cx.slot(o, CLASS, SLOT_OPTIONS)?.as_int32().unwrap_or(0) as u32;
    let digits = cx.slot(o, CLASS, SLOT_DIGITS)?.as_int32().unwrap_or(0) as u32;
    let increment = cx.slot(o, CLASS, SLOT_INCREMENT)?.as_int32().unwrap_or(1);
    let locale = cx.to_rust_string(locale)?;
    Ok(Resolved::unpack(locale, options, digits, increment as u16))
}

// ---- the ICU4X objects (crate::cache) ---------------------------------------------------------

/// The plural rules of one locale and type, with the range data if the locale has it.
enum Rules {
    WithRanges(PluralRulesWithRanges<PluralRules>),
    Plain(PluralRules),
    /// The CLDR rules of a language that the ICU4X data does not have (no range data).
    Cldr(cldr_extra::Rules),
}

/// The ICU4X objects of one (data locale, type, notation, compact display).
struct Icu {
    rules: Rules,
    /// The compact data (compact notation only): ComputeExponentForMagnitude.
    compact: Option<CompactDecimalFormatter>,
}

impl Icu {
    /// PluralRuleSelect on the operands `o`.
    fn category_for(&self, o: PluralOperands) -> PluralCategory {
        match &self.rules {
            Rules::WithRanges(r) => r.rules().category_for(o),
            Rules::Plain(r) => r.category_for(o),
            Rules::Cldr(r) => r.category_for(o),
        }
    }

    /// The categories the rules can give.
    fn categories(&self) -> Vec<PluralCategory> {
        match &self.rules {
            Rules::WithRanges(r) => r.rules().categories().collect(),
            Rules::Plain(r) => r.categories().collect(),
            Rules::Cldr(r) => r.categories().collect(),
        }
    }

    /// PluralRuleSelectRange: the range data, or the end category if the locale has none
    /// (the CLDR default).
    fn resolve_range(&self, start: PluralCategory, end: PluralCategory) -> PluralCategory {
        match &self.rules {
            Rules::WithRanges(r) => r.resolve_range(start, end),
            Rules::Plain(_) | Rules::Cldr(_) => end,
        }
    }
}

/// Whether the ICU4X compiled data has plural rules of type `ordinal` for `locale` itself or
/// through a parent other than "und".
fn has_own_rules(locale: &Locale, ordinal: bool) -> bool {
    use icu_plurals::provider::{Baked, PluralsCardinalV1, PluralsOrdinalV1};
    use icu_provider::prelude::*;
    let prefs = PluralRulesPreferences::from(locale);
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    let found = |l: Option<DataLocale>| l.is_some_and(|l| !l.is_unknown());
    if ordinal {
        let id = PluralsOrdinalV1::make_locale(prefs.locale_preferences);
        let req = DataRequest {
            id: DataIdentifierBorrowed::for_locale(&id),
            metadata,
        };
        DataProvider::<PluralsOrdinalV1>::load(&Baked, req).is_ok_and(|r| {
            found(r.metadata.locale) || (!id.is_unknown() && r.metadata.locale.is_none())
        })
    } else {
        let id = PluralsCardinalV1::make_locale(prefs.locale_preferences);
        let req = DataRequest {
            id: DataIdentifierBorrowed::for_locale(&id),
            metadata,
        };
        DataProvider::<PluralsCardinalV1>::load(&Baked, req).is_ok_and(|r| {
            found(r.metadata.locale) || (!id.is_unknown() && r.metadata.locale.is_none())
        })
    }
}

/// Where the plural rules of a locale come from.
enum Source {
    /// The ICU4X data of this locale (or of its ICU4X fallback, "und" at the end).
    Icu(Locale),
    /// The CLDR rules of [`cldr_extra`].
    Cldr(cldr_extra::Rules),
}

/// The source of the plural rules of `data_locale`. CLDR plural rules belong to the language
/// (`supplemental/plurals.xml`), but the ICU4X locale fallback follows the CLDR parent
/// locales, so a script variant whose parent is root ("sr-Latn", "az-Cyrl", "bs-Cyrl",
/// "uz-Cyrl", "pa-Arab") would get the "und" rules: such a locale takes the rules of its
/// language. A language that the ICU4X data does not have takes the [`cldr_extra`] rules.
fn plural_source(data_locale: &str, ordinal: bool) -> Source {
    let locale = icu_locale(data_locale);
    if has_own_rules(&locale, ordinal) {
        return Source::Icu(locale);
    }
    let language = Locale::from(icu_locale_core::LanguageIdentifier::from(
        locale.id.language,
    ));
    if language != locale && has_own_rules(&language, ordinal) {
        return Source::Icu(language);
    }
    match cldr_extra::rules_for(locale.id.language.as_str(), ordinal) {
        Some(rules) => Source::Cldr(rules),
        None => Source::Icu(locale),
    }
}

/// New ICU4X objects for `data_locale` and `r` (`None` if the data has no rules).
fn make_icu(data_locale: &str, r: &Resolved) -> Option<Icu> {
    let locale = icu_locale(data_locale);
    let options = PluralRulesOptions::default().with_type(if r.plural_type == 1 {
        PluralRuleType::Ordinal
    } else {
        PluralRuleType::Cardinal
    });
    let rules = match plural_source(data_locale, r.plural_type == 1) {
        Source::Cldr(rules) => Rules::Cldr(rules),
        Source::Icu(l) => {
            let prefs = PluralRulesPreferences::from(&l);
            match PluralRulesWithRanges::try_new(prefs, options) {
                Ok(w) => Rules::WithRanges(w),
                Err(_) => Rules::Plain(PluralRules::try_new(prefs, options).ok()?),
            }
        }
    };
    let compact = match r.compact_display() {
        Some(display) => {
            let prefs = CompactDecimalFormatterPreferences::from(&locale);
            let o = CompactDecimalFormatterOptions::default();
            if display == "long" {
                CompactDecimalFormatter::try_new_long(prefs, o).ok()
            } else {
                CompactDecimalFormatter::try_new_short(prefs, o).ok()
            }
        }
        None => None,
    };
    Some(Icu { rules, compact })
}

/// The cached ICU4X objects of `r` (the "und" data if its locale has none).
fn icu_for(r: &Resolved) -> Option<Rc<Icu>> {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    cache::get_or_make(Service::PluralRules, &data_locale, r.key(), || {
        make_icu(&data_locale, r).or_else(|| make_icu("und", r))
    })
}

// ---- ResolvePlural (17.5.2) and ResolvePluralRange (17.5.4) -----------------------------------

/// The category names, in the order of `pluralCategories` (17.3.2 step 4).
const CATEGORIES: [(PluralCategory, &str); 6] = [
    (PluralCategory::Zero, "zero"),
    (PluralCategory::One, "one"),
    (PluralCategory::Two, "two"),
    (PluralCategory::Few, "few"),
    (PluralCategory::Many, "many"),
    (PluralCategory::Other, "other"),
];

/// The name of the category `c`.
fn category_name(c: PluralCategory) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(k, _)| *k == c)
        .map_or("other", |(_, n)| n)
}

/// The result of ResolvePlural: `[[PluralCategory]]` and `[[FormattedString]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Plural {
    category: PluralCategory,
    formatted: String,
}

/// ComputeExponentForMagnitude ( pluralRules, magnitude ) (§16.5.13) for the notation of
/// `r`: 0 for "standard", the magnitude for "scientific", the magnitude rounded down to a
/// multiple of 3 for "engineering", the ICU4X compact data for "compact" (a magnitude above
/// i16::MAX takes the exponent of i16::MAX: the top pattern). Integer arithmetic only (no
/// `log10`, no `pow`; INTL-V2-FP3).
fn exponent_for_magnitude(r: &Resolved, icu: &Icu, magnitude: i64) -> i64 {
    match r.notation() {
        "scientific" => magnitude,
        "engineering" => magnitude.div_euclid(3) * 3,
        "compact" => icu.compact.as_ref().map_or(0, |c| {
            let m = magnitude.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            i64::from(c.compact_exponent_for_magnitude(m))
        }),
        _ => 0,
    }
}

/// ComputeExponent ( pluralRules, x ) (§16.5.12) and the significand x × 10^-exponent rounded
/// by FormatNumericToString: as NumberFormat shows `x` in the notation of `r`.
fn compute_exponent<D: DecimalValue>(r: &Resolved, icu: &Icu, x: &D) -> (i64, D) {
    let round = |e: i64| {
        let mut y = x.clone();
        y.multiply_pow10(-e);
        format_numeric_to_string(&r.digits, &mut y);
        y
    };
    // 1. If x = 0, return 0.
    if x.is_zero() {
        return (0, round(0));
    }
    // 3-6. The magnitude, its exponent, and x × 10^-exponent rounded.
    let magnitude = x.nonzero_magnitude_start();
    let e = exponent_for_magnitude(r, icu, magnitude);
    let y = round(e);
    // 7-9. A rounding that keeps the magnitude keeps the exponent.
    if y.is_zero() || y.nonzero_magnitude_start() == magnitude - e {
        return (e, y);
    }
    // 10. Return ComputeExponentForMagnitude(magnitude + 1).
    let e = exponent_for_magnitude(r, icu, magnitude + 1);
    (e, round(e))
}

/// The plural operands of `x` in the notation of `r` (the operand `c`/`e` is the exponent
/// of compact, scientific and engineering notation).
///
/// The operands are those of the shown value, so the zero padding of
/// `[[MinimumIntegerDigits]]` is not in `i` (ICU4X marks a value with a shown digit above
/// magnitude 17 with i = its low 18 digits + 10^18). A significand that is zero (and
/// negative zero: the sign is not an operand) gives i = f = t = w = 0, the `v` of its shown
/// fraction digits and `c` = the exponent, at most 255: every CLDR rule compares `e` with
/// 0..5 at most (ICU4X `from_significand_and_exponent` marks a zero with an exponent of 18 or
/// more as a large value).
///
/// A negative exponent (scientific or engineering notation of |x| < 1) has no ICU4X operand
/// (`c` is unsigned): such a value takes the operands of its rounded value with `c` = 0. So
/// does an exponent above 255 (`c` is a u8). A WideDecimal gives its operands through
/// [`DecimalValue::plural_decimal`] (ICU4X keeps at most 18 integer digits).
fn operands<D: DecimalValue>(r: &Resolved, icu: &Icu, x: &D) -> PluralOperands {
    let (e, mut y) = compute_exponent(r, icu, x);
    y.trim_start();
    if y.is_zero() {
        let (lower, _) = y.shown();
        return PluralOperands::from(RawPluralOperands {
            i: 0,
            v: (-(lower + e)).clamp(0, 18) as usize,
            w: 0,
            f: 0,
            t: 0,
            c: e.clamp(0, 255) as usize,
        });
    }
    if let Ok(e) = u8::try_from(e) {
        return PluralOperands::from(&CompactDecimal::from_significand_and_exponent(
            y.plural_decimal(),
            e,
        ));
    }
    y.multiply_pow10(e);
    PluralOperands::from(&y.plural_decimal())
}

/// Steps 4-11 of ResolvePlural on the finite value `x`.
fn resolve_finite<D: DecimalValue>(r: &Resolved, icu: &Icu, x: &D) -> Plural {
    // 4-5. Let res be FormatNumericToString(pluralRules, n); let s be res.[[FormattedString]].
    let mut s = x.clone();
    format_numeric_to_string(&r.digits, &mut s);
    // 6-10. Let p be PluralRuleSelect(locale, type, notation, compactDisplay, s).
    let category = icu.category_for(operands(r, icu, x));
    // 11. Return the Record { [[PluralCategory]]: p, [[FormattedString]]: s }. The text has
    //     no sign: FormatNumericToString (§16.5.2) makes it from the unsigned value, so -1
    //     and 1 (and -0 and 0) have the same text. The rounding above kept the sign.
    Plural {
        category,
        formatted: s.unsigned_text(),
    }
}

/// ResolvePlural ( pluralRules, n ) (17.5.2) on the Intl mathematical value `n`.
fn resolve_plural(r: &Resolved, icu: &Icu, n: &IntlMv) -> Plural {
    // 1-3. NaN and the infinities: "other" (the text only takes part in the equality test of
    //      ResolvePluralRange).
    match n {
        IntlMv::Finite(x) => resolve_finite(r, icu, x),
        IntlMv::Wide(x) => resolve_finite(r, icu, x),
        IntlMv::NaN => other("NaN"),
        IntlMv::PosInf => other("Infinity"),
        IntlMv::NegInf => other("-Infinity"),
    }
}

/// The ResolvePlural record of a non-finite value.
fn other(formatted: &str) -> Plural {
    Plural {
        category: PluralCategory::Other,
        formatted: formatted.to_string(),
    }
}

/// ResolvePluralRange ( pluralRules, x, y ) (17.5.4) after the NaN test of step 1.
fn resolve_plural_range(r: &Resolved, icu: &Icu, x: &IntlMv, y: &IntlMv) -> PluralCategory {
    // 2-3.
    let xp = resolve_plural(r, icu, x);
    let yp = resolve_plural(r, icu, y);
    // 4. If xp.[[FormattedString]] is yp.[[FormattedString]], return xp.[[PluralCategory]].
    if xp.formatted == yp.formatted {
        return xp.category;
    }
    // 5-9. Return PluralRuleSelectRange(locale, type, notation, compactDisplay, xp, yp).
    icu.resolve_range(xp.category, yp.category)
}

/// The cached ICU4X objects of `r`, or a TypeError if the data has none (not reached: the
/// "und" data is there).
fn icu_or_throw(cx: &mut Cx<'_>, r: &Resolved) -> JsResult<Rc<Icu>> {
    match icu_for(r) {
        Some(icu) => Ok(icu),
        None => Err(cx.throw_type_error("Intl.PluralRules: no plural data")),
    }
}

/// Intl.PluralRules.prototype.select ( value ) (17.3.3).
fn select<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let pr be the this value; ? RequireInternalSlot(pr, [[InitializedPluralRules]]).
    let r = read(cx, this)?;
    // 3. Let n be ? ToIntlMathematicalValue(value).
    let n = to_intl_mv(cx, args.get(0))?;
    // 4. Return ResolvePlural(pr, n).[[PluralCategory]].
    let icu = icu_or_throw(cx, &r)?;
    let p = resolve_plural(&r, &icu, &n);
    Ok(cx.new_string(category_name(p.category))?.into())
}

/// Intl.PluralRules.prototype.selectRange ( start, end ) (17.3.4).
fn select_range<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let pr be the this value; ? RequireInternalSlot(pr, [[InitializedPluralRules]]).
    let r = read(cx, this)?;
    // 3. If start is undefined or end is undefined, throw a TypeError exception.
    let (start, end) = (args.get(0), args.get(1));
    if start.is_undefined() || end.is_undefined() {
        return Err(cx.throw_type_error(
            "Intl.PluralRules.prototype.selectRange: start and end must not be undefined",
        ));
    }
    // 4-5. Let x be ? ToIntlMathematicalValue(start); let y be ? ToIntlMathematicalValue(end).
    let x = to_intl_mv(cx, start)?;
    let y = to_intl_mv(cx, end)?;
    // 6. Return ? ResolvePluralRange(pr, x, y). Step 1: NaN is a RangeError.
    if x == IntlMv::NaN || y == IntlMv::NaN {
        return Err(cx.throw_range_error(
            "Intl.PluralRules.prototype.selectRange: start and end must not be NaN",
        ));
    }
    let icu = icu_or_throw(cx, &r)?;
    let c = resolve_plural_range(&r, &icu, &x, &y);
    Ok(cx.new_string(category_name(c))?.into())
}

/// The plural categories of `icu` in the order zero, one, two, few, many, other (17.3.2 step
/// 4: every result of PluralRuleSelect for the locale and type).
fn plural_categories(icu: &Icu) -> Vec<&'static str> {
    let have = icu.categories();
    CATEGORIES
        .iter()
        .filter(|(c, _)| have.contains(c))
        .map(|(_, n)| *n)
        .collect()
}

/// Intl.PluralRules.prototype.resolvedOptions ( ) (17.3.2).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let pr be the this value; ? RequireInternalSlot(pr, [[InitializedPluralRules]]).
    let r = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. Let pluralCategories be the categories of PluralRuleSelect, in the order "zero",
    //    "one", "two", "few", "many", "other".
    let icu = icu_or_throw(cx, &r)?;
    let categories = plural_categories(&icu);
    // 5. For each row of Table 32, in table order: the slots that are not undefined.
    let put_str = |cx: &mut Cx<'s>, key: &str, v: &str| -> JsResult<()> {
        let v = cx.new_string(v)?;
        cx.create_data_property_or_throw(o, key, v)
    };
    let put_int = |cx: &mut Cx<'s>, key: &str, v: u16| -> JsResult<()> {
        cx.create_data_property_or_throw(o, key, Local::int32(i32::from(v)))
    };
    let d = r.digits;
    put_str(cx, "locale", &r.locale)?;
    put_str(cx, "type", TYPES[r.plural_type as usize])?;
    put_str(cx, "notation", r.notation())?;
    if let Some(cd) = r.compact_display() {
        put_str(cx, "compactDisplay", cd)?;
    }
    put_int(
        cx,
        "minimumIntegerDigits",
        u16::from(d.minimum_integer_digits),
    )?;
    // SetNumberFormatDigitOptions sets the fraction digits unless the rounding type is
    // significantDigits, and the significant digits unless it is fractionDigits.
    if d.rounding_type != RoundingType::SignificantDigits {
        put_int(
            cx,
            "minimumFractionDigits",
            u16::from(d.minimum_fraction_digits),
        )?;
        put_int(
            cx,
            "maximumFractionDigits",
            u16::from(d.maximum_fraction_digits),
        )?;
    }
    if d.rounding_type != RoundingType::FractionDigits {
        put_int(
            cx,
            "minimumSignificantDigits",
            u16::from(d.minimum_significant_digits),
        )?;
        put_int(
            cx,
            "maximumSignificantDigits",
            u16::from(d.maximum_significant_digits),
        )?;
    }
    // A new Array on every call.
    let list = crate::intl::string_array(cx, &categories)?;
    cx.create_data_property_or_throw(o, "pluralCategories", list)?;
    put_int(cx, "roundingIncrement", d.rounding_increment)?;
    put_str(cx, "roundingMode", d.rounding_mode.name())?;
    put_str(cx, "roundingPriority", d.computed_rounding_priority)?;
    put_str(
        cx,
        "trailingZeroDisplay",
        if d.strip_if_integer {
            "stripIfInteger"
        } else {
            "auto"
        },
    )?;
    // 6. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digits::WideDecimal;
    use crate::number_format::decimal::decimal_from_literal;

    fn resolved(locale: &str, plural_type: u32, notation: &str, long: bool) -> Resolved {
        let compact = notation == "compact";
        Resolved {
            locale: locale.to_string(),
            plural_type,
            notation: NOTATIONS.iter().position(|n| *n == notation).unwrap() as u32,
            compact_display: u32::from(long),
            digits: DigitOptions {
                minimum_integer_digits: 1,
                minimum_fraction_digits: 0,
                maximum_fraction_digits: if compact { 0 } else { 3 },
                minimum_significant_digits: 1,
                maximum_significant_digits: if compact { 2 } else { 21 },
                rounding_increment: 1,
                rounding_mode: RoundingMode::HalfExpand,
                rounding_type: if compact {
                    RoundingType::MorePrecision
                } else {
                    RoundingType::FractionDigits
                },
                computed_rounding_priority: if compact { "morePrecision" } else { "auto" },
                strip_if_integer: false,
            },
        }
    }

    fn sel(r: &Resolved, x: &str) -> &'static str {
        let icu = icu_for(r).unwrap();
        let x = decimal_from_literal(x).unwrap();
        category_name(resolve_plural(r, &icu, &IntlMv::Finite(x)).category)
    }

    /// The packed slots give back the same options.
    #[test]
    fn pack_round_trip() {
        let mut r = resolved("fr", 1, "compact", true);
        r.digits.rounding_mode = RoundingMode::HalfEven;
        r.digits.strip_if_integer = true;
        r.digits.rounding_increment = 25;
        let back = Resolved::unpack(
            r.locale.clone(),
            r.pack_options() as u32,
            r.pack_digits() as u32,
            25,
        );
        assert_eq!(back, r);
    }

    /// The lines of node v26.13.0 `new Intl.PluralRules(l, {notation}).select(x)`, except a
    /// negative exponent (see [`operands`]).
    #[test]
    fn notation_exponents_as_node() {
        let fr = resolved("fr", 0, "standard", false);
        assert_eq!(
            ["1000000", "1500000", "0.000001"].map(|x| sel(&fr, x)),
            ["many", "other", "one"]
        );
        let frc = resolved("fr", 0, "compact", false);
        assert_eq!(
            [
                "1000000",
                "1500000",
                "0.000001",
                "1000",
                "1500",
                "2000000000",
                "0"
            ]
            .map(|x| sel(&frc, x)),
            ["many", "many", "one", "other", "other", "many", "one"]
        );
        let frs = resolved("fr", 0, "scientific", false);
        assert_eq!(
            ["1000000", "1500000", "1000", "2", "1.5", "999990"].map(|x| sel(&frs, x)),
            ["many", "many", "other", "other", "one", "many"]
        );
        let fre = resolved("fr", 0, "engineering", false);
        assert_eq!(
            ["100000", "10000000", "999990", "1100000"].map(|x| sel(&fre, x)),
            ["other", "many", "other", "many"]
        );
        let ar = resolved("ar", 0, "scientific", false);
        assert_eq!(
            ["1", "2", "10", "100"].map(|x| sel(&ar, x)),
            ["one", "two", "few", "other"]
        );
    }

    /// A significand that rounds to zero, and the zero padding of minimumIntegerDigits: the
    /// shown value is 0 (or 1), so the operands are those of 0 (or 1), with `c` the exponent
    /// (at most 255). The lines of node v26.13.0 `new Intl.PluralRules(l, o).select(x)` with
    /// o = {notation, roundingIncrement: 5, maximumFractionDigits: 0} or
    /// {minimumIntegerDigits: 21}. Node throws a TypeError for a BigInt, so the WideDecimal
    /// lines (10^32768) take the result of the Number line of the same shape (1e300).
    ///
    /// Not as node: "fr" with 0E6 and up. The CLDR rules "one: i = 0,1" and "many: ... or
    /// e != 0..5" both hold for i = 0, e >= 6; ICU4X tests `one` first, node (ICU4C) gives
    /// "many". The old code gave "one" for 0E6 to 0E17 and "many" from 0E18 on (through the
    /// 10^18 mark of ICU4X, not through `e`); now each exponent gives "one".
    #[test]
    fn rounded_zero_and_padding_as_node() {
        let inc5 = |l: &str, notation: &str| {
            let mut r = resolved(l, 0, notation, false);
            r.digits.rounding_increment = 5;
            r.digits.maximum_fraction_digits = 0;
            r
        };
        let (ar, are) = (inc5("ar", "scientific"), inc5("ar", "engineering"));
        assert_eq!(
            [
                sel(&ar, "1e30"),
                sel(&ar, "-1e30"),
                sel(&are, "1e30"),
                sel(&ar, "1e17")
            ],
            ["zero", "zero", "zero", "zero"]
        );
        let lag = inc5("lag", "scientific");
        assert_eq!(
            ["1e30", "-1e30", "1e300"].map(|x| sel(&lag, x)),
            ["zero", "zero", "zero"]
        );
        let fr = inc5("fr", "scientific");
        assert_eq!(
            ["1000", "1e10", "1e30", "1e300"].map(|x| sel(&fr, x)),
            ["one", "one", "one", "one"]
        );
        let pad = |l: &str| {
            let mut r = resolved(l, 0, "standard", false);
            r.digits.minimum_integer_digits = 21;
            r
        };
        assert_eq!(
            [
                sel(&pad("en"), "1"),
                sel(&pad("ar"), "0"),
                sel(&pad("lag"), "-0")
            ],
            ["one", "zero", "zero"]
        );
        // 10^32768 and -10^32768: a WideDecimal.
        let wide = |r: &Resolved, negative: bool| {
            let mut digits = vec![b'1'];
            digits.extend(std::iter::repeat_n(b'0', 32_768));
            let x = IntlMv::Wide(WideDecimal::from_integer_digits(negative, &digits));
            category_name(resolve_plural(r, &icu_for(r).unwrap(), &x).category)
        };
        assert_eq!(
            [wide(&lag, false), wide(&lag, true), wide(&ar, false)],
            ["zero", "zero", "zero"]
        );
    }

    /// The categories in the spec order, and the ordinal rules.
    #[test]
    fn categories_in_order() {
        let cats = |l: &str, t: u32| {
            let r = resolved(l, t, "standard", false);
            plural_categories(&icu_for(&r).unwrap()).join(",")
        };
        assert_eq!(cats("ar", 0), "zero,one,two,few,many,other");
        assert_eq!(cats("fr", 0), "one,many,other");
        assert_eq!(cats("sl", 0), "one,two,few,other");
        assert_eq!(cats("ko", 0), "other");
        // A script variant whose CLDR parent is root takes the rules of its language.
        assert_eq!(cats("sr-Latn", 0), "one,few,other");
        assert_eq!(cats("az-Cyrl", 0), "one,other");
        assert_eq!(cats("uz-Cyrl", 0), "one,other");
        assert_eq!(cats("bs-Cyrl", 0), "one,few,other");
        assert_eq!(cats("zh-Hant", 0), "other");
        assert_eq!(cats("en", 1), "one,two,few,other");
        // A language that the ICU4X data does not have: the CLDR rules (cldr_extra).
        assert_eq!(cats("gv", 0), "one,two,few,many,other");
        assert_eq!(cats("kw", 0), "zero,one,two,few,many,other");
        assert_eq!(cats("kw", 1), "one,many,other");
        assert_eq!(cats("lag", 0), "zero,one,other");
        assert_eq!(cats("ff-Adlm", 0), "one,other");
        let en = resolved("en", 1, "standard", false);
        assert_eq!(
            ["1", "2", "3", "4", "11", "21", "22", "23", "111"].map(|x| sel(&en, x)),
            [
                "one", "two", "few", "other", "other", "one", "two", "few", "other"
            ]
        );
    }

    /// Ranges: the CLDR range data, and the same formatted text gives the start category.
    #[test]
    fn ranges() {
        let range = |l: &str, x: &str, y: &str| {
            let r = resolved(l, 0, "standard", false);
            let icu = icu_for(&r).unwrap();
            let mv = |s: &str| IntlMv::Finite(decimal_from_literal(s).unwrap());
            category_name(resolve_plural_range(&r, &icu, &mv(x), &mv(y)))
        };
        assert_eq!(range("sl", "102", "201"), "few");
        assert_eq!(range("sl", "1", "2"), "two");
        assert_eq!(range("ru", "1", "2"), "few");
        assert_eq!(range("ru", "1", "5"), "many");
        assert_eq!(range("ru", "2", "1"), "one");
        assert_eq!(range("en", "102", "201"), "other");
        // Same text: the start category (node gives the range category "few" and "other").
        assert_eq!(range("sl", "1", "1"), "one");
        assert_eq!(range("en", "1", "1.0001"), "one");
    }
}
