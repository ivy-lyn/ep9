//! CLDR side tables (INTL-V2-D5): CLDR data that the ICU4X compiled data does not give.
//!
//! ```text
//! tools/cldr-tables (own workspace) ── CLDR JSON (pinned by hash) ──► gen/*.rs (generated)
//!   gen/version.rs  CLDR_VERSION, CLDR_JSON_COMMIT
//!   gen/digits.rs   NUMBERING_SYSTEMS   the 78 numbering systems with simple digit mappings
//!   gen/t1.rs       T1_SYMBOLS, T1_ROWS NaN, infinity, exponent symbol, approximately sign and
//!                                       range pattern per CLDR locale and numbering system
//!   gen/locales.rs  CLDR_LOCALES        the availableLocales "full" ids (crate::available)
//!   gen/hour_cycles.rs  HOUR_CYCLE_*    the timeData preferred hour cycles (crate::locale_info)
//!   gen/aliases.rs  U_VALUE_ALIASES, T_VALUE_ALIASES  the bcp47 value aliases (crate::tag)
//!   gen/t2.rs       T2_*                the unit patterns of the sanctioned units and of the
//!                                       CLDR compound units of two of them, per CLDR locale
//!                                       and width (crate::number_format::unit)
//! ```
//!
//! The CLDR version of the tables is the CLDR version of the ICU4X compiled data (the tests of
//! `tools/cldr-tables` check it). A BUMP of ICU4X makes the tables again. Do not edit `gen/`:
//! run the tool (`tools/cldr-tables/README.md`).
//!
//! Lookup of T1: a row exists for each CLDR locale id and each numbering system that the locale
//! has symbols for. The CLDR JSON values are resolved (a child holds all values, not only the
//! values that differ from its parent), so the first row found on the fallback chain of a locale
//! (for example `en-US` -> `en`) is the answer.
//! If no locale on the chain has a row for `nu` (for example `en` with `arab`), take the `latn`
//! row of the same chain: every CLDR locale has one (test `tables_are_sorted_and_complete`).
//!
//! Lookup of T2 ([`unit_set_for`], [`unit_patterns`]): every CLDR locale has a unit set (the
//! data of all its units), so the first CLDR locale of the fallback chain of a locale
//! ([`locale_chain`]; "und" at its end is one) gives the set. T2 is shared parts: the distinct
//! texts, the distinct records (the patterns of one unit and width) and the distinct sets.
//!
//! ```text
//! data locale ─ locale_chain ─ CLDR_LOCALES index ─ T2_LOCALE_SETS ─ set
//! (set, width, unit) ─ T2_SETS ─ record ─ T2_RECORDS: [zero, one, two, few, many, other, per]
//!                                          text indices into T2_TEXT_STARTS / T2_TEXT
//! (set, width) ─ T2_PER ─ the compound pattern "per" ("{0}/{1}")
//! ```
// The W2 builders (NF-CORE, NF-SCIRANGE) use the tables; until then only the tests read them.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "INTL W2 services read the tables")
)]

// The generated files are module files (`#[path]`), not `include!` text: extref-lint scans
// each module file, and it refuses `include!` (rule `macro`, V11IN-22). `rustfmt::skip` keeps
// rustfmt out of them: the generator writes their form, and `cldr-tables --check` checks it.
/// `CLDR_VERSION`, `CLDR_JSON_COMMIT`.
#[rustfmt::skip]
#[path = "gen/version.rs"]
pub(crate) mod version;
/// `NUMBERING_SYSTEMS`, `NUMBERING_SYSTEM_NAMES`.
#[rustfmt::skip]
#[path = "gen/digits.rs"]
pub(crate) mod digits;
/// `CLDR_LOCALES`.
#[rustfmt::skip]
#[path = "gen/locales.rs"]
pub(crate) mod locales;
/// `HOUR_CYCLE_H12_REGIONS`, `HOUR_CYCLE_LANGUAGE_REGIONS`.
#[rustfmt::skip]
#[path = "gen/hour_cycles.rs"]
pub(crate) mod hour_cycles;
/// `U_VALUE_ALIASES`, `T_VALUE_ALIASES`.
#[rustfmt::skip]
#[path = "gen/aliases.rs"]
pub(crate) mod aliases;
/// `T1_SYMBOLS`, `T1_ROWS`.
#[rustfmt::skip]
#[path = "gen/t1.rs"]
mod t1;
/// `T2_UNITS`, `T2_TEXT`, `T2_TEXT_STARTS`, `T2_RECORDS`, `T2_PER`, `T2_SETS`,
/// `T2_LOCALE_SETS`.
#[rustfmt::skip]
#[path = "gen/t2.rs"]
mod t2;

use icu_plurals::PluralCategory;
use icu_provider::DataLocale;

use crate::tag::icu_locale;

/// The T1 values of one CLDR locale and numbering system.
#[derive(Debug)]
pub(crate) struct NumberSymbols {
    /// The string of NaN (CLDR `nan`).
    pub(crate) nan: &'static str,
    /// The string of an infinite value (CLDR `infinity`).
    pub(crate) infinity: &'static str,
    /// The exponent separator of scientific notation (CLDR `exponential`).
    pub(crate) exponential: &'static str,
    /// The sign of an approximate value (CLDR `approximatelySign`).
    pub(crate) approximately_sign: &'static str,
    /// The range pattern (CLDR misc pattern `range`): `{0}` and `{1}` are the two ends.
    pub(crate) range: &'static str,
}

/// The index of numbering system `nu` in `NUMBERING_SYSTEMS`.
fn numbering_system_index(nu: &str) -> Option<usize> {
    digits::NUMBERING_SYSTEMS
        .binary_search_by(|(name, _)| (*name).cmp(nu))
        .ok()
}

/// The digits 0 to 9 of numbering system `nu`, if it has a simple digit mapping (ECMA-402
/// table "Numbering systems with simple digit mappings").
pub(crate) fn numbering_system_digits(nu: &str) -> Option<&'static [char; 10]> {
    numbering_system_index(nu).map(|i| &digits::NUMBERING_SYSTEMS[i].1)
}

/// The T1 values of CLDR locale id `locale` (for example `de-CH`, `sr-Latn`, `und`) and
/// numbering system `nu`, if CLDR has symbols for that pair. No fallback: the caller walks the
/// locale fallback chain (module doc).
pub(crate) fn number_symbols(locale: &str, nu: &str) -> Option<&'static NumberSymbols> {
    let n = numbering_system_index(nu)?;
    let i = t1::T1_ROWS
        .binary_search_by(|(l, x, _)| (*l, usize::from(*x)).cmp(&(locale, n)))
        .ok()?;
    Some(&t1::T1_SYMBOLS[usize::from(t1::T1_ROWS[i].2)])
}

/// The CLDR locale ids to try for `data_locale`, in order: the ICU4X locale fallback chain
/// (it knows the CLDR parent locales: "zh-TW" -> "zh-Hant-TW" -> "zh-Hant"), then the
/// subtag cuts of `data_locale`, then "und".
pub(crate) fn locale_chain(data_locale: &str) -> Vec<String> {
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

// ---- T2: the unit patterns (INTL W3 NF-UNIT) --------------------------------------------------

/// The T2 widths, in the order of the `[[UnitDisplay]]` values of Intl.NumberFormat.
pub(crate) const UNIT_WIDTHS: [&str; 3] = ["short", "narrow", "long"];

/// T2 text `i`; `None` for text 0, the empty text ("none" in a record).
fn t2_text(i: u16) -> Option<&'static str> {
    let i = usize::from(i);
    let start = *t2::T2_TEXT_STARTS.get(i)? as usize;
    let end = *t2::T2_TEXT_STARTS.get(i + 1)? as usize;
    t2::T2_TEXT.get(start..end).filter(|t| !t.is_empty())
}

/// The T2 patterns of one unit and width in one locale (a T2 record).
#[derive(Clone, Copy, Debug)]
pub(crate) struct UnitPatterns(&'static [u16; 7]);

impl UnitPatterns {
    /// The unit pattern of the plural category `c` (CLDR `unitPattern-count-<c>`; the "other"
    /// pattern if CLDR has none for `c`). `{0}` is the number. A pattern has `{0}` at most
    /// once; with none, the pattern takes the place of the number (ar "ساعة", one hour), as
    /// in ICU.
    pub(crate) fn pattern(self, c: PluralCategory) -> &'static str {
        let form = match c {
            PluralCategory::Zero => 0,
            PluralCategory::One => 1,
            PluralCategory::Two => 2,
            PluralCategory::Few => 3,
            PluralCategory::Many => 4,
            PluralCategory::Other => 5,
        };
        t2_text(self.0[form])
            .or_else(|| t2_text(self.0[5]))
            .unwrap_or("{0}")
    }

    /// The per-unit pattern (CLDR `perUnitPattern`: en short hour "{0}/h"), if CLDR has one.
    pub(crate) fn per_unit(self) -> Option<&'static str> {
        t2_text(self.0[6])
    }
}

/// The T2 unit set of the CLDR locale id `locale`; no fallback.
fn unit_set(locale: &str) -> Option<usize> {
    let i = locales::CLDR_LOCALES.binary_search(&locale).ok()?;
    t2::T2_LOCALE_SETS.get(i).map(|s| usize::from(*s))
}

/// The T2 unit set of the data locale `data_locale`: the set of the first CLDR locale of its
/// fallback chain ([`locale_chain`]).
pub(crate) fn unit_set_for(data_locale: &str) -> Option<usize> {
    locale_chain(data_locale).iter().find_map(|l| unit_set(l))
}

/// The T2 patterns of `unit` (a sanctioned simple unit, or a CLDR compound unit "X-per-Y"
/// such as `kilometer-per-hour`) in the width `width` (an index of [`UNIT_WIDTHS`]) of the
/// unit set `set`; `None` for a unit that T2 does not list (most "X-per-Y" units: the caller
/// builds them from X, Y and [`per_pattern`]).
pub(crate) fn unit_patterns(set: usize, unit: &str, width: usize) -> Option<UnitPatterns> {
    let u = t2::T2_UNITS.binary_search(&unit).ok()?;
    let r = *t2::T2_SETS.get(set)?.get(width)?.get(u)?;
    t2::T2_RECORDS.get(usize::from(r)).map(UnitPatterns)
}

/// The compound pattern "per" (CLDR `per` `compoundUnitPattern`: en short "{0}/{1}") of the
/// unit set `set` in the width `width`.
pub(crate) fn per_pattern(set: usize, width: usize) -> Option<&'static str> {
    t2_text(*t2::T2_PER.get(set)?.get(width)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One value of each table, and the CLDR version (INTL-V2 W2-0 TABLES).
    #[test]
    fn one_value_per_table() {
        assert_eq!(version::CLDR_VERSION, "49.0.0");
        assert_eq!(version::CLDR_JSON_COMMIT.len(), 40);
        assert_eq!(
            numbering_system_digits("arab"),
            Some(&['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'])
        );
        assert_eq!(
            numbering_system_digits("roman"),
            None,
            "not a simple digit mapping"
        );
        let en = number_symbols("en", "latn").expect("en latn");
        assert_eq!(en.nan, "NaN");
        assert_eq!(en.infinity, "∞");
        assert_eq!(en.exponential, "E");
        assert_eq!(en.approximately_sign, "~");
        assert_eq!(en.range, "{0}–{1}");
        // A locale with its own values: Arabic in the arab numbering system.
        let ar = number_symbols("ar", "arab").expect("ar arab");
        assert_eq!(ar.exponential, "أس");
        assert!(
            number_symbols("en-US", "latn").is_none(),
            "no fallback in the table"
        );
        assert!(number_symbols("en", "arab").is_none());
    }

    /// The tables are sorted, so the binary searches find every row.
    #[test]
    fn tables_are_sorted_and_complete() {
        assert_eq!(
            digits::NUMBERING_SYSTEMS.len(),
            78,
            "the ECMA-402 table has 78 systems"
        );
        assert!(
            digits::NUMBERING_SYSTEMS
                .windows(2)
                .all(|w| w[0].0 < w[1].0)
        );
        assert!(
            digits::NUMBERING_SYSTEMS
                .iter()
                .map(|x| x.0)
                .eq(digits::NUMBERING_SYSTEM_NAMES)
        );
        assert!(
            t1::T1_ROWS
                .windows(2)
                .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
        );
        for (l, n, _) in &t1::T1_ROWS {
            let nu = digits::NUMBERING_SYSTEMS[usize::from(*n)].0;
            assert!(number_symbols(l, nu).is_some(), "{l} {nu}");
        }
        // Every CLDR locale has latn symbols.
        let latn = numbering_system_index("latn").expect("latn");
        let locales = t1::T1_ROWS
            .iter()
            .filter(|(_, n, _)| usize::from(*n) == latn)
            .count();
        assert_eq!(
            locales,
            t1::T1_ROWS
                .iter()
                .map(|r| r.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
    }

    /// T2 is complete and its indices are in range: a set for every CLDR locale, the 45
    /// sanctioned units, every text a slice of `T2_TEXT` (INTL W3 NF-UNIT).
    #[test]
    fn t2_is_complete() {
        assert_eq!(t2::T2_LOCALE_SETS.len(), locales::CLDR_LOCALES.len());
        assert!(t2::T2_UNITS.windows(2).all(|w| w[0] < w[1]));
        for u in crate::available::UNITS {
            assert!(t2::T2_UNITS.binary_search(u).is_ok(), "{u}");
        }
        let compound: Vec<&str> = t2::T2_UNITS
            .iter()
            .copied()
            .filter(|u| u.contains("-per-"))
            .collect();
        assert_eq!(
            compound,
            [
                "kilometer-per-hour",
                "liter-per-kilometer",
                "meter-per-second",
                "mile-per-gallon",
                "mile-per-hour"
            ]
        );
        assert_eq!(t2_text(0), None, "text 0 is the empty text");
        for i in 1..t2::T2_TEXT_STARTS.len() - 1 {
            assert!(t2_text(i as u16).is_some(), "text {i}");
        }
        assert_eq!(
            t2::T2_TEXT_STARTS.last().map(|x| *x as usize),
            Some(t2::T2_TEXT.len())
        );
        let texts = t2::T2_TEXT_STARTS.len() - 1;
        for r in &t2::T2_RECORDS {
            assert!(r.iter().all(|t| usize::from(*t) < texts));
            assert!(t2_text(r[5]).is_some(), "every record has an other form");
        }
        for (per, set) in t2::T2_PER.iter().zip(&t2::T2_SETS) {
            assert!(per.iter().all(|t| t2_text(*t).is_some()));
            for w in set {
                assert!(w.iter().all(|r| usize::from(*r) < t2::T2_RECORDS.len()));
            }
        }
        assert!(
            t2::T2_LOCALE_SETS
                .iter()
                .all(|s| usize::from(*s) < t2::T2_SETS.len())
        );
        assert!(unit_set("und").is_some() && unit_set("en-US").is_none());
    }

    /// One value of each kind of T2 at CLDR 49.0.0 (the test262 `unit-*.js` strings).
    #[test]
    fn t2_values() {
        let p = |l: &str, u: &str, width: &str| {
            let w = UNIT_WIDTHS.iter().position(|x| *x == width).expect(width);
            unit_patterns(unit_set_for(l).expect(l), u, w).expect(u)
        };
        use PluralCategory::{Few, One, Other};
        assert_eq!(
            p("en-US", "kilometer-per-hour", "short").pattern(Other),
            "{0} km/h"
        );
        assert_eq!(
            p("en", "kilometer-per-hour", "narrow").pattern(Other),
            "{0}km/h"
        );
        assert_eq!(
            p("en", "kilometer-per-hour", "long").pattern(One),
            "{0} kilometer per hour"
        );
        assert_eq!(
            p("zh-TW", "kilometer-per-hour", "long").pattern(Other),
            "每小時 {0} 公里"
        );
        assert_eq!(
            p("ja-JP", "kilometer-per-hour", "long").pattern(Other),
            "時速 {0} キロメートル"
        );
        assert_eq!(
            p("de-DE", "kilometer-per-hour", "short").pattern(Other),
            "{0} km/h"
        );
        assert_eq!(p("en", "percent", "short").pattern(Other), "{0}%");
        // A form that CLDR does not give is the other form (ja has only "other").
        assert_eq!(p("ja", "hour", "long").pattern(One), "{0} 時間");
        assert_eq!(p("ru", "hour", "long").pattern(Few), "{0} часа");
        // A pattern with no number.
        assert_eq!(p("ar", "hour", "long").pattern(One), "ساعة");
        assert_eq!(p("en", "hour", "short").per_unit(), Some("{0}/h"));
        assert_eq!(p("en", "stone", "short").per_unit(), None);
        let en = unit_set_for("en").expect("en");
        assert_eq!(per_pattern(en, 0), Some("{0}/{1}"));
        assert_eq!(per_pattern(en, 2), Some("{0} per {1}"));
        assert!(unit_patterns(en, "meter-per-hour", 0).is_none());
        assert!(unit_patterns(en, "parsec", 0).is_none());
        // zh-TW takes the CLDR parent zh-Hant, not zh (Simplified Chinese).
        assert_eq!(unit_set_for("zh-TW"), unit_set_for("zh-Hant"));
        assert_ne!(unit_set_for("zh-TW"), unit_set_for("zh"));
    }

    /// INTL W3 correction 4 (b3): T2 equals the ICU4X units data (CLDR 49.0.0) in every
    /// plural category, for each unit, width and locale that the ICU4X data has without
    /// fallback (the 5 categories area, duration, length, mass and volume). The plural
    /// category of a sample number comes from the cardinal rules that NumberFormat uses
    /// (`plural_rules::cardinal_rules`); ICU4X selects its pattern with the same rules. The
    /// `tools/cldr-tables` test compares only the "other" pattern (it has no `icu_plurals`).
    #[test]
    fn t2_equals_icu4x_in_every_plural_category() {
        use fixed_decimal::Decimal;
        use icu_experimental::dimension::provider::units::categorized_display_names::*;
        use icu_experimental::provider::Baked;
        use icu_plurals::PluralOperands;
        use icu_provider::prelude::*;

        /// The ICU4X patterns of `M` for (`attributes`, `locale`) without fallback, with
        /// `{0}` for the number, by the plural category of each operand of `samples`.
        fn icu4x<M>(
            attributes: &str,
            locale: &DataLocale,
            rules: &icu_plurals::PluralRules,
            samples: &[PluralOperands],
        ) -> Option<Vec<String>>
        where
            M: DataMarker<DataStruct = UnitsDisplayNames<'static>>,
            Baked: DataProvider<M>,
        {
            let mut metadata = DataRequestMetadata::default();
            metadata.silent = true;
            let req = DataRequest {
                id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
                    DataMarkerAttributes::try_from_str(attributes).ok()?,
                    locale,
                ),
                metadata,
            };
            let res = DataProvider::<M>::load(&Baked, req).ok()?;
            if res.metadata.locale.is_some() {
                return None;
            }
            let names = res.payload.get();
            Some(
                samples
                    .iter()
                    .map(|op| names.get(*op, rules).interpolate_to_string(["{0}"]))
                    .collect(),
            )
        }
        use icu_experimental::dimension::provider::units::display_names::UnitsDisplayNames;
        type Load = fn(
            &str,
            &DataLocale,
            &icu_plurals::PluralRules,
            &[PluralOperands],
        ) -> Option<Vec<String>>;
        let loaders: [Load; 15] = [
            icu4x::<UnitsNamesAreaCoreV1>,
            icu4x::<UnitsNamesAreaExtendedV1>,
            icu4x::<UnitsNamesAreaOutlierV1>,
            icu4x::<UnitsNamesDurationCoreV1>,
            icu4x::<UnitsNamesDurationExtendedV1>,
            icu4x::<UnitsNamesDurationOutlierV1>,
            icu4x::<UnitsNamesLengthCoreV1>,
            icu4x::<UnitsNamesLengthExtendedV1>,
            icu4x::<UnitsNamesLengthOutlierV1>,
            icu4x::<UnitsNamesMassCoreV1>,
            icu4x::<UnitsNamesMassExtendedV1>,
            icu4x::<UnitsNamesMassOutlierV1>,
            icu4x::<UnitsNamesVolumeCoreV1>,
            icu4x::<UnitsNamesVolumeExtendedV1>,
            icu4x::<UnitsNamesVolumeOutlierV1>,
        ];
        // Sample numbers: one per plural category of most rules (integers, decimals, and the
        // millions of the "many" of fr, es, it, pt).
        let mut numbers: Vec<String> = (0..=220).map(|i| i.to_string()).collect();
        numbers.extend(
            [
                "0.5", "1.5", "2.5", "0.1", "1.1", "2.1", "10.5", "1000000", "2000000",
            ]
            .map(String::from),
        );
        let (mut compared, mut locales_hit, mut units_hit) =
            (0usize, 0usize, std::collections::BTreeSet::new());
        let mut wrong = Vec::new();
        for locale in &locales::CLDR_LOCALES {
            let Some(rules) = crate::plural_rules::cardinal_rules(locale) else {
                continue;
            };
            // One sample per category of the rules.
            let mut samples: Vec<(PluralCategory, PluralOperands)> = Vec::new();
            for n in &numbers {
                let op = PluralOperands::from(&Decimal::try_from_str(n).expect("a number"));
                let c = rules.category_for(op);
                if !samples.iter().any(|(x, _)| *x == c) {
                    samples.push((c, op));
                }
            }
            let ops: Vec<PluralOperands> = samples.iter().map(|(_, op)| *op).collect();
            let data_locale = DataLocale::from(&icu_locale(locale));
            let set = unit_set(locale).expect("every CLDR locale has a T2 set");
            let mut hit = false;
            for (w, width) in UNIT_WIDTHS.iter().enumerate() {
                for unit in crate::available::UNITS {
                    let attributes = format!("{width}-{unit}");
                    let Some(icu) = loaders
                        .iter()
                        .find_map(|load| load(&attributes, &data_locale, &rules, &ops))
                    else {
                        continue;
                    };
                    let t2 = unit_patterns(set, unit, w).expect("T2 has every sanctioned unit");
                    for ((c, _), want) in samples.iter().zip(&icu) {
                        compared += 1;
                        if t2.pattern(*c) != want {
                            wrong.push(format!(
                                "{locale} {attributes} {c:?}: T2 {:?}, ICU4X {want:?}",
                                t2.pattern(*c)
                            ));
                        }
                    }
                    units_hit.insert(*unit);
                    hit = true;
                }
            }
            locales_hit += usize::from(hit);
        }
        eprintln!(
            "T2 vs ICU4X: {compared} patterns, {locales_hit} locales, {} units",
            units_hit.len()
        );
        assert!(
            wrong.is_empty(),
            "{} differ:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
        // The 5 ICU4X categories hold 30 of the 45 sanctioned units (not: bit, byte, celsius,
        // degree, fahrenheit, percent and the 9 kilo- to peta- bits and bytes).
        assert_eq!(units_hit.len(), 30, "{units_hit:?}");
        assert!(locales_hit >= 100, "{locales_hit}");
    }
}
