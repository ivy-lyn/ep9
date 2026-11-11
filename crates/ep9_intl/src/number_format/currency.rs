//! The styles "percent" and "currency" of Intl.NumberFormat (ECMA-402 §16.5.1, §16.5.3,
//! §16.5.4): CurrencyDigits, the currency and percent patterns, and the "currency" and
//! "percentSign" parts.
//!
//! ```text
//! partition ─ percent:  x × 100 ─ decimal::pieces ─ the ICU4X percent pattern (signed if a
//!           │           sign shows: the sign goes in place of {1}) ─ percentSign / literal
//!           └ currency: decimal::pieces ─ the pattern of [[CurrencyDisplay]] and
//!                       [[CurrencySign]] ─ currency / literal / minusSign parts
//! CurrencyData (crate::cache, key "cur|<data locale>|<nu>|<code>", options: the display)
//!   code, symbol, narrowSymbol: CurrencyEssentialsV1 patterns (standard, accounting; the
//!     "alpha next to number" form by the letters of the text) and the text: the
//!     CurrencySymbolsV1 symbol, or the ISO code (code display, or no symbol in the data)
//!   an empty symbol: CurrencyPatternsNoCurrencyV1 (no currency part)
//!   name: CurrencyPatternsDataV1 and CurrencyExtendedDataV1 by the plural category of the
//!     rounded value (the ISO code if the data has no name)
//! PercentData (crate::cache, key "pct|<data locale>"): DecimalPercentV1
//! ```
//!
//! ICU4X 2.4 gives no part API for these styles (the formatters `dimension::currency` and
//! `dimension::percent` give only the text), so the patterns are read from the data with
//! private-use marks in place of the number, the currency and the sign, and split at the
//! marks (INTL-I1 design, ICU research rows 4-5). The sign: a negative pattern of the data
//! (accounting "($1.00)", or one with a "-" such as nl "€ -1,00") shows the sign itself;
//! else the sign affix of the locale goes before the pattern ("-$1.00"), as UTS #35 §3.2.1
//! says. The number, its rounding and its compact affixes are the decimal style's
//! ([`decimal::pieces`]).
//!
//! No ICU4X object or data payload is in a slot (INTL-I1-D7): they live in `crate::cache`.

use std::borrow::Cow;
use std::rc::Rc;

use fixed_decimal::Decimal;
use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::provider::currency::essentials::CurrencyEssentialsV1;
use icu_experimental::dimension::provider::currency::extended::CurrencyExtendedDataV1;
use icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1;
use icu_experimental::dimension::provider::currency::no_currency::CurrencyPatternsNoCurrencyV1;
use icu_experimental::dimension::provider::currency::patterns::CurrencyPatternsDataV1;
use icu_experimental::dimension::provider::currency::symbols::CurrencySymbolsV1;
use icu_experimental::dimension::provider::percent::DecimalPercentV1;
use icu_experimental::provider::Baked;
use icu_plurals::{PluralOperands, PluralRules};
use icu_provider::prelude::*;

use super::decimal::{self, Format, IntlMv, PartType, Parts};
use super::{CURRENCY_DISPLAYS, CURRENCY_SIGNS, Resolved, STYLES};
use crate::cache::{self, Service};
use crate::digits::{DecimalValue, Interrupted, WideDecimal, Work};
use crate::plural_rules;
use crate::tag::icu_locale;

/// The mark of the number (`{0}`) in a pattern text.
const NUMBER: char = '\u{E000}';
/// The mark of the currency (`{1}` of a currency pattern) or of the sign (`{1}` of the
/// signed percent pattern) in a pattern text.
const OTHER: char = '\u{E001}';
/// The values that a two-placeholder pattern interpolates to its marked text.
const MARKS: [&str; 2] = ["\u{E000}", "\u{E001}"];

// ---- CurrencyDigits (16.5.1) ------------------------------------------------------------------

/// CurrencyDigits ( currency ) (16.5.1): the number of minor unit digits of `currency` (an
/// upper case ISO 4217 code), 2 for a code that the data does not list.
///
/// The data is CLDR's (`CurrencyFractionsV1` `digits`, not the cash digits), as in V8 and
/// SpiderMonkey (ICU), not the ISO 4217 list that ECMA-402 names: the two differ for a few
/// codes (CLDR 49: HUF 0, IQD 0, IRR 0, ALL 0; ISO 4217: 2, 3, 2, 2). Node v26.13.0 gives the
/// CLDR values.
pub fn currency_digits(currency: &str) -> u8 {
    let Ok(c) = CurrencyType::try_from_str(currency) else {
        return 2;
    };
    DataProvider::<CurrencyFractionsV1>::load(&Baked, DataRequest::default())
        .map_or(2, |r| r.payload.get().resolve(c).digits)
}

// ---- the ICU4X data (crate::cache) ------------------------------------------------------------

/// A request with the `silent` flag (a missing entry is not logged).
fn silent() -> DataRequestMetadata {
    let mut m = DataRequestMetadata::default();
    m.silent = true;
    m
}

/// The data of `M` for `locale` with the numbering system `nu` as the marker attributes (the
/// ICU4X `nu_id` request), else for `locale` alone.
fn load_nu<M>(locale: &DataLocale, nu: &str) -> Option<DataPayload<M>>
where
    M: DataMarker,
    Baked: DataProvider<M>,
{
    if let Ok(attributes) = DataMarkerAttributes::try_from_str(nu) {
        let req = DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, locale),
            metadata: silent(),
        };
        if let Ok(r) = DataProvider::<M>::load(&Baked, req) {
            return Some(r.payload);
        }
    }
    let req = DataRequest {
        id: DataIdentifierBorrowed::for_locale(locale),
        metadata: silent(),
    };
    DataProvider::<M>::load(&Baked, req).ok().map(|r| r.payload)
}

/// The marked texts of one currency sign (`[[CurrencySign]]`): the positive pattern and the
/// negative one of the data, if it has one.
#[derive(Default)]
struct Signed {
    positive: String,
    negative: Option<String>,
}

/// The names of a currency (`currencyDisplay: "name"`).
struct Names {
    patterns: DataPayload<CurrencyPatternsDataV1>,
    /// `None`: the data has no name; the ISO code is the name (UTS #35).
    names: Option<DataPayload<CurrencyExtendedDataV1>>,
    /// `None`: the "other" forms (see `plural_rules::cardinal_rules`).
    rules: Option<PluralRules>,
}

/// The currency data of one data locale, numbering system, currency and
/// `[[CurrencyDisplay]]`.
struct CurrencyData {
    /// The text of the "currency" part (empty: no part).
    text: String,
    /// "standard" and "accounting" (code, symbol and narrowSymbol).
    standard: Signed,
    accounting: Signed,
    /// The names (name only; then `standard` and `accounting` are not read).
    names: Option<Names>,
    /// The ISO code (the name if the data has none).
    code: String,
}

impl CurrencyData {
    /// The data of `data_locale`, `nu`, the upper case code `code` and the display
    /// `display` (a value of `CURRENCY_DISPLAYS`).
    fn new(data_locale: &str, nu: &str, code: &str, display: &str) -> Option<CurrencyData> {
        let locale = DataLocale::from(&icu_locale(data_locale));
        let currency = CurrencyType::try_from_str(code).ok();
        if display == "name" {
            let patterns = DataProvider::<CurrencyPatternsDataV1>::load(
                &Baked,
                DataRequest {
                    id: DataIdentifierBorrowed::for_locale(&locale),
                    metadata: silent(),
                },
            )
            .ok()?
            .payload;
            let names = DataMarkerAttributes::try_from_str(code).ok().and_then(|a| {
                DataProvider::<CurrencyExtendedDataV1>::load(
                    &Baked,
                    DataRequest {
                        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(a, &locale),
                        metadata: silent(),
                    },
                )
                .ok()
                .map(|r| r.payload)
            });
            return Some(CurrencyData {
                text: String::new(),
                standard: Signed::default(),
                accounting: Signed::default(),
                names: Some(Names {
                    patterns,
                    names,
                    rules: plural_rules::cardinal_rules(data_locale),
                }),
                code: code.to_string(),
            });
        }
        // The symbol of the width, if the data has one (a missing entry: the ISO code).
        let symbol = match (display, currency) {
            ("symbol" | "narrowSymbol", Some(c)) => {
                let width = if display == "symbol" {
                    CurrencySymbolsV1::SHORT
                } else {
                    CurrencySymbolsV1::NARROW
                };
                DataProvider::<CurrencySymbolsV1>::load(
                    &Baked,
                    DataRequest {
                        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
                            CurrencySymbolsV1::make_attributes(c, width, &mut Default::default()),
                            &locale,
                        ),
                        metadata: silent(),
                    },
                )
                .ok()
                .map(|r| r.payload)
            }
            _ => None,
        };
        if let Some(s) = &symbol
            && s.get().as_str().is_empty()
        {
            // An empty symbol: the locale shows no currency (pt-PT "PTE"), the no-currency
            // patterns.
            let p = load_nu::<CurrencyPatternsNoCurrencyV1>(&locale, nu)?;
            let p = p.get();
            return Some(CurrencyData {
                text: String::new(),
                standard: Signed {
                    positive: p.get_positive().interpolate_to_string(MARKS),
                    negative: p.get_negative().map(|n| n.interpolate_to_string(MARKS)),
                },
                accounting: Signed {
                    positive: p.get_positive_accounting().interpolate_to_string(MARKS),
                    negative: p
                        .get_negative_accounting()
                        .map(|n| n.interpolate_to_string(MARKS)),
                },
                names: None,
                code: code.to_string(),
            });
        }
        // The ISO code is a text of letters at both ends.
        let (text, starts, ends) = match &symbol {
            Some(s) => {
                let s = s.get();
                (
                    s.as_str().to_string(),
                    s.starts_with_letter(),
                    s.ends_with_letter(),
                )
            }
            None => (code.to_string(), true, true),
        };
        let e = load_nu::<CurrencyEssentialsV1>(&locale, nu)?;
        let e = e.get();
        Some(CurrencyData {
            text,
            standard: Signed {
                positive: e.get_positive(starts, ends).interpolate_to_string(MARKS),
                negative: e
                    .get_negative(starts, ends)
                    .map(|n| n.interpolate_to_string(MARKS)),
            },
            accounting: Signed {
                positive: e
                    .get_positive_accounting(starts, ends)
                    .interpolate_to_string(MARKS),
                negative: e
                    .get_negative_accounting(starts, ends)
                    .map(|n| n.interpolate_to_string(MARKS)),
            },
            names: None,
            code: code.to_string(),
        })
    }
}

/// The cached currency data of `f` and `r`.
fn currency_data(f: &Format<'_>, r: &Resolved) -> Option<Rc<CurrencyData>> {
    let code = r.currency.as_deref()?;
    let display = CURRENCY_DISPLAYS.get(r.currency_display as usize)?;
    let key = format!("cur|{}|{}|{}", f.data_locale, f.numbering_system, code);
    cache::get_or_make(
        Service::NumberFormat,
        &key,
        u64::from(r.currency_display),
        || CurrencyData::new(f.data_locale, f.numbering_system, code, display),
    )
}

/// The percent patterns of one data locale (the ICU4X data has no percent pattern per
/// numbering system).
struct PercentData {
    /// The pattern with the sign: `{0}` the number, `{1}` the sign.
    signed: String,
    /// The pattern with no sign.
    unsigned: String,
}

/// The cached percent data of `f`.
fn percent_data(f: &Format<'_>) -> Option<Rc<PercentData>> {
    let key = format!("pct|{}", f.data_locale);
    cache::get_or_make(Service::NumberFormat, &key, 0, || {
        let locale = DataLocale::from(&icu_locale(f.data_locale));
        let p = DataProvider::<DecimalPercentV1>::load(
            &Baked,
            DataRequest {
                id: DataIdentifierBorrowed::for_locale(&locale),
                metadata: silent(),
            },
        )
        .ok()?
        .payload;
        let p = p.get();
        Some(PercentData {
            signed: p.signed_pattern.interpolate_to_string(MARKS),
            unsigned: p.unsigned_pattern.interpolate_to_string([MARKS[0]]),
        })
    })
}

// ---- PartitionNumberPattern (16.5.3) ----------------------------------------------------------

/// PartitionNumberPattern ( numberFormat, x ) (16.5.3) for the styles "percent" and
/// "currency": the parts of `x` formatted with `f`.
pub fn partition(
    r: &Resolved,
    f: Format<'_>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<Parts, Interrupted> {
    if STYLES.get(r.style as usize) == Some(&"percent") {
        percent(f, x, work)
    } else {
        currency(r, f, x, work)
    }
}

/// Moves the parts of `body` to `out` (one budget unit per part).
fn push_body(out: &mut Parts, body: Parts, work: &mut Work) -> Result<(), Interrupted> {
    out.reserve(body.len());
    for part in body {
        work.add(1)?;
        out.push(part);
    }
    Ok(())
}

/// Pushes `run` (if not empty) as a part of type `t`.
fn flush(out: &mut Parts, t: PartType, run: &mut String) {
    if !run.is_empty() {
        out.push((t, core::mem::take(run)));
    }
}

/// x × 100 (16.5.3 step 2.a for the style "percent"). A Decimal whose highest digit would pass
/// the i16 magnitudes of fixed_decimal (a BigInt of about 32,766 digits) becomes a
/// [`WideDecimal`] first (a Decimal overflow gives zero).
fn times_100(x: &IntlMv, work: &mut Work) -> Result<IntlMv, Interrupted> {
    Ok(match x {
        IntlMv::Finite(d) if d.nonzero_magnitude_start() + 2 > i64::from(i16::MAX) => {
            // Only a BigInt has such a magnitude (a Number or String value is below 1e309),
            // so the value is an integer.
            let digits = d.absolute.to_string();
            let mut w = WideDecimal::from_integer_digits(d.is_negative(), digits.as_bytes(), work)?;
            w.multiply_pow10(2);
            IntlMv::Wide(w)
        }
        IntlMv::Finite(d) => {
            let mut d = d.copy(work)?;
            DecimalValue::multiply_pow10(&mut d, 2);
            IntlMv::Finite(d)
        }
        IntlMv::Wide(w) => {
            let mut w = w.copy(work)?;
            w.multiply_pow10(2);
            IntlMv::Wide(w)
        }
        other => other.clone(),
    })
}

/// Whether `c` is a literal in a percent pattern: a space or a bidi mark (the rest of the
/// pattern text is the percent sign).
fn is_percent_literal(c: char) -> bool {
    c.is_whitespace() || c == '\u{A0}' || c == '\u{202F}' || decimal::is_bidi_mark(c)
}

/// The style "percent": the number x × 100 in the percent pattern of the locale. A shown sign
/// takes the place of `{1}` in the signed pattern (tr "-%50"); with no sign, the unsigned
/// pattern. In the pattern text, the spaces and bidi marks are literal parts, the rest is
/// the percentSign part (node: de "50 %" is integer, literal U+00A0, percentSign; ar "50%"
/// with LRMs is integer, literal, percentSign, literal).
fn percent(f: Format<'_>, x: &IntlMv, work: &mut Work) -> Result<Parts, Interrupted> {
    let x = times_100(x, work)?;
    let (Some(data), Some(mut p)) = (percent_data(&f), decimal::pieces(&f, &x, false, work)?)
    else {
        return decimal::partition(&f, &x, work);
    };
    let body = core::mem::take(&mut p.body);
    let pattern = if p.sign.is_some() {
        &data.signed
    } else {
        &data.unsigned
    };
    let mut out = Parts::with_capacity(body.len() + 4);
    let mut body = Some(body);
    let mut run = String::new();
    let mut run_literal = false;
    for c in pattern.chars() {
        match c {
            NUMBER | OTHER => {
                let t = if run_literal {
                    PartType::Literal
                } else {
                    PartType::PercentSign
                };
                flush(&mut out, t, &mut run);
                if c == NUMBER {
                    push_body(&mut out, body.take().unwrap_or_default(), work)?;
                } else {
                    p.push_sign_prefix(&mut out);
                }
            }
            c => {
                let literal = is_percent_literal(c);
                if !run.is_empty() && literal != run_literal {
                    let t = if run_literal {
                        PartType::Literal
                    } else {
                        PartType::PercentSign
                    };
                    flush(&mut out, t, &mut run);
                }
                run_literal = literal;
                run.push(c);
            }
        }
    }
    let t = if run_literal {
        PartType::Literal
    } else {
        PartType::PercentSign
    };
    flush(&mut out, t, &mut run);
    if p.sign.is_some() {
        p.push_sign_suffix(&mut out);
    }
    Ok(out)
}

/// The pattern and the currency text of `names` for the rounded value `rounded` (its plural
/// category; "other" for NaN and the infinities).
fn name_pattern(names: &Names, code: &str, rounded: Option<&Decimal>) -> (String, String) {
    let (patterns, texts) = (names.patterns.get(), names.names.as_ref().map(|n| n.get()));
    match (&names.rules, rounded) {
        (Some(rules), Some(d)) => {
            let op = PluralOperands::from(d);
            (
                patterns.get(op, rules).interpolate_to_string(MARKS),
                texts.map_or_else(|| code.to_string(), |t| t.get(op, rules).to_string()),
            )
        }
        _ => (
            patterns
                .elements
                .get_default()
                .1
                .interpolate_to_string(MARKS),
            texts.map_or_else(
                || code.to_string(),
                |t| t.elements.get_default().1.to_string(),
            ),
        ),
    }
}

/// The style "currency": the number in the currency pattern of `[[CurrencyDisplay]]` and
/// `[[CurrencySign]]` (GetNumberFormatPattern, 16.5.4). A minus sign takes the negative
/// pattern of the data if it has one (that pattern shows the sign: "($987.00)", nl
/// "€ -1,00"); else the locale's sign affix goes before the pattern ("-987,00 $"). In the
/// pattern text, `{1}` is the currency part, a "-" is a minusSign part (the locale's minus
/// sign), the rest are literal parts ("(", ")", spaces, bidi marks).
fn currency(
    r: &Resolved,
    f: Format<'_>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<Parts, Interrupted> {
    let Some(data) = currency_data(&f, r) else {
        return decimal::partition(&f, x, work);
    };
    let Some(mut p) = decimal::pieces(&f, x, data.names.is_some(), work)? else {
        return Ok(Parts::new());
    };
    let body = core::mem::take(&mut p.body);
    let accounting = CURRENCY_SIGNS.get(r.currency_sign as usize) == Some(&"accounting");
    let (pattern, text, explicit): (Cow<'_, str>, Cow<'_, str>, bool) = match &data.names {
        Some(names) => {
            let (pattern, text) = name_pattern(names, &data.code, p.rounded.as_ref());
            (pattern.into(), text.into(), false)
        }
        None => {
            let signed = if accounting {
                &data.accounting
            } else {
                &data.standard
            };
            let text = Cow::Borrowed(data.text.as_str());
            match (&signed.negative, p.sign) {
                (Some(n), Some(true)) => (n.as_str().into(), text, true),
                _ => (signed.positive.as_str().into(), text, false),
            }
        }
    };
    let mut out = Parts::with_capacity(body.len() + 6);
    if !explicit {
        p.push_sign_prefix(&mut out);
    }
    let mut body = Some(body);
    let mut run = String::new();
    for c in pattern.chars() {
        match c {
            NUMBER => {
                flush(&mut out, PartType::Literal, &mut run);
                push_body(&mut out, body.take().unwrap_or_default(), work)?;
            }
            OTHER => {
                flush(&mut out, PartType::Literal, &mut run);
                if !text.is_empty() {
                    out.push((PartType::Currency, text.to_string()));
                }
            }
            '-' if explicit => {
                flush(&mut out, PartType::Literal, &mut run);
                out.push((PartType::MinusSign, p.minus_sign()));
            }
            c => run.push(c),
        }
    }
    flush(&mut out, PartType::Literal, &mut run);
    if !explicit {
        p.push_sign_suffix(&mut out);
    }
    Ok(out)
}
