//! Number digit options (ECMA-402 §16.5.3 SetNumberFormatDigitOptions) and the rounding of
//! FormatNumericToString (§16.5.5, ToRawPrecision §16.5.8, ToRawFixed §16.5.9) on
//! `fixed_decimal::Decimal`. PluralRules (W2) and NumberFormat (W2) share them (INTL-I1
//! §4.1).
//!
//! ```text
//! options ─ set_number_format_digit_options(mnfdDefault, mxfdDefault, notation) ─> DigitOptions
//!           (the [[MinimumIntegerDigits]] .. [[ComputedRoundingPriority]] fields; packed into
//!            the slots of the service object by the service)
//! Decimal x ─ format_numeric_to_string(&DigitOptions) ─> x rounded and padded (the
//!             [[RoundedNumber]] and its digits; FormattedString = x.to_string())
//! ```

use ep9_vm::{Cx, JsObject, JsResult, Local};
use fixed_decimal::{Decimal, RoundingIncrement, SignedRoundingMode, UnsignedRoundingMode};

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

/// `[[RoundingMode]]` (the 9 values of §16.5.3 step 9).
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

    const ALL: [RoundingMode; 9] = [
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

/// The allowed `roundingIncrement` values (§16.5.3 step 8).
const INCREMENTS: [u16; 15] = [
    1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000,
];

/// SetNumberFormatDigitOptions ( intlObj, options, mnfdDefault, mxfdDefault, notation )
/// (§16.5.3). `compact`: notation is "compact".
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

/// The highest magnitude of a nonzero digit of `x`, or 0 for zero (the `e` of ToRawPrecision).
fn magnitude(x: &Decimal) -> i16 {
    if x.absolute.is_zero() {
        0
    } else {
        x.absolute.nonzero_magnitude_start()
    }
}

/// ToRawPrecision ( x, minPrecision, maxPrecision, unsignedRoundingMode ) (§16.5.8): rounds
/// `x` to `max` significant digits and keeps at least `min`. Gives `[[RoundingMagnitude]]`.
fn to_raw_precision(x: &mut Decimal, min: u8, max: u8, mode: RoundingMode) -> i16 {
    let (min, max) = (i16::from(min), i16::from(max));
    let e = magnitude(x);
    x.round_with_mode(e - max + 1, mode.signed());
    // A carry (9.99 -> 10) moves e up one; n keeps p digits.
    let e = magnitude(x);
    let rm = e - max + 1;
    x.absolute.trim_end();
    x.absolute.pad_end(rm.min(0).max(e - min + 1).min(0));
    x.absolute.pad_start(1);
    rm
}

/// ToRawFixed ( x, minFraction, maxFraction, roundingIncrement, unsignedRoundingMode )
/// (§16.5.9): rounds `x` to `max` fraction digits (in steps of `increment`) and keeps at
/// least `min`. Gives `[[RoundingMagnitude]]`.
fn to_raw_fixed(x: &mut Decimal, min: u8, max: u8, increment: u16, mode: RoundingMode) -> i16 {
    let f = i16::from(max);
    // increment = m x 10^k with m in {1, 2, 5, 25}.
    let (mut m, mut k) = (increment, 0i16);
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
    x.round_with_mode_and_increment(-f + k, mode.signed(), inc);
    x.absolute.trim_end();
    x.absolute.pad_end(-i16::from(min));
    x.absolute.pad_start(1);
    -f
}

/// The rounding of FormatNumericToString ( intlObject, x ) (§16.5.5) on `x`: the rounding of
/// the object's `[[RoundingType]]`, `stripIfInteger`, and the `[[MinimumIntegerDigits]]`
/// padding. `x.to_string()` is then the `[[FormattedString]]` (with its sign; -0 keeps the
/// minus sign of the Decimal).
pub fn format_numeric_to_string(opts: &DigitOptions, x: &mut Decimal) {
    let mode = opts.rounding_mode;
    match opts.rounding_type {
        RoundingType::SignificantDigits => {
            to_raw_precision(
                x,
                opts.minimum_significant_digits,
                opts.maximum_significant_digits,
                mode,
            );
        }
        RoundingType::FractionDigits => {
            to_raw_fixed(
                x,
                opts.minimum_fraction_digits,
                opts.maximum_fraction_digits,
                opts.rounding_increment,
                mode,
            );
        }
        RoundingType::MorePrecision | RoundingType::LessPrecision => {
            let mut s = x.clone();
            let mut f = x.clone();
            let srm = to_raw_precision(
                &mut s,
                opts.minimum_significant_digits,
                opts.maximum_significant_digits,
                mode,
            );
            let frm = to_raw_fixed(
                &mut f,
                opts.minimum_fraction_digits,
                opts.maximum_fraction_digits,
                opts.rounding_increment,
                mode,
            );
            let s_wins = (srm <= frm) == (opts.rounding_type == RoundingType::MorePrecision);
            *x = if s_wins { s } else { f };
        }
    }
    // 5. If trailingZeroDisplay is "stripIfInteger" and x is an integer, drop the fraction.
    if opts.strip_if_integer {
        x.absolute.trim_end_if_integer();
    }
    // 6-9. Pad to [[MinimumIntegerDigits]].
    x.absolute
        .pad_start(i16::from(opts.minimum_integer_digits.max(1)));
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
        format_numeric_to_string(o, &mut x);
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
}
