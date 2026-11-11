//! Number digit options (ECMA-402 §16.1.2 SetNumberFormatDigitOptions) and the rounding of
//! FormatNumericToString (§16.5.2, ToRawPrecision §16.5.7, ToRawFixed §16.5.8) on
//! `fixed_decimal::Decimal` and [`WideDecimal`]. PluralRules (W2) and NumberFormat (W2) share them (INTL-I1
//! §4.1).
//!
//! ```text
//! options ─ set_number_format_digit_options(mnfdDefault, mxfdDefault, notation) ─> DigitOptions
//!           (the [[MinimumIntegerDigits]] .. [[ComputedRoundingPriority]] fields; packed into
//!            the slots of the service object by the service)
//! Decimal x ─ format_numeric_to_string(&DigitOptions) ─> x rounded and padded (the
//!             [[RoundedNumber]] and its digits; FormattedString = x.unsigned_text())
//! WideDecimal x: the same steps (trait DecimalValue), for a BigInt of WIDE_DIGITS or more
//!             digits, which a Decimal (i16 magnitudes) cannot hold
//! ```
//!
//! Interrupt polls (H23): each pass over the digits of a WideDecimal counts its digits in a
//! [`Work`] (one budget unit per [`POLL_DIGITS`]), and a termination stops it with
//! [`Interrupted`]. The builtin makes the `Work` with [`polled`], which turns `Interrupted`
//! into the `Err` of `Cx::poll_interrupt`. A Decimal (at most 2^16 digits) does not count
//! its own copy, rounding and text.

use ep9_vm::{Cx, JsObject, JsResult, Local};
use fixed_decimal::{Decimal, RoundingIncrement, Sign, SignedRoundingMode, UnsignedRoundingMode};

use crate::options::{default_number_option, get_number_option, get_string_option};

/// The numbering systems with simple digit mappings (ECMA-402 Table 14): the digit zero of
/// each system whose digits 0-9 are contiguous code points. `hanidec` is not contiguous: see
/// [`HANIDEC`]. The ICU4X decimal data has only the systems that CLDR locales use, so a
/// service formats the others with this table (each entry checked against node v26.13.0).
pub const SIMPLE_DIGITS: &[(&str, char)] = &[
    ("adlm", '\u{1E950}'),
    ("ahom", '\u{11730}'),
    ("arab", '\u{0660}'),
    ("arabext", '\u{06F0}'),
    ("bali", '\u{1B50}'),
    ("beng", '\u{09E6}'),
    ("bhks", '\u{11C50}'),
    ("brah", '\u{11066}'),
    ("cakm", '\u{11136}'),
    ("cham", '\u{AA50}'),
    ("deva", '\u{0966}'),
    ("diak", '\u{11950}'),
    ("fullwide", '\u{FF10}'),
    ("gara", '\u{10D40}'),
    ("gong", '\u{11DA0}'),
    ("gonm", '\u{11D50}'),
    ("gujr", '\u{0AE6}'),
    ("gukh", '\u{16130}'),
    ("guru", '\u{0A66}'),
    ("hmng", '\u{16B50}'),
    ("hmnp", '\u{1E140}'),
    ("java", '\u{A9D0}'),
    ("kali", '\u{A900}'),
    ("kawi", '\u{11F50}'),
    ("khmr", '\u{17E0}'),
    ("knda", '\u{0CE6}'),
    ("krai", '\u{16D70}'),
    ("lana", '\u{1A80}'),
    ("lanatham", '\u{1A90}'),
    ("laoo", '\u{0ED0}'),
    ("latn", '0'),
    ("lepc", '\u{1C40}'),
    ("limb", '\u{1946}'),
    ("mathbold", '\u{1D7CE}'),
    ("mathdbl", '\u{1D7D8}'),
    ("mathmono", '\u{1D7F6}'),
    ("mathsanb", '\u{1D7EC}'),
    ("mathsans", '\u{1D7E2}'),
    ("mlym", '\u{0D66}'),
    ("modi", '\u{11650}'),
    ("mong", '\u{1810}'),
    ("mroo", '\u{16A60}'),
    ("mtei", '\u{ABF0}'),
    ("mymr", '\u{1040}'),
    ("mymrepka", '\u{116DA}'),
    ("mymrpao", '\u{116D0}'),
    ("mymrshan", '\u{1090}'),
    ("mymrtlng", '\u{A9F0}'),
    ("nagm", '\u{1E4F0}'),
    ("newa", '\u{11450}'),
    ("nkoo", '\u{07C0}'),
    ("olck", '\u{1C50}'),
    ("onao", '\u{1E5F1}'),
    ("orya", '\u{0B66}'),
    ("osma", '\u{104A0}'),
    ("outlined", '\u{1CCF0}'),
    ("rohg", '\u{10D30}'),
    ("saur", '\u{A8D0}'),
    ("segment", '\u{1FBF0}'),
    ("shrd", '\u{111D0}'),
    ("sind", '\u{112F0}'),
    ("sinh", '\u{0DE6}'),
    ("sora", '\u{110F0}'),
    ("sund", '\u{1BB0}'),
    ("sunu", '\u{11BF0}'),
    ("takr", '\u{116C0}'),
    ("talu", '\u{19D0}'),
    ("tamldec", '\u{0BE6}'),
    ("telu", '\u{0C66}'),
    ("thai", '\u{0E50}'),
    ("tibt", '\u{0F20}'),
    ("tirh", '\u{114D0}'),
    ("tnsa", '\u{16AC0}'),
    ("tols", '\u{11DE0}'),
    ("vaii", '\u{A620}'),
    ("wara", '\u{118E0}'),
    ("wcho", '\u{1E2F0}'),
];

/// The `hanidec` digits 0-9 (not contiguous).
pub const HANIDEC: [char; 10] = [
    '\u{3007}', '\u{4E00}', '\u{4E8C}', '\u{4E09}', '\u{56DB}', '\u{4E94}', '\u{516D}', '\u{4E03}',
    '\u{516B}', '\u{4E5D}',
];

/// The digit `d` (0-9) of the numbering system `system`, or `None` for a system with no
/// simple digit mapping.
pub fn simple_digit(system: &str, d: u8) -> Option<char> {
    if d > 9 {
        return None;
    }
    if system == "hanidec" {
        return Some(HANIDEC[usize::from(d)]);
    }
    let zero = SIMPLE_DIGITS.iter().find(|(n, _)| *n == system)?.1;
    char::from_u32(zero as u32 + u32::from(d))
}

/// `[[RoundingMode]]` (the 9 values of §16.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    Ceil,
    Floor,
    Expand,
    Trunc,
    HalfCeil,
    HalfFloor,
    HalfExpand,
    HalfTrunc,
    HalfEven,
}

impl RoundingMode {
    /// The option strings, in the order of the spec's list.
    pub const NAMES: [&'static str; 9] = [
        "ceil",
        "floor",
        "expand",
        "trunc",
        "halfCeil",
        "halfFloor",
        "halfExpand",
        "halfTrunc",
        "halfEven",
    ];

    pub(crate) const ALL: [RoundingMode; 9] = [
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

    /// The option string.
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    fn from_name(s: &str) -> Option<RoundingMode> {
        Self::NAMES
            .iter()
            .position(|n| *n == s)
            .map(|i| Self::ALL[i])
    }

    /// GetUnsignedRoundingMode ( roundingMode, sign ) (§16.5.16) for a value with the sign
    /// `negative`.
    fn unsigned(self, negative: bool) -> UnsignedRoundingMode {
        use UnsignedRoundingMode as U;
        match (self, negative) {
            (RoundingMode::Ceil, false)
            | (RoundingMode::Floor, true)
            | (RoundingMode::Expand, _) => U::Expand,
            (RoundingMode::Ceil, true)
            | (RoundingMode::Floor, false)
            | (RoundingMode::Trunc, _) => U::Trunc,
            (RoundingMode::HalfCeil, false)
            | (RoundingMode::HalfFloor, true)
            | (RoundingMode::HalfExpand, _) => U::HalfExpand,
            (RoundingMode::HalfCeil, true)
            | (RoundingMode::HalfFloor, false)
            | (RoundingMode::HalfTrunc, _) => U::HalfTrunc,
            (RoundingMode::HalfEven, _) => U::HalfEven,
        }
    }

    fn signed(self) -> SignedRoundingMode {
        use UnsignedRoundingMode as U;
        match self {
            RoundingMode::Ceil => SignedRoundingMode::Ceil,
            RoundingMode::Floor => SignedRoundingMode::Floor,
            RoundingMode::Expand => SignedRoundingMode::Unsigned(U::Expand),
            RoundingMode::Trunc => SignedRoundingMode::Unsigned(U::Trunc),
            RoundingMode::HalfCeil => SignedRoundingMode::HalfCeil,
            RoundingMode::HalfFloor => SignedRoundingMode::HalfFloor,
            RoundingMode::HalfExpand => SignedRoundingMode::Unsigned(U::HalfExpand),
            RoundingMode::HalfTrunc => SignedRoundingMode::Unsigned(U::HalfTrunc),
            RoundingMode::HalfEven => SignedRoundingMode::Unsigned(U::HalfEven),
        }
    }
}

/// `[[RoundingType]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingType {
    FractionDigits,
    SignificantDigits,
    MorePrecision,
    LessPrecision,
}

/// The digit fields that SetNumberFormatDigitOptions sets on the Intl object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigitOptions {
    pub minimum_integer_digits: u8,
    pub minimum_fraction_digits: u8,
    pub maximum_fraction_digits: u8,
    pub minimum_significant_digits: u8,
    pub maximum_significant_digits: u8,
    pub rounding_increment: u16,
    pub rounding_mode: RoundingMode,
    pub rounding_type: RoundingType,
    /// `[[ComputedRoundingPriority]]`: "auto", "morePrecision" or "lessPrecision".
    pub computed_rounding_priority: &'static str,
    /// `[[TrailingZeroDisplay]]`: false "auto", true "stripIfInteger".
    pub strip_if_integer: bool,
}

/// The allowed `roundingIncrement` values (§16.1.2).
const INCREMENTS: [u16; 15] = [
    1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000,
];

/// SetNumberFormatDigitOptions ( intlObj, options, mnfdDefault, mxfdDefault, notation )
/// (§16.1.2). `compact`: notation is "compact".
pub fn set_number_format_digit_options<'s>(
    cx: &mut Cx<'s>,
    options: Local<'s, JsObject>,
    mnfd_default: u8,
    mxfd_default: u8,
    compact: bool,
) -> JsResult<DigitOptions> {
    // 1. Let mnid be ? GetNumberOption(options, "minimumIntegerDigits", 1, 21, 1).
    let mnid = get_number_option(cx, options, "minimumIntegerDigits", 1, 21, Some(1))?.unwrap_or(1);
    // 2-5.
    let mnfd = cx.get(options, "minimumFractionDigits")?;
    let mxfd = cx.get(options, "maximumFractionDigits")?;
    let mnsd = cx.get(options, "minimumSignificantDigits")?;
    let mxsd = cx.get(options, "maximumSignificantDigits")?;
    // 7. Let roundingIncrement be ? GetNumberOption(options, "roundingIncrement", 1, 5000, 1).
    let increment =
        get_number_option(cx, options, "roundingIncrement", 1, 5000, Some(1))?.unwrap_or(1) as u16;
    // 8. If roundingIncrement is not in the list, throw a RangeError exception.
    if !INCREMENTS.contains(&increment) {
        return Err(cx.throw_range_error("roundingIncrement value is out of range"));
    }
    // 9. roundingMode.
    let mode = get_string_option(
        cx,
        options,
        "roundingMode",
        &RoundingMode::NAMES,
        Some("halfExpand"),
    )?
    .and_then(|s| RoundingMode::from_name(&s))
    .unwrap_or(RoundingMode::HalfExpand);
    // 10. roundingPriority.
    let priority = get_string_option(
        cx,
        options,
        "roundingPriority",
        &["auto", "morePrecision", "lessPrecision"],
        Some("auto"),
    )?
    .unwrap_or_default();
    // 11. trailingZeroDisplay.
    let tzd = get_string_option(
        cx,
        options,
        "trailingZeroDisplay",
        &["auto", "stripIfInteger"],
        Some("auto"),
    )?
    .unwrap_or_default();
    // 13. If roundingIncrement is not 1, set mxfdDefault to mnfdDefault.
    let mxfd_default = if increment != 1 {
        mnfd_default
    } else {
        mxfd_default
    };
    let mut out = DigitOptions {
        minimum_integer_digits: mnid as u8,
        minimum_fraction_digits: mnfd_default,
        maximum_fraction_digits: mxfd_default,
        minimum_significant_digits: 1,
        maximum_significant_digits: 21,
        rounding_increment: increment,
        rounding_mode: mode,
        rounding_type: RoundingType::FractionDigits,
        computed_rounding_priority: "auto",
        strip_if_integer: tzd == "stripIfInteger",
    };
    // 17-18.
    let has_sd = !(mnsd.is_undefined() && mxsd.is_undefined());
    let has_fd = !(mnfd.is_undefined() && mxfd.is_undefined());
    // 19-21.
    let mut need_sd = true;
    let mut need_fd = true;
    if priority == "auto" {
        need_sd = has_sd;
        if need_sd || (!has_fd && compact) {
            need_fd = false;
        }
    }
    // 22.
    if need_sd {
        if has_sd {
            let lo = default_number_option(cx, mnsd, "minimumSignificantDigits", 1, 21, Some(1))?
                .unwrap_or(1);
            let hi = default_number_option(cx, mxsd, "maximumSignificantDigits", lo, 21, Some(21))?
                .unwrap_or(21);
            out.minimum_significant_digits = lo as u8;
            out.maximum_significant_digits = hi as u8;
        } else {
            out.minimum_significant_digits = 1;
            out.maximum_significant_digits = 21;
        }
    }
    // 23.
    if need_fd {
        if has_fd {
            let lo = default_number_option(cx, mnfd, "minimumFractionDigits", 0, 100, None)?;
            let hi = default_number_option(cx, mxfd, "maximumFractionDigits", 0, 100, None)?;
            let (lo, hi) = match (lo, hi) {
                (None, Some(h)) => (i32::from(mnfd_default).min(h), h),
                (Some(l), None) => (l, i32::from(mxfd_default).max(l)),
                (Some(l), Some(h)) => {
                    if l > h {
                        return Err(cx.throw_range_error(
                            "minimumFractionDigits is greater than maximumFractionDigits",
                        ));
                    }
                    (l, h)
                }
                (None, None) => (i32::from(mnfd_default), i32::from(mxfd_default)),
            };
            out.minimum_fraction_digits = lo as u8;
            out.maximum_fraction_digits = hi as u8;
        } else {
            out.minimum_fraction_digits = mnfd_default;
            out.maximum_fraction_digits = mxfd_default;
        }
    }
    // 24-26.
    if !need_sd && !need_fd {
        out.minimum_fraction_digits = 0;
        out.maximum_fraction_digits = 0;
        out.minimum_significant_digits = 1;
        out.maximum_significant_digits = 2;
        out.rounding_type = RoundingType::MorePrecision;
        out.computed_rounding_priority = "morePrecision";
    } else if priority == "auto" {
        out.rounding_type = if need_sd {
            RoundingType::SignificantDigits
        } else {
            RoundingType::FractionDigits
        };
        out.computed_rounding_priority = "auto";
    } else if priority == "morePrecision" {
        out.rounding_type = RoundingType::MorePrecision;
        out.computed_rounding_priority = "morePrecision";
    } else {
        out.rounding_type = RoundingType::LessPrecision;
        out.computed_rounding_priority = "lessPrecision";
    }
    // 27.
    if increment != 1 {
        if out.rounding_type != RoundingType::FractionDigits {
            return Err(cx.throw_type_error(
                "roundingIncrement needs fraction digit rounding (no significant digits)",
            ));
        }
        if out.maximum_fraction_digits != out.minimum_fraction_digits {
            return Err(cx.throw_range_error(
                "roundingIncrement needs maximumFractionDigits equal to minimumFractionDigits",
            ));
        }
    }
    Ok(out)
}

// ---- interrupt polls (H23 "Interrupt poll") ---------------------------------------------------

/// A poll returned `false` (an isolate termination): the work stops. [`polled`] gives the
/// builtin the `Err` of the poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interrupted;

/// The digits of work per interrupt budget unit (DL M6-ORCH-457). A T0 tuning value (H23
/// Freeze table "tuning (gated)"), as the 256 code units of a string search step.
pub const POLL_DIGITS: usize = 4096;

/// The poll counter of one long WideDecimal operation (the `ep9_bigint::Work` form; this
/// crate does not depend on `ep9_bigint`): [`Work::add`] counts digits and calls the poll
/// once per [`POLL_DIGITS`] of them.
pub struct Work<'p> {
    poll: &'p mut dyn FnMut() -> bool,
    digits: usize,
}

impl<'p> Work<'p> {
    /// A counter at 0 that calls `poll`.
    pub fn new(poll: &'p mut dyn FnMut() -> bool) -> Work<'p> {
        Work { poll, digits: 0 }
    }

    /// Counts `n` digits of work and calls the poll once per [`POLL_DIGITS`] of them.
    /// `Err(Interrupted)` as soon as a call returns `false`.
    pub fn add(&mut self, n: usize) -> Result<(), Interrupted> {
        self.digits += n;
        while self.digits >= POLL_DIGITS {
            self.digits -= POLL_DIGITS;
            if !(self.poll)() {
                return Err(Interrupted);
            }
        }
        Ok(())
    }
}

/// Runs `f` with a [`Work`] whose poll is `Cx::poll_interrupt`: the result of `f`, or the
/// `Err` of the poll that stopped it (a termination).
pub fn polled<'s, T>(
    cx: &mut Cx<'s>,
    f: impl FnOnce(&mut Work<'_>) -> Result<T, Interrupted>,
) -> JsResult<T> {
    let mut stop = Ok(());
    let r = f(&mut Work::new(&mut || {
        stop = cx.poll_interrupt();
        stop.is_ok()
    }));
    stop?;
    // `Interrupted` comes only from a poll that returned `false`, so `stop` is then `Err`.
    Ok(r.unwrap_or_else(|Interrupted| unreachable!("Interrupted after an Ok poll")))
}

/// Runs `f` with a [`Work`] whose poll never stops it (the unit tests).
#[cfg(test)]
pub(crate) fn unpolled<T>(f: impl FnOnce(&mut Work<'_>) -> Result<T, Interrupted>) -> T {
    let mut poll = || true;
    f(&mut Work::new(&mut poll)).unwrap_or_else(|Interrupted| unreachable!("no stop"))
}

// ---- the value of FormatNumericToString ------------------------------------------------------

/// The operations of FormatNumericToString on a decimal value, with the magnitudes as i64:
/// `Decimal` (fixed_decimal; magnitudes are i16) and [`WideDecimal`] (any size). Each
/// method acts as the `fixed_decimal` method of the same name. The display range is the
/// range of the digits that the text shows: the padding zeros are in it.
pub trait DecimalValue: Clone {
    /// Whether the sign is negative (negative zero too).
    fn is_negative(&self) -> bool;
    fn is_zero(&self) -> bool;
    /// The magnitude of the highest nonzero digit (0 for zero).
    fn nonzero_magnitude_start(&self) -> i64;
    /// The magnitude of the lowest nonzero digit (0 for zero).
    fn nonzero_magnitude_end(&self) -> i64;
    /// The display range: (lowest, highest) magnitude.
    fn shown(&self) -> (i64, i64);
    /// The digit at magnitude `m` (0 outside the digits).
    fn digit_at(&self, m: i64) -> u8;
    fn multiply_pow10(&mut self, k: i64);
    /// A copy (the passes over the digits count in `work`).
    fn copy(&self, work: &mut Work) -> Result<Self, Interrupted>;
    /// Rounds to a multiple of `increment` × 10^`position` with `mode` (the sign of the value
    /// picks the unsigned mode).
    fn round(
        &mut self,
        position: i64,
        mode: RoundingMode,
        increment: RoundingIncrement,
        work: &mut Work,
    ) -> Result<(), Interrupted>;
    fn trim_end(&mut self);
    fn trim_end_if_integer(&mut self);
    /// Removes the leading zero padding: the highest shown magnitude is then
    /// max(0, the highest nonzero magnitude).
    fn trim_start(&mut self);
    fn pad_start(&mut self, position: i64);
    fn pad_end(&mut self, position: i64);
    /// The digits in the display range, with a '.' before magnitude -1 and no sign: the
    /// `[[FormattedString]]` of FormatNumericToString (§16.5.2, made from the unsigned value).
    fn unsigned_text(&self, work: &mut Work) -> Result<String, Interrupted>;
    /// A Decimal with the same ICU4X plural operands and compact pattern (for a value with
    /// an integer part above 18 digits, see [`WideDecimal`]); the value itself for a Decimal.
    fn plural_decimal(&self) -> Decimal;
}

/// `m` as an i16 magnitude (a Decimal magnitude is in the i16 range).
fn m16(m: i64) -> i16 {
    m.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

impl DecimalValue for Decimal {
    fn is_negative(&self) -> bool {
        self.sign == Sign::Negative
    }
    fn is_zero(&self) -> bool {
        self.absolute.is_zero()
    }
    fn nonzero_magnitude_start(&self) -> i64 {
        i64::from(self.absolute.nonzero_magnitude_start())
    }
    fn nonzero_magnitude_end(&self) -> i64 {
        i64::from(self.absolute.nonzero_magnitude_end())
    }
    fn shown(&self) -> (i64, i64) {
        let r = self.absolute.magnitude_range();
        (i64::from(*r.start()), i64::from(*r.end()))
    }
    fn digit_at(&self, m: i64) -> u8 {
        self.absolute.digit_at(m16(m))
    }
    fn multiply_pow10(&mut self, k: i64) {
        self.absolute.multiply_pow10(m16(k));
    }
    fn copy(&self, _work: &mut Work) -> Result<Self, Interrupted> {
        Ok(self.clone())
    }
    fn round(
        &mut self,
        position: i64,
        mode: RoundingMode,
        increment: RoundingIncrement,
        _work: &mut Work,
    ) -> Result<(), Interrupted> {
        self.round_with_mode_and_increment(m16(position), mode.signed(), increment);
        Ok(())
    }
    fn trim_end(&mut self) {
        self.absolute.trim_end();
    }
    fn trim_end_if_integer(&mut self) {
        self.absolute.trim_end_if_integer();
    }
    fn trim_start(&mut self) {
        self.absolute.trim_start();
    }
    fn pad_start(&mut self, position: i64) {
        self.absolute.pad_start(m16(position));
    }
    fn pad_end(&mut self, position: i64) {
        self.absolute.pad_end(m16(position));
    }
    fn unsigned_text(&self, _work: &mut Work) -> Result<String, Interrupted> {
        Ok(self.absolute.to_string())
    }
    fn plural_decimal(&self) -> Decimal {
        self.clone()
    }
}

/// The digit count from which a BigInt is a [`WideDecimal`]: a Decimal has i16 magnitudes,
/// so it reads at most 32,768 integer digits (magnitude 32,767), and a rounding carry past
/// magnitude 32,767 sets it to zero (fixed_decimal 0.7.3 `overflow`). So a Decimal holds a
/// BigInt of at most 32,767 digits.
pub const WIDE_DIGITS: usize = 32_768;

/// An exact decimal of any size: a BigInt of [`WIDE_DIGITS`] digits or more (up to the
/// BigInt limit of 2^30 bits, about 323 million digits), and the values that the steps of
/// FormatNumericToString and ComputeExponent make from it. The digits are bytes 0-9, most
/// significant first. Every pass over them is linear in the digit count and counts its digits
/// in a [`Work`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WideDecimal {
    negative: bool,
    /// The digits from the highest nonzero one to the lowest nonzero one (empty for zero).
    digits: Vec<u8>,
    /// The magnitude of `digits[0]` (0 for zero).
    start: i64,
    /// The display range: `lower <= min(0, end)`, `upper >= max(0, start)`.
    lower: i64,
    upper: i64,
}

impl WideDecimal {
    /// The integer with the ASCII digits `digits` (no sign) and the sign `negative`.
    pub fn from_integer_digits(
        negative: bool,
        digits: &[u8],
        work: &mut Work,
    ) -> Result<WideDecimal, Interrupted> {
        debug_assert!(digits.iter().all(u8::is_ascii_digit));
        let mut v = Vec::with_capacity(digits.len());
        for chunk in digits.chunks(POLL_DIGITS) {
            v.extend(chunk.iter().map(|b| b - b'0'));
            work.add(chunk.len())?;
        }
        let start = v.len() as i64 - 1;
        WideDecimal::new(negative, v, start, work)
    }

    /// The value `digits` (most significant first, leading and trailing zeros allowed)
    /// with `digits[0]` at magnitude `start`, with no padding.
    fn new(
        negative: bool,
        mut digits: Vec<u8>,
        mut start: i64,
        work: &mut Work,
    ) -> Result<WideDecimal, Interrupted> {
        let lead = zeros(digits.iter(), work)?;
        if lead > 0 {
            work.add(digits.len() - lead)?;
            digits.drain(..lead);
        }
        start -= lead as i64;
        let trail = zeros(digits.iter().rev(), work)?;
        digits.truncate(digits.len() - trail);
        if digits.is_empty() {
            start = 0;
        }
        let mut w = WideDecimal {
            negative,
            digits,
            start,
            lower: 0,
            upper: 0,
        };
        w.upper = w.start.max(0);
        w.lower = w.end().min(0);
        Ok(w)
    }

    /// The magnitude of the lowest nonzero digit (0 for zero).
    fn end(&self) -> i64 {
        if self.digits.is_empty() {
            0
        } else {
            self.start - self.digits.len() as i64 + 1
        }
    }
}

/// The count of the leading zero digits of `digits` (counted in `work`).
fn zeros<'a>(digits: impl Iterator<Item = &'a u8>, work: &mut Work) -> Result<usize, Interrupted> {
    let mut n = 0;
    for d in digits {
        if *d != 0 {
            break;
        }
        n += 1;
        if n % POLL_DIGITS == 0 {
            work.add(POLL_DIGITS)?;
        }
    }
    work.add(n % POLL_DIGITS)?;
    Ok(n)
}

/// Long division of the digits `n` (most significant first) by `m`: the quotient digits (the
/// same count) and the remainder.
fn div_small(n: &[u8], m: u32, work: &mut Work) -> Result<(Vec<u8>, u32), Interrupted> {
    let mut rem = 0u32;
    let mut q = Vec::with_capacity(n.len());
    for chunk in n.chunks(POLL_DIGITS) {
        q.extend(chunk.iter().map(|d| {
            let v = rem * 10 + u32::from(*d);
            rem = v % m;
            (v / m) as u8
        }));
        work.add(chunk.len())?;
    }
    Ok((q, rem))
}

/// The digits `n` (most significant first) times the small `m`, plus `add` (most significant
/// first; may be one digit longer).
fn mul_add_small(n: &[u8], m: u32, add: u32, work: &mut Work) -> Result<Vec<u8>, Interrupted> {
    let mut out = Vec::with_capacity(n.len() + 2);
    let mut carry = add;
    for chunk in n.rchunks(POLL_DIGITS) {
        for d in chunk.iter().rev() {
            let v = u32::from(*d) * m + carry;
            out.push((v % 10) as u8);
            carry = v / 10;
        }
        work.add(chunk.len())?;
    }
    while carry > 0 {
        out.push((carry % 10) as u8);
        carry /= 10;
    }
    work.add(out.len())?;
    out.reverse();
    Ok(out)
}

impl DecimalValue for WideDecimal {
    fn is_negative(&self) -> bool {
        self.negative
    }
    fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }
    fn nonzero_magnitude_start(&self) -> i64 {
        self.start
    }
    fn nonzero_magnitude_end(&self) -> i64 {
        self.end()
    }
    fn shown(&self) -> (i64, i64) {
        (self.lower, self.upper)
    }
    fn digit_at(&self, m: i64) -> u8 {
        if self.digits.is_empty() || m > self.start || m < self.end() {
            0
        } else {
            self.digits[(self.start - m) as usize]
        }
    }
    fn copy(&self, work: &mut Work) -> Result<Self, Interrupted> {
        work.add(self.digits.len())?;
        Ok(self.clone())
    }
    fn multiply_pow10(&mut self, k: i64) {
        if !self.digits.is_empty() {
            self.start += k;
        }
        self.upper = (self.upper + k).max(0);
        self.lower = (self.lower + k).min(0);
    }
    /// ApplyUnsignedRoundingMode (§16.5.17) on |x| with the multiples of increment ×
    /// 10^position: |x| = q × unit + rest, 0 <= rest < unit; rest is compared with unit / 2.
    fn round(
        &mut self,
        position: i64,
        mode: RoundingMode,
        increment: RoundingIncrement,
        work: &mut Work,
    ) -> Result<(), Interrupted> {
        let m: u32 = match increment {
            RoundingIncrement::MultiplesOf2 => 2,
            RoundingIncrement::MultiplesOf5 => 5,
            RoundingIncrement::MultiplesOf25 => 25,
            _ => 1,
        };
        if self.digits.is_empty() || (m == 1 && position <= self.end()) {
            return Ok(());
        }
        let end = self.end();
        // The digits at magnitude >= position, as an integer (zeros below `end`).
        let high: Vec<u8> = if position > self.start {
            Vec::new()
        } else {
            work.add((self.start - position + 1) as usize)?;
            let mut h = self.digits
                [..((self.start - position + 1) as usize).min(self.digits.len())]
                .to_vec();
            h.resize((self.start - position + 1) as usize, 0);
            h
        };
        let (q, r) = div_small(&high, m, work)?;
        // The digits below `position` as a fraction f of 10^position: f = 0, < 1/2, = 1/2 or
        // > 1/2 (the digits have no trailing zero, so a digit below position - 1 is nonzero
        // when end < position - 1).
        let low_zero = end >= position;
        let first = self.digit_at(position - 1);
        let f_half = if low_zero {
            core::cmp::Ordering::Less
        } else if first != 5 {
            first.cmp(&5)
        } else if end < position - 1 {
            core::cmp::Ordering::Greater
        } else {
            core::cmp::Ordering::Equal
        };
        // rest / unit = (r + f) / m, compared with 1/2: 2r + 2f against m.
        let twice = 2 * r;
        let half = if twice + 2 <= m {
            core::cmp::Ordering::Less
        } else if twice + 1 == m {
            f_half
        } else if twice == m {
            if low_zero {
                core::cmp::Ordering::Equal
            } else {
                core::cmp::Ordering::Greater
            }
        } else {
            core::cmp::Ordering::Greater
        };
        let exact = r == 0 && low_zero;
        let odd = q.last().is_some_and(|d| d % 2 == 1);
        use core::cmp::Ordering as O;
        let up = !exact
            && match mode.unsigned(self.negative) {
                UnsignedRoundingMode::Expand => true,
                UnsignedRoundingMode::Trunc => false,
                UnsignedRoundingMode::HalfExpand => half != O::Less,
                UnsignedRoundingMode::HalfTrunc => half == O::Greater,
                _ => half == O::Greater || (half == O::Equal && odd),
            };
        // (q + up) × m at magnitude `position`.
        let q = mul_add_small(&q, 1, u32::from(up), work)?;
        let digits = mul_add_small(&q, m, 0, work)?;
        let start = position + digits.len() as i64 - 1;
        *self = WideDecimal::new(self.negative, digits, start, work)?;
        Ok(())
    }
    fn trim_end(&mut self) {
        self.lower = self.end().min(0);
    }
    fn trim_end_if_integer(&mut self) {
        if self.end() >= 0 {
            self.lower = 0;
        }
    }
    fn trim_start(&mut self) {
        self.upper = self.start.max(0);
    }
    fn pad_start(&mut self, position: i64) {
        if position <= 0 {
            return;
        }
        self.upper = (position - 1).max(self.start.max(0));
    }
    fn pad_end(&mut self, position: i64) {
        if position > 0 {
            return;
        }
        self.lower = position.min(self.end());
    }
    fn unsigned_text(&self, work: &mut Work) -> Result<String, Interrupted> {
        let mut out = String::with_capacity((self.upper - self.lower + 2) as usize);
        let mut m = self.upper;
        while m >= self.lower {
            if m == -1 {
                out.push('.');
            }
            out.push(char::from(b'0' + self.digit_at(m)));
            work.add(1)?;
            m -= 1;
        }
        Ok(out)
    }
    /// The ICU4X plural operands keep at most 18 integer digits, and mark a larger integer
    /// part (icu_plurals 2.4 `PluralOperands::from_significand_and_exponent`: i = the low 18
    /// digits + 10^18). So the Decimal with a 1 at magnitude 18 (if the value is above it),
    /// the digits at magnitudes 17 to 0, and the fraction digits (with the same display
    /// range) gives the same operands, and the same ICU4X compact pattern (the top one).
    fn plural_decimal(&self) -> Decimal {
        let mut text = String::new();
        let top = if self.upper > 17 {
            text.push('1');
            17
        } else {
            self.upper
        };
        let mut m = top;
        let lower = self.lower.max(-300);
        while m >= lower {
            if m == -1 {
                text.push('.');
            }
            text.push(char::from(b'0' + self.digit_at(m)));
            m -= 1;
        }
        let mut d = Decimal::try_from_str(&text).unwrap_or_default();
        if self.negative {
            d.sign = Sign::Negative;
        }
        d
    }
}

/// The highest magnitude of a nonzero digit of `x`, or 0 for zero (the `e` of ToRawPrecision).
fn magnitude<D: DecimalValue>(x: &D) -> i64 {
    if x.is_zero() {
        0
    } else {
        x.nonzero_magnitude_start()
    }
}

/// ToRawPrecision ( x, minPrecision, maxPrecision, unsignedRoundingMode ) (§16.5.7): rounds
/// `x` to `max` significant digits and keeps at least `min`. Gives `[[RoundingMagnitude]]`.
fn to_raw_precision<D: DecimalValue>(
    x: &mut D,
    min: u8,
    max: u8,
    mode: RoundingMode,
    work: &mut Work,
) -> Result<i64, Interrupted> {
    let (min, max) = (i64::from(min), i64::from(max));
    let e = magnitude(x);
    x.round(e - max + 1, mode, RoundingIncrement::MultiplesOf1, work)?;
    // A carry (9.99 -> 10) moves e up one; n keeps p digits.
    let e = magnitude(x);
    let rm = e - max + 1;
    x.trim_end();
    x.pad_end(rm.min(0).max(e - min + 1).min(0));
    x.pad_start(1);
    Ok(rm)
}

/// ToRawFixed ( x, minFraction, maxFraction, roundingIncrement, unsignedRoundingMode )
/// (§16.5.8): rounds `x` to `max` fraction digits (in steps of `increment`) and keeps at
/// least `min`. Gives `[[RoundingMagnitude]]`.
fn to_raw_fixed<D: DecimalValue>(
    x: &mut D,
    min: u8,
    max: u8,
    increment: u16,
    mode: RoundingMode,
    work: &mut Work,
) -> Result<i64, Interrupted> {
    let f = i64::from(max);
    // increment = m x 10^k with m in {1, 2, 5, 25}.
    let (mut m, mut k) = (increment, 0i64);
    while m % 10 == 0 {
        m /= 10;
        k += 1;
    }
    let inc = match m {
        2 => RoundingIncrement::MultiplesOf2,
        5 => RoundingIncrement::MultiplesOf5,
        25 => RoundingIncrement::MultiplesOf25,
        _ => RoundingIncrement::MultiplesOf1,
    };
    x.round(-f + k, mode, inc, work)?;
    x.trim_end();
    x.pad_end(-i64::from(min));
    x.pad_start(1);
    Ok(-f)
}

/// The rounding of FormatNumericToString ( intlObject, x ) (§16.5.2) on `x`: the rounding of
/// the object's `[[RoundingType]]`, `stripIfInteger`, and the `[[MinimumIntegerDigits]]`
/// padding. `x.unsigned_text()` is then the `[[FormattedString]]`; `x` keeps its sign (the
/// `[[RoundedNumber]]`; -0 keeps the minus sign).
pub fn format_numeric_to_string<D: DecimalValue>(
    opts: &DigitOptions,
    x: &mut D,
    work: &mut Work,
) -> Result<(), Interrupted> {
    let mode = opts.rounding_mode;
    match opts.rounding_type {
        RoundingType::SignificantDigits => {
            to_raw_precision(
                x,
                opts.minimum_significant_digits,
                opts.maximum_significant_digits,
                mode,
                work,
            )?;
        }
        RoundingType::FractionDigits => {
            to_raw_fixed(
                x,
                opts.minimum_fraction_digits,
                opts.maximum_fraction_digits,
                opts.rounding_increment,
                mode,
                work,
            )?;
        }
        RoundingType::MorePrecision | RoundingType::LessPrecision => {
            let mut s = x.copy(work)?;
            let mut f = x.copy(work)?;
            let srm = to_raw_precision(
                &mut s,
                opts.minimum_significant_digits,
                opts.maximum_significant_digits,
                mode,
                work,
            )?;
            let frm = to_raw_fixed(
                &mut f,
                opts.minimum_fraction_digits,
                opts.maximum_fraction_digits,
                opts.rounding_increment,
                mode,
                work,
            )?;
            let s_wins = (srm <= frm) == (opts.rounding_type == RoundingType::MorePrecision);
            *x = if s_wins { s } else { f };
        }
    }
    // 5. If trailingZeroDisplay is "stripIfInteger" and x is an integer, drop the fraction.
    if opts.strip_if_integer {
        x.trim_end_if_integer();
    }
    // 6-9. Pad to [[MinimumIntegerDigits]].
    x.pad_start(i64::from(opts.minimum_integer_digits.max(1)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(rt: RoundingType) -> DigitOptions {
        DigitOptions {
            minimum_integer_digits: 1,
            minimum_fraction_digits: 0,
            maximum_fraction_digits: 3,
            minimum_significant_digits: 1,
            maximum_significant_digits: 21,
            rounding_increment: 1,
            rounding_mode: RoundingMode::HalfExpand,
            rounding_type: rt,
            computed_rounding_priority: "auto",
            strip_if_integer: false,
        }
    }

    fn fmt(o: &DigitOptions, s: &str) -> String {
        let mut x: Decimal = s.parse().unwrap();
        unpolled(|w| format_numeric_to_string(o, &mut x, w));
        x.to_string()
    }

    /// The lines of node v26.13.0 `new Intl.NumberFormat("en", o).format(x)` without grouping.
    #[test]
    fn rounding_as_node() {
        let fd = opts(RoundingType::FractionDigits);
        assert_eq!(fmt(&fd, "1.23456"), "1.235");
        assert_eq!(fmt(&fd, "1.5"), "1.5");
        assert_eq!(fmt(&fd, "0"), "0");
        let mut two = fd;
        two.minimum_fraction_digits = 2;
        two.maximum_fraction_digits = 2;
        assert_eq!(fmt(&two, "1"), "1.00");
        let mut inc = two;
        inc.rounding_increment = 5;
        assert_eq!(fmt(&inc, "1.234"), "1.25");
        let mut sd = opts(RoundingType::SignificantDigits);
        sd.maximum_significant_digits = 2;
        assert_eq!(fmt(&sd, "9.99"), "10");
        assert_eq!(fmt(&sd, "0.012345"), "0.012");
        assert_eq!(fmt(&sd, "12345"), "12000");
        sd.minimum_significant_digits = 2;
        assert_eq!(fmt(&sd, "1"), "1.0");
        let mut mi = fd;
        mi.minimum_integer_digits = 3;
        assert_eq!(fmt(&mi, "5.5"), "005.5");
        let mut strip = two;
        strip.strip_if_integer = true;
        assert_eq!(fmt(&strip, "2"), "2");
        assert_eq!(fmt(&strip, "2.5"), "2.50");
    }

    /// A WideDecimal rounds as a Decimal: the same value for each rounding mode, increment
    /// and position on values that both hold (the Decimal is the reference).
    #[test]
    fn wide_rounds_as_decimal() {
        let values = [
            "225", "235", "-225", "2251", "-2249", "995", "1000", "7", "-7", "12345", "0", "-0",
            "4999", "5001", "25", "75", "-125",
        ];
        for v in values {
            let negative = v.starts_with('-');
            let digits = v.trim_start_matches('-');
            for mode in RoundingMode::ALL {
                for inc in [
                    RoundingIncrement::MultiplesOf1,
                    RoundingIncrement::MultiplesOf2,
                    RoundingIncrement::MultiplesOf5,
                    RoundingIncrement::MultiplesOf25,
                ] {
                    for position in 0..6 {
                        let mut d: Decimal = v.parse().unwrap();
                        let mut w = unpolled(|k| {
                            WideDecimal::from_integer_digits(negative, digits.as_bytes(), k)
                        });
                        unpolled(|k| DecimalValue::round(&mut d, position, mode, inc, k));
                        unpolled(|k| w.round(position, mode, inc, k));
                        // The value (the display range after a rounding is not used: the
                        // callers set it with trim_end and the pads).
                        let value = |t: String| {
                            let t = t.trim_start_matches('0').to_string();
                            if t.is_empty() { "0".to_string() } else { t }
                        };
                        assert_eq!(
                            (value(unpolled(|k| w.unsigned_text(k))), w.is_negative()),
                            (value(unpolled(|k| d.unsigned_text(k))), d.is_negative()),
                            "{v} {mode:?} {inc:?} {position}"
                        );
                    }
                }
            }
        }
    }

    /// FormatNumericToString on a WideDecimal and a Decimal of the same value give the same
    /// text, with significant digits, fraction digits, the priorities and the padding.
    #[test]
    fn wide_formats_as_decimal() {
        let mut list = vec![opts(RoundingType::FractionDigits)];
        let mut sd = opts(RoundingType::SignificantDigits);
        sd.maximum_significant_digits = 2;
        list.push(sd);
        sd.minimum_significant_digits = 2;
        sd.maximum_significant_digits = 5;
        list.push(sd);
        let mut fd = opts(RoundingType::FractionDigits);
        fd.minimum_fraction_digits = 2;
        fd.maximum_fraction_digits = 2;
        fd.rounding_increment = 5000;
        fd.minimum_integer_digits = 9;
        list.push(fd);
        fd.strip_if_integer = true;
        list.push(fd);
        let mut more = opts(RoundingType::MorePrecision);
        more.maximum_significant_digits = 2;
        list.push(more);
        more.rounding_type = RoundingType::LessPrecision;
        list.push(more);
        for o in list {
            for v in [
                "123456789",
                "-987654321",
                "1",
                "0",
                "99999",
                "1234567890123456789012345",
            ] {
                let negative = v.starts_with('-');
                let mut d: Decimal = v.parse().unwrap();
                let mut w = unpolled(|k| {
                    WideDecimal::from_integer_digits(
                        negative,
                        v.trim_start_matches('-').as_bytes(),
                        k,
                    )
                });
                unpolled(|k| format_numeric_to_string(&o, &mut d, k));
                unpolled(|k| format_numeric_to_string(&o, &mut w, k));
                assert_eq!(
                    unpolled(|k| w.unsigned_text(k)),
                    unpolled(|k| d.unsigned_text(k)),
                    "{v} {o:?}"
                );
                assert_eq!(w.shown(), d.shown(), "{v} {o:?}");
                // A shift (compact notation) gives a fraction.
                let mut d: Decimal = v.parse().unwrap();
                let mut w = unpolled(|k| {
                    WideDecimal::from_integer_digits(
                        negative,
                        v.trim_start_matches('-').as_bytes(),
                        k,
                    )
                });
                DecimalValue::multiply_pow10(&mut d, -4);
                w.multiply_pow10(-4);
                unpolled(|k| format_numeric_to_string(&o, &mut d, k));
                unpolled(|k| format_numeric_to_string(&o, &mut w, k));
                assert_eq!(
                    unpolled(|k| w.unsigned_text(k)),
                    unpolled(|k| d.unsigned_text(k)),
                    "{v} {o:?} / 10^4"
                );
            }
        }
    }

    /// The plural Decimal of a WideDecimal keeps the low 18 integer digits and marks a larger
    /// integer part with a 1 at magnitude 18.
    #[test]
    fn wide_plural_decimal() {
        let mut big = vec![b'1'];
        big.extend(std::iter::repeat_n(b'0', 40_000));
        *big.last_mut().unwrap() = b'7';
        let w = unpolled(|k| WideDecimal::from_integer_digits(false, &big, k));
        assert_eq!(w.plural_decimal().to_string(), "1000000000000000007");
        let w = unpolled(|k| WideDecimal::from_integer_digits(true, b"42", k));
        assert_eq!(w.plural_decimal().to_string(), "-42");
    }
}
