//! The style "unit" of Intl.NumberFormat (ECMA-402 §16.5.3, §16.5.4): the unit patterns of
//! the CLDR T2 table and the "unit" parts.
//!
//! ```text
//! partition ─ decimal::pieces (the sign, the number, the rounded value)
//!           ─ the unit pattern of the plural category of the rounded value ("{0} km/h")
//!           ─ parts: the text before and after {0} (unit, with its ends as literal parts),
//!             the sign and the number at {0}
//! UnitData (crate::cache, key "unit|<data locale>|<unit>", options: [[UnitDisplay]])
//!   T2 has the unit (45 simple units, 5 CLDR "X-per-Y" units): its patterns
//!   else "X-per-Y": the patterns of X in the per-unit pattern of Y ("{0}/h"), or in the
//!     compound pattern "per" ("{0}/{1}") with the "one" name of Y (UTS #35 §6.4, as ICU)
//!   the cardinal plural rules of the data locale (plural_rules::cardinal_rules)
//! ```
//!
//! ECMA-402 does not convert a value to another unit, and this file does not: the value is
//! formatted in the unit that the caller gives. No `icu_experimental::units` code (its
//! converter does `f64` math; DL INTL-V2-FP3, the guard test `tests/fp_guard.rs`). The unit
//! names and patterns come from T2 (`crate::cldr`, CLDR JSON by `tools/cldr-tables`), not
//! from the ICU4X units data: that data has only 5 unit categories (no speed, digital,
//! temperature, angle or percent).
//!
//! The parts follow ICU (node v26.13.0): the text of the pattern before and after `{0}` is a
//! "unit" part, and its spaces and bidi marks at the ends next to the number or the string
//! ends are "literal" parts (en "-987 km/h": minusSign, integer, literal " ", unit "km/h").
//! A pattern with no `{0}` takes the place of the number and its sign (ar "ساعة" for one
//! hour).
//!
//! No ICU4X object or data payload is in a slot (INTL-I1-D7): they live in `crate::cache`.

use std::rc::Rc;

use icu_plurals::{PluralCategory, PluralOperands, PluralRules};

use super::decimal::{self, Format, IntlMv, PartType, Parts};
use super::{Resolved, UNIT_DISPLAYS};
use crate::cache::{self, Service};
use crate::cldr::{self, UnitPatterns};
use crate::digits::{Interrupted, Work};
use crate::plural_rules;

/// The plural categories, in the order of [`UnitData::patterns`].
const CATEGORIES: [PluralCategory; 6] = [
    PluralCategory::Zero,
    PluralCategory::One,
    PluralCategory::Two,
    PluralCategory::Few,
    PluralCategory::Many,
    PluralCategory::Other,
];

/// The unit patterns of one data locale, unit and width.
struct UnitData {
    /// The pattern of each category of [`CATEGORIES`].
    patterns: [String; 6],
    /// The cardinal rules of the data locale; `None`: the "other" pattern (see
    /// `plural_rules::cardinal_rules`).
    rules: Option<PluralRules>,
}

impl UnitData {
    /// The patterns of `unit` in the width `width` (an index of `UNIT_DISPLAYS`) for
    /// `data_locale`; `None` if T2 has no data for it.
    fn new(data_locale: &str, unit: &str, width: usize) -> Option<UnitData> {
        let set = cldr::unit_set_for(data_locale)?;
        let patterns = match cldr::unit_patterns(set, unit, width) {
            Some(p) => CATEGORIES.map(|c| p.pattern(c).to_string()),
            None => {
                let (x, y) = unit.split_once("-per-")?;
                let x = cldr::unit_patterns(set, x, width)?;
                let y = cldr::unit_patterns(set, y, width)?;
                let per = per_unit_pattern(y, cldr::per_pattern(set, width)?);
                CATEGORIES.map(|c| per.replacen("{0}", x.pattern(c), 1))
            }
        };
        Some(UnitData {
            patterns,
            rules: plural_rules::cardinal_rules(data_locale),
        })
    }

    /// The pattern of the plural category of `rounded` (the rounded value; `None` for NaN
    /// and the infinities: "other").
    fn pattern(&self, rounded: Option<&fixed_decimal::Decimal>) -> &str {
        let c = match (&self.rules, rounded) {
            (Some(rules), Some(d)) => rules.category_for(PluralOperands::from(d)),
            _ => PluralCategory::Other,
        };
        let i = CATEGORIES.iter().position(|x| *x == c).unwrap_or(5);
        &self.patterns[i]
    }
}

/// The pattern "{0} per Y" of the denominator `y` of a unit "X-per-Y" (UTS #35 §6.4, as ICU
/// LongNameHandler): the per-unit pattern of `y` if CLDR has one (en short hour "{0}/h"); else
/// the compound pattern `per` ("{0}/{1}") with `{1}` the name of `y`: its "one" pattern
/// without `{0}` and without the spaces at its ends (CLDR deriveCompound: the denominator
/// takes the form "one").
fn per_unit_pattern(y: UnitPatterns, per: &str) -> String {
    if let Some(p) = y.per_unit() {
        return p.to_string();
    }
    let name = y.pattern(PluralCategory::One).replacen("{0}", "", 1);
    per.replacen("{1}", name.trim(), 1)
}

/// The cached unit data of `f` and `r`.
fn unit_data(f: &Format<'_>, r: &Resolved) -> Option<Rc<UnitData>> {
    let unit = r.unit.as_deref()?;
    let width = r.unit_display as usize;
    UNIT_DISPLAYS.get(width)?;
    let key = format!("unit|{}|{}", f.data_locale, unit);
    cache::get_or_make(Service::NumberFormat, &key, width as u64, || {
        UnitData::new(f.data_locale, unit, width)
    })
}

/// Whether `c` is a literal at an end of the text of a unit pattern (ICU's default
/// ignorables: the space separators Zs, TAB, the Bidi_Control marks and the variation
/// selectors): ICU does not count it in the unit field, so the part is "literal".
fn is_unit_literal(c: char) -> bool {
    matches!(
        c,
        '\t' | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{061C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{180B}'..='\u{180D}'
            | '\u{180F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Pushes the text `text` of a unit pattern (before or after `{0}`): the literals at its ends
/// as "literal" parts, the rest as one "unit" part.
fn push_unit_text(out: &mut Parts, text: &str) {
    let core = text.trim_matches(is_unit_literal);
    if core.is_empty() {
        if !text.is_empty() {
            out.push((PartType::Literal, text.to_string()));
        }
        return;
    }
    let start = text.len() - text.trim_start_matches(is_unit_literal).len();
    if start > 0 {
        out.push((PartType::Literal, text[..start].to_string()));
    }
    out.push((PartType::Unit, core.to_string()));
    let end = start + core.len();
    if end < text.len() {
        out.push((PartType::Literal, text[end..].to_string()));
    }
}

/// PartitionNumberPattern ( numberFormat, x ) (16.5.3) for the style "unit": the parts of
/// `x` formatted with `f`, in the unit pattern of `[[Unit]]` and `[[UnitDisplay]]`
/// (GetNumberFormatPattern, 16.5.4: the pattern of the plural category of the rounded value,
/// as ICU). The sign and the number go in place of `{0}`.
pub fn partition(
    r: &Resolved,
    f: Format<'_>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<Parts, Interrupted> {
    let Some(data) = unit_data(&f, r) else {
        return decimal::partition(&f, x, work);
    };
    let Some(mut p) = decimal::pieces(&f, x, true, work)? else {
        return Ok(Parts::new());
    };
    let body = core::mem::take(&mut p.body);
    let pattern = data.pattern(p.rounded.as_ref());
    let Some((before, after)) = pattern.split_once("{0}") else {
        // No number in the pattern: the pattern takes its place.
        let mut out = Parts::with_capacity(3);
        push_unit_text(&mut out, pattern);
        return Ok(out);
    };
    let mut out = Parts::with_capacity(body.len() + 6);
    push_unit_text(&mut out, before);
    p.push_sign_prefix(&mut out);
    for part in body {
        work.add(1)?;
        out.push(part);
    }
    p.push_sign_suffix(&mut out);
    push_unit_text(&mut out, after);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T2 is indexed by the `[[UnitDisplay]]` index.
    #[test]
    fn widths_are_the_unit_displays() {
        assert_eq!(UNIT_DISPLAYS, cldr::UNIT_WIDTHS);
    }

    /// The parts of a pattern text: the spaces and bidi marks at the ends are literals.
    #[test]
    fn unit_text_parts() {
        let parts = |t: &str| {
            let mut out = Parts::new();
            push_unit_text(&mut out, t);
            out.iter()
                .map(|(t, v)| format!("{}:{v}", t.name()))
                .collect::<Vec<_>>()
        };
        assert_eq!(parts(" km/h"), ["literal: ", "unit:km/h"]);
        assert_eq!(parts("每小時 "), ["unit:每小時", "literal: "]);
        assert_eq!(
            parts("\u{A0}kilometers per hour\u{200F}"),
            [
                "literal:\u{A0}",
                "unit:kilometers per hour",
                "literal:\u{200F}"
            ]
        );
        assert_eq!(parts("%"), ["unit:%"]);
        assert_eq!(parts(" "), ["literal: "]);
        assert!(parts("").is_empty());
    }

    /// An "X-per-Y" unit that T2 does not list: X in the per-unit pattern of Y, else in the
    /// "per" pattern with the name of Y (node v26.13.0: en "2 m/h", "2 m/st",
    /// "2 stones per meter"; ja "2 ストーン毎エーカー").
    #[test]
    fn per_units_as_node() {
        let pat = |l: &str, u: &str, width: usize, c: PluralCategory| {
            let d = UnitData::new(l, u, width).expect(u);
            d.patterns[CATEGORIES.iter().position(|x| *x == c).unwrap()].clone()
        };
        assert_eq!(
            pat("en", "meter-per-hour", 0, PluralCategory::Other),
            "{0} m/h"
        );
        assert_eq!(
            pat("en", "meter-per-stone", 0, PluralCategory::Other),
            "{0} m/st"
        );
        assert_eq!(
            pat("en", "stone-per-meter", 2, PluralCategory::Other),
            "{0} stones per meter"
        );
        assert_eq!(
            pat("en", "meter-per-hour", 2, PluralCategory::One),
            "{0} meter per hour"
        );
        assert_eq!(
            pat("ja", "stone-per-acre", 2, PluralCategory::Other),
            "{0} ストーン毎エーカー"
        );
        // T2 lists the CLDR compound unit itself.
        assert_eq!(
            pat("zh-TW", "kilometer-per-hour", 2, PluralCategory::Other),
            "每小時 {0} 公里"
        );
        assert!(UnitData::new("en", "meter-per-parsec", 0).is_none());
    }
}
