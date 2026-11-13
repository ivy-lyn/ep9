//! The CLDR plural rules of the available locales that the ICU4X compiled data does not have.
//!
//! The ICU4X compiled data holds the plural rules of the CLDR "recommended" locales only, but
//! the available locales are the CLDR "full" list (`crate::available`). A language of that list
//! with no ICU4X rules would get the "und" rules (every value "other"; test262
//! `plural-categories-order.js` asks for "gv"). This file holds the CLDR rules of those
//! languages (CLDR 49 `supplemental/plurals.json` and `ordinals.json`, the cldr-json commit of
//! `tools/cldr-tables/cldr-json.commit`; the `@integer`/`@decimal` samples are cut). A unit test
//! checks that the ICU4X data still has no rules for each language here.
//!
//! ```text
//! rule      = and ("or" and)*          and   = relation ("and" relation)*
//! relation  = operand ["%" int] ("=" | "!=") range ("," range)*      range = int [".." int]
//! operand   = n i v w f t e c          (the CLDR operands, from icu_plurals RawPluralOperands)
//! ```
//!
//! Integer arithmetic only. `n` is the absolute value: it is in a list of integers only when it
//! has no visible nonzero fraction digit (`t = 0`).
//!
//! None of these locales has CLDR plural range data other than "gsw", whose ranges give the end
//! category, so the range of these locales is the end category (the caller's default).

use icu_plurals::{PluralCategory, PluralOperands, RawPluralOperands};

/// The cardinal rules: language, then (category, rule) in the order zero, one, two, few, many.
const CARDINAL: &[(&str, &[(&str, &str)])] = &[
    ("asa", &[("one", "n = 1")]),
    ("bem", &[("one", "n = 1")]),
    ("bez", &[("one", "n = 1")]),
    ("ce", &[("one", "n = 1")]),
    ("cgg", &[("one", "n = 1")]),
    ("ff", &[("one", "i = 0,1")]),
    ("fur", &[("one", "n = 1")]),
    ("gsw", &[("one", "n = 1")]),
    (
        "gv",
        &[
            ("one", "v = 0 and i % 10 = 1"),
            ("two", "v = 0 and i % 10 = 2"),
            ("few", "v = 0 and i % 100 = 0,20,40,60,80"),
            ("many", "v != 0"),
        ],
    ),
    ("haw", &[("one", "n = 1")]),
    ("jgo", &[("one", "n = 1")]),
    ("jmc", &[("one", "n = 1")]),
    ("kab", &[("one", "i = 0,1")]),
    ("kkj", &[("one", "n = 1")]),
    ("kl", &[("one", "n = 1")]),
    ("ksb", &[("one", "n = 1")]),
    (
        "kw",
        &[
            ("zero", "n = 0"),
            ("one", "n = 1"),
            (
                "two",
                "n % 100 = 2,22,42,62,82 or n % 1000 = 0 and n % 100000 = 1000..20000,40000,60000,80000 or n != 0 and n % 1000000 = 100000",
            ),
            ("few", "n % 100 = 3,23,43,63,83"),
            ("many", "n != 1 and n % 100 = 1,21,41,61,81"),
        ],
    ),
    ("lag", &[("zero", "n = 0"), ("one", "i = 0,1 and n != 0")]),
    ("lg", &[("one", "n = 1")]),
    ("ln", &[("one", "n = 0..1")]),
    ("mas", &[("one", "n = 1")]),
    ("mg", &[("one", "n = 0..1")]),
    ("mgo", &[("one", "n = 1")]),
    ("naq", &[("one", "n = 1"), ("two", "n = 2")]),
    ("nd", &[("one", "n = 1")]),
    ("nnh", &[("one", "n = 1")]),
    ("nyn", &[("one", "n = 1")]),
    ("os", &[("one", "n = 1")]),
    ("rof", &[("one", "n = 1")]),
    ("rwk", &[("one", "n = 1")]),
    ("saq", &[("one", "n = 1")]),
    ("se", &[("one", "n = 1"), ("two", "n = 2")]),
    ("seh", &[("one", "n = 1")]),
    ("shi", &[("one", "i = 0 or n = 1"), ("few", "n = 2..10")]),
    ("smn", &[("one", "n = 1"), ("two", "n = 2")]),
    ("sn", &[("one", "n = 1")]),
    ("teo", &[("one", "n = 1")]),
    ("tzm", &[("one", "n = 0..1 or n = 11..99")]),
    ("vun", &[("one", "n = 1")]),
    ("wae", &[("one", "n = 1")]),
    ("xog", &[("one", "n = 1")]),
    ("yi", &[("one", "i = 1 and v = 0")]),
];

/// The ordinal rules, in the form of [`CARDINAL`].
const ORDINAL: &[(&str, &[(&str, &str)])] = &[(
    "kw",
    &[
        (
            "one",
            "n = 1..4 or n % 100 = 1..4,21..24,41..44,61..64,81..84",
        ),
        ("many", "n = 5 or n % 100 = 5"),
    ],
)];

/// A CLDR operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operand {
    N,
    I,
    V,
    W,
    F,
    T,
    C,
}

/// One relation: `operand [% modulo] (= | !=) ranges`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Relation {
    operand: Operand,
    modulo: Option<u64>,
    negated: bool,
    ranges: Vec<(u64, u64)>,
}

/// The parsed rules of one language and type: per category, the "or" list of "and" lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    categories: Vec<(PluralCategory, Vec<Vec<Relation>>)>,
}

/// The category of the CLDR name `s`.
fn category(s: &str) -> Option<PluralCategory> {
    Some(match s {
        "zero" => PluralCategory::Zero,
        "one" => PluralCategory::One,
        "two" => PluralCategory::Two,
        "few" => PluralCategory::Few,
        "many" => PluralCategory::Many,
        _ => return None,
    })
}

/// The relation in the words `w` (`["i", "%", "10", "=", "1,2..4"]`).
fn relation(w: &[&str]) -> Option<Relation> {
    let operand = match *w.first()? {
        "n" => Operand::N,
        "i" => Operand::I,
        "v" => Operand::V,
        "w" => Operand::W,
        "f" => Operand::F,
        "t" => Operand::T,
        "e" | "c" => Operand::C,
        _ => return None,
    };
    let (modulo, rest) = match w.get(1) {
        Some(&"%") => (Some(w.get(2)?.parse().ok()?), &w[3..]),
        _ => (None, &w[1..]),
    };
    let [op, list] = rest else {
        return None;
    };
    let negated = match *op {
        "=" => false,
        "!=" => true,
        _ => return None,
    };
    let ranges = list
        .split(',')
        .map(|r| match r.split_once("..") {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => r.parse().ok().map(|v| (v, v)),
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Relation {
        operand,
        modulo: modulo.filter(|m| *m != 0),
        negated,
        ranges,
    })
}

/// The "or" list of "and" lists of the rule text `s`.
fn condition(s: &str) -> Option<Vec<Vec<Relation>>> {
    let words: Vec<&str> = s.split_whitespace().collect();
    words
        .split(|w| *w == "or")
        .map(|and| {
            and.split(|w| *w == "and")
                .map(relation)
                .collect::<Option<Vec<_>>>()
        })
        .collect()
}

/// The parsed table rows of one language.
fn parse(rows: &[(&str, &str)]) -> Option<Rules> {
    let categories = rows
        .iter()
        .map(|(c, r)| Some((category(c)?, condition(r)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(Rules { categories })
}

/// The CLDR rules of `language` and type (`None` if this table does not have them).
pub fn rules_for(language: &str, ordinal: bool) -> Option<Rules> {
    let table = if ordinal { ORDINAL } else { CARDINAL };
    let (_, rows) = table.iter().find(|(l, _)| *l == language)?;
    parse(rows)
}

impl Relation {
    /// Whether the relation holds for the operands `o`.
    fn holds(&self, o: &RawPluralOperands) -> bool {
        let (value, integral) = match self.operand {
            Operand::N => (o.i, o.t == 0),
            Operand::I => (o.i, true),
            Operand::V => (o.v as u64, true),
            Operand::W => (o.w as u64, true),
            Operand::F => (o.f, true),
            Operand::T => (o.t, true),
            Operand::C => (o.c as u64, true),
        };
        let value = self.modulo.map_or(value, |m| value % m);
        let inside = integral && self.ranges.iter().any(|(a, b)| (*a..=*b).contains(&value));
        inside != self.negated
    }
}

impl Rules {
    /// The category of the operands `o`: the first category whose rule holds, else "other".
    pub fn category_for(&self, o: PluralOperands) -> PluralCategory {
        let o = RawPluralOperands::from(o);
        self.categories
            .iter()
            .find(|(_, or)| or.iter().any(|and| and.iter().all(|r| r.holds(&o))))
            .map_or(PluralCategory::Other, |(c, _)| *c)
    }

    /// The categories of the rules and "other".
    pub fn categories(&self) -> impl Iterator<Item = PluralCategory> + '_ {
        self.categories
            .iter()
            .map(|(c, _)| *c)
            .chain([PluralCategory::Other])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row parses, and the ICU4X compiled data still has no rules for its language (if
    /// it has, the row must go).
    #[test]
    fn every_row_parses_and_is_not_in_the_icu4x_data() {
        for (ordinal, table) in [(false, CARDINAL), (true, ORDINAL)] {
            for (language, rows) in table {
                assert!(parse(rows).is_some(), "{language}: a rule does not parse");
                let locale = super::super::icu_locale(language);
                assert!(
                    !super::super::has_own_rules(&locale, ordinal),
                    "{language} (ordinal {ordinal}): the ICU4X data has rules now"
                );
                assert!(
                    crate::available::CLDR_LOCALES
                        .iter()
                        .any(|l| l.split('-').next() == Some(language)),
                    "{language}: not an available language"
                );
            }
        }
    }

    /// The relations of "kw" on the edge values (node v26.13.0 gives the same categories).
    #[test]
    fn kw_relations() {
        let r = rules_for("kw", false).unwrap();
        let sel = |x: u64| r.category_for(PluralOperands::from(x));
        assert_eq!(
            [0, 1, 2, 3, 21, 22, 1000, 21000, 100_000, 1_100_000, 4].map(sel),
            [
                PluralCategory::Zero,
                PluralCategory::One,
                PluralCategory::Two,
                PluralCategory::Few,
                PluralCategory::Many,
                PluralCategory::Two,
                PluralCategory::Two,
                PluralCategory::Other,
                PluralCategory::Two,
                PluralCategory::Two,
                PluralCategory::Other,
            ]
        );
    }
}
