//! Intl.RelativeTimeFormat (ECMA-402 §18; block 38): `%Intl.RelativeTimeFormat%` and
//! `%Intl.RelativeTimeFormat.prototype%`, a module of the lazy group "Intl" (INTL-I1-D8): its
//! properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.RelativeTimeFormat {w, c}     %Intl.RelativeTimeFormat%  prototype {}, supportedLocalesOf
//! Intl.RelativeTimeFormat.prototype  %Intl.RelativeTimeFormat.prototype%  constructor {w, c},
//!                                      resolvedOptions, format, formatToParts {w, c},
//!                                      [@@toStringTag] "Intl.RelativeTimeFormat" {c}
//! slots (ObjectClass::IntlRelativeTimeFormat, SLOTS = 3):
//!   0 [[Locale]]           a String ("nu" is its only Unicode extension key)
//!   1 options              an int32: bits 0-1 [[Style]] (index in STYLES), bit 2 [[Numeric]]
//!   2 [[NumberingSystem]]  a String
//! format(value, unit) ─ ToNumber, ToString ─ PartitionRelativeTimePattern:
//!   unit "days" -> "day" (RangeError for another unit; RangeError for NaN, ±∞)
//!   cache::get_or_make(Service::RelativeTimeFormat, data locale, style) ─> Rc<Patterns>
//!     (the ICU4X data of the 8 units, and the plural rules of the locale)
//!   numeric "auto" and an integer value with a relative string (-0 is "0") ─> one literal
//!   else past (value < 0 or -0) / future pattern by the plural category of |value|
//!     |value| ─ number_format::partition_numeric([[NumberFormat]]: the NF defaults, nu)
//!     ─> number parts, each with the field `unit`, put in the pattern's place
//! ```
//!
//! `[[NumberFormat]]` and `[[PluralRules]]` are not objects here: the number parts come from
//! `number_format` with the resolved options that `new Intl.NumberFormat(locale,
//! { numberingSystem })` has (`number_format::decimal_defaults`), and the plural category
//! from the cardinal rules that `new Intl.PluralRules(locale)` selects with
//! (`plural_rules::cardinal_rules`) on the operands of the same rounded value (the
//! PluralRules defaults round as the NumberFormat defaults do: 0 to 3 fraction digits). The
//! ICU4X `RelativeTimeFormatter` is not used: it has its own decimal formatter (no ep9 digit
//! table, no NumberFormat rounding) and gives no `unit` part. No ICU4X object is in a slot
//! (INTL-I1-D7).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-relative_time_format.txt`.

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};
use fixed_decimal::Decimal;
use icu_experimental::provider::Baked;
use icu_experimental::relativetime::RelativeTimeFormatterPreferences;
use icu_experimental::relativetime::provider::*;
use icu_locale_core::preferences::LocalePreferences;
use icu_plurals::{PluralOperands, PluralRules};
use icu_provider::marker::ErasedMarker;
use icu_provider::prelude::*;

use crate::cache::{self, Service};
use crate::digits::{
    DecimalValue, DigitOptions, Interrupted, Work, format_numeric_to_string, polled,
};
use crate::locale::is_unicode_type;
use crate::number_format::{self, decimal};
use crate::options::{coerce_options_to_object, get_locale_matcher, get_string_option, index_of};
use crate::plural_rules;
use crate::resolve;
use crate::tag::{self, icu_locale};
use decimal::{IntlMv, PartType};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 38;

/// The class of an Intl.RelativeTimeFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlRelativeTimeFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the [`Packed`] `[[Style]]` and `[[Numeric]]` (an int32).
pub const SLOT_OPTIONS: usize = 1;
/// Slot 2: `[[NumberingSystem]]` (a String).
pub const SLOT_NUMBERING_SYSTEM: usize = 2;
/// The slot count of an Intl.RelativeTimeFormat object.
pub const SLOTS: usize = 3;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const FORMAT: u16 = 2;
    pub const FORMAT_TO_PARTS: u16 = 3;
    pub const RESOLVED_OPTIONS: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("RelativeTimeFormat", 0, constructor), // 0 (18.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (18.2.2)
    BuiltinDef::method("format", 2, format),                       // 2 (18.3.3)
    BuiltinDef::method("formatToParts", 2, format_to_parts),       // 3 (18.3.4)
    BuiltinDef::method("resolvedOptions", 0, resolved_options),    // 4 (18.3.2)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlRelativeTimeFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlRelativeTimeFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlRelativeTimeFormat,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlRelativeTimeFormatPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlRelativeTimeFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlRelativeTimeFormat,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, format, formatToParts.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("format"), def::FORMAT),
            PropDef::method(PropKey::Str("formatToParts"), def::FORMAT_TO_PARTS),
            // Intl.RelativeTimeFormat.prototype [ @@toStringTag ] ({c}, 18.3.5).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.RelativeTimeFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("RelativeTimeFormat"),
            IntrinsicId::IntlRelativeTimeFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "relative_time_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Style]]` values, in their packed order.
pub const STYLES: [&str; 3] = ["long", "short", "narrow"];
/// `[[Numeric]]` values, in their packed order.
pub const NUMERICS: [&str; 2] = ["always", "auto"];
/// The units of SingularRelativeTimeUnit (18.5.1), in the order of [`Patterns`].
pub const UNITS: [&str; 8] = [
    "second", "minute", "hour", "day", "week", "month", "quarter", "year",
];

/// `[[Style]]` and `[[Numeric]]` of a relative time format, packed in one int32 (slot 1):
/// bits 0-1 the index in [`STYLES`], bit 2 the index in [`NUMERICS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed(pub u32);

impl Packed {
    /// Packs the index `style` in [`STYLES`] and `numeric` in [`NUMERICS`].
    pub fn new(style: usize, numeric: usize) -> Packed {
        Packed((style as u32 & 3) | (numeric as u32 & 1) << 2)
    }

    /// The index of `[[Style]]` in [`STYLES`].
    pub fn style_index(self) -> usize {
        ((self.0 & 3) as usize).min(STYLES.len() - 1)
    }

    /// `[[Style]]`.
    pub fn style(self) -> &'static str {
        STYLES[self.style_index()]
    }

    /// `[[Numeric]]`.
    pub fn numeric(self) -> &'static str {
        NUMERICS[(self.0 >> 2 & 1) as usize]
    }
}

/// The internal slots of an Intl.RelativeTimeFormat (§18.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[Style]]` and `[[Numeric]]`.
    pub options: Packed,
    /// `[[NumberingSystem]]`.
    pub numbering_system: String,
}

/// Intl.RelativeTimeFormat steps 3-17 (18.1.1; the object creation of step 2 is the
/// caller's): the internal slots for `locales` and `options`, in the spec order of the
/// observable steps. No ICU4X object is made.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 3. ResolveOptions(%Intl.RelativeTimeFormat%, ..., locales, options, « coerce-options »):
    //    CanonicalizeLocaleList, CoerceOptionsToObject, localeMatcher, the option of the
    //    relevant extension key "nu" (numberingSystem; a RangeError if it does not match the
    //    type nonterminal), ResolveLocale.
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    let options = coerce_options_to_object(cx, options)?;
    get_locale_matcher(cx, options)?;
    let nu = get_string_option(cx, options, "numberingSystem", &[], None)?;
    if let Some(n) = &nu
        && !is_unicode_type(n)
    {
        return Err(cx.throw_range_error(&format!("Invalid numberingSystem : {n}")));
    }
    let r = resolve::resolve_locale(cx, &requested, &[("nu", nu)], &["nu"], &|l, _| {
        number_format::nu_data(l)
    })?;
    // 6-9. [[Locale]], [[LocaleData]] (the data locale) and [[NumberingSystem]] = r.[[nu]].
    let numbering_system = r
        .keywords
        .iter()
        .find(|(k, _)| k == "nu")
        .map_or_else(|| "latn".to_string(), |(_, v)| v.clone());
    // 10-11. Let style be ? GetOption(options, "style", string, « "long", "short", "narrow" »,
    //        "long").
    let style = get_string_option(cx, options, "style", &STYLES, Some(STYLES[0]))?;
    // 12-13. Let numeric be ? GetOption(options, "numeric", string, « "always", "auto" »,
    //        "always").
    let numeric = get_string_option(cx, options, "numeric", &NUMERICS, Some(NUMERICS[0]))?;
    // 14-17. [[NumberFormat]] and [[PluralRules]] come from [[Locale]] and [[NumberingSystem]]
    //        at each format (number_format::decimal_defaults, plural_rules::cardinal_rules).
    Ok(Resolved {
        locale: r.locale,
        options: Packed::new(
            index_of(&STYLES, &style.as_deref().unwrap_or(STYLES[0])) as usize,
            index_of(&NUMERICS, &numeric.as_deref().unwrap_or(NUMERICS[0])) as usize,
        ),
        numbering_system,
    })
}

// ---- the ICU4X data (crate::cache) ------------------------------------------------------------

/// The ICU4X data of one unit and style.
type UnitData = DataPayload<ErasedMarker<RelativeTimePatternData<'static>>>;

/// The relative time data of one data locale and style: the patterns of the 8 units, in
/// [`UNITS`] order, and the plural rules that pick a pattern.
pub struct Patterns {
    units: Vec<UnitData>,
    /// `None`: the "other" pattern (see `plural_rules::cardinal_rules`).
    rules: Option<PluralRules>,
}

/// The data of the marker `M` for the locale preferences `prefs` (the ICU4X data fallback
/// ends at root, so every locale has it).
fn load<M>(prefs: LocalePreferences) -> Option<UnitData>
where
    M: DataMarker<DataStruct = RelativeTimePatternData<'static>>,
    Baked: DataProvider<M>,
{
    let locale = M::make_locale(prefs);
    let response = DataProvider::<M>::load(
        &Baked,
        DataRequest {
            id: DataIdentifierBorrowed::for_locale(&locale),
            ..Default::default()
        },
    )
    .ok()?;
    Some(response.payload.cast())
}

/// The data of the 8 units, in [`UNITS`] order, of the markers `$m` (one per unit).
macro_rules! units {
    ($prefs:expr; $($m:ty),*) => {
        vec![$(load::<$m>($prefs)?),*]
    };
}

impl Patterns {
    /// The data of `data_locale` (canonical, no extension) in the style with the index
    /// `style` in [`STYLES`]; `None` if a unit has no data (not the case with the compiled
    /// data).
    pub fn new(data_locale: &str, style: usize) -> Option<Patterns> {
        let locale = icu_locale(data_locale);
        let prefs = RelativeTimeFormatterPreferences::from(&locale).locale_preferences;
        let units = match STYLES.get(style).copied() {
            Some("short") => units!(prefs;
                DatetimeRelativeSecondShortV1, DatetimeRelativeMinuteShortV1,
                DatetimeRelativeHourShortV1, DatetimeRelativeDayShortV1,
                DatetimeRelativeWeekShortV1, DatetimeRelativeMonthShortV1,
                DatetimeRelativeQuarterShortV1, DatetimeRelativeYearShortV1),
            Some("narrow") => units!(prefs;
                DatetimeRelativeSecondNarrowV1, DatetimeRelativeMinuteNarrowV1,
                DatetimeRelativeHourNarrowV1, DatetimeRelativeDayNarrowV1,
                DatetimeRelativeWeekNarrowV1, DatetimeRelativeMonthNarrowV1,
                DatetimeRelativeQuarterNarrowV1, DatetimeRelativeYearNarrowV1),
            _ => units!(prefs;
                DatetimeRelativeSecondLongV1, DatetimeRelativeMinuteLongV1,
                DatetimeRelativeHourLongV1, DatetimeRelativeDayLongV1,
                DatetimeRelativeWeekLongV1, DatetimeRelativeMonthLongV1,
                DatetimeRelativeQuarterLongV1, DatetimeRelativeYearLongV1),
        };
        Some(Patterns {
            units,
            rules: plural_rules::cardinal_rules(data_locale),
        })
    }

    /// The relative string of `unit` (an index in [`UNITS`]) for `offset` ("yesterday" for
    /// -1 day), if the data has one.
    pub fn relative(&self, unit: usize, offset: i8) -> Option<&str> {
        self.units.get(unit)?.get().get_relative(offset)
    }

    /// The pattern text of `unit` for `past` or the future and the plural operands `op`,
    /// with `mark` in the place of the number (the text has no mark if the pattern has no
    /// placeholder).
    fn pattern(&self, unit: usize, past: bool, op: PluralOperands, mark: &str) -> Option<String> {
        let data = self.units.get(unit)?.get();
        let elements = if past { &data.past } else { &data.future };
        let pattern = match &self.rules {
            Some(rules) => elements.get(op, rules),
            None => elements.elements.get_default().1,
        };
        Some(pattern.interpolate((mark,)).to_string())
    }
}

/// The cached data of the `[[Locale]]` `locale` and the style index `style` (the root data
/// if the locale has none).
fn patterns_for(locale: &str, style: usize) -> Option<Rc<Patterns>> {
    let data_locale = tag::remove_unicode_extension(locale);
    cache::get_or_make(
        Service::RelativeTimeFormat,
        &data_locale,
        style as u64,
        || Patterns::new(&data_locale, style).or_else(|| Patterns::new("und", style)),
    )
}

// ---- PartitionRelativeTimePattern (18.5.2) ----------------------------------------------------

/// SingularRelativeTimeUnit ( unit ) (18.5.1): the index in [`UNITS`] of `unit` or of its
/// plural ("days" -> "day"); `None` for any other string (the caller's RangeError).
pub fn singular_unit(unit: &str) -> Option<usize> {
    let singular = match unit.strip_suffix('s') {
        Some(s) if UNITS.contains(&s) => s,
        _ => unit,
    };
    UNITS.iter().position(|u| *u == singular)
}

/// A part of a formatted relative time: a literal of the pattern (no unit), or a part of the
/// number with the unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Literal(String),
    Number(PartType, String),
}

/// The mark that stands for the number in a pattern (no CLDR relative time pattern has a
/// private-use character).
const MARK: &str = "\u{E000}";

/// The relative string of a value whose ToString is the integer text of `offset`, under
/// `[[Numeric]]` "auto" (18.5.2 step 6): `Some` only for an integral value in the i8 range
/// (-0 gives "0": ToString(-0) is "0"). A compare of the saturating cast, no float library
/// call (H36).
fn auto_offset(value: f64) -> Option<i8> {
    let offset = value as i8;
    (f64::from(offset) == value).then_some(offset)
}

/// The ICU4X plural operands of ResolvePlural(`[[PluralRules]]`, |value|) (17.5.2): the operands
/// of `x` rounded by FormatNumericToString with `digits` (the PluralRules defaults: the
/// NumberFormat defaults) in standard notation.
fn plural_operands(
    digits: &DigitOptions,
    x: &IntlMv,
    work: &mut Work,
) -> Result<PluralOperands, Interrupted> {
    let IntlMv::Finite(d) = x else {
        // Not reached: a finite Number is a Decimal. The "other" form.
        return Ok(PluralOperands::from(&Decimal::from(0u8)));
    };
    let mut y = d.copy(work)?;
    format_numeric_to_string(digits, &mut y, work)?;
    y.trim_start();
    Ok(PluralOperands::from(&y))
}

/// Steps 6-12 of PartitionRelativeTimePattern ( relativeTimeFormat, value, unit ) (18.5.2)
/// for the finite `value` and the unit index `unit`, with `abs` the Intl mathematical value of
/// |value|: the relative string of "auto", else MakePartsList ( pattern, unit, parts )
/// (18.5.3) of the past or future pattern and the number parts of |value|. `None` if the data
/// of the locale is missing (not the case with the compiled data).
pub fn partition(
    r: &Resolved,
    value: f64,
    abs: &IntlMv,
    unit: usize,
    work: &mut Work,
) -> Result<Option<Vec<Part>>, Interrupted> {
    let Some(patterns) = patterns_for(&r.locale, r.options.style_index()) else {
        return Ok(None);
    };
    // 6. If relativeTimeFormat.[[Numeric]] is "auto", the relative string of ToString(value),
    //    if the data has one, as one literal part.
    if r.options.numeric() == "auto"
        && let Some(offset) = auto_offset(value)
        && let Some(s) = patterns.relative(unit, offset)
    {
        return Ok(Some(vec![Part::Literal(s.to_string())]));
    }
    // 7. If value is -0 or value < 0, let tl be "past" (and value is now -value); else
    //    "future".
    let past = value.is_sign_negative();
    // 9. Let fv be PartitionNumberPattern(relativeTimeFormat.[[NumberFormat]], ℝ(value)).
    let nf = number_format::decimal_defaults(&r.locale, &r.numbering_system);
    let fv = number_format::partition_numeric(&nf, abs, work)?;
    // 10-11. Let pr be ResolvePlural(relativeTimeFormat.[[PluralRules]], value)
    //        .[[PluralCategory]]; let pattern be po.[[<pr>]].
    let op = plural_operands(&nf.digits, abs, work)?;
    let Some(text) = patterns.pattern(unit, past, op, MARK) else {
        return Ok(None);
    };
    // 12. MakePartsList ( pattern, unit, fv ): the literals of the pattern (an empty one is
    //     not a part) and, in the place of "{0}", the parts of fv.
    let mut out = Vec::with_capacity(fv.len() + 2);
    let literal = |out: &mut Vec<Part>, s: &str| {
        if !s.is_empty() {
            out.push(Part::Literal(s.to_string()));
        }
    };
    match text.split_once(MARK) {
        Some((before, after)) => {
            literal(&mut out, before);
            for (t, v) in fv {
                work.add(v.len())?;
                out.push(Part::Number(t, v));
            }
            literal(&mut out, after);
        }
        None => literal(&mut out, &text),
    }
    Ok(Some(out))
}

/// PartitionRelativeTimePattern ( relativeTimeFormat, value, unit ) (18.5.2) for the
/// Intl.RelativeTimeFormat `r`: the parts and the singular unit.
fn partition_relative_time_pattern(
    cx: &mut Cx<'_>,
    r: &Resolved,
    value: f64,
    unit: &str,
) -> JsResult<(Vec<Part>, &'static str)> {
    // 1. If value is NaN, +∞𝔽 or -∞𝔽, throw a RangeError exception.
    if !value.is_finite() {
        return Err(cx.throw_range_error(&format!(
            "Invalid value {} for Intl.RelativeTimeFormat",
            if value.is_nan() { "NaN" } else { "Infinity" }
        )));
    }
    // 2. Let unit be ? SingularRelativeTimeUnit(unit).
    let Some(unit) = singular_unit(unit) else {
        return Err(cx.throw_range_error(&format!("Invalid unit argument : {unit}")));
    };
    // 7.b. Set value to -value: the number and its plural are those of |value|.
    let abs = decimal::mv_of_number(cx, value.abs())?;
    match polled(cx, |w| partition(r, value, &abs, unit, w))? {
        Some(parts) => Ok((parts, UNITS[unit])),
        None => Err(cx.throw_type_error("Intl.RelativeTimeFormat: no relative time data")),
    }
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.RelativeTimeFormat ( [ locales [ , options ] ] ) (18.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.RelativeTimeFormat requires 'new'"));
    };
    // 2. Let relativeTimeFormat be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%Intl.RelativeTimeFormat.prototype%", « [[InitializedRelativeTimeFormat]], [[Locale]],
    //    [[LocaleData]], [[Style]], [[Numeric]], [[NumberFormat]], [[NumberingSystem]],
    //    [[PluralRules]] »).
    let proto =
        cx.get_prototype_from_constructor(nt, IntrinsicId::IntlRelativeTimeFormatPrototype)?;
    let rtf = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3-17.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(rtf, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(rtf, CLASS, SLOT_OPTIONS, Local::int32(r.options.0 as i32))?;
    let nu = cx.new_string(&r.numbering_system)?;
    cx.set_slot(rtf, CLASS, SLOT_NUMBERING_SYSTEM, nu)?;
    // 18. Return relativeTimeFormat.
    Ok(rtf.into())
}

/// Intl.RelativeTimeFormat.supportedLocalesOf ( locales [ , options ] ) (18.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.RelativeTimeFormat%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The internal slots of the relative time format `o` (TypeError if `o` is not an
/// Intl.RelativeTimeFormat: RequireInternalSlot(o, `[[InitializedRelativeTimeFormat]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<Resolved> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let options = cx.slot(o, CLASS, SLOT_OPTIONS)?;
    let nu = cx.slot(o, CLASS, SLOT_NUMBERING_SYSTEM)?;
    let locale = cx.to_rust_string(locale)?;
    let numbering_system = cx.to_rust_string(nu)?;
    Ok(Resolved {
        locale,
        options: Packed(options.as_int32().unwrap_or(0) as u32),
        numbering_system,
    })
}

/// The steps 1-4 of format and formatToParts: the relative time format of `this`, then
/// ToNumber(value) and ToString(unit), then PartitionRelativeTimePattern.
fn read_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
) -> JsResult<(Vec<Part>, &'static str)> {
    // 1-2. Let relativeTimeFormat be the this value; ? RequireInternalSlot(relativeTimeFormat,
    //      [[InitializedRelativeTimeFormat]]).
    let r = read(cx, this)?;
    // 3. Let value be ? ToNumber(value).
    let value = cx.to_number(args.get(0))?;
    // 4. Let unit be ? ToString(unit).
    let unit = cx.to_rust_string(args.get(1))?;
    partition_relative_time_pattern(cx, &r, value, &unit)
}

/// Intl.RelativeTimeFormat.prototype.format ( value, unit ) (18.3.3).
fn format<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (parts, _) = read_parts(cx, this, args)?;
    // 5. Return FormatRelativeTime(relativeTimeFormat, value, unit) (18.5.4): the
    //    concatenation of the parts.
    let mut out = String::new();
    for p in &parts {
        match p {
            Part::Literal(v) | Part::Number(_, v) => out.push_str(v),
        }
    }
    Ok(cx.new_string(&out)?.into())
}

/// Intl.RelativeTimeFormat.prototype.formatToParts ( value, unit ) (18.3.4).
fn format_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (parts, unit) = read_parts(cx, this, args)?;
    // 5. Return FormatRelativeTimeToParts(relativeTimeFormat, value, unit) (18.5.5): an
    //    Array of { type, value } and, for a part of the number, unit.
    let array = cx.new_array();
    for (i, p) in parts.iter().enumerate() {
        cx.poll_interrupt()?;
        let o = cx.new_object();
        let (t, v, u) = match p {
            Part::Literal(v) => ("literal", v, None),
            Part::Number(t, v) => (t.name(), v, Some(unit)),
        };
        let t = cx.new_string(t)?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string(v)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        if let Some(u) = u {
            let u = cx.new_string(u)?;
            cx.create_data_property_or_throw(o, "unit", u)?;
        }
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Intl.RelativeTimeFormat.prototype.resolvedOptions ( ) (18.3.2).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let relativeTimeFormat be the this value; ? RequireInternalSlot(relativeTimeFormat,
    //      [[InitializedRelativeTimeFormat]]).
    let r = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 31 (locale, style, numeric, numberingSystem), in table order:
    //    CreateDataPropertyOrThrow(options, p, v).
    for (key, value) in [
        ("locale", r.locale.as_str()),
        ("style", r.options.style()),
        ("numeric", r.options.numeric()),
        ("numberingSystem", r.numbering_system.as_str()),
    ] {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)?;
    }
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digits::unpolled;

    fn mv(s: &str) -> IntlMv {
        IntlMv::Finite(decimal::decimal_from_literal(s).unwrap())
    }

    /// The parts of `value` (whose |value| has the decimal text `abs`) as text: the number
    /// parts in brackets with their type.
    fn fmt(locale: &str, style: usize, auto: bool, value: f64, abs: &str, unit: &str) -> String {
        let r = Resolved {
            locale: locale.to_string(),
            options: Packed::new(style, usize::from(auto)),
            numbering_system: "latn".to_string(),
        };
        let unit = singular_unit(unit).unwrap();
        let parts = unpolled(|w| partition(&r, value, &mv(abs), unit, w)).unwrap();
        parts
            .iter()
            .map(|p| match p {
                Part::Literal(v) => v.clone(),
                Part::Number(t, v) => format!("[{}:{v}]", t.name()),
            })
            .collect()
    }

    #[test]
    fn packed_round_trips() {
        for (s, style) in STYLES.iter().enumerate() {
            for (n, numeric) in NUMERICS.iter().enumerate() {
                let p = Packed::new(s, n);
                assert_eq!((p.style(), p.numeric()), (*style, *numeric));
            }
        }
    }

    #[test]
    fn singular_units() {
        assert_eq!(singular_unit("days"), Some(3));
        assert_eq!(singular_unit("quarter"), Some(6));
        assert_eq!(singular_unit("years"), Some(7));
        for bad in ["", "s", "DAY", "dayss", "decade", "milliseconds", "day "] {
            assert_eq!(singular_unit(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn auto_offsets() {
        assert_eq!(auto_offset(-0.0), Some(0));
        assert_eq!(auto_offset(-1.0), Some(-1));
        assert_eq!(auto_offset(127.0), Some(127));
        assert_eq!(auto_offset(128.0), None);
        assert_eq!(auto_offset(1.5), None);
        assert_eq!(auto_offset(-128.5), None);
        assert_eq!(auto_offset(f64::NAN), None);
    }

    /// English and Polish: the plural category of the rounded value picks the pattern, the
    /// number has the NumberFormat defaults, -0 is past, and "auto" takes the relative string
    /// of "0" for -0 (the W3 briefs correction 3).
    #[test]
    fn english_and_polish_patterns() {
        assert_eq!(fmt("en", 0, false, 1.0, "1", "day"), "in [integer:1] day");
        assert_eq!(
            fmt("en", 0, false, -2.0, "2", "days"),
            "[integer:2] days ago"
        );
        assert_eq!(
            fmt("en", 0, false, 1000.0, "1000", "second"),
            "in [integer:1][group:,][integer:000] seconds"
        );
        // 1.0004 rounds to 1 (0 to 3 fraction digits): the "one" pattern.
        assert_eq!(
            fmt("en", 0, false, 1.0004, "1.0004", "hour"),
            "in [integer:1] hour"
        );
        assert_eq!(
            fmt("en", 0, false, 1.5, "1.5", "hour"),
            "in [integer:1][decimal:.][fraction:5] hours"
        );
        assert_eq!(
            fmt("en", 0, false, -0.0, "0", "day"),
            "[integer:0] days ago"
        );
        assert_eq!(fmt("en", 0, true, -0.0, "0", "day"), "today");
        assert_eq!(fmt("en", 0, true, -1.0, "1", "day"), "yesterday");
        assert_eq!(fmt("en", 0, true, 2.0, "2", "day"), "in [integer:2] days");
        assert_eq!(fmt("en", 1, false, 3.0, "3", "month"), "in [integer:3] mo.");
        assert_eq!(fmt("pl", 0, false, 2.0, "2", "day"), "za [integer:2] dni");
        assert_eq!(
            fmt("pl", 0, false, -1.0, "1", "year"),
            "[integer:1] rok temu"
        );
        assert_eq!(fmt("pl", 0, false, 5.0, "5", "year"), "za [integer:5] lat");
        assert_eq!(
            fmt("pl", 0, false, 1000.0, "1000", "minute"),
            "za [integer:1000] minut"
        );
    }

    /// A script variant whose CLDR parent is root takes the plural rules of its language, as
    /// Intl.PluralRules does (sr-Latn: 2 is "few", 5 is "other"; the root rules give "other"
    /// for both).
    #[test]
    fn a_script_variant_takes_the_rules_of_its_language() {
        assert_eq!(
            fmt("sr-Latn", 0, false, 2.0, "2", "year"),
            "za [integer:2] godine"
        );
        assert_eq!(
            fmt("sr-Latn", 0, false, 5.0, "5", "year"),
            "za [integer:5] godina"
        );
    }

    /// Every available locale has data for every style and unit, and each past and future
    /// pattern has the number in it.
    #[test]
    fn every_locale_has_patterns() {
        let mut checked = 0;
        for locale in crate::available::locales() {
            for style in 0..STYLES.len() {
                let p = Patterns::new(locale, style).expect("relative time data");
                for unit in 0..UNITS.len() {
                    for past in [false, true] {
                        let op = PluralOperands::from(&Decimal::from(5u8));
                        let text = p.pattern(unit, past, op, MARK).unwrap();
                        assert!(text.contains(MARK), "{locale} {style} {unit}: {text}");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 1000);
    }
}
