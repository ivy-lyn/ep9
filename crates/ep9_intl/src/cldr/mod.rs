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
// The W2 builders (NF-CORE, NF-SCIRANGE) use the tables; until then only the tests read them.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "INTL W2 services read the tables")
)]

/// `CLDR_VERSION`, `CLDR_JSON_COMMIT`.
pub(crate) mod version {
    include!("gen/version.rs");
}
/// `NUMBERING_SYSTEMS`, `NUMBERING_SYSTEM_NAMES`.
pub(crate) mod digits {
    include!("gen/digits.rs");
}
/// `CLDR_LOCALES`.
pub(crate) mod locales {
    include!("gen/locales.rs");
}
/// `HOUR_CYCLE_H12_REGIONS`, `HOUR_CYCLE_LANGUAGE_REGIONS`.
pub(crate) mod hour_cycles {
    include!("gen/hour_cycles.rs");
}
/// `U_VALUE_ALIASES`, `T_VALUE_ALIASES`.
pub(crate) mod aliases {
    include!("gen/aliases.rs");
}
mod t1 {
    use super::NumberSymbols;
    include!("gen/t1.rs");
}

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
}
