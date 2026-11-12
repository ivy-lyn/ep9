//! The number ranges of Intl.NumberFormat (ECMA-402 §16.5.19-§16.5.23):
//! PartitionNumberRangePattern, FormatApproximately, CollapseNumberRange and
//! FormatNumericRange(ToParts), for `formatRange` and `formatRangeToParts` (ids 34.5, 34.6).
//!
//! ```text
//! partition(x, y) ─ xp = PartitionNumberPattern(x), yp = PartitionNumberPattern(y)
//!   same text ─ FormatApproximately(xp): the approximately sign (CLDR T1) before the sign
//!               of the number (else at the start of the style pattern); every part "shared"
//!   else      ─ [xp "startRange"] [separator "shared"] [yp "endRange"] (the CLDR T1 range
//!               pattern "{0}–{1}"), then CollapseNumberRange (below)
//! ```
//!
//! ECMA-402 leaves CollapseNumberRange and the place of the approximately sign to the
//! implementation. This file follows ICU (NumberRangeFormatterImpl, collapse "auto", identity
//! fallback "approximately"), checked against node v26.13.0. Each side is cut in layers:
//!
//! ```text
//! [outer prefix][middle prefix][number][inner][middle suffix][outer suffix]
//!   inner   the exponent parts (scientific, engineering): never collapsed
//!   middle  the sign, the currency symbol or code, the percent sign, the compact affix, the
//!           literals of their patterns: collapsed if both sides have the same middle and
//!           it has more than one code point ("+$2.90–3.10", "-€3.00–5.00", but "$3 – $5")
//!   outer   the unit, or the currency name: always collapsed ("3–5 km/h"); its plural form
//!           is the one of the range category (CLDR plural ranges: en "0–1 kilometers per
//!           hour"), also when no end has it (sl 102 two, 201 one: few "102–201 metri")
//! ```
//!
//! The separator gets a space on each side that has none if a layer of the first side shows
//! twice (its inner, or its middle when not collapsed): "$3 – $5", "-3 – 5", "1E3 – 2E3",
//! but "3–5" and "3–-5". A collapsed layer is "shared", around the two sides.
//!
//! No ICU4X object is in a slot (INTL-I1-D7): the plural rules with ranges live in
//! `crate::cache`.

use std::rc::Rc;

use fixed_decimal::{Decimal, Sign};
use icu_plurals::{PluralCategory, PluralOperands, PluralRules, PluralRulesWithRanges};

use super::decimal::{self, IntlMv, PartType, Parts, SignDisplay};
use super::{CURRENCY_DISPLAYS, Resolved, STYLES, partition_numeric, partition_numeric_as};
use crate::cache::{self, Service};
use crate::digits::{Interrupted, Work};
use crate::plural_rules;
use crate::tag::{self, icu_locale};

/// The `[[Source]]` of a part of a number range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    StartRange,
    EndRange,
    Shared,
}

impl Source {
    /// The `source` string of formatRangeToParts.
    pub fn name(self) -> &'static str {
        match self {
            Source::StartRange => "startRange",
            Source::EndRange => "endRange",
            Source::Shared => "shared",
        }
    }
}

/// The parts of a formatted number range, in order.
pub type RangeParts = Vec<(PartType, String, Source)>;

/// A part of one formatted number.
type Part = (PartType, String);
/// The middle layer of one formatted number: its prefix and suffix parts.
type Middle<'p> = (&'p [Part], &'p [Part]);

/// The range pattern when the data has none (CLDR root).
const ROOT_RANGE: &str = "{0}–{1}";
/// The approximately sign when the data has none (CLDR root).
const ROOT_APPROXIMATELY: &str = "~";

/// Whether the style of `r` has an outer layer (ICU's long name modifier): the style "unit"
/// (not the unit "percent": ICU formats it as the style "percent") and the currency names.
fn has_outer(r: &Resolved) -> bool {
    match STYLES.get(r.style as usize) {
        Some(&"unit") => r.unit.as_deref() != Some("percent"),
        Some(&"currency") => CURRENCY_DISPLAYS.get(r.currency_display as usize) == Some(&"name"),
        _ => false,
    }
}

/// Whether `t` is a part of the number itself.
fn is_number(t: PartType) -> bool {
    matches!(
        t,
        PartType::Integer
            | PartType::Group
            | PartType::Decimal
            | PartType::Fraction
            | PartType::Nan
            | PartType::Infinity
    )
}

/// Whether `t` is a sign part of the number (not of its exponent).
fn is_sign(t: PartType) -> bool {
    matches!(t, PartType::MinusSign | PartType::PlusSign)
}

/// Whether the part `(t, v)` is a literal of bidi marks only (the marks of a sign affix).
fn is_bidi_literal(t: PartType, v: &str) -> bool {
    t == PartType::Literal && !v.is_empty() && v.chars().all(decimal::is_bidi_mark)
}

/// The layers of one formatted number, as index ranges of its parts (module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layers {
    /// The end of the outer prefix (it starts at 0).
    outer_prefix: usize,
    /// The start and the end of the number parts.
    number: (usize, usize),
    /// The end of the inner (exponent) parts: they start at `number.1`.
    inner: usize,
    /// The start of the outer suffix (it ends at the last part).
    outer_suffix: usize,
}

impl Layers {
    /// The layers of `p`; `None` if `p` has no number part (a unit pattern with no `{0}`).
    /// The scans count one budget unit per part (H23: a WideDecimal number has many parts).
    fn of(p: &Parts, outer: bool, work: &mut Work) -> Result<Option<Layers>, Interrupted> {
        let Some(start) = position(p, 0, work, |t, _| {
            matches!(t, PartType::Integer | PartType::Nan | PartType::Infinity)
        })?
        else {
            return Ok(None);
        };
        let end = position(p, start, work, |t, _| !is_number(t))?.unwrap_or(p.len());
        // The exponent: its separator, sign (with bidi marks) and digits.
        let mut inner = end;
        if matches!(p.get(end), Some((PartType::ExponentSeparator, _))) {
            inner = end + 1;
            let mut k = end + 1;
            while let Some((t, v)) = p.get(k) {
                work.add(1)?;
                match t {
                    PartType::ExponentInteger => inner = k + 1,
                    PartType::ExponentMinusSign => {}
                    _ if is_bidi_literal(*t, v) => {}
                    _ => break,
                }
                k += 1;
            }
        }
        let (mut outer_prefix, mut outer_suffix) = (0, p.len());
        if outer {
            let is_outer =
                |t: PartType| matches!(t, PartType::Unit | PartType::Currency | PartType::Literal);
            outer_prefix = position(p, 0, work, |t, _| !is_outer(t))?.unwrap_or(start);
            // The bidi marks of a sign affix go with the sign (the middle layer).
            while outer_prefix > 0
                && is_bidi_literal(p[outer_prefix - 1].0, &p[outer_prefix - 1].1)
                && p.get(outer_prefix).is_some_and(|(t, _)| is_sign(*t))
            {
                outer_prefix -= 1;
            }
            while outer_suffix > inner && is_outer(p[outer_suffix - 1].0) {
                work.add(1)?;
                outer_suffix -= 1;
            }
        }
        Ok(Some(Layers {
            outer_prefix: outer_prefix.min(start),
            number: (start, end),
            inner,
            outer_suffix,
        }))
    }

    /// The parts of the middle layer (prefix and suffix) of `p`.
    fn middle<'p>(&self, p: &'p Parts) -> Middle<'p> {
        (
            &p[self.outer_prefix..self.number.0],
            &p[self.inner..self.outer_suffix],
        )
    }
}

/// The index of the first part of `p` from `from` on for which `pred` holds (one budget
/// unit per part that it looks at).
fn position(
    p: &[Part],
    from: usize,
    work: &mut Work,
    pred: impl Fn(PartType, &str) -> bool,
) -> Result<Option<usize>, Interrupted> {
    for (i, (t, v)) in p.iter().enumerate().skip(from) {
        work.add(1)?;
        if pred(*t, v) {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// The code points of the texts of `parts`.
fn code_points(parts: &[Part]) -> usize {
    parts.iter().map(|(_, v)| v.chars().count()).sum()
}

/// The cardinal plural rules with the CLDR plural ranges of one data locale (the rules of the
/// unit and currency name patterns, `plural_rules::cardinal_rules`), and the operands of a
/// sample value of each category of the rules ([`samples`]).
struct RangeRules(Option<(PluralRulesWithRanges<PluralRules>, Samples)>);

/// The operands of a sample value of each plural category that has one.
type Samples = Vec<(PluralCategory, PluralOperands)>;

/// The sample values of [`samples`], in the order they are tried: integers, then values with
/// fraction digits, then 0 and 1 last (a pattern may have an explicit "0" or "1" form, which
/// ICU4X picks by the value; the range category is a category, not a value).
const SAMPLE_VALUES: [&str; 8] = ["1000000", "1000", "0.5", "1.5", "0.1", "1.1", "0", "1"];

/// The operands of a sample value of each category of `rules`: the first of the integers 2 to
/// 200 and [`SAMPLE_VALUES`] whose category it is. The category of a range may be the
/// category of neither end (sl 102 is two, 201 is one, the range is few), so the range form
/// of a unit or currency name pattern comes from such a value.
fn samples(rules: &PluralRules) -> Samples {
    let mut out = Samples::new();
    let values = (2u32..=200).map(Decimal::from).chain(
        SAMPLE_VALUES
            .iter()
            .filter_map(|s| Decimal::try_from_str(s).ok()),
    );
    for d in values {
        let o = PluralOperands::from(&d);
        let c = rules.category_for(o);
        if !out.iter().any(|(k, _)| *k == c) {
            out.push((c, o));
        }
    }
    out
}

/// The cached plural rules with ranges of `data_locale`.
fn range_rules(data_locale: &str) -> Option<Rc<RangeRules>> {
    let key = format!("prange|{data_locale}");
    cache::get_or_make(Service::NumberFormat, &key, 0, || {
        let rules = plural_rules::cardinal_rules(data_locale);
        Some(RangeRules(rules.and_then(|rules| {
            let prefs = (&icu_locale(data_locale)).into();
            let samples = samples(&rules);
            PluralRulesWithRanges::try_new_with_rules(prefs, rules)
                .ok()
                .map(|r| (r, samples))
        })))
    })
}

/// The plural category of the outer layer of `x` (the category of its rounded value, as the
/// unit and currency name patterns pick it).
fn outer_category(
    r: &Resolved,
    rules: &PluralRulesWithRanges<PluralRules>,
    x: &IntlMv,
    work: &mut Work,
) -> Result<PluralCategory, Interrupted> {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let f = r.format(&data_locale);
    Ok(
        match decimal::pieces(&f, x, true, work)?.and_then(|p| p.rounded) {
            Some(d) => rules.rules().category_for(PluralOperands::from(&d)),
            None => PluralCategory::Other,
        },
    )
}

/// The side whose outer layer the range takes: the one whose plural category is the range
/// category (CLDR plural ranges), or the pattern form of the range category.
enum OuterForm {
    Start,
    /// The end (also with no range data: the CLDR default range category is the end one).
    End,
    /// Neither end has the range category: the form of these operands (a sample value of
    /// the category).
    Other(PluralOperands),
}

/// The [`OuterForm`] of the range from `x` to `y`.
fn outer_form(
    r: &Resolved,
    x: &IntlMv,
    y: &IntlMv,
    work: &mut Work,
) -> Result<OuterForm, Interrupted> {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let Some(rules) = range_rules(&data_locale) else {
        return Ok(OuterForm::End);
    };
    let Some((rules, samples)) = rules.0.as_ref() else {
        return Ok(OuterForm::End);
    };
    let cx = outer_category(r, rules, x, work)?;
    let cy = outer_category(r, rules, y, work)?;
    let c = rules.resolve_range(cx, cy);
    Ok(if c == cy {
        OuterForm::End
    } else if c == cx {
        OuterForm::Start
    } else {
        samples
            .iter()
            .find(|(k, _)| *k == c)
            .map_or(OuterForm::End, |(_, o)| OuterForm::Other(*o))
    })
}

/// Appends `parts` with the source `source` (one budget unit per part).
fn push_all(
    out: &mut RangeParts,
    parts: &[Part],
    source: Source,
    work: &mut Work,
) -> Result<(), Interrupted> {
    out.reserve(parts.len());
    for (t, v) in parts {
        work.add(1)?;
        out.push((*t, v.clone(), source));
    }
    Ok(())
}

/// Whether `c` is a white space of an ICU pattern (PatternProps::isWhiteSpace).
fn is_pattern_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{B}'
            | '\u{C}'
            | '\r'
            | ' '
            | '\u{85}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// The text before `{0}`, between `{0}` and `{1}`, and after `{1}` of the range pattern
/// `pattern` (the root pattern if it is not of that form).
fn split_range_pattern(pattern: &str) -> (&str, &str, &str) {
    fn parse(p: &str) -> Option<(&str, &str, &str)> {
        let (before, rest) = p.split_once("{0}")?;
        let (between, after) = rest.split_once("{1}")?;
        Some((before, between, after))
    }
    parse(pattern)
        .or_else(|| parse(ROOT_RANGE))
        .unwrap_or(("", "–", ""))
}

/// The approximately sign part.
fn approximately(sign: &str) -> Part {
    (PartType::ApproximatelySign, sign.to_string())
}

/// The index range in `p` of the sign symbol around the sign part at `i`: the part, and the
/// bidi marks of the sign affix `affix` at the end of the literal before it and at the start
/// of the literal after it (the parts are cut there). The parts of the symbol are then
/// `p[range]`.
fn sign_symbol(p: &mut Parts, i: usize, affix: &str) -> core::ops::Range<usize> {
    let core = affix.trim_matches(decimal::is_bidi_mark);
    let lead = &affix[..affix.len() - affix.trim_start_matches(decimal::is_bidi_mark).len()];
    let trail = &affix[lead.len() + core.len()..];
    let (mut start, mut end) = (i, i + 1);
    if !trail.is_empty()
        && let Some((PartType::Literal, v)) = p.get(end)
        && let Some(rest) = v.strip_prefix(trail)
    {
        let rest = rest.to_string();
        p[end].1 = trail.to_string();
        if !rest.is_empty() {
            p.insert(end + 1, (PartType::Literal, rest));
        }
        end += 1;
    }
    if !lead.is_empty()
        && start > 0
        && let (PartType::Literal, v) = &p[start - 1]
        && let Some(rest) = v.strip_suffix(lead)
    {
        let rest = rest.to_string();
        if rest.is_empty() {
            start -= 1;
        } else {
            p[start - 1].1 = rest;
            p.insert(start, (PartType::Literal, lead.to_string()));
            end += 1;
        }
    }
    start..end
}

/// The negative value with the magnitude of the finite or infinite value `x`, and whether it
/// is -x (`x` was not negative; a negative `x` is itself: signDisplay "never"). `None` for
/// NaN and for a WideDecimal.
fn negative_of(x: &IntlMv) -> Option<(IntlMv, bool)> {
    match x {
        IntlMv::Finite(d) => {
            let negated = d.sign != Sign::Negative;
            let mut d = d.clone();
            d.sign = Sign::Negative;
            Some((IntlMv::Finite(d), negated))
        }
        IntlMv::PosInf => Some((IntlMv::NegInf, true)),
        IntlMv::NegInf => Some((IntlMv::NegInf, false)),
        IntlMv::Wide(_) | IntlMv::NaN => None,
    }
}

/// FormatApproximately ( numberFormat, x ) (16.5.20) on the parts `p` of x, as ICU: the
/// approximately sign of the locale goes where the pattern has its sign.
///
/// - A sign shows: the approximately sign goes before the sign symbol ("~-1", "~+1"; he
///   "\u{200F}~\u{200E}-3.00": after the mark of the pattern, before the mark of the symbol).
/// - A minus that a currency pattern shows with no sign (accounting "($3.00)"): no
///   approximately sign.
/// - No sign: the parts of the negative value, with the approximately sign in place of the
///   minus sign symbol (nl "€ ~3,00", de-CH "CHF≈3.00", ar "\u{200F}~3.00 €"); if they have
///   no minus sign part, the approximately sign goes at the start of the style pattern, after
///   the outer layer ("~$3", "~3 €"). The negative value -x is rounded with the mode for -x
///   ([`crate::digits::RoundingMode::for_negated`]), so its number is the number of `p`
///   (ceil: 1.1 is "~2", not the "~1" of -1.1 rounded up); a negative x (signDisplay
///   "never") is rounded with its own mode.
///
/// Every part is "shared".
fn format_approximately(
    r: &Resolved,
    mut p: Parts,
    x: &IntlMv,
    work: &mut Work,
) -> Result<RangeParts, Interrupted> {
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let f = r.format(&data_locale);
    let sign = decimal::t1_row(&f)
        .map(|t| t.approximately_sign)
        .filter(|s| !s.is_empty())
        .unwrap_or(ROOT_APPROXIMATELY);
    let (minus, plus) = decimal::sign_affixes(&f).unwrap_or_default();
    if let Some(i) = position(&p, 0, work, |t, _| is_sign(t))? {
        let affix = if p[i].0 == PartType::MinusSign {
            &minus
        } else {
            &plus
        };
        let at = sign_symbol(&mut p, i, affix).start;
        p.insert(at, approximately(sign));
        return shared(p, work);
    }
    if STYLES.get(r.style as usize) == Some(&"currency")
        && decimal::pieces(&f, x, false, work)?.and_then(|q| q.sign) == Some(true)
    {
        return shared(p, work);
    }
    if let Some((negative, negated)) = negative_of(x) {
        let mut rn = r.clone();
        rn.sign_display = SignDisplay::Auto;
        if negated {
            rn.digits.rounding_mode = rn.digits.rounding_mode.for_negated();
        }
        let mut n = partition_numeric(&rn, &negative, work)?;
        if let Some(i) = position(&n, 0, work, |t, _| t == PartType::MinusSign)? {
            let symbol = sign_symbol(&mut n, i, &minus);
            n.splice(symbol, [approximately(sign)]);
            return shared(n, work);
        }
    }
    let at = Layers::of(&p, has_outer(r), work)?.map_or(0, |l| l.outer_prefix);
    p.insert(at, approximately(sign));
    shared(p, work)
}

/// The parts `p`, each with the source "shared" (one budget unit per part).
fn shared(p: Parts, work: &mut Work) -> Result<RangeParts, Interrupted> {
    let mut out = RangeParts::with_capacity(p.len());
    for (t, v) in p {
        work.add(1)?;
        out.push((t, v, Source::Shared));
    }
    Ok(out)
}

/// Whether the texts of `a` and `b` are the same (FormatNumeric(x) = FormatNumeric(y)). The
/// compare counts the text length in `work`.
fn same_text(a: &Parts, b: &Parts, work: &mut Work) -> Result<bool, Interrupted> {
    Ok(decimal::join(a, work)? == decimal::join(b, work)?)
}

/// PartitionNumberRangePattern ( numberFormat, x, y ) (16.5.19) after the NaN test of step 1
/// (the caller's): the parts of the range from `x` to `y` formatted with `r`, with their
/// sources, collapsed (CollapseNumberRange, 16.5.21; module doc).
pub fn partition(
    r: &Resolved,
    x: &IntlMv,
    y: &IntlMv,
    work: &mut Work,
) -> Result<RangeParts, Interrupted> {
    // 2-3. Let xResult be ? PartitionNumberPattern(numberFormat, x); the same for y.
    let xp = partition_numeric(r, x, work)?;
    let yp = partition_numeric(r, y, work)?;
    // 4. If FormatNumeric(numberFormat, x) is FormatNumeric(numberFormat, y), return
    //    FormatApproximately(numberFormat, xResult) with every [[Source]] "shared".
    if same_text(&xp, &yp, work)? {
        return format_approximately(r, xp, x, work);
    }
    // 5-9. xResult "startRange", the range separator "shared", yResult "endRange"; then
    //      10. CollapseNumberRange.
    let data_locale = tag::remove_unicode_extension(&r.locale);
    let f = r.format(&data_locale);
    let pattern = decimal::t1_row(&f).map_or(ROOT_RANGE, |t| t.range);
    let (before, between, after) = split_range_pattern(pattern);
    let outer = has_outer(r);
    let (Some(lx), Some(ly)) = (Layers::of(&xp, outer, work)?, Layers::of(&yp, outer, work)?)
    else {
        // A side with no number (a unit pattern with no `{0}`): no collapse.
        let mut out = RangeParts::with_capacity(xp.len() + yp.len() + 3);
        push_text(&mut out, before);
        push_all(&mut out, &xp, Source::StartRange, work)?;
        push_text(&mut out, between);
        push_all(&mut out, &yp, Source::EndRange, work)?;
        push_text(&mut out, after);
        return Ok(out);
    };
    // The outer layers (empty, or the unit or currency name) always collapse, in the plural
    // form of the range category (also if both ends have the same form: sl 1 and 101 are
    // one, the range is few).
    let form = if outer {
        outer_form(r, x, y, work)?
    } else {
        OuterForm::Start
    };
    // The parts of y in the pattern form of the range category, if no end has it.
    let zp = match form {
        OuterForm::Other(o) => Some(partition_numeric_as(r, y, Some(o), work)?),
        _ => None,
    };
    let lz = match &zp {
        Some(zp) => Layers::of(zp, outer, work)?,
        None => None,
    };
    let (op, os) = match (form, &zp, lz) {
        (OuterForm::Start, _, _) => (&xp[..lx.outer_prefix], &xp[lx.outer_suffix..]),
        (OuterForm::Other(_), Some(zp), Some(lz)) => {
            (&zp[..lz.outer_prefix], &zp[lz.outer_suffix..])
        }
        // A form with no number (a unit pattern with no `{0}`) has no outer layer: the end's.
        _ => (&yp[..ly.outer_prefix], &yp[ly.outer_suffix..]),
    };
    // The middle layers collapse if they are the same and have more than one code point.
    let (mx, my) = (lx.middle(&xp), ly.middle(&yp));
    let collapse_middle = mx == my && code_points(mx.0) + code_points(mx.1) > 1;
    // The spacing: a separator space where the pattern has none, if a layer of x repeats.
    let repeat =
        lx.inner > lx.number.1 || (!collapse_middle && code_points(mx.0) + code_points(mx.1) > 0);
    let mut separator = between.to_string();
    if repeat {
        if !separator.starts_with(is_pattern_space) {
            separator.insert(0, ' ');
        }
        if !separator.ends_with(is_pattern_space) {
            separator.push(' ');
        }
    }
    let mut out = RangeParts::with_capacity(xp.len() + yp.len() + 3);
    push_all(&mut out, op, Source::Shared, work)?;
    if collapse_middle {
        push_all(&mut out, mx.0, Source::Shared, work)?;
    }
    push_text(&mut out, before);
    let side = |out: &mut RangeParts,
                p: &Parts,
                l: &Layers,
                m: &Middle<'_>,
                source: Source,
                work: &mut Work|
     -> Result<(), Interrupted> {
        if !collapse_middle {
            push_all(out, m.0, source, work)?;
        }
        push_all(out, &p[l.number.0..l.inner], source, work)?;
        if !collapse_middle {
            push_all(out, m.1, source, work)?;
        }
        Ok(())
    };
    side(&mut out, &xp, &lx, &mx, Source::StartRange, work)?;
    push_text(&mut out, &separator);
    side(&mut out, &yp, &ly, &my, Source::EndRange, work)?;
    push_text(&mut out, after);
    if collapse_middle {
        push_all(&mut out, mx.1, Source::Shared, work)?;
    }
    push_all(&mut out, os, Source::Shared, work)?;
    Ok(out)
}

/// Pushes the pattern text `text` (if not empty) as a "shared" literal part.
fn push_text(out: &mut RangeParts, text: &str) {
    if !text.is_empty() {
        out.push((PartType::Literal, text.to_string(), Source::Shared));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digits::unpolled;

    /// The parts of a test number: `(type name, value)` pairs.
    fn parts(v: &[(PartType, &str)]) -> Parts {
        v.iter().map(|(t, s)| (*t, (*s).to_string())).collect()
    }

    /// The layers of the parts of a unit, a currency name, an exponent and a bidi sign.
    #[test]
    fn layers() {
        use PartType::*;
        // "-1.5E3 km/h" (unit: outer).
        let p = parts(&[
            (MinusSign, "-"),
            (Integer, "1"),
            (Decimal, "."),
            (Fraction, "5"),
            (ExponentSeparator, "E"),
            (ExponentInteger, "3"),
            (Literal, " "),
            (Unit, "km/h"),
        ]);
        let l = unpolled(|w| Layers::of(&p, true, w)).unwrap();
        assert_eq!(
            (l.outer_prefix, l.number, l.inner, l.outer_suffix),
            (0, (1, 4), 6, 6)
        );
        // zh-TW "每小時 ‎-1 公里": the bidi mark goes with the sign.
        let p = parts(&[
            (Unit, "每小時"),
            (Literal, " "),
            (Literal, "\u{200E}"),
            (MinusSign, "-"),
            (Integer, "1"),
            (Literal, " "),
            (Unit, "公里"),
        ]);
        let l = unpolled(|w| Layers::of(&p, true, w)).unwrap();
        assert_eq!(
            (l.outer_prefix, l.number, l.inner, l.outer_suffix),
            (2, (4, 5), 5, 5)
        );
        // "($3.00)" (no outer): all middle.
        let p = parts(&[
            (Literal, "("),
            (Currency, "$"),
            (Integer, "3"),
            (Decimal, "."),
            (Fraction, "00"),
            (Literal, ")"),
        ]);
        let l = unpolled(|w| Layers::of(&p, false, w)).unwrap();
        assert_eq!(
            (l.outer_prefix, l.number, l.inner, l.outer_suffix),
            (0, (2, 5), 5, 6)
        );
        let (a, b) = l.middle(&p);
        assert_eq!(code_points(a) + code_points(b), 3);
        // ar "1.2E‎-4": the exponent with its bidi mark is the inner layer.
        let p = parts(&[
            (Integer, "1"),
            (Decimal, "."),
            (Fraction, "2"),
            (ExponentSeparator, "E"),
            (Literal, "\u{200E}"),
            (ExponentMinusSign, "-"),
            (ExponentInteger, "4"),
        ]);
        let l = unpolled(|w| Layers::of(&p, false, w)).unwrap();
        assert_eq!((l.number, l.inner), ((0, 3), 7));
        assert!(unpolled(|w| Layers::of(&parts(&[(Unit, "ساعة")]), true, w)).is_none());
    }

    /// Each category of the cardinal rules has a sample value (the range form of a category
    /// that no end has), for the locales with CLDR plural ranges and many categories; sl:
    /// two is 2, few is 3, one is 101 (not 1: the integers 2 to 200 come first).
    #[test]
    fn every_category_has_a_sample() {
        for l in [
            "sl", "lv", "ru", "uk", "pl", "cs", "lt", "ar", "he", "ga", "cy", "br", "mt", "ro",
            "hr", "fr", "pt", "it", "es", "ca", "en", "is", "mk", "gd",
        ] {
            let rules = plural_rules::cardinal_rules(l).expect(l);
            let s = samples(&rules);
            for c in rules.categories() {
                let o = s.iter().find(|(k, _)| *k == c).map(|(_, o)| *o);
                assert_eq!(o.map(|o| rules.category_for(o)), Some(c), "{l} {c:?}");
            }
        }
        let sl = samples(&plural_rules::cardinal_rules("sl").expect("sl"));
        let of = |c| sl.iter().find(|(k, _)| *k == c).map(|(_, o)| *o);
        let op = |n: u32| Some(PluralOperands::from(&Decimal::from(n)));
        assert_eq!(
            (
                of(PluralCategory::Two),
                of(PluralCategory::Few),
                of(PluralCategory::One)
            ),
            (op(2), op(3), op(101))
        );
    }

    /// The range pattern parts and the ICU pattern spaces.
    #[test]
    fn range_patterns() {
        assert_eq!(split_range_pattern("{0}–{1}"), ("", "–", ""));
        assert_eq!(split_range_pattern("{0} - {1}"), ("", " - ", ""));
        assert_eq!(split_range_pattern("从{0}到{1}"), ("从", "到", ""));
        assert_eq!(split_range_pattern("bad"), ("", "–", ""));
        assert!(is_pattern_space(' ') && is_pattern_space('\u{200F}'));
        assert!(!is_pattern_space('\u{A0}'));
        assert_eq!(Source::Shared.name(), "shared");
    }
}
