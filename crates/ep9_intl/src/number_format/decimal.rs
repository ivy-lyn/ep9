//! The number core of NumberFormat (ECMA-402 §16.5): the Intl mathematical value of a value
//! (ToIntlMathematicalValue, §16.5.17), and the parts of a formatted number (FormatNumeric,
//! PartitionNumberPattern §16.5.4, ComputeExponent §16.5.12) for the decimal style in
//! standard and compact notation.
//!
//! ```text
//! value ─ to_intl_mv ─> IntlMv::{Finite(Decimal), Wide(WideDecimal), PosInf, NegInf, NaN}
//!   Number   digits of Number::toString ("1e+21", "5e-324") -> Decimal::try_from_str (no ryu,
//!            INTL-V2-FP2)
//!   BigInt   BigInt::toString -> Decimal; from WIDE_DIGITS digits on -> WideDecimal (exact;
//!            a Decimal has i16 magnitudes)
//!   String   ToNumber (NaN, +-Infinity, +-0) then the exact decimal text with the trailing
//!            fraction zeros cut (INTL-V2-D20: "1.500" is the value 1.5)
//! IntlMv + Options ─ partition ─> [(type, value)]
//!   compact: ComputeExponent on the ICU4X compact data; x * 10^-e
//!   FormatNumericToString (crate::digits, rounding on Decimal or WideDecimal)
//!   sign by signDisplay; digits of the numbering system (CLDR digit table); grouping by the
//!   ICU4X DecimalSymbols sizes; NaN and infinity from the CLDR T1 table
//!   compact affix: the ICU4X compact pattern, split around the number
//! ```
//!
//! No f64 arithmetic: every digit comes from a Decimal (H36).

use std::rc::Rc;

use fixed_decimal::{Decimal, Sign};
use icu_decimal::options::{
    CompactDecimalFormatterOptions, DecimalFormatterOptions, GroupingStrategy,
};
use icu_decimal::preferences::CompactDecimalFormatterPreferences;
use icu_decimal::provider::{Baked, DecimalSymbolsV1};
use icu_decimal::{CompactDecimalFormatter, DecimalFormatter, DecimalFormatterPreferences};
use icu_locale_core::Locale;
use icu_locale_core::extensions::unicode::Value;
use icu_locale_core::preferences::extensions::unicode::keywords::NumberingSystem;
use icu_provider::prelude::*;

use ep9_vm::{Cx, JsResult, Local, ToPrimitiveHint};

use crate::cache::{self, Service};
use crate::cldr;
use crate::digits::{
    DecimalValue, DigitOptions, WIDE_DIGITS, WideDecimal, format_numeric_to_string,
};

/// An Intl mathematical value (§16.5.15): a finite value (negative zero is a zero with
/// [`Sign::Negative`]), an infinity or not-a-number. `Wide` is a finite value that a Decimal
/// cannot hold: a BigInt of [`WIDE_DIGITS`] digits or more.
#[derive(Clone, Debug, PartialEq)]
pub enum IntlMv {
    Finite(Decimal),
    Wide(WideDecimal),
    PosInf,
    NegInf,
    NaN,
}

/// The JS StrWhiteSpaceChar code points (WhiteSpace and LineTerminator, ECMA-262 §7.2, §7.3).
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'
            | '\u{B}'
            | '\u{C}'
            | ' '
            | '\u{A0}'
            | '\u{FEFF}'
            | '\u{A}'
            | '\u{D}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// The Decimal of a finite, nonzero decimal literal `s` (ASCII; an optional sign, digits with
/// at most one '.', an optional exponent with an optional sign). The form of ECMA-262
/// StrDecimalLiteral that `Decimal::try_from_str` does not read ("5.", "5.e3", "1e+21") is
/// put into its form first. `None` if `s` is not such a literal.
pub fn decimal_from_literal(s: &str) -> Option<Decimal> {
    let (neg, body) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int, frac) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut text = String::with_capacity(s.len() + 2);
    if neg {
        text.push('-');
    }
    text.push_str(if int.is_empty() { "0" } else { int });
    if !frac.is_empty() {
        text.push('.');
        text.push_str(frac);
    }
    if let Some(e) = exponent {
        let digits = e.strip_prefix('+').unwrap_or(e);
        let unsigned = digits.strip_prefix('-').unwrap_or(digits);
        if unsigned.is_empty() || !unsigned.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.push('e');
        text.push_str(digits);
    }
    let mut d = Decimal::try_from_str(&text).ok()?;
    d.absolute.trim_start();
    d.absolute.trim_end();
    if d.sign == Sign::Positive {
        d.sign = Sign::None;
    }
    Some(d)
}

/// The Decimal of the unsigned integer in base `radix` (2, 8 or 16) with the digits `s`.
fn decimal_from_radix(s: &str, radix: u32) -> Option<Decimal> {
    if s.is_empty() {
        return None;
    }
    // Decimal digits, least significant first.
    let mut digits: Vec<u8> = vec![0];
    for c in s.chars() {
        let mut carry = c.to_digit(radix)?;
        for d in digits.iter_mut() {
            let v = u32::from(*d) * radix + carry;
            *d = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let text: String = digits.iter().rev().map(|d| char::from(b'0' + d)).collect();
    let mut d = Decimal::try_from_str(&text).ok()?;
    d.absolute.trim_start();
    Some(d)
}

/// The Intl mathematical value of the Number `x`: its digits are the digits of
/// Number::toString(x) (INTL-V2-FP2), read by `Decimal::try_from_str`.
pub fn mv_of_number(cx: &mut Cx<'_>, x: f64) -> JsResult<IntlMv> {
    if x.is_nan() {
        return Ok(IntlMv::NaN);
    }
    if x.is_infinite() {
        return Ok(if x > 0.0 {
            IntlMv::PosInf
        } else {
            IntlMv::NegInf
        });
    }
    if x == 0.0 {
        let mut d = Decimal::from(0);
        if x.is_sign_negative() {
            d.sign = Sign::Negative;
        }
        return Ok(IntlMv::Finite(d));
    }
    let text = cx.to_rust_string(Local::number(x))?;
    Ok(decimal_from_literal(&text).map_or(IntlMv::NaN, IntlMv::Finite))
}

/// The Intl mathematical value of the integer text `text` (the BigInt::toString digits with
/// an optional '-'): a Decimal, or a [`WideDecimal`] from [`WIDE_DIGITS`] digits on. Exact
/// for each BigInt (never NaN).
pub fn mv_of_integer(text: &str) -> IntlMv {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, text),
    };
    if digits.len() < WIDE_DIGITS
        && let Some(d) = decimal_from_literal(text)
    {
        return IntlMv::Finite(d);
    }
    IntlMv::Wide(WideDecimal::from_integer_digits(
        negative,
        digits.as_bytes(),
    ))
}

/// ToIntlMathematicalValue ( value ) (§16.5.15).
pub fn to_intl_mv<'s>(cx: &mut Cx<'s>, value: Local<'s>) -> JsResult<IntlMv> {
    // 1. If value is a Number, return ℝ(value) (or the special values).
    if let Some(x) = value.as_number() {
        return mv_of_number(cx, x);
    }
    // 2. Let primValue be ? ToPrimitive(value, number).
    let prim = cx.to_primitive(value, ToPrimitiveHint::Number)?;
    // 3. If primValue is a BigInt, return ℝ(primValue).
    if cx.is_bigint(prim) {
        let s = cx.bigint_to_string(prim, 10)?;
        let text = cx.to_rust_string(s)?;
        return Ok(mv_of_integer(&text));
    }
    // 4-5. If primValue is not a String, return ℝ(? ToNumber(primValue)).
    if !prim.is_string() {
        let x = cx.to_number(prim)?;
        return mv_of_number(cx, x);
    }
    // 6-11. A String: StringIntlMV of its StringNumericLiteral. ToNumber gives NaN for a text
    // that is not one, and the result of RoundMVResult for the infinities and the zeros.
    let x = cx.to_number(prim)?;
    if x.is_nan() {
        return Ok(IntlMv::NaN);
    }
    if x.is_infinite() {
        return Ok(if x > 0.0 {
            IntlMv::PosInf
        } else {
            IntlMv::NegInf
        });
    }
    let text = cx.to_rust_string(prim)?;
    let text = text.trim_matches(is_js_space);
    if x == 0.0 {
        // The zero of a text: negative-zero for a "-" sign (RoundMVResult gives +0 for the
        // value, step 10).
        let mut d = Decimal::from(0);
        if text.starts_with('-') {
            d.sign = Sign::Negative;
        }
        return Ok(IntlMv::Finite(d));
    }
    let lower = text.get(..2).map(str::to_ascii_lowercase);
    let d = match lower.as_deref() {
        Some("0x") => decimal_from_radix(&text[2..], 16),
        Some("0o") => decimal_from_radix(&text[2..], 8),
        Some("0b") => decimal_from_radix(&text[2..], 2),
        _ => decimal_from_literal(text),
    };
    // A text that ToNumber reads but Decimal cannot hold (more than i16::MAX digits) keeps
    // the Number's digits.
    match d {
        Some(d) => Ok(IntlMv::Finite(d)),
        None => mv_of_number(cx, x),
    }
}

// ---- the options that the parts need --------------------------------------------------------

/// `[[UseGrouping]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    Never,
    Always,
    Auto,
    Min2,
}

/// `[[SignDisplay]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignDisplay {
    Auto,
    Never,
    Always,
    ExceptZero,
    Negative,
}

/// The fields of a NumberFormat that FormatNumeric reads (decimal style).
#[derive(Clone, Debug)]
pub struct Format<'a> {
    /// The data locale: `[[Locale]]` without its Unicode extension.
    pub data_locale: &'a str,
    /// `[[NumberingSystem]]`.
    pub numbering_system: &'a str,
    pub digits: DigitOptions,
    /// `None`: standard notation; `Some(long)`: compact notation with `[[CompactDisplay]]`
    /// "long" (true) or "short".
    pub compact: Option<bool>,
    pub grouping: Grouping,
    pub sign_display: SignDisplay,
}

// ---- the ICU4X objects (crate::cache) ---------------------------------------------------------

/// The ICU4X data of one (data locale, numbering system, compact display).
struct Icu {
    /// The symbols of the locale (and of the numbering system if the data has them).
    symbols: DataPayload<DecimalSymbolsV1>,
    /// A decimal formatter with no grouping (only to split the compact pattern).
    plain: Option<DecimalFormatter>,
    /// The compact formatter, no grouping (compact notation only).
    compact: Option<CompactDecimalFormatter>,
}

/// The ICU4X locale of `data_locale` ("und" if ICU4X cannot read it).
fn icu_locale(data_locale: &str) -> Locale {
    Locale::try_from_str(data_locale).unwrap_or(Locale::UNKNOWN)
}

/// The preferences of `data_locale` with the numbering system `nu` (if ICU4X reads it).
fn prefs(data_locale: &str, nu: Option<&str>) -> DecimalFormatterPreferences {
    let mut p = DecimalFormatterPreferences::from(&icu_locale(data_locale));
    p.numbering_system = nu.and_then(|nu| {
        Value::try_from_str(nu)
            .ok()
            .and_then(|v| NumberingSystem::try_from(v).ok())
    });
    p
}

/// The decimal symbols of `data_locale` for the numbering system `nu` (the locale's own
/// symbols if the data has none for `nu`; `None` for the locale's default system).
fn load_symbols(data_locale: &str, nu: Option<&str>) -> Option<DataPayload<DecimalSymbolsV1>> {
    let p = prefs(data_locale, nu);
    let locale = DecimalSymbolsV1::make_locale(p.locale_preferences);
    let silent = || {
        let mut m = DataRequestMetadata::default();
        m.silent = true;
        m
    };
    if let Some(nu) = nu
        && let Ok(attributes) = DataMarkerAttributes::try_from_str(nu)
    {
        let req = DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale),
            metadata: silent(),
        };
        if let Ok(r) = DataProvider::<DecimalSymbolsV1>::load(&Baked, req) {
            return Some(r.payload);
        }
    }
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_locale(&locale),
        metadata: silent(),
    };
    DataProvider::<DecimalSymbolsV1>::load(&Baked, req)
        .ok()
        .map(|r| r.payload)
}

/// The default numbering system of `data_locale` (the `numsys` of its decimal symbols;
/// "latn" if the data has none).
pub fn default_numbering_system(data_locale: &str) -> String {
    load_symbols(data_locale, None)
        .map(|p| p.get().numsys().to_string())
        .unwrap_or_else(|| "latn".to_string())
}

/// The cached ICU4X data of `f`.
fn icu_for(f: &Format<'_>) -> Option<Rc<Icu>> {
    let key = format!("{}|{}", f.data_locale, f.numbering_system);
    let options = match f.compact {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    cache::get_or_make(Service::NumberFormat, &key, options, || {
        let symbols = load_symbols(f.data_locale, Some(f.numbering_system))
            .or_else(|| load_symbols("und", None))?;
        let (plain, compact) = match f.compact {
            None => (None, None),
            Some(long) => {
                let p = prefs(f.data_locale, Some(f.numbering_system));
                let mut cp = CompactDecimalFormatterPreferences::from(&icu_locale(f.data_locale));
                cp.numbering_system = p.numbering_system;
                let never = DecimalFormatterOptions::from(GroupingStrategy::Never);
                let c_options = CompactDecimalFormatterOptions::from(GroupingStrategy::Never);
                let compact = if long {
                    CompactDecimalFormatter::try_new_long(cp, c_options)
                } else {
                    CompactDecimalFormatter::try_new_short(cp, c_options)
                };
                (DecimalFormatter::try_new(p, never).ok(), compact.ok())
            }
        };
        Some(Icu {
            symbols,
            plain,
            compact,
        })
    })
}

// ---- PartitionNumberPattern -------------------------------------------------------------------

/// The `[[Type]]` of a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartType {
    Integer,
    Group,
    Decimal,
    Fraction,
    MinusSign,
    PlusSign,
    Nan,
    Infinity,
    Compact,
    Literal,
}

impl PartType {
    /// The `type` string of formatToParts.
    pub fn name(self) -> &'static str {
        match self {
            PartType::Integer => "integer",
            PartType::Group => "group",
            PartType::Decimal => "decimal",
            PartType::Fraction => "fraction",
            PartType::MinusSign => "minusSign",
            PartType::PlusSign => "plusSign",
            PartType::Nan => "nan",
            PartType::Infinity => "infinity",
            PartType::Compact => "compact",
            PartType::Literal => "literal",
        }
    }
}

/// The parts of a formatted number, in order.
pub type Parts = Vec<(PartType, String)>;

/// The CLDR locale ids to try for `data_locale`, in order: the ICU4X locale fallback chain
/// (it knows the CLDR parent locales: "zh-TW" -> "zh-Hant-TW" -> "zh-Hant"), then the
/// subtag cuts of `data_locale`, then "und".
fn locale_chain(data_locale: &str) -> Vec<String> {
    use icu_locale::LocaleFallbacker;
    use icu_locale::fallback::LocaleFallbackConfig;
    let mut v: Vec<String> = Vec::new();
    let mut it = LocaleFallbacker::new()
        .for_config(LocaleFallbackConfig::default())
        .fallback_for(DataLocale::from(&icu_locale(data_locale)));
    while !it.get().is_unknown() && v.len() < 16 {
        v.push(it.get().to_string());
        it.step();
    }
    let mut l = data_locale;
    loop {
        if !v.iter().any(|x| x == l) {
            v.push(l.to_string());
        }
        match l.rfind('-') {
            Some(i) => l = &l[..i],
            None => break,
        }
    }
    v.push("und".to_string());
    v
}

/// The T1 values (NaN, infinity) of `data_locale` and `nu`: the first row on the locale
/// chain ([`locale_chain`]), else the `latn` row of the same chain.
fn t1_symbols(data_locale: &str, nu: &str) -> Option<&'static cldr::NumberSymbols> {
    let chain = locale_chain(data_locale);
    chain
        .iter()
        .find_map(|l| cldr::number_symbols(l, nu))
        .or_else(|| chain.iter().find_map(|l| cldr::number_symbols(l, "latn")))
}

/// Whether a group separator follows the digit at `magnitude` of a number whose highest
/// integer digit is at `upper` (the ICU4X grouping rule with the ECMA-402 strategies:
/// "always" ignores the locale's minimum grouping digits).
fn group_after(
    upper: i64,
    magnitude: i64,
    grouping: Grouping,
    primary: u8,
    secondary: u8,
    min: u8,
) -> bool {
    if primary == 0 {
        return false;
    }
    let primary = i64::from(primary);
    if magnitude < primary {
        return false;
    }
    let min_grouping = match grouping {
        Grouping::Never => return false,
        Grouping::Always => 1,
        Grouping::Auto => i64::from(min.max(1)),
        Grouping::Min2 => i64::from(min.max(2)),
    };
    if upper < primary + min_grouping - 1 {
        return false;
    }
    let secondary = if secondary == 0 {
        primary
    } else {
        i64::from(secondary)
    };
    (magnitude - primary) % secondary == 0
}

/// The parts of the unsigned number `x` (integer, group, decimal, fraction) with the digits
/// of `nu` and the separators of `icu`.
fn number_parts<D: DecimalValue>(x: &D, nu: &str, icu: &Icu, grouping: Grouping, out: &mut Parts) {
    let digits: [char; 10] = cldr::numbering_system_digits(nu)
        .copied()
        .unwrap_or(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9']);
    let symbols = icu.symbols.get();
    let sizes = symbols.grouping_sizes;
    let (lower, upper) = x.shown();
    let (lower, upper) = (lower.min(0), upper.max(0));
    let mut int = String::new();
    for m in (0..=upper).rev() {
        int.push(digits[usize::from(x.digit_at(m))]);
        if group_after(
            upper,
            m,
            grouping,
            sizes.primary,
            sizes.secondary,
            sizes.min_grouping,
        ) {
            out.push((PartType::Integer, core::mem::take(&mut int)));
            out.push((PartType::Group, symbols.grouping_separator().to_string()));
        }
    }
    out.push((PartType::Integer, int));
    if lower < 0 {
        out.push((PartType::Decimal, symbols.decimal_separator().to_string()));
        let frac: String = (lower..0)
            .rev()
            .map(|m| digits[usize::from(x.digit_at(m))])
            .collect();
        out.push((PartType::Fraction, frac));
    }
}

/// The sign of a formatted value under `[[SignDisplay]]` (GetNumberFormatPattern, §16.5.11):
/// `negative` is the sign of the value (negative-zero included), `zero` whether the rounded
/// value is a zero, `nan` whether it is NaN. `Some(true)` minus, `Some(false)` plus.
fn sign_of(display: SignDisplay, negative: bool, zero: bool, nan: bool) -> Option<bool> {
    match display {
        SignDisplay::Never => None,
        SignDisplay::Auto => negative.then_some(true),
        SignDisplay::Always => Some(negative),
        SignDisplay::ExceptZero => {
            if nan || zero {
                None
            } else {
                Some(negative)
            }
        }
        SignDisplay::Negative => (negative && !zero).then_some(true),
    }
}

/// The literal and compact parts of a compact affix: runs of space characters are literals.
fn affix_parts(affix: &str, out: &mut Parts) {
    let mut run = String::new();
    let mut run_space = false;
    for c in affix.chars() {
        let space = c.is_whitespace() || c == '\u{A0}' || c == '\u{202F}';
        if !run.is_empty() && space != run_space {
            let t = if run_space {
                PartType::Literal
            } else {
                PartType::Compact
            };
            out.push((t, core::mem::take(&mut run)));
        }
        run_space = space;
        run.push(c);
    }
    if !run.is_empty() {
        let t = if run_space {
            PartType::Literal
        } else {
            PartType::Compact
        };
        out.push((t, run));
    }
}

/// ComputeExponent ( numberFormat, x ) (§16.5.12) for compact notation, and the rounded
/// x × 10^-exponent: the exponent of the ICU4X compact data for the magnitude of x, again for
/// magnitude + 1 if the rounding moved the magnitude. A magnitude above i16::MAX (a
/// WideDecimal) takes the exponent of i16::MAX: the top pattern of the data.
fn compact_round<D: DecimalValue>(
    c: &CompactDecimalFormatter,
    digits: &DigitOptions,
    x: &D,
) -> (u8, D) {
    let round = |e: u8| {
        let mut y = x.clone();
        y.multiply_pow10(-i64::from(e));
        format_numeric_to_string(digits, &mut y);
        y
    };
    let exponent_for = |m: i64| {
        c.compact_exponent_for_magnitude(m.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16)
    };
    // 1. If x = 0, return 0.
    if x.is_zero() {
        return (0, round(0));
    }
    // 3-4. The magnitude and its exponent.
    let magnitude = x.nonzero_magnitude_start();
    let e = exponent_for(magnitude);
    // 5-6. x × 10^-e, rounded.
    let y = round(e);
    // 7-9.
    if y.is_zero() || y.nonzero_magnitude_start() == magnitude - i64::from(e) {
        return (e, y);
    }
    // 10. ComputeExponentForMagnitude(magnitude + 1).
    let e = exponent_for(magnitude + 1);
    (e, round(e))
}

/// The prefix and suffix of the compact pattern of the unsigned significand `y` with the
/// exponent `e` (the ICU4X compact text split around the plain number text). `y` is the
/// [`DecimalValue::plural_decimal`] of the significand: the pattern depends only on the
/// magnitude and the plural category.
fn compact_affixes(icu: &Icu, y: &Decimal, e: u8) -> Option<(String, String)> {
    let (c, plain) = (icu.compact.as_ref()?, icu.plain.as_ref()?);
    let mut abs = y.clone();
    abs.sign = Sign::None;
    let whole = c.format_with_exponent(&abs, e).ok()?.to_string();
    let number = plain.format(&abs).to_string();
    let i = whole.find(&number)?;
    Some((
        whole[..i].to_string(),
        whole[i + number.len()..].to_string(),
    ))
}

/// The body parts of the finite value `d` formatted with `f` (compact affixes and the
/// number, no sign), and whether the rounded value is zero.
fn finite_parts<D: DecimalValue>(f: &Format<'_>, icu: &Icu, d: &D, body: &mut Parts) -> bool {
    let (exponent, y) = match (f.compact, icu.compact.as_ref()) {
        (Some(_), Some(c)) => compact_round(c, &f.digits, d),
        _ => {
            let mut y = d.clone();
            format_numeric_to_string(&f.digits, &mut y);
            (0, y)
        }
    };
    let affixes = if exponent > 0 {
        compact_affixes(icu, &y.plural_decimal(), exponent)
    } else {
        None
    };
    if let Some((prefix, _)) = &affixes {
        affix_parts(prefix, body);
    }
    number_parts(&y, f.numbering_system, icu, f.grouping, body);
    if let Some((_, suffix)) = &affixes {
        affix_parts(suffix, body);
    }
    y.is_zero()
}

/// PartitionNumberPattern ( numberFormat, x ) (§16.5.4) for the decimal style: the parts of
/// `x` formatted with `f`.
pub fn partition(f: &Format<'_>, x: &IntlMv) -> Parts {
    let mut out = Parts::new();
    let Some(icu) = icu_for(f) else {
        return out;
    };
    let symbols = icu.symbols.get();
    let t1 = t1_symbols(f.data_locale, f.numbering_system);
    // The number and its sign.
    let (negative, nan) = match x {
        IntlMv::NaN => (false, true),
        IntlMv::PosInf => (false, false),
        IntlMv::NegInf => (true, false),
        IntlMv::Finite(d) => (d.is_negative(), false),
        IntlMv::Wide(d) => (d.is_negative(), false),
    };
    let mut body = Parts::new();
    let rounded_zero = match x {
        IntlMv::NaN => {
            body.push((PartType::Nan, t1.map_or("NaN", |t| t.nan).to_string()));
            false
        }
        IntlMv::PosInf | IntlMv::NegInf => {
            body.push((
                PartType::Infinity,
                t1.map_or("∞", |t| t.infinity).to_string(),
            ));
            false
        }
        IntlMv::Finite(d) => finite_parts(f, &icu, d, &mut body),
        IntlMv::Wide(d) => finite_parts(f, &icu, d, &mut body),
    };
    match sign_of(f.sign_display, negative, rounded_zero, nan) {
        Some(true) => {
            push_text(&mut out, PartType::MinusSign, symbols.minus_sign_prefix());
            out.extend(body);
            push_text(&mut out, PartType::MinusSign, symbols.minus_sign_suffix());
        }
        Some(false) => {
            push_text(&mut out, PartType::PlusSign, symbols.plus_sign_prefix());
            out.extend(body);
            push_text(&mut out, PartType::PlusSign, symbols.plus_sign_suffix());
        }
        None => out.extend(body),
    }
    out
}

/// Pushes a part of type `t` with `text`, if `text` is not empty.
fn push_text(out: &mut Parts, t: PartType, text: &str) {
    if !text.is_empty() {
        out.push((t, text.to_string()));
    }
}

/// The text of `parts`.
pub fn join(parts: &Parts) -> String {
    parts.iter().map(|(_, s)| s.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INTL-V2-FP2: the digits of 1e21, 5e-324 and 0.1 + 0.2 are the digits of
    /// Number::toString ("1e+21", "5e-324", "0.30000000000000004"), read into a Decimal.
    #[test]
    fn number_digits_are_number_to_string() {
        let d = decimal_from_literal("1e+21").unwrap();
        assert_eq!(d.to_string(), "1000000000000000000000");
        let d = decimal_from_literal("5e-324").unwrap();
        assert_eq!(d.absolute.nonzero_magnitude_start(), -324);
        assert_eq!(d.absolute.nonzero_magnitude_end(), -324);
        assert_eq!(d.absolute.digit_at(-324), 5);
        let d = decimal_from_literal("0.30000000000000004").unwrap();
        assert_eq!(d.to_string(), "0.30000000000000004");
        let d = decimal_from_literal("-1.5e-7").unwrap();
        assert_eq!(d.to_string(), "-0.00000015");
    }

    /// INTL-V2-D20: a String input gives a mathematical value, so the trailing fraction zeros
    /// go ("1.500" is 1.5; "012.340" is 12.34).
    #[test]
    fn string_values_have_no_trailing_zeros() {
        assert_eq!(decimal_from_literal("1.500").unwrap().to_string(), "1.5");
        assert_eq!(
            decimal_from_literal("012.340").unwrap().to_string(),
            "12.34"
        );
        assert_eq!(decimal_from_literal("5.").unwrap().to_string(), "5");
        assert_eq!(decimal_from_literal(".5e1").unwrap().to_string(), "5");
        assert_eq!(decimal_from_literal("+7").unwrap().to_string(), "7");
        assert!(decimal_from_literal("1e").is_none());
        assert!(decimal_from_literal(".").is_none());
        assert!(decimal_from_literal("1_0").is_none());
        assert_eq!(decimal_from_radix("ff", 16).unwrap().to_string(), "255");
        assert_eq!(
            decimal_from_radix("10000000000000000000000", 16)
                .unwrap()
                .to_string(),
            "309485009821345068724781056"
        );
    }

    /// The literal forms of ECMA-262 StrDecimalLiteral that `Decimal::try_from_str` does
    /// not read, and a hex value.
    #[test]
    fn literals() {
        let s = |t: &str| decimal_from_literal(t).map(|d| d.to_string());
        assert_eq!(s("1e+21").as_deref(), Some("1000000000000000000000"));
        assert_eq!(s("1.500").as_deref(), Some("1.5"));
        assert_eq!(s("-0.0012").as_deref(), Some("-0.0012"));
        assert_eq!(s("5.").as_deref(), Some("5"));
        assert_eq!(s(".5").as_deref(), Some("0.5"));
        assert_eq!(s("1x"), None);
        assert_eq!(
            decimal_from_radix("ff", 16)
                .map(|d| d.to_string())
                .as_deref(),
            Some("255")
        );
    }

    /// The Decimal limit (fixed_decimal 0.7.3: i16 magnitudes): 32,769 digits do not parse,
    /// so a BigInt takes a WideDecimal from WIDE_DIGITS (32,768) digits on, also at the
    /// edge where a carry would pass magnitude 32,767.
    #[test]
    fn large_integers_are_wide() {
        let digits = |n: usize| format!("1{}", "0".repeat(n - 1));
        assert!(decimal_from_literal(&digits(32_769)).is_none());
        assert!(matches!(mv_of_integer(&digits(32_767)), IntlMv::Finite(_)));
        assert!(matches!(mv_of_integer(&digits(32_768)), IntlMv::Wide(_)));
        match mv_of_integer(&format!("-{}", digits(40_000))) {
            IntlMv::Wide(w) => {
                assert!(w.is_negative());
                assert_eq!(w.nonzero_magnitude_start(), 39_999);
            }
            other => panic!("{other:?}"),
        }
    }

    /// The T1 lookup finds the CLDR parent locales ("zh-TW" has the zh-Hant NaN).
    #[test]
    fn t1_finds_cldr_parents() {
        assert_eq!(t1_symbols("zh-TW", "latn").map(|t| t.nan), Some("非數值"));
        assert_eq!(t1_symbols("en-US", "latn").map(|t| t.nan), Some("NaN"));
        assert_eq!(t1_symbols("zz", "latn").map(|t| t.nan), Some("NaN"));
    }

    #[test]
    fn grouping_as_icu4x() {
        // en: primary 3, min 1. "1,000"; min2: "1000", "10,000".
        assert!(group_after(3, 3, Grouping::Auto, 3, 3, 1));
        assert!(!group_after(3, 3, Grouping::Min2, 3, 3, 1));
        assert!(group_after(4, 3, Grouping::Min2, 3, 3, 1));
        // es: min 2: "1000" for auto, "1.000" for always.
        assert!(!group_after(3, 3, Grouping::Auto, 3, 3, 2));
        assert!(group_after(3, 3, Grouping::Always, 3, 3, 2));
        // en-IN: 3 then 2: 1,00,000.
        assert!(group_after(5, 5, Grouping::Auto, 3, 2, 1));
        assert!(!group_after(5, 4, Grouping::Auto, 3, 2, 1));
        assert!(!group_after(9, 9, Grouping::Never, 3, 3, 1));
    }
}
