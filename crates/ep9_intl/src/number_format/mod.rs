//! Intl.NumberFormat (ECMA-402 §16; block 34): `%Intl.NumberFormat%` and
//! `%Intl.NumberFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.NumberFormat {w, c}     %Intl.NumberFormat%  prototype {}, supportedLocalesOf
//! Intl.NumberFormat.prototype  %Intl.NumberFormat.prototype%  constructor {w, c},
//!                                resolvedOptions, formatToParts, get format,
//!                                [@@toStringTag] "Intl.NumberFormat" {c}
//! slots (ObjectClass::IntlNumberFormat): 0 [[Locale]], 1 the packed options, 2 the packed
//!   digit options, 3 [[RoundingIncrement]], 4 [[NumberingSystem]], 5 [[Currency]],
//!   6 [[Unit]], 7 [[BoundFormat]]
//! format(x) ─ ToIntlMathematicalValue ─ FormatNumeric ─ decimal::partition (ICU4X data
//!   through crate::cache at the first format; no ICU4X object in a slot, INTL-I1-D7)
//! ```
//!
//! INTL W2 NF-CORE: the constructor with every option of §16.1 (the style options too, so
//! `resolvedOptions` is complete), `supportedLocalesOf`, `format`, `formatToParts` and
//! `resolvedOptions`; the decimal style in standard and compact notation. W3 fills the rest
//! of FormatNumeric through [`partition_numeric`]: the styles percent and currency
//! (NF-CURRENCY, with [`currency_digits`]), unit (NF-UNIT), the notations scientific and
//! engineering (NF-SCIRANGE); until then they format as the decimal style. `formatRange` and
//! `formatRangeToParts` (NF-SCIRANGE) are not defined yet (ids append-only, after 4).
//! `Number.prototype.toLocaleString` and `BigInt.prototype.toLocaleString` call
//! [`to_locale_string`].
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-number_format.txt`.

pub mod decimal;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef,
    PropKey, WellKnownSymbol,
};

use crate::available;
use crate::digits::{DigitOptions, RoundingMode, RoundingType, set_number_format_digit_options};
use crate::options::{
    BoolOrString, coerce_options_to_object, get_boolean_or_string_number_format_option,
    get_locale_matcher, get_string_option,
};
use crate::resolve;
use crate::tag;
use decimal::{Format, Grouping, IntlMv, Parts, SignDisplay};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 34;

/// The class of an Intl.NumberFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlNumberFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the packed options (`Resolved::pack_options`, an int32).
pub const SLOT_OPTIONS: usize = 1;
/// Slot 2: the packed digit options (`Resolved::pack_digits`, an int32).
pub const SLOT_DIGITS: usize = 2;
/// Slot 3: `[[RoundingIncrement]]` (an int32).
pub const SLOT_INCREMENT: usize = 3;
/// Slot 4: `[[NumberingSystem]]` (a String).
pub const SLOT_NUMBERING_SYSTEM: usize = 4;
/// Slot 5: `[[Currency]]` (a String; undefined unless the style is "currency").
pub const SLOT_CURRENCY: usize = 5;
/// Slot 6: `[[Unit]]` (a String; undefined unless the style is "unit").
pub const SLOT_UNIT: usize = 6;
/// Slot 7: `[[BoundFormat]]` (undefined until the first `format` get).
pub const SLOT_BOUND_FORMAT: usize = 7;
/// The slot count of an Intl.NumberFormat object.
pub const SLOTS: usize = 8;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const GET_FORMAT: u16 = 2;
    pub const FORMAT_TO_PARTS: u16 = 3;
    pub const RESOLVED_OPTIONS: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("NumberFormat", 0, constructor), // 0 (16.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (16.2.2)
    BuiltinDef::getter("format", get_format),                // 2 (16.3.3)
    BuiltinDef::method("formatToParts", 1, format_to_parts), // 3 (16.3.6)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 4 (16.3.2)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlNumberFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlNumberFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlNumberFormat,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlNumberFormatPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlNumberFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlNumberFormat,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, formatToParts, format.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("formatToParts"), def::FORMAT_TO_PARTS),
            // get Intl.NumberFormat.prototype.format ({c}, 16.3.3).
            PropDef::accessor(PropKey::Str("format"), Some(def::GET_FORMAT), None),
            // Intl.NumberFormat.prototype [ @@toStringTag ] ({c}, 16.3.7).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.NumberFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("NumberFormat"),
            IntrinsicId::IntlNumberFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "number_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Style]]` values, in their packed order.
const STYLES: [&str; 4] = ["decimal", "percent", "currency", "unit"];
/// `[[CurrencyDisplay]]` values.
const CURRENCY_DISPLAYS: [&str; 4] = ["code", "symbol", "narrowSymbol", "name"];
/// `[[CurrencySign]]` values.
const CURRENCY_SIGNS: [&str; 2] = ["standard", "accounting"];
/// `[[UnitDisplay]]` values.
const UNIT_DISPLAYS: [&str; 3] = ["short", "narrow", "long"];
/// `[[Notation]]` values.
const NOTATIONS: [&str; 4] = ["standard", "scientific", "engineering", "compact"];
/// `[[CompactDisplay]]` values.
const COMPACT_DISPLAYS: [&str; 2] = ["short", "long"];
/// `[[SignDisplay]]` values.
const SIGN_DISPLAYS: [&str; 5] = ["auto", "never", "always", "exceptZero", "negative"];
/// The `[[RoundingMode]]` of each name of `RoundingMode::NAMES`, in that order.
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
/// `[[ComputedRoundingPriority]]` values.
const PRIORITIES: [&str; 3] = ["auto", "morePrecision", "lessPrecision"];
/// `[[UseGrouping]]` values, in their packed order (0: false).
const GROUPINGS: [Grouping; 4] = [
    Grouping::Never,
    Grouping::Always,
    Grouping::Auto,
    Grouping::Min2,
];
/// `[[SignDisplay]]` values, in the order of [`SIGN_DISPLAYS`].
const SIGNS: [SignDisplay; 5] = [
    SignDisplay::Auto,
    SignDisplay::Never,
    SignDisplay::Always,
    SignDisplay::ExceptZero,
    SignDisplay::Negative,
];

/// The index of `v` in `list` (0 if absent).
fn index_of<T: PartialEq>(list: &[T], v: &T) -> u32 {
    list.iter().position(|x| x == v).unwrap_or(0) as u32
}

/// The internal slots of a NumberFormat (§16.4), unpacked.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[NumberingSystem]]`.
    pub numbering_system: String,
    /// `[[Style]]`: an index in `STYLES`.
    pub style: u32,
    /// `[[Currency]]` (upper case), style "currency" only.
    pub currency: Option<String>,
    /// `[[CurrencyDisplay]]`, `[[CurrencySign]]`: indices.
    pub currency_display: u32,
    pub currency_sign: u32,
    /// `[[Unit]]`, style "unit" only.
    pub unit: Option<String>,
    /// `[[UnitDisplay]]`: an index.
    pub unit_display: u32,
    /// The digit options of SetNumberFormatDigitOptions.
    pub digits: DigitOptions,
    /// `[[Notation]]`, `[[CompactDisplay]]`: indices.
    pub notation: u32,
    pub compact_display: u32,
    /// `[[UseGrouping]]`.
    pub grouping: Grouping,
    /// `[[SignDisplay]]`.
    pub sign_display: SignDisplay,
}

impl Resolved {
    /// The packed options (slot 1):
    ///
    /// ```text
    /// bits 0-1   [[Style]]            bits 2-3   [[CurrencyDisplay]]   bit 4  [[CurrencySign]]
    /// bits 5-6   [[UnitDisplay]]      bits 7-8   [[Notation]]          bit 9  [[CompactDisplay]]
    /// bits 10-11 [[UseGrouping]]      bits 12-14 [[SignDisplay]]       bits 15-18 [[RoundingMode]]
    /// bits 19-20 [[RoundingType]]     bits 21-22 [[ComputedRoundingPriority]]
    /// bit 23     [[TrailingZeroDisplay]] "stripIfInteger"
    /// ```
    fn pack_options(&self) -> i32 {
        let d = &self.digits;
        (self.style
            | self.currency_display << 2
            | self.currency_sign << 4
            | self.unit_display << 5
            | self.notation << 7
            | self.compact_display << 9
            | index_of(&GROUPINGS, &self.grouping) << 10
            | index_of(&SIGNS, &self.sign_display) << 12
            | index_of(&ROUNDING_MODES, &d.rounding_mode) << 15
            | index_of(&ROUNDING_TYPES, &d.rounding_type) << 19
            | index_of(&PRIORITIES, &d.computed_rounding_priority) << 21
            | u32::from(d.strip_if_integer) << 23) as i32
    }

    /// The packed digit counts (slot 2): `[[MinimumIntegerDigits]]` bits 0-4,
    /// `[[MinimumFractionDigits]]` 5-11, `[[MaximumFractionDigits]]` 12-18,
    /// `[[MinimumSignificantDigits]]` 19-23, `[[MaximumSignificantDigits]]` 24-28.
    fn pack_digits(&self) -> i32 {
        let d = &self.digits;
        (u32::from(d.minimum_integer_digits)
            | u32::from(d.minimum_fraction_digits) << 5
            | u32::from(d.maximum_fraction_digits) << 12
            | u32::from(d.minimum_significant_digits) << 19
            | u32::from(d.maximum_significant_digits) << 24) as i32
    }

    /// The fields of FormatNumeric for the decimal style; `data_locale` is `[[Locale]]`
    /// without its Unicode extension.
    fn format<'a>(&'a self, data_locale: &'a str) -> Format<'a> {
        Format {
            data_locale,
            numbering_system: &self.numbering_system,
            digits: self.digits,
            compact: (NOTATIONS[self.notation as usize] == "compact")
                .then_some(self.compact_display == 1),
            grouping: self.grouping,
            sign_display: self.sign_display,
        }
    }
}

/// Writes `r` into the slots of the NumberFormat `nf`.
fn store<'s>(cx: &mut Cx<'s>, nf: Local<'s, JsObject>, r: &Resolved) -> JsResult<()> {
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(nf, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(nf, CLASS, SLOT_OPTIONS, Local::int32(r.pack_options()))?;
    cx.set_slot(nf, CLASS, SLOT_DIGITS, Local::int32(r.pack_digits()))?;
    cx.set_slot(
        nf,
        CLASS,
        SLOT_INCREMENT,
        Local::int32(i32::from(r.digits.rounding_increment)),
    )?;
    let nu = cx.new_string(&r.numbering_system)?;
    cx.set_slot(nf, CLASS, SLOT_NUMBERING_SYSTEM, nu)?;
    if let Some(c) = &r.currency {
        let c = cx.new_string(c)?;
        cx.set_slot(nf, CLASS, SLOT_CURRENCY, c)?;
    }
    if let Some(u) = &r.unit {
        let u = cx.new_string(u)?;
        cx.set_slot(nf, CLASS, SLOT_UNIT, u)?;
    }
    Ok(())
}

/// The resolved options of the NumberFormat `nf` (TypeError if `nf` is not one:
/// RequireInternalSlot(nf, `[[InitializedNumberFormat]]`)).
fn read<'s>(cx: &mut Cx<'s>, nf: Local<'s>) -> JsResult<Resolved> {
    let locale = cx.slot(nf, CLASS, SLOT_LOCALE)?;
    let locale = cx.to_rust_string(locale)?;
    let o = cx.slot(nf, CLASS, SLOT_OPTIONS)?.as_int32().unwrap_or(0) as u32;
    let d = cx.slot(nf, CLASS, SLOT_DIGITS)?.as_int32().unwrap_or(0) as u32;
    let increment = cx.slot(nf, CLASS, SLOT_INCREMENT)?.as_int32().unwrap_or(1);
    let nu = cx.slot(nf, CLASS, SLOT_NUMBERING_SYSTEM)?;
    let numbering_system = cx.to_rust_string(nu)?;
    let currency = cx.slot(nf, CLASS, SLOT_CURRENCY)?;
    let currency = if currency.is_undefined() {
        None
    } else {
        Some(cx.to_rust_string(currency)?)
    };
    let unit = cx.slot(nf, CLASS, SLOT_UNIT)?;
    let unit = if unit.is_undefined() {
        None
    } else {
        Some(cx.to_rust_string(unit)?)
    };
    let field = |shift: u32, bits: u32| (o >> shift) & ((1 << bits) - 1);
    let dfield = |shift: u32, bits: u32| ((d >> shift) & ((1 << bits) - 1)) as u8;
    let pick = |i: u32, n: usize| (i as usize).min(n - 1);
    Ok(Resolved {
        locale,
        numbering_system,
        style: field(0, 2),
        currency,
        currency_display: field(2, 2),
        currency_sign: field(4, 1),
        unit,
        unit_display: field(5, 2).min(2),
        digits: DigitOptions {
            minimum_integer_digits: dfield(0, 5),
            minimum_fraction_digits: dfield(5, 7),
            maximum_fraction_digits: dfield(12, 7),
            minimum_significant_digits: dfield(19, 5),
            maximum_significant_digits: dfield(24, 5),
            rounding_increment: increment as u16,
            rounding_mode: ROUNDING_MODES[pick(field(15, 4), 9)],
            rounding_type: ROUNDING_TYPES[pick(field(19, 2), 4)],
            computed_rounding_priority: PRIORITIES[pick(field(21, 2), 3)],
            strip_if_integer: field(23, 1) != 0,
        },
        notation: field(7, 2),
        compact_display: field(9, 1),
        grouping: GROUPINGS[pick(field(10, 2), 4)],
        sign_display: SIGNS[pick(field(12, 3), 5)],
    })
}

// ---- InitializeNumberFormat (in Intl.NumberFormat, 16.1.1) ------------------------------------

/// Whether `s` matches the Unicode locale identifier `type` nonterminal: alphanum{3,8}
/// subtags joined by "-" (UTS 35 §3.2).
fn is_unicode_type(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|t| (3..=8).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// IsWellFormedCurrencyCode ( currency ) (6.3.1): three ASCII letters.
fn is_well_formed_currency_code(c: &str) -> bool {
    c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic())
}

/// IsWellFormedUnitIdentifier ( unitIdentifier ) (6.6.1): a sanctioned simple unit, or two
/// joined by "-per-".
fn is_well_formed_unit_identifier(u: &str) -> bool {
    let simple = |s: &str| available::UNITS.contains(&s);
    if simple(u) {
        return true;
    }
    match u.find("-per-") {
        Some(i) => simple(&u[..i]) && simple(&u[i + 5..]),
        None => false,
    }
}

/// CurrencyDigits ( currency ) (16.5.1). Stub: 2 for every currency until W3 NF-CURRENCY
/// fills it with the ISO 4217 minor units.
pub fn currency_digits(_currency: &str) -> u8 {
    2
}

/// `localeData.[[<locale>]].[[nu]]` (§16.2.3): the default numbering system of the locale
/// first, then every numbering system with a simple digit mapping (ECMA-402 Table 14).
fn nu_data(data_locale: &str) -> resolve::KeyData {
    let first = decimal::default_numbering_system(data_locale);
    let mut v = vec![first.clone()];
    v.extend(
        available::NUMBERING_SYSTEMS
            .iter()
            .filter(|n| **n != first)
            .map(|n| (*n).to_string()),
    );
    v
}

/// InitializeNumberFormat steps 2-31 (the current draft has these steps in Intl.NumberFormat,
/// 16.1.1; the step numbers follow the separate InitializeNumberFormat of earlier editions;
/// the object creation is the caller's): the
/// internal slots of a NumberFormat for `locales` and `options`, in the spec order of the
/// observable steps. No ICU4X object is made (the locale data reads the baked symbols).
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 1. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 2. Set options to ? CoerceOptionsToObject(options).
    let options = coerce_options_to_object(cx, options)?;
    // 4-5. Let matcher be ? GetOption(options, "localeMatcher", ...).
    get_locale_matcher(cx, options)?;
    // 6. Let numberingSystem be ? GetOption(options, "numberingSystem", string, empty,
    //    undefined).
    let nu = get_string_option(cx, options, "numberingSystem", &[], None)?;
    // 7. If numberingSystem is not undefined and does not match the type nonterminal, throw
    //    a RangeError exception.
    if let Some(n) = &nu
        && !is_unicode_type(n)
    {
        return Err(cx.throw_range_error(&format!("Invalid numberingSystem : {n}")));
    }
    // 9. Let r be ResolveLocale(%Intl.NumberFormat%.[[AvailableLocales]], requestedLocales,
    //    opt, « "nu" », localeData).
    let r = resolve::resolve_locale(cx, &requested, &[("nu", nu)], &["nu"], &|l, _| nu_data(l))?;
    let numbering_system = r
        .keywords
        .iter()
        .find(|(k, _)| k == "nu")
        .map_or_else(|| "latn".to_string(), |(_, v)| v.clone());
    // 11. Perform ? SetNumberFormatUnitOptions(numberFormat, options) (16.1.3).
    let unit_options = set_number_format_unit_options(cx, options)?;
    let style = unit_options.style;
    // 13. Let notation be ? GetOption(options, "notation", string, « "standard",
    //     "scientific", "engineering", "compact" », "standard").
    let notation = get_string_option(cx, options, "notation", &NOTATIONS, Some("standard"))?
        .unwrap_or_default();
    let notation = index_of(&NOTATIONS, &notation.as_str());
    // 15-16. The default fraction digits: the currency digits for style "currency" in
    //        standard notation; else 0 and 3 (0 for "percent").
    let (mnfd_default, mxfd_default) = if STYLES[style as usize] == "currency" && notation == 0 {
        let c = currency_digits(unit_options.currency.as_deref().unwrap_or(""));
        (c, c)
    } else if STYLES[style as usize] == "percent" {
        (0, 0)
    } else {
        (0, 3)
    };
    // 17. Perform ? SetNumberFormatDigitOptions(numberFormat, options, mnfdDefault,
    //     mxfdDefault, notation).
    let compact = NOTATIONS[notation as usize] == "compact";
    let digits = set_number_format_digit_options(cx, options, mnfd_default, mxfd_default, compact)?;
    // 18. Let compactDisplay be ? GetOption(options, "compactDisplay", string, « "short",
    //     "long" », "short").
    let compact_display = get_string_option(
        cx,
        options,
        "compactDisplay",
        &COMPACT_DISPLAYS,
        Some("short"),
    )?
    .unwrap_or_default();
    // 19-20. Let defaultUseGrouping be "auto"; "min2" in compact notation.
    let default_grouping = if compact { "min2" } else { "auto" };
    // 22-25. useGrouping: "true" and "false" give the default, true gives "always".
    let grouping = match get_boolean_or_string_number_format_option(
        cx,
        options,
        "useGrouping",
        &["min2", "auto", "always", "true", "false"],
        BoolOrString::Str(default_grouping.to_string()),
    )? {
        BoolOrString::True => Grouping::Always,
        BoolOrString::False => Grouping::Never,
        BoolOrString::Str(s) => match s.as_str() {
            "min2" => Grouping::Min2,
            "always" => Grouping::Always,
            _ => Grouping::Auto,
        },
    };
    // 26-27. Let signDisplay be ? GetOption(options, "signDisplay", string, « "auto",
    //        "never", "always", "exceptZero", "negative" », "auto").
    let sign_display = get_string_option(cx, options, "signDisplay", &SIGN_DISPLAYS, Some("auto"))?
        .unwrap_or_default();
    let sign_display = SIGNS[index_of(&SIGN_DISPLAYS, &sign_display.as_str()) as usize];
    Ok(Resolved {
        locale: r.locale,
        numbering_system,
        style,
        currency: unit_options.currency,
        currency_display: unit_options.currency_display,
        currency_sign: unit_options.currency_sign,
        unit: unit_options.unit,
        unit_display: unit_options.unit_display,
        digits,
        notation,
        // [[CompactDisplay]] is set only in compact notation (step 20); "short" otherwise,
        // which resolvedOptions does not show.
        compact_display: if compact {
            index_of(&COMPACT_DISPLAYS, &compact_display.as_str())
        } else {
            0
        },
        grouping,
        sign_display,
    })
}

/// The result of SetNumberFormatUnitOptions.
struct UnitOptions {
    style: u32,
    currency: Option<String>,
    currency_display: u32,
    currency_sign: u32,
    unit: Option<String>,
    unit_display: u32,
}

/// SetNumberFormatUnitOptions ( intlObj, options ) (16.1.3).
fn set_number_format_unit_options<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
) -> JsResult<UnitOptions> {
    // 1. Let style be ? GetOption(options, "style", string, « "decimal", "percent",
    //    "currency", "unit" », "decimal").
    let style =
        get_string_option(cx, options, "style", &STYLES, Some("decimal"))?.unwrap_or_default();
    // 3. Let currency be ? GetOption(options, "currency", string, empty, undefined).
    let currency = get_string_option(cx, options, "currency", &[], None)?;
    match &currency {
        // 4. If currency is undefined and style is "currency", throw a TypeError exception.
        None if style == "currency" => {
            return Err(cx.throw_type_error("Currency code is required with currency style."));
        }
        // 5. If IsWellFormedCurrencyCode(currency) is false, throw a RangeError exception.
        Some(c) if !is_well_formed_currency_code(c) => {
            return Err(cx.throw_range_error(&format!("Invalid currency code : {c}")));
        }
        _ => {}
    }
    // 6. Let currencyDisplay be ? GetOption(options, "currencyDisplay", string, « "code",
    //    "symbol", "narrowSymbol", "name" », "symbol").
    let currency_display = get_string_option(
        cx,
        options,
        "currencyDisplay",
        &CURRENCY_DISPLAYS,
        Some("symbol"),
    )?
    .unwrap_or_default();
    // 7. Let currencySign be ? GetOption(options, "currencySign", string, « "standard",
    //    "accounting" », "standard").
    let currency_sign = get_string_option(
        cx,
        options,
        "currencySign",
        &CURRENCY_SIGNS,
        Some("standard"),
    )?
    .unwrap_or_default();
    // 8. Let unit be ? GetOption(options, "unit", string, empty, undefined).
    let unit = get_string_option(cx, options, "unit", &[], None)?;
    match &unit {
        // 9. If unit is undefined and style is "unit", throw a TypeError exception.
        None if style == "unit" => {
            return Err(cx.throw_type_error("Unit is required with unit style."));
        }
        // 10. If IsWellFormedUnitIdentifier(unit) is false, throw a RangeError exception.
        Some(u) if !is_well_formed_unit_identifier(u) => {
            return Err(cx.throw_range_error(&format!("Invalid unit argument : {u}")));
        }
        _ => {}
    }
    // 11. Let unitDisplay be ? GetOption(options, "unitDisplay", string, « "short",
    //     "narrow", "long" », "short").
    let unit_display =
        get_string_option(cx, options, "unitDisplay", &UNIT_DISPLAYS, Some("short"))?
            .unwrap_or_default();
    // 12-13. The currency fields for style "currency" (the code in upper case), the unit
    //        fields for style "unit".
    let is_currency = style == "currency";
    let is_unit = style == "unit";
    Ok(UnitOptions {
        style: index_of(&STYLES, &style.as_str()),
        currency: currency
            .filter(|_| is_currency)
            .map(|c| c.to_ascii_uppercase()),
        currency_display: index_of(&CURRENCY_DISPLAYS, &currency_display.as_str()),
        currency_sign: index_of(&CURRENCY_SIGNS, &currency_sign.as_str()),
        unit: unit.filter(|_| is_unit),
        unit_display: index_of(&UNIT_DISPLAYS, &unit_display.as_str()),
    })
}

// ---- FormatNumeric (16.5.5) -------------------------------------------------------------------

/// PartitionNumberPattern ( numberFormat, x ) (16.5.3): the parts of `x` formatted with `r`.
///
/// The decimal style in standard and compact notation is NF-CORE's. The W3 builders fill the
/// other branches here: NF-CURRENCY the styles "percent" and "currency", NF-UNIT the style
/// "unit", NF-SCIRANGE the notations "scientific" and "engineering". Until then those
/// format as the decimal style in standard notation (a stub path, not a spec result).
pub fn partition_numeric(r: &Resolved, x: &IntlMv) -> Parts {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let mut f = r.format(&data_locale);
    let standard_decimal = STYLES[r.style as usize] == "decimal"
        && matches!(NOTATIONS[r.notation as usize], "standard" | "compact");
    if !standard_decimal {
        // W3 stub path (see above).
        f.compact = None;
    }
    decimal::partition(&f, x)
}

/// FormatNumeric ( numberFormat, x ) (16.5.5).
pub fn format_numeric(r: &Resolved, x: &IntlMv) -> String {
    decimal::join(&partition_numeric(r, x))
}

/// The NumberFormat-like `toLocaleString` of `x` (Number.prototype.toLocaleString,
/// 20.2.1 in ECMA-402; BigInt.prototype.toLocaleString, 20.3.1 in ECMA-402): the
/// NumberFormat of `locales` and `options` (its construction is not observable) formats `x`.
pub fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    x: &IntlMv,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Local<'s>> {
    let r = initialize(cx, locales, options)?;
    let s = format_numeric(&r, x);
    Ok(cx.new_string(&s)?.into())
}

// ---- the JS functions -------------------------------------------------------------------------

/// Intl.NumberFormat ( [ locales [ , options ] ] ) (16.1.1). NewTarget undefined is allowed
/// (step 1 uses the active function object). The legacy constructor path of ChainNumberFormat
/// (normative optional, `Intl.FallbackSymbol`) is W4's.
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
    // 2. Let numberFormat be ? OrdinaryCreateFromConstructor(newTarget,
    //    "%Intl.NumberFormat.prototype%", internalSlotsList).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlNumberFormatPrototype)?;
    let nf = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3. Perform ? InitializeNumberFormat(numberFormat, locales, options).
    let r = initialize(cx, args.get(0), args.get(1))?;
    store(cx, nf, &r)?;
    // 5. Return numberFormat.
    Ok(nf.into())
}

/// Intl.NumberFormat.supportedLocalesOf ( locales [ , options ] ) (16.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// get Intl.NumberFormat.prototype.format (16.3.3).
fn get_format<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let nf be ? UnwrapNumberFormat(this value) (RequireInternalSlot).
    let bound = cx.slot(this, CLASS, SLOT_BOUND_FORMAT)?;
    // 4. If nf.[[BoundFormat]] is undefined, then
    if bound.is_undefined() {
        // a-b. Let F be a new built-in function object from the Number Format Function
        //      closure of this getter (16.3.3; length 1, name ""), with F.[[NumberFormat]] nf (the closure data).
        let f = cx.new_builtin_closure(number_format_function, "", 1, this);
        // c. Set nf.[[BoundFormat]] to F.
        cx.set_slot(this, CLASS, SLOT_BOUND_FORMAT, f)?;
        return Ok(f.into());
    }
    // 5. Return nf.[[BoundFormat]].
    Ok(bound)
}

/// A Number Format function (the closure of get Intl.NumberFormat.prototype.format, 16.3.3).
fn number_format_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let nf be F.[[NumberFormat]].
    let nf = cx.closure_data(args.callee());
    let r = read(cx, nf)?;
    // 3-4. Let x be ? ToIntlMathematicalValue(value).
    let x = decimal::to_intl_mv(cx, args.get(0))?;
    // 5. Return FormatNumeric(nf, x).
    let s = format_numeric(&r, &x);
    Ok(cx.new_string(&s)?.into())
}

/// Intl.NumberFormat.prototype.formatToParts ( value ) (16.3.6).
fn format_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let nf be the this value; ? RequireInternalSlot(nf, [[InitializedNumberFormat]]).
    let r = read(cx, this)?;
    // 3. Let x be ? ToIntlMathematicalValue(value).
    let x = decimal::to_intl_mv(cx, args.get(0))?;
    // 4. Return FormatNumericToParts(nf, x) (16.5.6).
    let parts = partition_numeric(&r, &x);
    let array = cx.new_array();
    for (i, (t, value)) in parts.iter().enumerate() {
        let o = cx.new_object();
        let t = cx.new_string(t.name())?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Intl.NumberFormat.prototype.resolvedOptions ( ) (16.3.2).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let nf be ? UnwrapNumberFormat(this value).
    let r = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 26, in table order: the slots that are not undefined.
    let put_str = |cx: &mut Cx<'s>, key: &str, v: &str| -> JsResult<()> {
        let v = cx.new_string(v)?;
        cx.create_data_property_or_throw(o, key, v)
    };
    let put_int = |cx: &mut Cx<'s>, key: &str, v: i32| -> JsResult<()> {
        cx.create_data_property_or_throw(o, key, Local::int32(v))
    };
    put_str(cx, "locale", &r.locale)?;
    put_str(cx, "numberingSystem", &r.numbering_system)?;
    put_str(cx, "style", STYLES[r.style as usize])?;
    if let Some(c) = &r.currency {
        put_str(cx, "currency", c)?;
        put_str(
            cx,
            "currencyDisplay",
            CURRENCY_DISPLAYS[r.currency_display as usize],
        )?;
        put_str(cx, "currencySign", CURRENCY_SIGNS[r.currency_sign as usize])?;
    }
    if let Some(u) = &r.unit {
        put_str(cx, "unit", u)?;
        put_str(cx, "unitDisplay", UNIT_DISPLAYS[r.unit_display as usize])?;
    }
    let d = &r.digits;
    put_int(
        cx,
        "minimumIntegerDigits",
        i32::from(d.minimum_integer_digits),
    )?;
    // SetNumberFormatDigitOptions sets the fraction digits unless the rounding type is
    // significantDigits, and the significant digits unless it is fractionDigits.
    if d.rounding_type != RoundingType::SignificantDigits {
        put_int(
            cx,
            "minimumFractionDigits",
            i32::from(d.minimum_fraction_digits),
        )?;
        put_int(
            cx,
            "maximumFractionDigits",
            i32::from(d.maximum_fraction_digits),
        )?;
    }
    if d.rounding_type != RoundingType::FractionDigits {
        put_int(
            cx,
            "minimumSignificantDigits",
            i32::from(d.minimum_significant_digits),
        )?;
        put_int(
            cx,
            "maximumSignificantDigits",
            i32::from(d.maximum_significant_digits),
        )?;
    }
    match r.grouping {
        Grouping::Never => {
            cx.create_data_property_or_throw(o, "useGrouping", Local::boolean(false))?
        }
        Grouping::Always => put_str(cx, "useGrouping", "always")?,
        Grouping::Auto => put_str(cx, "useGrouping", "auto")?,
        Grouping::Min2 => put_str(cx, "useGrouping", "min2")?,
    }
    put_str(cx, "notation", NOTATIONS[r.notation as usize])?;
    if NOTATIONS[r.notation as usize] == "compact" {
        put_str(
            cx,
            "compactDisplay",
            COMPACT_DISPLAYS[r.compact_display as usize],
        )?;
    }
    put_str(
        cx,
        "signDisplay",
        SIGN_DISPLAYS[index_of(&SIGNS, &r.sign_display) as usize],
    )?;
    put_int(cx, "roundingIncrement", i32::from(d.rounding_increment))?;
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
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_identifiers() {
        assert!(is_well_formed_unit_identifier("meter"));
        assert!(is_well_formed_unit_identifier("kilometer-per-hour"));
        assert!(!is_well_formed_unit_identifier("meter-per"));
        assert!(!is_well_formed_unit_identifier("km"));
        assert!(is_well_formed_currency_code("eur"));
        assert!(!is_well_formed_currency_code("EU"));
        assert!(!is_well_formed_currency_code("EU1"));
    }
}
