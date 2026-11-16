//! T3 (the `t3` mode; INTL-V2 plan 2.2, the W4 DTF-PATTERN row): the CLDR date data of
//! Intl.DateTimeFormat that the ICU4X compiled data does not give.
//!
//! ```text
//! cldr-json (cldr-json.commit)                          crates/ep9_intl/src/cldr/gen/t3.rs
//!   cldr-core/supplemental/dayPeriods.json
//!     dayPeriodRuleSet: the day period of each hour,
//!     and "noon" / "midnight" (`_at`) ────────────────►  T3_RULE_LOCALES, T3_RULES
//!   cldr-dates-full/main/<locale>/ca-gregorian.json
//!     dayPeriods.format.{abbreviated,narrow,wide}:
//!       the flexible day period names ─────────────────►  T3_TEXT, T3_TEXT_STARTS, T3_NAMES
//!     dateTimeFormats.availableFormats yMd, yM, Md ───►  T3_SETS, T3_LOCALE_SETS
//!   cldr-numbers-full/main/<locale>/numbers.json
//!     symbols-numberSystem-<nu> decimal, of every
//!     locale, the most frequent sign per system ─────►  T3_DECIMALS
//! ```
//!
//! The flexible day periods (UTS 35 pattern symbol "B", the ECMA-402 option `dayPeriod`):
//! ICU4X has no data for them (icu4x#487). The availableFormats patterns of the numeric date
//! skeletons give the field widths that ICU4C (DateTimePatternGenerator) uses for a numeric
//! year, month and day ("d.M.y" in German, where the ICU4X short and medium patterns have
//! "dd.MM."). The decimal signs: ICU4C writes the decimal sign of the numbering system before
//! the fractional seconds, from the root locale when the locale has no symbols for the system
//! ("٫" for `en-u-nu-arab`); the CLDR JSON has no root symbols but "latn", and the ICU4X data
//! has the symbols of a system only in the locales that use it, so T3 holds, per system other
//! than "latn", the sign that most CLDR locales with symbols for it have (ties: the first in
//! sort order), when it is not ".".
//!
//! The day period ids of a rule (`T3_RULES`): 0 morning1, 1 morning2, 2 afternoon1,
//! 3 afternoon2, 4 evening1, 5 evening2, 6 night1, 7 night2, 8 am, 9 pm ([`PERIODS`]). The
//! names of a record (`T3_NAMES`) are those of ids 0-7 and of "noon" (index 8). "midnight"
//! has no name in T3: ICU4C does not write it (ICU 57 and later), so a reader does not need it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::{git_head, lit, number_lines, read_json, str_at};

/// The file the `t3` mode writes, relative to the output directory.
pub const OUTPUT_FILE: &str = "t3.rs";

/// The day periods of a rule, in id order (`T3_RULES` values).
pub const PERIODS: [&str; 10] = [
    "morning1",
    "morning2",
    "afternoon1",
    "afternoon2",
    "evening1",
    "evening2",
    "night1",
    "night2",
    "am",
    "pm",
];

/// The names of a T3 name record, in order: the first 8 [`PERIODS`] and "noon".
pub const NAMES: [&str; 9] = [
    "morning1",
    "morning2",
    "afternoon1",
    "afternoon2",
    "evening1",
    "evening2",
    "night1",
    "night2",
    "noon",
];

/// The name widths of a T3 set, in order (ECMA-402 `dayPeriod` "short", "narrow", "long").
pub const WIDTHS: [&str; 3] = ["abbreviated", "narrow", "wide"];

/// The availableFormats skeletons of a T3 set, in order.
pub const SKELETONS: [&str; 3] = ["yMd", "yM", "Md"];

/// Rule flag: the rule set has "noon" (`_at` 12:00).
pub const FLAG_NOON: u8 = 1;
/// Rule flag: the rule set has "midnight" (`_at` 00:00).
pub const FLAG_MIDNIGHT: u8 = 2;

/// The T3 data, read from one CLDR JSON checkout.
#[derive(Debug, PartialEq, Eq)]
pub struct T3 {
    /// The CLDR version (`cldr-core/package.json` and `cldr-dates-full/package.json`).
    pub cldr_version: String,
    /// The cldr-json commit of the checkout.
    pub cldr_commit: String,
    /// The `dayPeriodRuleSet` locales, sorted.
    pub rule_locales: Vec<String>,
    /// Per rule locale: the day period id of each hour 0-23, and the flags.
    pub rules: Vec<([u8; 24], u8)>,
    /// The distinct texts (names and patterns), sorted; text 0 is the empty text ("none").
    pub texts: Vec<String>,
    /// The distinct name records, sorted: the text of each of [`NAMES`].
    pub names: Vec<[u16; 9]>,
    /// The distinct sets, sorted: the name record of each of [`WIDTHS`], then the text of
    /// the pattern of each of [`SKELETONS`].
    pub sets: Vec<[u16; 6]>,
    /// The set of each locale of `locales`, in that order.
    pub locale_sets: Vec<u16>,
    /// The `availableLocales.json` "full" ids, sorted (the order of `CLDR_LOCALES`).
    pub locales: Vec<String>,
    /// (numbering system, decimal sign) where the sign is not ".", sorted.
    pub decimals: Vec<(String, String)>,
}

impl T3 {
    /// The text of index `i` (`None` for the empty text).
    pub fn text(&self, i: u16) -> Option<&str> {
        self.texts
            .get(usize::from(i))
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    /// The set of the CLDR locale `locale`.
    pub fn set(&self, locale: &str) -> Option<&[u16; 6]> {
        let l = self
            .locales
            .binary_search_by(|x| x.as_str().cmp(locale))
            .ok()?;
        self.sets.get(usize::from(self.locale_sets[l]))
    }

    /// The rule of the rule locale `locale`.
    pub fn rule(&self, locale: &str) -> Option<&([u8; 24], u8)> {
        let i = self
            .rule_locales
            .binary_search_by(|x| x.as_str().cmp(locale))
            .ok()?;
        self.rules.get(i)
    }
}

/// The hour of a rule time "HH:00" (0-24).
fn hour(t: &str, file: &Path) -> Result<usize, String> {
    match t.split_once(':') {
        Some((h, "00")) => h
            .parse::<usize>()
            .ok()
            .filter(|h| *h <= 24)
            .ok_or_else(|| format!("{}: bad rule time {t}", file.display())),
        _ => Err(format!(
            "{}: rule time {t} is not on the hour (T3 holds hours)",
            file.display()
        )),
    }
}

/// The rules of `dayPeriods.json`: (locale, hours, flags), sorted by locale.
fn rules(file: &Path) -> Result<Vec<(String, [u8; 24], u8)>, String> {
    let json = read_json(file)?;
    let sets = json
        .pointer("/supplemental/dayPeriodRuleSet")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{}: no dayPeriodRuleSet", file.display()))?;
    let mut out = Vec::new();
    for (locale, set) in sets {
        let set = set
            .as_object()
            .ok_or_else(|| format!("{}: {locale}", file.display()))?;
        let mut hours = [u8::MAX; 24];
        let mut flags = 0;
        for (period, r) in set {
            if let Some(at) = r.get("_at").and_then(Value::as_str) {
                flags |= match (period.as_str(), at) {
                    ("noon", "12:00") => FLAG_NOON,
                    ("midnight", "00:00") => FLAG_MIDNIGHT,
                    _ => {
                        return Err(format!(
                            "{}: {locale} {period} at {at}: not noon or midnight",
                            file.display()
                        ));
                    }
                };
                continue;
            }
            let id = PERIODS
                .iter()
                .position(|p| p == period)
                .ok_or_else(|| format!("{}: {locale}: unknown period {period}", file.display()))?;
            let from = hour(str_at(r, &["_from"], file)?, file)?;
            let before = hour(str_at(r, &["_before"], file)?, file)?;
            // A period that passes midnight ("21:00" before "06:00") wraps.
            let mut h = from % 24;
            loop {
                if hours[h] != u8::MAX {
                    return Err(format!(
                        "{}: {locale}: hour {h} in two periods",
                        file.display()
                    ));
                }
                // id < 10: fits.
                hours[h] = id as u8;
                h = (h + 1) % 24;
                if h == before % 24 {
                    break;
                }
            }
        }
        if hours.contains(&u8::MAX) {
            return Err(format!(
                "{}: {locale}: an hour has no period",
                file.display()
            ));
        }
        out.push((locale.clone(), hours, flags));
    }
    out.sort();
    Ok(out)
}

/// Reads T3 from the checkout `cldr` (the directory of `sync-cldr-json.sh`), which must be at
/// the commit `pin`.
pub fn read(cldr: &Path, pin: &str) -> Result<T3, String> {
    let head = git_head(cldr)?;
    if head != pin {
        return Err(format!(
            "{} is at {head}, not at the pin {pin}: run tools/cldr-tables/sync-cldr-json.sh",
            cldr.display()
        ));
    }
    let root = cldr.join("cldr-json");
    let core_pkg = root.join("cldr-core/package.json");
    let cldr_version = str_at(&read_json(&core_pkg)?, &["version"], &core_pkg)?.to_owned();
    let dates_pkg = root.join("cldr-dates-full/package.json");
    let dates_version = str_at(&read_json(&dates_pkg)?, &["version"], &dates_pkg)?.to_owned();
    if dates_version != cldr_version {
        return Err(format!(
            "cldr-dates-full {dates_version} != cldr-core {cldr_version}"
        ));
    }

    // The locales: the "full" list must equal the directories of cldr-dates-full/main.
    let avail_file = root.join("cldr-core/availableLocales.json");
    let full: BTreeSet<String> = read_json(&avail_file)?
        .pointer("/availableLocales/full")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: no availableLocales.full", avail_file.display()))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let main = root.join("cldr-dates-full/main");
    let mut dirs = BTreeSet::new();
    for entry in std::fs::read_dir(&main).map_err(|e| format!("{}: {e}", main.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", main.display()))?;
        dirs.insert(entry.file_name().to_string_lossy().into_owned());
    }
    if dirs != full {
        return Err(format!(
            "cldr-dates-full/main != availableLocales.full: only dirs {:?}, only list {:?}",
            dirs.difference(&full).collect::<Vec<_>>(),
            full.difference(&dirs).collect::<Vec<_>>()
        ));
    }

    let rules = rules(&root.join("cldr-core/supplemental/dayPeriods.json"))?;

    // Per locale: the 3 name lists and the 3 patterns, as texts.
    let mut raw: Vec<([[String; 9]; 3], [String; 3])> = Vec::new();
    for locale in &dirs {
        let file = main.join(locale).join("ca-gregorian.json");
        let json = read_json(&file)?;
        let greg = json
            .pointer(&format!("/main/{locale}/dates/calendars/gregorian"))
            .ok_or_else(|| format!("{}: no gregorian", file.display()))?;
        let names = WIDTHS.map(|w| {
            NAMES.map(|n| {
                greg.pointer(&format!("/dayPeriods/format/{w}/{n}"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            })
        });
        let mut patterns = [String::new(), String::new(), String::new()];
        for (i, s) in SKELETONS.iter().enumerate() {
            patterns[i] =
                str_at(greg, &["dateTimeFormats", "availableFormats", s], &file)?.to_owned();
        }
        raw.push((names, patterns));
    }
    let mut texts: BTreeSet<String> = BTreeSet::from([String::new()]);
    for (names, patterns) in &raw {
        texts.extend(names.iter().flatten().cloned());
        texts.extend(patterns.iter().cloned());
    }
    let texts: Vec<String> = texts.into_iter().collect();
    if texts.len() > usize::from(u16::MAX) {
        return Err("more than 65535 T3 texts".into());
    }
    // binary_search cannot fail below: each list holds every value it is searched for.
    let t = |s: &String| texts.binary_search(s).unwrap_or_default() as u16;
    let names: Vec<[u16; 9]> = raw
        .iter()
        .flat_map(|(n, _)| n.iter().map(|w| w.each_ref().map(t)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let rec = |w: &[String; 9]| {
        names
            .binary_search(&w.each_ref().map(t))
            .unwrap_or_default() as u16
    };
    let per_locale: Vec<[u16; 6]> = raw
        .iter()
        .map(|(n, p)| {
            [
                rec(&n[0]),
                rec(&n[1]),
                rec(&n[2]),
                t(&p[0]),
                t(&p[1]),
                t(&p[2]),
            ]
        })
        .collect();
    let sets: Vec<[u16; 6]> = per_locale
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let locale_sets = per_locale
        .iter()
        .map(|s| sets.binary_search(s).unwrap_or_default() as u16)
        .collect();
    let (rule_locales, rules) = rules.into_iter().map(|(l, h, f)| (l, (h, f))).unzip();
    let decimals = decimals(&root.join("cldr-numbers-full/main"), &dirs)?;
    Ok(T3 {
        cldr_version,
        cldr_commit: head,
        rule_locales,
        rules,
        texts,
        names,
        sets,
        locale_sets,
        locales: dirs.into_iter().collect(),
        decimals,
    })
}

/// The decimal sign of each numbering system other than "latn": the most frequent
/// `symbols-numberSystem-<nu>` decimal of the locales `dirs` (ties: the first in sort order);
/// only the systems whose sign is not ".".
fn decimals(main: &Path, dirs: &BTreeSet<String>) -> Result<Vec<(String, String)>, String> {
    let mut count: std::collections::BTreeMap<(String, String), usize> = Default::default();
    for locale in dirs {
        let file = main.join(locale).join("numbers.json");
        let json = read_json(&file)?;
        let numbers = json
            .pointer(&format!("/main/{locale}/numbers"))
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{}: no main/{locale}/numbers", file.display()))?;
        for (key, sym) in numbers {
            let Some(nu) = key.strip_prefix("symbols-numberSystem-") else {
                continue;
            };
            if nu == "latn" {
                continue;
            }
            let sign = str_at(sym, &["decimal"], &file)?.to_owned();
            *count.entry((nu.to_owned(), sign)).or_default() += 1;
        }
    }
    let mut best: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for ((nu, sign), n) in count {
        let e = best.entry(nu).or_insert((0, String::new()));
        // The map iterates the signs of one system in sort order: a later sign wins only
        // with a higher count.
        if n > e.0 {
            *e = (n, sign);
        }
    }
    Ok(best
        .into_iter()
        .filter(|(_, (_, sign))| sign != ".")
        .map(|(nu, (_, sign))| (nu, sign))
        .collect())
}

/// The text of `t3.rs`.
pub fn render(t: &T3) -> String {
    let mut u = format!(
        "// @generated by tools/cldr-tables (INTL-V2-D5): do not edit. Make it again with\n\
         //   tools/cldr-tables/sync-cldr-json.sh\n\
         //   cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml -- t3\n\
         // and check it with `-- t3 --check` (README.md there).\n\
         // Source: CLDR {} JSON (unicode-org/cldr-json commit {}), cldr-core/supplemental/dayPeriods.json,\n\
         // cldr-dates-full/main/*/ca-gregorian.json dayPeriods.format and availableFormats yMd, yM, Md.\n\
         // The CLDR version is the CLDR version of the ICU4X compiled data (tools/cldr-tables tests).\n\
         // Data: Copyright (c) Unicode, Inc. SPDX-License-Identifier: Unicode-3.0\n\n",
        t.cldr_version, t.cldr_commit
    );
    let _ = writeln!(
        u,
        "/// The locales of the CLDR day period rules (`dayPeriodRuleSet`), sorted."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_RULE_LOCALES: [&str; {}] = [",
        t.rule_locales.len()
    );
    for l in &t.rule_locales {
        let _ = writeln!(u, "    {},", lit(l));
    }
    u.push_str("];\n\n");
    let _ = writeln!(
        u,
        "/// The rule of each locale of `T3_RULE_LOCALES`: the day period of each hour 0-23 (0 morning1,\n\
         /// 1 morning2, 2 afternoon1, 3 afternoon2, 4 evening1, 5 evening2, 6 night1, 7 night2, 8 am,\n\
         /// 9 pm), and the flags ({} noon at 12:00, {} midnight at 00:00).",
        FLAG_NOON, FLAG_MIDNIGHT
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_RULES: [([u8; 24], u8); {}] = [",
        t.rules.len()
    );
    for (h, f) in &t.rules {
        let h: Vec<String> = h.iter().map(ToString::to_string).collect();
        let _ = writeln!(u, "    ([{}], {f}),", h.join(", "));
    }
    u.push_str("];\n\n");

    let _ = writeln!(
        u,
        "/// The distinct T3 texts (day period names, date patterns), sorted and joined: text `i` is\n\
         /// `T3_TEXT[T3_TEXT_STARTS[i] as usize..T3_TEXT_STARTS[i + 1] as usize]`. Text 0 is the empty\n\
         /// text: \"none\"."
    );
    u.push_str("pub(crate) static T3_TEXT: &str = concat!(\n");
    let mut starts = vec![0usize];
    for text in &t.texts {
        let _ = writeln!(u, "    {},", lit(text));
        starts.push(starts[starts.len() - 1] + text.len());
    }
    u.push_str(");\n\n");
    let _ = writeln!(
        u,
        "/// The byte offset of each text of `T3_TEXT`, and the length of `T3_TEXT` last."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_TEXT_STARTS: [u32; {}] = [",
        starts.len()
    );
    number_lines(
        &mut u,
        &starts.iter().map(ToString::to_string).collect::<Vec<_>>(),
        16,
    );
    u.push_str("];\n\n");

    let _ = writeln!(
        u,
        "/// The distinct name records, sorted: the texts of the names (format context) of morning1,\n\
         /// morning2, afternoon1, afternoon2, evening1, evening2, night1, night2 and noon."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_NAMES: [[u16; 9]; {}] = [",
        t.names.len()
    );
    for r in &t.names {
        let r: Vec<String> = r.iter().map(ToString::to_string).collect();
        let _ = writeln!(u, "    [{}],", r.join(", "));
    }
    u.push_str("];\n\n");

    let _ = writeln!(
        u,
        "/// The distinct T3 sets (the data of one locale), sorted: the name record (an index of\n\
         /// `T3_NAMES`) of the widths abbreviated, narrow and wide, then the texts of the availableFormats\n\
         /// patterns of the skeletons yMd, yM and Md."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_SETS: [[u16; 6]; {}] = [",
        t.sets.len()
    );
    for s in &t.sets {
        let s: Vec<String> = s.iter().map(ToString::to_string).collect();
        let _ = writeln!(u, "    [{}],", s.join(", "));
    }
    u.push_str("];\n\n");

    let _ = writeln!(
        u,
        "/// The set (an index of `T3_SETS`) of each locale of `CLDR_LOCALES`, in that order."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_LOCALE_SETS: [u16; {}] = [",
        t.locale_sets.len()
    );
    number_lines(
        &mut u,
        &t.locale_sets
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        16,
    );
    u.push_str("];\n\n");

    let _ = writeln!(
        u,
        "/// The decimal sign of each numbering system other than \"latn\" whose sign is not \".\" (the sign that\n\
         /// most CLDR locales with symbols for the system have), sorted by system."
    );
    let _ = writeln!(
        u,
        "pub(crate) static T3_DECIMALS: [(&str, &str); {}] = [",
        t.decimals.len()
    );
    for (nu, sign) in &t.decimals {
        let _ = writeln!(u, "    ({}, {}),", lit(nu), lit(sign));
    }
    u.push_str("];\n");
    u
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_paths, read_pin, stale};

    fn t3() -> T3 {
        let (cldr, pin, _) = default_paths();
        let pin = read_pin(&pin).expect("pin");
        read(&cldr, &pin).expect("run tools/cldr-tables/sync-cldr-json.sh first")
    }

    /// The generated file is the output of the generator (the `t3 --check` form), and two
    /// runs give the same text.
    #[test]
    fn generated_file_is_current_and_deterministic() {
        let t = t3();
        let text = render(&t);
        assert_eq!(text, render(&t3()));
        let (_, _, out) = default_paths();
        let files = std::collections::BTreeMap::from([(OUTPUT_FILE, text)]);
        assert!(
            stale(&out, &files).is_empty(),
            "run cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml -- t3"
        );
    }

    /// The locales are those of the T1 and T2 tables (`CLDR_LOCALES`), and every list is
    /// sorted, so the readers can index and search them.
    #[test]
    fn locales_equal_the_tables_and_lists_are_sorted() {
        let t = t3();
        let (cldr, pin, _) = default_paths();
        let tables = crate::read(&cldr, &read_pin(&pin).expect("pin")).expect("tables");
        assert_eq!(t.locales, tables.locales);
        assert_eq!(t.locale_sets.len(), t.locales.len());
        assert!(t.rule_locales.windows(2).all(|w| w[0] < w[1]));
        assert!(t.texts.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(t.texts[0], "");
        assert!(t.names.windows(2).all(|w| w[0] < w[1]));
        assert!(t.sets.windows(2).all(|w| w[0] < w[1]));
        // Each rule maps all 24 hours to a period id.
        for (hours, _) in &t.rules {
            assert!(hours.iter().all(|h| usize::from(*h) < PERIODS.len()));
        }
        // Each locale has the three patterns.
        for s in &t.sets {
            for p in &s[3..] {
                assert!(t.text(*p).is_some());
            }
        }
    }

    /// Values that test262 and ICU4C give (CLDR 49).
    #[test]
    fn t3_values() {
        let t = t3();
        let (en, flags) = t.rule("en").expect("en rule");
        assert_eq!(*flags, FLAG_NOON | FLAG_MIDNIGHT);
        // en: morning1 00:00-12:00, afternoon1 12:00-18:00, evening1 18:00-21:00, night1 21:00-24:00.
        assert_eq!(en[0], 0);
        assert_eq!(en[11], 0);
        assert_eq!(en[12], 2);
        assert_eq!(en[18], 4);
        assert_eq!(en[23], 6);
        // de: night1 00:00-05:00 (a period that does not wrap), evening1 18:00-24:00.
        let (de, flags) = t.rule("de").expect("de rule");
        assert_eq!(*flags, FLAG_MIDNIGHT);
        assert_eq!((de[4], de[5], de[23]), (6, 0, 4));
        // und: am and pm only.
        let (und, _) = t.rule("und").expect("und rule");
        assert_eq!((und[0], und[12]), (8, 9));
        let set = t.set("en").expect("en set");
        let name = |w: usize, n: usize| t.text(t.names[usize::from(set[w])][n]);
        assert_eq!(name(0, 8), Some("noon"));
        assert_eq!(name(1, 8), Some("n"));
        assert_eq!(name(2, 0), Some("in the morning"));
        assert_eq!(name(2, 6), Some("at night"));
        assert_eq!(name(2, 1), None, "en has no morning2");
        assert_eq!(t.text(set[3]), Some("M/d/y"));
        let de = t.set("de").expect("de set");
        assert_eq!(t.text(de[3]), Some("d.M.y"));
        assert_eq!(t.text(de[5]), Some("d.M."));
        let gb = t.set("en-GB").expect("en-GB set");
        assert_eq!(t.text(gb[3]), Some("dd/MM/y"));
        let sign = |nu: &str| {
            t.decimals
                .iter()
                .find(|(n, _)| n == nu)
                .map(|(_, s)| s.as_str())
        };
        assert_eq!(sign("arab"), Some("\u{66b}"));
        assert_eq!(sign("arabext"), Some("\u{66b}"));
        assert_eq!(sign("deva"), None, "deva has \".\"");
    }
}
