//! The styles "percent" and "currency" of Intl.NumberFormat (ECMA-402 §16.5.1, §16.5.3,
//! §16.5.4): CurrencyDigits, the currency and percent patterns, and the "currency" and
//! "percentSign" parts.
//!
//! Stub (INTL W3 RTF skeleton): INTL W3 NF-CURRENCY fills this file. Until then
//! [`partition`] formats as the decimal style in standard notation (a stub path, not a spec
//! result), as NF-CORE did.

use super::Resolved;
use super::decimal::{self, Format, IntlMv, Parts};
use crate::digits::{Interrupted, Work};

/// PartitionNumberPattern ( numberFormat, x ) (16.5.3) for the styles "percent" and
/// "currency": the parts of `x` formatted with `f`. Stub: the decimal style in standard
/// notation.
pub fn partition(
    _r: &Resolved,
    mut f: Format<'_>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<Parts, Interrupted> {
    f.compact = None;
    decimal::partition(&f, x, work)
}
