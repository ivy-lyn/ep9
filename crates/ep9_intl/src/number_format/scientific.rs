//! The notations "scientific" and "engineering" of Intl.NumberFormat (ECMA-402 §16.5.3,
//! §16.5.12, §16.5.13): ComputeExponent and the "exponentSeparator", "exponentMinusSign" and
//! "exponentInteger" parts.
//!
//! ```text
//! partition ─ style "decimal"            ─ decimal::partition ┐
//!           ├ styles "percent", "currency" ─ currency::partition ├ decimal::pieces with
//!           └ style "unit"                 ─ unit::partition     ┘ Format::exponent set
//! decimal::pieces: (e, y) = ComputeExponent(x) on the magnitude of the Decimal
//!   (decimal::compute_exponent, shared with Intl.PluralRules); the number parts of y, then
//!   the exponent parts: "E" (CLDR T1 `exponential`), the minus sign of the locale, the digits
//!   of |e| in the numbering system
//! ```
//!
//! The exponent is integer arithmetic on the magnitude of the Decimal (the place of its
//! highest nonzero digit): e = m ("scientific") or m rounded down to a multiple of 3
//! ("engineering"), and again for m + 1 when the rounding moves the magnitude (999,999 with
//! 3 fraction digits is "1E6", not "10E5"). No `log10`, no `pow`, no f64 (INTL-V2-FP3, H36).
//!
//! The exponent parts follow the number, inside the style pattern: "$1.234E3", "5E1 %"
//! (de), "1E3 meters" (node v26.13.0). The plural category of a unit or currency name is
//! the one of the significand × 10^e, as ICU.

use super::decimal::{self, Format, IntlMv, Parts};
use super::{Resolved, STYLES};
use crate::digits::{Interrupted, Work};

/// PartitionNumberPattern ( numberFormat, x ) (16.5.3) for the notations "scientific" and
/// "engineering": the parts of `x` formatted with `f` (its [`Format::exponent`] is set) in the
/// pattern of `[[Style]]`.
pub fn partition(
    r: &Resolved,
    f: Format<'_>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<Parts, Interrupted> {
    match STYLES.get(r.style as usize) {
        Some(&"percent" | &"currency") => super::currency::partition(r, f, x, work),
        Some(&"unit") => super::unit::partition(r, f, x, work),
        _ => decimal::partition(&f, x, work),
    }
}

#[cfg(test)]
mod tests {
    use super::super::decimal::Exponent;

    /// ComputeExponentForMagnitude (§16.5.13): the magnitude, or a multiple of 3 below it
    /// (also for a negative magnitude: 0.000345 is 345E-6).
    #[test]
    fn exponent_for_magnitude() {
        let s = |m| Exponent::Scientific.for_magnitude(m);
        let e = |m| Exponent::Engineering.for_magnitude(m);
        assert_eq!((s(-4), s(0), s(5)), (-4, 0, 5));
        assert_eq!(
            (e(-4), e(-3), e(-1), e(0), e(2), e(3), e(5)),
            (-6, -3, -3, 0, 0, 3, 3)
        );
        assert_eq!(e(323_000_001), 323_000_001 / 3 * 3);
    }
}
