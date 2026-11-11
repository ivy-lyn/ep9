//! cldr-tables (INTL-V2-D5): the CLDR side tables of `ep9_intl` that the ICU4X data does not
//! give. It reads the pinned CLDR JSON (`sync-cldr-json.sh`) and writes Rust files into
//! `crates/ep9_intl/src/cldr/gen/`.
//!
//! ```text
//! cldr-json (cldr-json.commit)                      crates/ep9_intl/src/cldr/gen/
//!   cldr-core/package.json  version  ─────────────►  version.rs  CLDR_VERSION, CLDR_JSON_COMMIT
//!   cldr-core/supplemental/numberingSystems.json
//!     the "numeric" systems (78) ─────────────────►  digits.rs   NUMBERING_SYSTEMS (name, digits)
//!   cldr-numbers-full/main/<locale>/numbers.json
//!     symbols-numberSystem-<nu>   nan, infinity,
//!       exponential, approximatelySign
//!     miscPatterns-numberSystem-<nu>  range ──────►  t1.rs       T1_SYMBOLS (distinct records)
//!                                                                T1_ROWS (locale, nu, record)
//! ```
//!
//! The CLDR JSON locale files are resolved (a child holds every value, not only the values that
//! differ from its parent), so a row is the full answer for its locale. Output is deterministic:
//! every list is sorted, and the text depends only on the inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The files the generator writes, relative to the output directory.
pub const OUTPUT_FILES: [&str; 3] = ["digits.rs", "t1.rs", "version.rs"];

/// The T1 values of one locale and numbering system (CLDR `symbols-numberSystem-<nu>` and
/// `miscPatterns-numberSystem-<nu>`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NumberSymbols {
    /// `nan`: the string of NaN.
    pub nan: String,
    /// `infinity`: the string of an infinite value.
    pub infinity: String,
    /// `exponential`: the exponent separator of scientific notation.
    pub exponential: String,
    /// `approximatelySign`: the sign of an approximate value.
    pub approximately_sign: String,
    /// `range` (misc pattern): the range pattern, `{0}` and `{1}` are the two ends.
    pub range: String,
}

/// The tables, read from one CLDR JSON checkout.
#[derive(Debug, PartialEq, Eq)]
pub struct Tables {
    /// The CLDR version (`cldr-core/package.json` and `cldr-numbers-full/package.json`).
    pub cldr_version: String,
    /// The cldr-json commit of the checkout.
    pub cldr_commit: String,
    /// The numbering systems of type "numeric", sorted by name, with their 10 digits.
    pub numbering_systems: Vec<(String, [char; 10])>,
    /// The distinct T1 records, sorted.
    pub t1_symbols: Vec<NumberSymbols>,
    /// (locale, index into `numbering_systems`, index into `t1_symbols`), sorted by locale and
    /// then by numbering system index.
    pub t1_rows: Vec<(String, u8, u8)>,
    /// The default numbering system of each locale (not written; the tests use it).
    pub default_numbering_system: BTreeMap<String, String>,
}

impl Tables {
    /// The T1 record of `locale` and numbering system `nu`, if CLDR has one.
    pub fn symbols(&self, locale: &str, nu: &str) -> Option<&NumberSymbols> {
        let n = self
            .numbering_systems
            .iter()
            .position(|(name, _)| name == nu)?;
        let i = self
            .t1_rows
            .binary_search_by(|(l, x, _)| (l.as_str(), *x as usize).cmp(&(locale, n)))
            .ok()?;
        Some(&self.t1_symbols[self.t1_rows[i].2 as usize])
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn str_at<'a>(v: &'a Value, path: &[&str], file: &Path) -> Result<&'a str, String> {
    let mut cur = v;
    for key in path {
        cur = cur
            .get(key)
            .ok_or_else(|| format!("{}: no {path:?}", file.display()))?;
    }
    cur.as_str()
        .ok_or_else(|| format!("{}: {path:?} is not a string", file.display()))
}

/// The commit that `git rev-parse HEAD` gives in `dir`.
fn git_head(dir: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD^{commit}"])
        .output()
        .map_err(|e| format!("git rev-parse in {}: {e}", dir.display()))?;
    if !out.status.success() {
        return Err(format!("git rev-parse in {} failed", dir.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Reads the tables from the checkout `cldr` (the directory that `sync-cldr-json.sh` makes).
/// `pin` is the commit of `cldr-json.commit`: the checkout must be at it.
pub fn read(cldr: &Path, pin: &str) -> Result<Tables, String> {
    let head = git_head(cldr)?;
    if head != pin {
        return Err(format!(
            "{} is at {head}, not at the pin {pin}: run tools/cldr-tables/sync-cldr-json.sh",
            cldr.display()
        ));
    }
    let root = cldr.join("cldr-json");

    // The CLDR version: the two packages must agree.
    let core_pkg = root.join("cldr-core/package.json");
    let cldr_version = str_at(&read_json(&core_pkg)?, &["version"], &core_pkg)?.to_owned();
    let num_pkg = root.join("cldr-numbers-full/package.json");
    let num_json = read_json(&num_pkg)?;
    let num_version = str_at(&num_json, &["version"], &num_pkg)?;
    if num_version != cldr_version {
        return Err(format!(
            "cldr-numbers-full {num_version} != cldr-core {cldr_version}"
        ));
    }

    // The digit table: every numbering system of type "numeric".
    let ns_file = root.join("cldr-core/supplemental/numberingSystems.json");
    let ns_json = read_json(&ns_file)?;
    let ns_map = ns_json
        .pointer("/supplemental/numberingSystems")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{}: no numberingSystems", ns_file.display()))?;
    let mut numbering_systems = Vec::new();
    for (name, v) in ns_map {
        if v.get("_type").and_then(Value::as_str) != Some("numeric") {
            continue;
        }
        let digits = str_at(v, &["_digits"], &ns_file)?;
        let chars: Vec<char> = digits.chars().collect();
        let arr: [char; 10] = chars
            .try_into()
            .map_err(|_| format!("{}: {name} has not 10 digits", ns_file.display()))?;
        numbering_systems.push((name.clone(), arr));
    }
    numbering_systems.sort();
    if numbering_systems.len() > usize::from(u8::MAX) {
        return Err("more than 255 numbering systems: widen the T1 row index".into());
    }

    // The locales: the "full" list must equal the directories of cldr-numbers-full/main.
    let avail_file = root.join("cldr-core/availableLocales.json");
    let avail_json = read_json(&avail_file)?;
    let full: BTreeSet<String> = avail_json
        .pointer("/availableLocales/full")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: no availableLocales.full", avail_file.display()))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let main = root.join("cldr-numbers-full/main");
    let mut dirs = BTreeSet::new();
    for entry in std::fs::read_dir(&main).map_err(|e| format!("{}: {e}", main.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", main.display()))?;
        dirs.insert(entry.file_name().to_string_lossy().into_owned());
    }
    if dirs != full {
        let only_dirs: Vec<_> = dirs.difference(&full).collect();
        let only_full: Vec<_> = full.difference(&dirs).collect();
        return Err(format!(
            "cldr-numbers-full/main != availableLocales.full: only dirs {only_dirs:?}, only list {only_full:?}"
        ));
    }

    // T1: one row per locale and numbering system with symbols.
    let mut raw: Vec<(String, u8, NumberSymbols)> = Vec::new();
    let mut default_numbering_system = BTreeMap::new();
    for locale in &dirs {
        let file = main.join(locale).join("numbers.json");
        let json = read_json(&file)?;
        let numbers = json
            .pointer(&format!("/main/{locale}/numbers"))
            .ok_or_else(|| format!("{}: no main/{locale}/numbers", file.display()))?;
        default_numbering_system.insert(
            locale.clone(),
            str_at(numbers, &["defaultNumberingSystem"], &file)?.to_owned(),
        );
        let obj = numbers
            .as_object()
            .ok_or_else(|| format!("{}: numbers", file.display()))?;
        for key in obj.keys() {
            let Some(nu) = key.strip_prefix("symbols-numberSystem-") else {
                continue;
            };
            let n = numbering_systems
                .iter()
                .position(|(name, _)| name == nu)
                .ok_or_else(|| {
                    format!("{}: {nu} is not a numeric numbering system", file.display())
                })?;
            let sym = &obj[key];
            let misc_key = format!("miscPatterns-numberSystem-{nu}");
            let misc = obj
                .get(&misc_key)
                .ok_or_else(|| format!("{}: no {misc_key}", file.display()))?;
            let record = NumberSymbols {
                nan: str_at(sym, &["nan"], &file)?.to_owned(),
                infinity: str_at(sym, &["infinity"], &file)?.to_owned(),
                exponential: str_at(sym, &["exponential"], &file)?.to_owned(),
                approximately_sign: str_at(sym, &["approximatelySign"], &file)?.to_owned(),
                range: str_at(misc, &["range"], &file)?.to_owned(),
            };
            // u8 fits: checked above.
            raw.push((locale.clone(), n as u8, record));
        }
    }
    let t1_symbols: Vec<NumberSymbols> = raw
        .iter()
        .map(|(_, _, r)| r.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if t1_symbols.len() > usize::from(u8::MAX) + 1 {
        return Err("more than 256 distinct T1 records: widen the T1 row index".into());
    }
    let mut t1_rows: Vec<(String, u8, u8)> = raw
        .into_iter()
        .map(|(l, n, r)| {
            // binary_search cannot fail: t1_symbols holds every record.
            let i = t1_symbols.binary_search(&r).unwrap_or_default();
            (l, n, i as u8)
        })
        .collect();
    t1_rows.sort();

    Ok(Tables {
        cldr_version,
        cldr_commit: head,
        numbering_systems,
        t1_symbols,
        t1_rows,
        default_numbering_system,
    })
}

/// A Rust string literal: printable ASCII as is, every other char as `\u{..}`.
fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
        }
    }
    out.push('"');
    out
}

/// A Rust char literal in the form of [`lit`].
fn char_lit(c: char) -> String {
    match c {
        '\'' => "'\\''".into(),
        '\\' => "'\\\\'".into(),
        ' '..='~' => format!("'{c}'"),
        _ => format!("'\\u{{{:x}}}'", u32::from(c)),
    }
}

fn header(t: &Tables, what: &str) -> String {
    format!(
        "// @generated by tools/cldr-tables (INTL-V2-D5): do not edit. Make it again with\n\
         //   tools/cldr-tables/sync-cldr-json.sh\n\
         //   cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml\n\
         // and check it with `-- --check` (README.md there).\n\
         // Source: CLDR {} JSON (unicode-org/cldr-json commit {}), {what}.\n\
         // The CLDR version is the CLDR version of the ICU4X compiled data (tools/cldr-tables tests).\n\
         // Data: Copyright (c) Unicode, Inc. SPDX-License-Identifier: Unicode-3.0\n\n",
        t.cldr_version, t.cldr_commit
    )
}

/// The generated files: name (one of [`OUTPUT_FILES`]) -> text.
pub fn render(t: &Tables) -> BTreeMap<&'static str, String> {
    let mut files = BTreeMap::new();

    let mut v = header(t, "cldr-core/package.json");
    let _ = writeln!(v, "/// The CLDR release of the tables.");
    let _ = writeln!(
        v,
        "pub(crate) const CLDR_VERSION: &str = {};",
        lit(&t.cldr_version)
    );
    let _ = writeln!(v, "/// The unicode-org/cldr-json commit of the tables.");
    let _ = writeln!(
        v,
        "pub(crate) const CLDR_JSON_COMMIT: &str = {};",
        lit(&t.cldr_commit)
    );
    files.insert("version.rs", v);

    let mut d = header(
        t,
        "cldr-core/supplemental/numberingSystems.json, the \"numeric\" systems",
    );
    let _ = writeln!(
        d,
        "/// The numbering systems with simple digit mappings (ECMA-402 table \
         \"Numbering systems with simple\n\
         /// digit mappings\"): name and the digits 0 to 9, sorted by name."
    );
    let _ = writeln!(
        d,
        "pub(crate) static NUMBERING_SYSTEMS: [(&str, [char; 10]); {}] = [",
        t.numbering_systems.len()
    );
    for (name, digits) in &t.numbering_systems {
        let ds: Vec<String> = digits.iter().map(|c| char_lit(*c)).collect();
        let _ = writeln!(d, "    ({}, [{}]),", lit(name), ds.join(", "));
    }
    d.push_str("];\n");
    files.insert("digits.rs", d);

    let mut s = header(
        t,
        "cldr-numbers-full/main/*/numbers.json, symbols-numberSystem-* and miscPatterns-numberSystem-*",
    );
    let _ = writeln!(s, "/// The distinct T1 records, sorted.");
    let _ = writeln!(
        s,
        "pub(crate) static T1_SYMBOLS: [NumberSymbols; {}] = [",
        t.t1_symbols.len()
    );
    for r in &t.t1_symbols {
        let _ = writeln!(
            s,
            "    NumberSymbols {{ nan: {}, infinity: {}, exponential: {}, approximately_sign: {}, range: {} }},",
            lit(&r.nan),
            lit(&r.infinity),
            lit(&r.exponential),
            lit(&r.approximately_sign),
            lit(&r.range)
        );
    }
    s.push_str("];\n\n");
    let _ = writeln!(
        s,
        "/// (CLDR locale id, index into `NUMBERING_SYSTEMS`, index into `T1_SYMBOLS`), sorted by locale id\n\
         /// and then by numbering system index. One row per locale and numbering system with symbols."
    );
    let _ = writeln!(
        s,
        "pub(crate) static T1_ROWS: [(&str, u8, u8); {}] = [",
        t.t1_rows.len()
    );
    for (l, n, i) in &t.t1_rows {
        let _ = writeln!(s, "    ({}, {n}, {i}),", lit(l));
    }
    s.push_str("];\n");
    files.insert("t1.rs", s);

    files
}

/// The default paths: the checkout and the pin file next to this crate, the output in
/// `crates/ep9_intl/src/cldr/gen` of the repository.
pub fn default_paths() -> (PathBuf, PathBuf, PathBuf) {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    (
        here.join("cldr-json"),
        here.join("cldr-json.commit"),
        here.join("../../crates/ep9_intl/src/cldr/gen"),
    )
}

/// The commit of the pin file.
pub fn read_pin(pin_file: &Path) -> Result<String, String> {
    let text =
        std::fs::read_to_string(pin_file).map_err(|e| format!("{}: {e}", pin_file.display()))?;
    Ok(text.trim().to_owned())
}

/// The files of `out` that differ from `files` (or are missing).
pub fn stale(out: &Path, files: &BTreeMap<&'static str, String>) -> Vec<String> {
    files
        .iter()
        .filter(|(name, text)| {
            std::fs::read(out.join(name)).ok().as_deref() != Some(text.as_bytes())
        })
        .map(|(name, _)| (*name).to_owned())
        .collect()
}
