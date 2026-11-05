//! `cargo xtask fp-check` (H36, docs/plan/08-repo.md section 7, docs/plan/10-quality.md section
//! 5): what ships computes no JS-visible number with the platform libm and no FMA outside
//! `ep9_math`.
//!
//! | Check | Rule | How |
//! |---|---|---|
//! | banned symbols | No undefined reference to a libm-class symbol (`BANNED_BASE` and `BANNED_EXTRA`: transcendental `math.h` names with their `f`/`l` forms, `fma`, `fmod`, `remainder`, `remquo`, the compiler-rt `__powi*` helpers, Apple/MSVC/glibc aliases) unless `expectations/fp-check-allow.toml` lists that symbol in that function | `llvm-nm -u` on the object; `llvm-objdump -dr -C` names the function of each relocation to the symbol |
//! | FMA instructions | FMA mnemonics (aarch64 `fmadd fmsub fnmadd fnmsub fmla* fmls* fnmla fnmls`, SVE `fmad fmsb fnmad fnmsb`, `bfmla bfmls`; x86 `vfmadd* vfmsub* vfnmadd* vfnmsub*`) only in `ep9_math::` functions | the same disassembly; on x86-64 Mach-O without the jump-table bytes (LC_DATA_IN_CODE), and decoded again from each table end, as in `disasm-check` |
//! | second math crate | `libm`, `pxfm`, `core_maths` and `micromath` are not in the package's normal or build graph, on any target, unless an `[[allow-crate]]` entry lists the crate and every path from the package to it goes through one of the entry's `via` crates | `cargo tree -e normal,build --target all`; for an allowed crate, the inverted tree `-i <crate> --no-dedupe --prefix depth` |
//! | allow sites | `clippy::disallowed_methods` is named (allow, expect, cfg_attr) only in `crates/ep9_math/src/exact.rs`; `tools/*` may name it | text scan of `crates/**/*.rs`; syn names the enclosing item |
//!
//! The object is what ships: `cargo rustc --release --target T -p ep9_cli --bin ep9 --
//! --emit=obj=F -C linker=true`. The release profile has fat LTO and one codegen unit, so F is
//! the post-LTO object of the whole program: it has the same imports as the linked binary, and
//! dead code (for example the libm calls in non-LTO num_* rlibs) is gone. `-C linker=true` skips
//! the real link, so one Linux or macOS host checks every target without target linkers. Default
//! builds: the 5 `disasm-check` targets plus x86_64-unknown-linux-gnu with
//! `-Ctarget-cpu=x86-64-v3` for every crate (where `mul_add` becomes an inline `vfmadd`).
//!
//! The allow file is the subset of TOML below; fp-check reads it by hand (the lock has no TOML
//! crate). More than 5 entries (`[[allow]]` and `[[allow-crate]]` together) reopens H36 and
//! fails the check.
//!
//! ```toml
//! [[allow]]
//! symbols = ["fmod", "pow"]                 # banned symbols, as `nm` prints them on ELF
//! function = "crate::path::function"        # demangled path; generic and closure suffixes match
//! targets = "all"                           # or ["x86_64-pc-windows-msvc", ...]
//! reopen = "when the entry can go"
//! reason = "why it is harmless"             # optional
//!
//! [[allow-crate]]                           # H36 Rules row 4 "unless allowlisted"
//! crates = ["core_maths", "libm"]           # math crates (MATH_CRATES) this entry lets in
//! via = ["temporal_rs"]                     # every path to each crate must pass one of these
//! reopen = "when the entry can go"
//! reason = "why it is harmless"             # optional
//! ```
//!
//! An `[[allow-crate]]` entry only lets the crate into the graph. The symbol and FMA scans of
//! the object still run: code of the crate that reaches the object passes only if it has no
//! banned symbol and no FMA instruction (for `libm`, `force-soft-floats` keeps `fma` in soft
//! code).
//!
//! Exit codes: 0 pass, 1 a finding that the allow file does not cover (each finding names its
//! function), 2 usage error, tool error (a missing llvm tool or rustup target) or build error.
//! Fixture tests: `tools/xtask/tests/fpcheck.rs` (`--ignored`; they need llvm-tools and the
//! rustup targets).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::disasm::{
    ALL_TARGETS, Block, drop_data, find_tool, parse_data_in_code, parse_disasm,
    redecode_after_data, repo_root, run_capture,
};

const USAGE: &str = "\
usage: cargo xtask fp-check [options]
  --root DIR            workspace to check (default: this repo)
  -p, --package NAME    package that builds the shipped binary (default: ep9_cli)
  --bin NAME            its binary (default: ep9)
  --target SPEC         TRIPLE or TRIPLE@CPU (CPU: -Ctarget-cpu for every crate); repeat for
                        more. Default: the 5 disasm-check targets and
                        x86_64-unknown-linux-gnu@x86-64-v3
  --allow FILE          allow list (default: ROOT/expectations/fp-check-allow.toml; a missing
                        default file is an empty list)
  --target-dir DIR      cargo target dir (default: $CARGO_TARGET_DIR or ROOT/target, plus
                        /xtask-fpcheck)
  --objdump PATH, --nm PATH   tools (default: rustup llvm-tools, then PATH)
  -v, --verbose         also list each FMA site inside ep9_math";

/// Banned libm-class names (H36 Rules row 2). Each also bans its `f` and `l` forms, the MSVC
/// `_CI` intrinsic form and the old glibc `__<name>_finite` form.
const BANNED_BASE: &[&str] = &[
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "cbrt",
    "cos",
    "cosh",
    "cospi",
    "erf",
    "erfc",
    "exp",
    "exp10",
    "exp2",
    "expm1",
    "fma",
    "fmod",
    "hypot",
    "j0",
    "j1",
    "jn",
    "lgamma",
    "lgamma_r",
    "log",
    "log10",
    "log1p",
    "log2",
    "pow",
    "pown",
    "powr",
    "remainder",
    "remquo",
    "sin",
    "sincos",
    "sinh",
    "sinpi",
    "tan",
    "tanh",
    "tanpi",
    "tgamma",
    "y0",
    "y1",
    "yn",
];

/// Banned names outside the BANNED_BASE pattern: compiler-rt `powi` helpers (`f64::powi`), Apple
/// and MSVC aliases.
const BANNED_EXTRA: &[&str] = &[
    "__powidf2",
    "__powisf2",
    "__powixf2",
    "__powitf2",
    "__powikf2",
    "__sincos_stret",
    "__sincosf_stret",
    "__sincospi_stret",
    "__sincospif_stret",
    "__exp10",
    "__exp10f",
    "__sinpi",
    "__sinpif",
    "__cospi",
    "__cospif",
    "__tanpi",
    "__tanpif",
    "_hypot",
    "_hypotf",
];

/// Second math crates (H36 Rules row 4).
const MATH_CRATES: &[&str] = &["libm", "pxfm", "core_maths", "micromath"];

/// The one engine file that may name `clippy::disallowed_methods` (H36 Rules row 1).
const ALLOW_SITE_FILE: &str = "crates/ep9_math/src/exact.rs";

/// H36 "Reopens if": `fp-check-allow.toml` grows past 5 entries.
const MAX_ALLOW_ENTRIES: usize = 5;

/// Extra build for x86-64 with FMA in every crate (`mul_add` becomes `vfmadd`).
const V3_SPEC: &str = "x86_64-unknown-linux-gnu@x86-64-v3";

struct Opts {
    root: PathBuf,
    package: String,
    bin: String,
    specs: Vec<Spec>,
    allow: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    objdump: Option<PathBuf>,
    nm: Option<PathBuf>,
    verbose: bool,
}

/// One build: a target triple, optionally with `-Ctarget-cpu` for every crate.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Spec {
    target: String,
    cpu: Option<String>,
}

impl Spec {
    fn parse(s: &str) -> Result<Spec, String> {
        let (target, cpu) = match s.split_once('@') {
            Some((t, c)) => (t, Some(c.to_string())),
            None => (s, None),
        };
        if target.is_empty() || cpu.as_deref() == Some("") {
            return Err(format!("--target {s:?}: expected TRIPLE or TRIPLE@CPU"));
        }
        Ok(Spec {
            target: target.to_string(),
            cpu,
        })
    }

    fn label(&self) -> String {
        match &self.cpu {
            Some(c) => format!("{}@{c}", self.target),
            None => self.target.clone(),
        }
    }
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        root: repo_root(),
        package: "ep9_cli".into(),
        bin: "ep9".into(),
        specs: Vec::new(),
        allow: None,
        target_dir: None,
        objdump: None,
        nm: None,
        verbose: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--root" => o.root = PathBuf::from(val()?),
            "-p" | "--package" => o.package = val()?,
            "--bin" => o.bin = val()?,
            "--target" => o.specs.push(Spec::parse(&val()?)?),
            "--allow" => o.allow = Some(PathBuf::from(val()?)),
            "--target-dir" => o.target_dir = Some(PathBuf::from(val()?)),
            "--objdump" => o.objdump = Some(PathBuf::from(val()?)),
            "--nm" => o.nm = Some(PathBuf::from(val()?)),
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    if o.specs.is_empty() {
        o.specs = ALL_TARGETS
            .iter()
            .map(|t| Spec::parse(t))
            .chain([Spec::parse(V3_SPEC)])
            .collect::<Result<_, _>>()?;
    }
    o.root = o
        .root
        .canonicalize()
        .map_err(|e| format!("--root {}: {e}", o.root.display()))?;
    Ok(o)
}

/// A tool path that answers `--version`; a missing tool is an error (exit 2).
fn checked_tool(name: &str, over: &Option<PathBuf>) -> Result<PathBuf, String> {
    let p = find_tool(name, over)?;
    run_capture(Command::new(&p).arg("--version"))
        .map_err(|e| format!("{name} does not run ({}): {e}", p.display()))?;
    Ok(p)
}

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let objdump = checked_tool("llvm-objdump", &o.objdump)?;
    let nm = checked_tool("llvm-nm", &o.nm)?;

    let (allow_path, required) = match &o.allow {
        Some(p) => (p.clone(), true),
        None => (o.root.join("expectations/fp-check-allow.toml"), false),
    };
    let file = if allow_path.exists() || required {
        let text = fs::read_to_string(&allow_path)
            .map_err(|e| format!("{}: {e}", allow_path.display()))?;
        parse_allow_file(&text).map_err(|e| format!("{}: {e}", allow_path.display()))?
    } else {
        AllowFile::default()
    };
    let total = file.len();
    let allow = &file.symbols;
    println!(
        "fp-check: allow list {}: {} entr{}",
        allow_path.display(),
        total,
        if total == 1 { "y" } else { "ies" }
    );
    for (i, a) in file.crates.iter().enumerate() {
        println!(
            "fp-check: allow-crate entry {}: {} via {} (reopen: {}; reason: {})",
            i + 1,
            a.crates.join(", "),
            a.via.join(", "),
            a.reopen,
            a.reason.as_deref().unwrap_or("-")
        );
    }
    for (i, a) in allow.iter().enumerate() {
        println!(
            "fp-check: allow entry {}: {} in {} on {} (reopen: {}; reason: {})",
            i + 1,
            a.symbols.join(", "),
            a.function,
            a.targets
                .as_ref()
                .map_or("all targets".into(), |t| t.join(", ")),
            a.reopen,
            a.reason.as_deref().unwrap_or("-")
        );
    }

    let mut results: Vec<(String, bool)> = Vec::new();
    let mut used = vec![0usize; allow.len()];

    // 0. The allow list itself.
    let ok = total <= MAX_ALLOW_ENTRIES;
    if !ok {
        println!(
            "fp-check FAIL allow list: {total} entries; more than {MAX_ALLOW_ENTRIES} reopens H36"
        );
    }
    results.push(("allow list size".into(), ok));

    // 1. Allow sites in engine source.
    let sites = scan_allow_sites(&o.root)?;
    for s in &sites {
        println!(
            "fp-check FAIL source: {}:{}: names clippy::disallowed_methods in {} (only {ALLOW_SITE_FILE} may)",
            s.file, s.line, s.item
        );
    }
    println!(
        "fp-check source: {} site{} outside {ALLOW_SITE_FILE}",
        sites.len(),
        if sites.len() == 1 { "" } else { "s" }
    );
    results.push(("source (allow sites)".into(), sites.is_empty()));

    // 2. Second math crate.
    let found = math_crates(&o)?;
    let mut graph_ok = true;
    for c in &found {
        let Some((i, entry)) = file
            .crates
            .iter()
            .enumerate()
            .find(|(_, a)| a.crates.iter().any(|x| x == c))
        else {
            graph_ok = false;
            println!(
                "fp-check FAIL graph: {} depends on the math crate {c} (H36: only ep9_math)",
                o.package
            );
            continue;
        };
        let tree = inverted_tree(&o, c)?;
        let bad = paths_not_via(&tree, &entry.via);
        if bad.is_empty() {
            println!(
                "fp-check graph: {c}: allowed by allow-crate entry {} (every path goes through {}; reopen: {})",
                i + 1,
                entry.via.join(" or "),
                entry.reopen
            );
        } else {
            graph_ok = false;
            for p in &bad {
                println!(
                    "fp-check FAIL graph: {c} is reached by {} not through {} (allow-crate entry {})",
                    p.join(" <- "),
                    entry.via.join(" or "),
                    i + 1
                );
            }
        }
    }
    println!(
        "fp-check graph: {} has {} of {}",
        o.package,
        if found.is_empty() {
            "none".to_string()
        } else {
            found.join(", ")
        },
        MATH_CRATES.join(", ")
    );
    results.push(("graph (second math crate)".into(), graph_ok));

    // 3. Each build.
    for spec in &o.specs {
        let obj = build_object(&o, spec)?;
        let scan = scan_object(&objdump, &nm, &obj)?;
        let label = spec.label();
        println!(
            "fp-check {label}: object {} ({})",
            obj.display(),
            scan.format
        );
        let mut ok = true;
        for (sym, func) in &scan.banned {
            match allow.iter().position(|a| a.covers(sym, func, &spec.target)) {
                Some(i) => {
                    used[i] += 1;
                    println!(
                        "fp-check {label}: {sym} in {func}: allowed by entry {} (reopen: {})",
                        i + 1,
                        allow[i].reopen
                    );
                }
                None => {
                    ok = false;
                    println!(
                        "fp-check FAIL {label}: {sym} in {func}: banned libm symbol (H36), not in the allow list"
                    );
                }
            }
        }
        for (func, insn) in &scan.fma_outside {
            ok = false;
            println!(
                "fp-check FAIL {label}: FMA instruction `{insn}` in {func}: only ep9_math:: functions may use FMA (H36)"
            );
        }
        if o.verbose {
            for (func, insn) in &scan.fma_inside {
                println!("fp-check {label}: FMA `{insn}` in {func} (ep9_math: allowed)");
            }
        }
        println!(
            "fp-check {label}: {} banned symbol reference{}, FMA instructions: {} outside ep9_math, {} inside",
            scan.banned.len(),
            if scan.banned.len() == 1 { "" } else { "s" },
            scan.fma_outside.len(),
            scan.fma_inside.len()
        );
        results.push((label, ok));
    }

    for (i, n) in used.iter().enumerate() {
        if *n == 0 {
            println!(
                "fp-check note: allow entry {} ({} in {}) matched nothing; remove it if it is stale",
                i + 1,
                allow[i].symbols.join(", "),
                allow[i].function
            );
        }
    }
    println!();
    for (what, ok) in &results {
        println!("fp-check {}: {what}", if *ok { "PASS" } else { "FAIL" });
    }
    Ok(results.iter().all(|(_, ok)| *ok))
}

// ---------------------------------------------------------------------------------------------
// Allow list

/// The parsed allow file.
#[derive(Debug, Default, PartialEq, Eq)]
struct AllowFile {
    /// `[[allow]]`: banned symbols in one function.
    symbols: Vec<AllowEntry>,
    /// `[[allow-crate]]`: math crates in the graph, reached only through `via` crates.
    crates: Vec<AllowCrate>,
}

impl AllowFile {
    fn len(&self) -> usize {
        self.symbols.len() + self.crates.len()
    }
}

/// An `[[allow-crate]]` entry (H36 Rules row 4, "unless allowlisted").
#[derive(Debug, PartialEq, Eq)]
struct AllowCrate {
    /// Math crates ([`MATH_CRATES`]) that may be in the graph.
    crates: Vec<String>,
    /// Every path from the package to each crate must go through one of these packages.
    via: Vec<String>,
    reopen: String,
    reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct AllowEntry {
    symbols: Vec<String>,
    function: String,
    /// `None`: all targets.
    targets: Option<Vec<String>>,
    reopen: String,
    reason: Option<String>,
}

impl AllowEntry {
    fn covers(&self, symbol: &str, function: &str, target: &str) -> bool {
        self.symbols.iter().any(|s| s == symbol)
            && function_matches(&self.function, function)
            && self
                .targets
                .as_ref()
                .is_none_or(|ts| ts.iter().any(|t| t == target))
    }
}

/// `name` is the function `path`, or an instance of it (`path::<T>`), or a closure or item in
/// it (`path::{closure#0}`).
fn function_matches(path: &str, name: &str) -> bool {
    name == path
        || name
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with("::<") || rest.starts_with("::{"))
}

/// One TOML value of the subset: a string or a one-line array of strings.
#[derive(Debug, PartialEq, Eq)]
enum TomlValue {
    Str(String),
    List(Vec<String>),
}

/// Reads a basic TOML string at the start of `s`; returns it and the rest.
fn toml_string(s: &str) -> Result<(String, &str), String> {
    let mut chars = s.strip_prefix('"').ok_or("expected '\"'")?.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((out, &s[i + 2..])),
            '\\' => match chars.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                other => return Err(format!("unsupported escape {other:?}")),
            },
            c => out.push(c),
        }
    }
    Err("unterminated string".into())
}

/// Strips a `#` comment and whitespace from what follows a value.
fn toml_rest_ok(rest: &str) -> Result<(), String> {
    let r = rest.trim_start();
    if r.is_empty() || r.starts_with('#') {
        Ok(())
    } else {
        Err(format!("unexpected text after the value: {r:?}"))
    }
}

fn toml_value(s: &str) -> Result<TomlValue, String> {
    let s = s.trim_start();
    if s.starts_with('"') {
        let (v, rest) = toml_string(s)?;
        toml_rest_ok(rest)?;
        return Ok(TomlValue::Str(v));
    }
    let mut rest = s
        .strip_prefix('[')
        .ok_or("expected a string or a one-line array of strings")?
        .trim_start();
    let mut items = Vec::new();
    loop {
        if let Some(r) = rest.strip_prefix(']') {
            toml_rest_ok(r)?;
            return Ok(TomlValue::List(items));
        }
        let (v, r) = toml_string(rest)?;
        items.push(v);
        rest = r.trim_start();
        if let Some(r) = rest.strip_prefix(',') {
            rest = r.trim_start();
        } else if !rest.starts_with(']') {
            return Err("expected ',' or ']' in the array".into());
        }
    }
}

/// The `[[allow]]` entries of the allow file (tests).
#[cfg(test)]
fn parse_allow(text: &str) -> Result<Vec<AllowEntry>, String> {
    parse_allow_file(text).map(|f| f.symbols)
}

/// Parses the allow file: `[[allow]]` tables with the keys of [`AllowEntry`] and
/// `[[allow-crate]]` tables with the keys of [`AllowCrate`].
fn parse_allow_file(text: &str) -> Result<AllowFile, String> {
    let mut tables: Vec<(bool, BTreeMap<String, TomlValue>)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = |e: String| format!("line {}: {e}", n + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let head = line.split('#').next().unwrap_or("").trim();
            let is_crate = match head {
                "[[allow]]" => false,
                "[[allow-crate]]" => true,
                _ => {
                    return Err(at(format!(
                        "only [[allow]] and [[allow-crate]] tables are allowed, found {head}"
                    )));
                }
            };
            tables.push((is_crate, BTreeMap::new()));
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| at("expected key = value".into()))?;
        let k = k.trim().to_string();
        let (_, t) = tables
            .last_mut()
            .ok_or_else(|| at("key outside an [[allow]] table".into()))?;
        let v = toml_value(v).map_err(at)?;
        if t.insert(k.clone(), v).is_some() {
            return Err(at(format!("duplicate key {k}")));
        }
    }
    let mut file = AllowFile::default();
    for (i, (is_crate, t)) in tables.into_iter().enumerate() {
        if is_crate {
            file.crates.push(parse_allow_crate(i, t)?);
        } else {
            file.symbols.push(parse_allow_entry(i, t)?);
        }
    }
    Ok(file)
}

/// One `[[allow-crate]]` table (the `i`-th table of the file).
fn parse_allow_crate(i: usize, mut t: BTreeMap<String, TomlValue>) -> Result<AllowCrate, String> {
    let at = |e: &str| format!("[[allow-crate]] entry {}: {e}", i + 1);
    let crates = match t.remove("crates") {
        Some(TomlValue::List(v)) if !v.is_empty() => v,
        _ => return Err(at("crates must be a non-empty array of strings")),
    };
    let via = match t.remove("via") {
        Some(TomlValue::List(v)) if !v.is_empty() && v.iter().all(|s| !s.is_empty()) => v,
        _ => return Err(at("via must be a non-empty array of package names")),
    };
    let reopen = match t.remove("reopen") {
        Some(TomlValue::Str(s)) if !s.is_empty() => s,
        _ => return Err(at("reopen must be a non-empty string")),
    };
    let reason = match t.remove("reason") {
        None => None,
        Some(TomlValue::Str(s)) => Some(s),
        Some(_) => return Err(at("reason must be a string")),
    };
    if let Some(k) = t.keys().next() {
        return Err(at(&format!("unknown key {k}")));
    }
    for c in &crates {
        if !MATH_CRATES.contains(&c.as_str()) {
            return Err(at(&format!(
                "{c} is not a math crate ({})",
                MATH_CRATES.join(", ")
            )));
        }
    }
    Ok(AllowCrate {
        crates,
        via,
        reopen,
        reason,
    })
}

/// One `[[allow]]` table (the `i`-th table of the file).
fn parse_allow_entry(i: usize, mut t: BTreeMap<String, TomlValue>) -> Result<AllowEntry, String> {
    let at = |e: &str| format!("[[allow]] entry {}: {e}", i + 1);
    let symbols = match t.remove("symbols") {
        Some(TomlValue::List(v)) if !v.is_empty() => v,
        _ => return Err(at("symbols must be a non-empty array of strings")),
    };
    let function = match t.remove("function") {
        Some(TomlValue::Str(s)) if !s.is_empty() => s,
        _ => return Err(at("function must be a non-empty string")),
    };
    let targets = match t.remove("targets") {
        Some(TomlValue::Str(s)) if s == "all" => None,
        Some(TomlValue::List(v)) if !v.is_empty() => Some(v),
        _ => return Err(at("targets must be \"all\" or a non-empty array")),
    };
    let reopen = match t.remove("reopen") {
        Some(TomlValue::Str(s)) if !s.is_empty() => s,
        _ => return Err(at("reopen must be a non-empty string")),
    };
    let reason = match t.remove("reason") {
        None => None,
        Some(TomlValue::Str(s)) => Some(s),
        Some(_) => return Err(at("reason must be a string")),
    };
    if let Some(k) = t.keys().next() {
        return Err(at(&format!("unknown key {k}")));
    }
    for s in &symbols {
        if !is_banned(s) {
            return Err(at(&format!("{s} is not a banned symbol")));
        }
    }
    Ok(AllowEntry {
        symbols,
        function,
        targets,
        reopen,
        reason,
    })
}

// ---------------------------------------------------------------------------------------------
// Symbols and instructions

fn is_banned(sym: &str) -> bool {
    if BANNED_EXTRA.contains(&sym) {
        return true;
    }
    let base = sym
        .strip_prefix("_CI")
        .or_else(|| {
            sym.strip_prefix("__")
                .and_then(|s| s.strip_suffix("_finite"))
        })
        .unwrap_or(sym);
    if BANNED_BASE.contains(&base) {
        return true;
    }
    // f and l forms (sinf, sinl, lgammaf_r, ...).
    let unsuffixed = |s: &str| {
        s.strip_suffix('f')
            .or_else(|| s.strip_suffix('l'))
            .is_some_and(|b| BANNED_BASE.contains(&b))
    };
    unsuffixed(base)
        || base
            .strip_suffix("_r")
            .is_some_and(|b| unsuffixed(b) && b.starts_with("lgamma"))
}

/// The C name of a symbol as the object spells it: no Mach-O `_`, no COFF `__imp_`, no ELF
/// `@version`, no relocation addend.
fn c_name(sym: &str, macho: bool) -> &str {
    let mut s = sym;
    for sep in ["+0x", "-0x"] {
        if let Some(i) = s.rfind(sep) {
            s = &s[..i];
        }
    }
    if let Some(i) = s.find('@') {
        s = &s[..i];
    }
    if macho {
        s = s.strip_prefix('_').unwrap_or(s);
    }
    s.strip_prefix("__imp_").unwrap_or(s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arch {
    A64,
    X64,
}

fn arch_of(format: &str) -> Result<Arch, String> {
    let f = format.to_ascii_lowercase();
    if f.contains("aarch64") || f.contains("arm64") {
        Ok(Arch::A64)
    } else if f.contains("x86-64") || f.contains("x86_64") {
        Ok(Arch::X64)
    } else {
        Err(format!("unsupported object format {format:?}"))
    }
}

/// An FMA instruction (fused: one rounding of a*b+c).
fn is_fma(arch: Arch, mnemonic: &str) -> bool {
    let m = mnemonic.to_ascii_lowercase();
    match arch {
        Arch::A64 => {
            matches!(
                m.as_str(),
                "fmadd" | "fmsub" | "fnmadd" | "fnmsub" | "fmad" | "fmsb" | "fnmad" | "fnmsb"
            ) || ["fmla", "fmls", "fnmla", "fnmls", "bfmla", "bfmls"]
                .iter()
                .any(|p| m.starts_with(p))
        }
        Arch::X64 => ["vfmadd", "vfmsub", "vfnmadd", "vfnmsub"]
            .iter()
            .any(|p| m.starts_with(p)),
    }
}

/// An assembler-local label, not a function: Mach-O `ltmp0`, `L...`, `l_...`; ELF `.L...` and
/// `$x`/`$d`; COFF `$ehgcr_...`.
fn is_temp_label(name: &str) -> bool {
    name.starts_with("ltmp")
        || name.starts_with(".L")
        || name.starts_with('L')
        || name.starts_with("l_")
        || name.starts_with('$')
}

fn in_ep9_math(name: &str) -> bool {
    name.starts_with("ep9_math::") || name.starts_with("<ep9_math::")
}

/// Gives each block the name of the function that holds its code. llvm-objdump labels a block
/// with one symbol of its address, and that can be an assembler-local label: on Mach-O
/// (addresses are unique in its one text section) `functions_at` names the function at that
/// address; otherwise the block continues the function before it (COFF `$ehgcr_*` labels sit
/// inside a function).
fn function_names(blocks: &[Block], functions_at: &BTreeMap<u64, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(blocks.len());
    for b in blocks {
        let name = if !is_temp_label(&b.raw) {
            b.raw.clone()
        } else if let Some(f) = functions_at.get(&b.addr) {
            f.clone()
        } else if let Some(prev) = out.last() {
            prev.clone()
        } else {
            b.raw.clone()
        };
        out.push(name);
    }
    out
}

/// Non-local function symbols by address, from `llvm-nm -C --defined-only` (Mach-O only).
fn parse_nm_functions(out: &str) -> BTreeMap<u64, String> {
    let mut m: BTreeMap<u64, String> = BTreeMap::new();
    for line in out.lines() {
        let mut it = line.splitn(3, ' ');
        let (Some(addr), Some(kind), Some(name)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(addr) = u64::from_str_radix(addr, 16) else {
            continue;
        };
        if !matches!(kind, "T" | "t") || is_temp_label(name) {
            continue;
        }
        // Prefer a global symbol when two share an address.
        match m.get(&addr) {
            Some(_) if kind == "t" => {}
            _ => {
                m.insert(addr, name.to_string());
            }
        }
    }
    m
}

/// Undefined symbols from `llvm-nm -u` (one per line, last word).
fn parse_nm_undefined(out: &str) -> BTreeSet<String> {
    out.lines()
        .filter_map(|l| l.split_whitespace().last())
        .filter(|s| !s.ends_with(':'))
        .map(str::to_string)
        .collect()
}

/// x86-64 Mach-O keeps jump tables in `__text` (LC_DATA_IN_CODE), and `-d` decodes their bytes
/// as code. A decode that runs across a table end hides the instruction there (an FMA, say), so
/// as in disasm-check the code after each table is decoded again from the table end, and then
/// the table bytes are dropped. `data`: `(offset, length)` entries; in an object the offsets are
/// addresses. `dis(start, stop)` disassembles the object from `start` up to `stop` (`None`: the
/// end).
fn drop_macho_data(
    blocks: &mut [Block],
    data: &[(u64, u64)],
    mut dis: impl FnMut(u64, Option<u64>) -> Result<Vec<Block>, String>,
) -> Result<(), String> {
    let mut starts: Vec<u64> = blocks.iter().map(|b| b.addr).collect();
    starts.sort_unstable();
    redecode_after_data(blocks, 0, data, |_, start, stop| {
        // Stop at the next symbol: the decode selects by address, since a demangled name can
        // hold commas and `--disassemble-symbols` takes a comma list.
        let next = starts.get(starts.partition_point(|&a| a < start)).copied();
        match [stop, next].into_iter().flatten().min() {
            Some(t) if t <= start => Ok(Vec::new()),
            stop => dis(start, stop),
        }
    })?;
    drop_data(blocks, 0, data);
    Ok(())
}

#[derive(Debug, Default)]
struct ObjScan {
    format: String,
    /// (banned symbol, function) per relocation, deduplicated.
    banned: Vec<(String, String)>,
    /// (function, instruction).
    fma_outside: Vec<(String, String)>,
    fma_inside: Vec<(String, String)>,
}

/// Scans parsed disassembly. `undefined`: C names of the object's undefined symbols.
fn scan_blocks(
    format: &str,
    blocks: &[Block],
    names: &[String],
    undefined: &BTreeSet<String>,
) -> Result<ObjScan, String> {
    let arch = arch_of(format)?;
    let macho = format.to_ascii_lowercase().starts_with("mach-o");
    let mut scan = ObjScan {
        format: format.to_string(),
        ..ObjScan::default()
    };
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for (b, name) in blocks.iter().zip(names) {
        for i in &b.insns {
            if let Some(r) = &i.reloc {
                let sym = c_name(r, macho);
                if is_banned(sym) && undefined.contains(sym) {
                    referenced.insert(sym.to_string());
                    if seen.insert((sym.to_string(), name.clone())) {
                        scan.banned.push((sym.to_string(), name.clone()));
                    }
                }
            }
            let mnemonic = i.text.split_whitespace().next().unwrap_or("");
            if is_fma(arch, mnemonic) {
                // Without llvm-objdump's comment ("# xmm0 = ...", "// =...").
                let text = i.text.split(" #").next().unwrap_or("");
                let text = text.split("//").next().unwrap_or("");
                let site = (
                    name.clone(),
                    text.split_whitespace().collect::<Vec<_>>().join(" "),
                );
                if in_ep9_math(name) {
                    scan.fma_inside.push(site);
                } else {
                    scan.fma_outside.push(site);
                }
            }
        }
    }
    // A banned import that no instruction references (a function pointer in data, say) still
    // ships: report it without a function.
    for sym in undefined {
        if is_banned(sym) && !referenced.contains(sym) {
            scan.banned
                .push((sym.clone(), "<no code reference: data or unknown>".into()));
        }
    }
    Ok(scan)
}

fn scan_object(objdump: &Path, nm: &Path, obj: &Path) -> Result<ObjScan, String> {
    let dis = run_capture(
        Command::new(objdump)
            .args(["-d", "-r", "-C", "--no-show-raw-insn"])
            .arg(obj),
    )?;
    let (format, mut blocks) = parse_disasm(&dis);
    if format.is_empty() {
        return Err(format!(
            "llvm-objdump printed no file format for {}",
            obj.display()
        ));
    }
    let macho = format.to_ascii_lowercase().starts_with("mach-o");
    let mut functions_at = BTreeMap::new();
    if macho {
        functions_at = parse_nm_functions(&run_capture(
            Command::new(nm).args(["-C", "--defined-only"]).arg(obj),
        )?);
        if arch_of(&format)? == Arch::X64 {
            let data = parse_data_in_code(&run_capture(
                Command::new(objdump)
                    .args(["--macho", "--data-in-code"])
                    .arg(obj),
            )?);
            drop_macho_data(&mut blocks, &data, |start, stop| {
                let out = run_capture(
                    Command::new(objdump)
                        .args(["-d", "-r", "-C", "--no-show-raw-insn"])
                        .arg(format!("--start-address={start:#x}"))
                        .args(stop.map(|t| format!("--stop-address={t:#x}")))
                        .arg(obj),
                )?;
                Ok(parse_disasm(&out).1)
            })?;
        }
    }
    let undefined: BTreeSet<String> =
        parse_nm_undefined(&run_capture(Command::new(nm).arg("-u").arg(obj))?)
            .iter()
            .map(|s| c_name(s, macho).to_string())
            .collect();
    let names = function_names(&blocks, &functions_at);
    scan_blocks(&format, &blocks, &names, &undefined)
}

// ---------------------------------------------------------------------------------------------
// Builds and the crate graph

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// `--locked` when the workspace has a lock file (fixtures have none).
fn locked(root: &Path) -> Option<&'static str> {
    root.join("Cargo.lock").exists().then_some("--locked")
}

/// The rustflags cargo would use, plus `extra`, as CARGO_ENCODED_RUSTFLAGS.
fn encoded_rustflags(extra: &str) -> String {
    let mut flags: Vec<String> = match std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        Ok(s) if !s.is_empty() => s.split('\x1f').map(str::to_string).collect(),
        _ => std::env::var("RUSTFLAGS")
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
    };
    flags.push(extra.to_string());
    flags.join("\x1f")
}

/// Builds the post-LTO object for `spec` and returns its path.
fn build_object(o: &Opts, spec: &Spec) -> Result<PathBuf, String> {
    let base = o
        .target_dir
        .clone()
        .or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
        .unwrap_or_else(|| o.root.join("target"))
        .join("xtask-fpcheck");
    let tdir = match &spec.cpu {
        Some(c) => base.join(format!("cpu-{c}")),
        None => base.clone(),
    };
    // One directory per checked workspace: two roots that share a target dir (CARGO_TARGET_DIR)
    // must never overwrite each other's object.
    let root_tag = o
        .root
        .to_string_lossy()
        .bytes()
        .fold(0x811c_9dc5u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
    let objdir = base.join("obj").join(format!("{root_tag:08x}"));
    fs::create_dir_all(&objdir).map_err(|e| format!("{}: {e}", objdir.display()))?;
    let obj = objdir.join(format!("{}-{}.o", o.bin, spec.label().replace('@', "-")));
    if obj.exists() {
        fs::remove_file(&obj).map_err(|e| format!("{}: {e}", obj.display()))?;
    }
    let mut cmd = cargo();
    cmd.arg("rustc")
        .args(locked(&o.root))
        .arg("--manifest-path")
        .arg(o.root.join("Cargo.toml"))
        .args(["-p", &o.package, "--bin", &o.bin, "--release", "--target"])
        .arg(&spec.target)
        .arg("--target-dir")
        .arg(&tdir)
        .arg("--")
        .arg(format!("--emit=obj={}", obj.display()))
        .args(["-C", "linker=true"]);
    if let Some(c) = &spec.cpu {
        // Every crate, not only the final one: with fat LTO each function keeps the target-cpu
        // of the crate that defined it.
        cmd.env(
            "CARGO_ENCODED_RUSTFLAGS",
            encoded_rustflags(&format!("-Ctarget-cpu={c}")),
        );
    }
    eprintln!("fp-check: building {} for {}", o.package, spec.label());
    run_capture(&mut cmd)?;
    if !obj.exists() {
        return Err(format!("rustc wrote no object at {}", obj.display()));
    }
    Ok(obj)
}

/// Names of the package's normal and build dependencies on every target (`cargo tree`).
fn parse_tree_packages(out: &str) -> BTreeSet<String> {
    out.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The inverted dependency tree of `krate` in the package's normal and build graph on every
/// target: `(depth, package name)` per line, every path printed (`--no-dedupe`).
fn inverted_tree(o: &Opts, krate: &str) -> Result<Vec<(usize, String)>, String> {
    let out = run_capture(
        cargo()
            .arg("tree")
            .args(locked(&o.root))
            .arg("--manifest-path")
            .arg(o.root.join("Cargo.toml"))
            .args([
                "-p",
                &o.package,
                "-e",
                "normal,build",
                "--target",
                "all",
                "-i",
                krate,
                "--no-dedupe",
                "--prefix",
                "depth",
                "-f",
                "{p}",
            ]),
    )?;
    let tree = parse_depth_tree(&out)?;
    if tree.first().is_none_or(|(d, n)| *d != 0 || n != krate) {
        return Err(format!("cargo tree -i {krate} did not start with {krate}"));
    }
    Ok(tree)
}

/// Parses `cargo tree --prefix depth -f {p}` output: each line is the depth, then
/// `name vX.Y.Z ...`.
fn parse_depth_tree(out: &str) -> Result<Vec<(usize, String)>, String> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let digits = l.len() - l.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            let depth = l[..digits]
                .parse()
                .map_err(|_| format!("cargo tree line without a depth: {l:?}"))?;
            let name = l[digits..]
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("cargo tree line without a package: {l:?}"))?;
            Ok((depth, name.to_string()))
        })
        .collect()
}

/// The paths of an inverted tree (crate first, then each dependent up to a root) that pass
/// no `via` package. A path ends at a leaf of the inverted tree: the package itself or
/// another root.
fn paths_not_via(tree: &[(usize, String)], via: &[String]) -> Vec<Vec<String>> {
    let mut bad = Vec::new();
    // stack[d] = (name, covered): the path from the crate to the current line.
    let mut stack: Vec<(String, bool)> = Vec::new();
    for (k, (depth, name)) in tree.iter().enumerate() {
        stack.truncate(*depth);
        let covered = via.iter().any(|v| v == name) || stack.last().is_some_and(|(_, c)| *c);
        stack.push((name.clone(), covered));
        let leaf = tree.get(k + 1).is_none_or(|(d, _)| d <= depth);
        if leaf && !covered {
            bad.push(stack.iter().map(|(n, _)| n.clone()).collect());
        }
    }
    bad
}

fn math_crates(o: &Opts) -> Result<Vec<String>, String> {
    let out = run_capture(
        cargo()
            .arg("tree")
            .args(locked(&o.root))
            .arg("--manifest-path")
            .arg(o.root.join("Cargo.toml"))
            .args([
                "-p",
                &o.package,
                "-e",
                "normal,build",
                "--target",
                "all",
                "--prefix",
                "none",
                "-f",
                "{p}",
            ]),
    )?;
    let pkgs = parse_tree_packages(&out);
    if !pkgs.contains(&o.package) {
        return Err(format!("cargo tree did not list {}", o.package));
    }
    Ok(MATH_CRATES
        .iter()
        .filter(|c| pkgs.contains(**c))
        .map(|c| c.to_string())
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Allow sites in source

#[derive(Debug, PartialEq, Eq)]
struct AllowSite {
    /// Path relative to the root, with `/`.
    file: String,
    line: usize,
    /// The innermost item around the site, as a path (`crate::module::function`).
    item: String,
}

/// Lines (1-based) that name `clippy::disallowed_methods` outside a `//` comment.
fn site_lines(src: &str) -> Vec<usize> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| {
            l.find("disallowed_methods")
                .is_some_and(|i| !l[..i].contains("//"))
        })
        .map(|(n, _)| n + 1)
        .collect()
}

/// Item line ranges with their paths.
struct ItemSpans {
    path: Vec<String>,
    spans: Vec<(usize, usize, String)>,
}

impl ItemSpans {
    fn item(&mut self, span: proc_macro2::Span, name: String, f: impl FnOnce(&mut Self)) {
        self.path.push(name);
        self.spans
            .push((span.start().line, span.end().line, self.path.join("::")));
        f(self);
        self.path.pop();
    }
}

impl<'ast> Visit<'ast> for ItemSpans {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        self.item(i.span(), i.sig.ident.to_string(), |s| {
            visit::visit_item_fn(s, i)
        });
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.item(i.span(), i.sig.ident.to_string(), |s| {
            visit::visit_impl_item_fn(s, i)
        });
    }
    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        self.item(i.span(), i.sig.ident.to_string(), |s| {
            visit::visit_trait_item_fn(s, i)
        });
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let ty = i.self_ty.to_token_stream().to_string().replace(' ', "");
        let name = match &i.trait_ {
            Some((_, t, _)) => format!(
                "<{ty} as {}>",
                t.to_token_stream().to_string().replace(' ', "")
            ),
            None => ty,
        };
        self.item(i.span(), name, |s| visit::visit_item_impl(s, i));
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_mod(s, i)
        });
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_trait(s, i)
        });
    }
    fn visit_item_macro(&mut self, i: &'ast syn::ItemMacro) {
        let name = match &i.ident {
            Some(id) => format!("{id}!"),
            None => format!("{}!", i.mac.path.to_token_stream()),
        };
        self.item(i.span(), name, |s| visit::visit_item_macro(s, i));
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_const(s, i)
        });
    }
    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_static(s, i)
        });
    }
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_struct(s, i)
        });
    }
    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        self.item(i.span(), i.ident.to_string(), |s| {
            visit::visit_item_enum(s, i)
        });
    }
}

/// `crates/<crate>/src/a/b.rs` -> `<crate>::a::b` (`lib.rs`, `main.rs` and `mod.rs` add nothing).
fn module_path(rel: &str) -> String {
    let parts: Vec<&str> = rel.split('/').collect();
    let mut out: Vec<String> = Vec::new();
    if parts.len() >= 2 && parts[0] == "crates" {
        out.push(parts[1].to_string());
    }
    let src = parts.iter().position(|p| *p == "src").map_or(2, |i| i + 1);
    for (i, p) in parts.iter().enumerate().skip(src) {
        let last = i + 1 == parts.len();
        let p = if last { p.trim_end_matches(".rs") } else { p };
        if last && matches!(p, "lib" | "main" | "mod") {
            continue;
        }
        out.push(p.to_string());
    }
    out.join("::")
}

/// The sites in one file (`rel`: path from the root, with `/`).
fn file_sites(rel: &str, src: &str) -> Vec<AllowSite> {
    let lines = site_lines(src);
    if lines.is_empty() {
        return Vec::new();
    }
    let mut spans = ItemSpans {
        path: Vec::new(),
        spans: Vec::new(),
    };
    // A file syn cannot parse still reports its sites, named by module only.
    if let Ok(f) = syn::parse_file(src) {
        spans.visit_file(&f);
    }
    let module = module_path(rel);
    lines
        .into_iter()
        .map(|line| {
            let inner = spans
                .spans
                .iter()
                .filter(|(s, e, _)| (*s..=*e).contains(&line))
                .min_by_key(|(s, e, _)| e - s);
            let item = match inner {
                Some((_, _, p)) => format!("{module}::{p}"),
                None => format!("{module} (module level)"),
            };
            AllowSite {
                file: rel.to_string(),
                line,
                item,
            }
        })
        .collect()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if name.starts_with('.') || name == "target" {
                continue;
            }
            rs_files(&p, out)?;
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Every site in `ROOT/crates/**/*.rs` except the allowed file.
fn scan_allow_sites(root: &Path) -> Result<Vec<AllowSite>, String> {
    let crates = root.join("crates");
    let mut files = Vec::new();
    if crates.is_dir() {
        rs_files(&crates, &mut files)?;
    }
    let mut out = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if rel == ALLOW_SITE_FILE {
            continue;
        }
        let src = fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        out.extend(file_sites(&rel, &src));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disasm::Insn;

    #[test]
    fn banned_names() {
        for s in [
            "fmod",
            "pow",
            "powf",
            "sinl",
            "fma",
            "fmaf",
            "exp10",
            "cbrt",
            "__powidf2",
            "__sincos_stret",
            "_CIpow",
            "__pow_finite",
            "lgammaf_r",
            "remquo",
            "_hypot",
        ] {
            assert!(is_banned(s), "{s}");
        }
        for s in [
            "ceil",
            "floor",
            "trunc",
            "round",
            "sqrt",
            "fabs",
            "ldexp",
            "frexp",
            "memcpy",
            "scalbn",
            "nextafter",
            "logb",
            "ilogb",
            "modf",
            "copysign",
            "fmin",
            "fmax",
            "f",
            "",
        ] {
            assert!(!is_banned(s), "{s}");
        }
    }

    #[test]
    fn c_names() {
        assert_eq!(c_name("fmod-0x4", false), "fmod");
        assert_eq!(c_name("_fmod", true), "fmod");
        assert_eq!(c_name("__imp_pow", false), "pow");
        assert_eq!(c_name("pow@GLIBC_2.29", false), "pow");
        assert_eq!(c_name(".text._RNvXs-0x4", false), ".text._RNvXs");
        assert_eq!(c_name("_fmod", false), "_fmod");
    }

    #[test]
    fn fma_mnemonics() {
        for m in [
            "fmadd", "fnmsub", "fmla", "fmlal2", "fmls", "bfmlalb", "fmad",
        ] {
            assert!(is_fma(Arch::A64, m), "{m}");
        }
        for m in ["fmul", "fadd", "fmov", "fmax", "mla", "madd", "vfmadd213sd"] {
            assert!(!is_fma(Arch::A64, m), "{m}");
        }
        for m in [
            "vfmadd213sd",
            "vfnmsub231pd",
            "vfmaddsub132ps",
            "vfmsubadd213pd",
        ] {
            assert!(is_fma(Arch::X64, m), "{m}");
        }
        for m in ["vmulsd", "mulsd", "addsd", "vpmadd52luq", "fmadd"] {
            assert!(!is_fma(Arch::X64, m), "{m}");
        }
    }

    #[test]
    fn allow_file_subset() {
        let text = r#"
# comment
[[allow]]
symbols = ["fmod", "pow"]  # trailing comment
function = "oxc_semantic::ts_enum::eval::evaluate_expression"
targets = "all"
reopen = "H3 enum lowering"
reason = "TS enum folding; values never read"

[[allow]]
symbols = ["sin"]
function = "a::b"
targets = ["x86_64-pc-windows-msvc"]
reopen = "x"
"#;
        let v = parse_allow(text).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].symbols, ["fmod", "pow"]);
        assert_eq!(v[0].targets, None);
        assert!(v[0].covers(
            "pow",
            "oxc_semantic::ts_enum::eval::evaluate_expression",
            "aarch64-apple-darwin"
        ));
        assert!(v[0].covers(
            "fmod",
            "oxc_semantic::ts_enum::eval::evaluate_expression::<u8>",
            "x"
        ));
        assert!(!v[0].covers(
            "fmod",
            "oxc_semantic::ts_enum::eval::evaluate_expression_two",
            "x"
        ));
        assert!(!v[0].covers(
            "sin",
            "oxc_semantic::ts_enum::eval::evaluate_expression",
            "x"
        ));
        assert!(v[1].covers("sin", "a::b", "x86_64-pc-windows-msvc"));
        assert!(!v[1].covers("sin", "a::b", "aarch64-apple-darwin"));

        for bad in [
            "symbols = [\"pow\"]",            // outside a table
            "[allow]\nsymbols = [\"pow\"]",   // wrong header
            "[[allow]]\nsymbols = [\"pow\"]", // missing keys
            "[[allow]]\nsymbols = [\"ceil\"]\nfunction = \"f\"\ntargets = \"all\"\nreopen = \"r\"", // not banned
            "[[allow]]\nsymbols = [\"pow\"]\nfunction = \"f\"\ntargets = \"all\"\nreopen = \"r\"\nextra = \"x\"",
            "[[allow]]\nsymbols = [\"pow\" \"fmod\"]",
            "[[allow]]\nfunction = \"f\" junk",
        ] {
            assert!(parse_allow(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn allow_crate_tables() {
        let text = r#"
[[allow]]
symbols = ["pow"]
function = "a::b"
targets = "all"
reopen = "x"

[[allow-crate]]
crates = ["core_maths", "libm"]
via = ["temporal_rs", "calendrical_calculations"]
reopen = "r"
reason = "why"
"#;
        let f = parse_allow_file(text).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(f.crates[0].crates, ["core_maths", "libm"]);
        assert_eq!(f.crates[0].via, ["temporal_rs", "calendrical_calculations"]);
        assert_eq!(f.crates[0].reason.as_deref(), Some("why"));
        for bad in [
            "[[allow-crate]]\ncrates = [\"serde\"]\nvia = [\"a\"]\nreopen = \"r\"", // not a math crate
            "[[allow-crate]]\ncrates = [\"libm\"]\nreopen = \"r\"",                 // no via
            "[[allow-crate]]\ncrates = [\"libm\"]\nvia = []\nreopen = \"r\"",       // empty via
            "[[allow-crate]]\ncrates = [\"libm\"]\nvia = [\"a\"]\nreopen = \"r\"\nfunction = \"f\"",
            "[[allow-crates]]\ncrates = [\"libm\"]",
        ] {
            assert!(parse_allow_file(bad).is_err(), "{bad}");
        }
    }

    // `cargo tree -p ep9_cli -i libm --no-dedupe --prefix depth -f {p}` shape: libm is reached
    // through core_maths from temporal_rs and calendrical_calculations.
    const LIBM_TREE: &str = "0libm v0.2.16
1core_maths v0.1.1
2calendrical_calculations v0.2.4
3temporal_rs v0.2.6
4ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
5ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
6ep9_cli v0.0.0 (/w/crates/ep9_cli)
2temporal_rs v0.2.6
3ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
4ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
5ep9_cli v0.0.0 (/w/crates/ep9_cli)
1ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
2ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
3ep9_cli v0.0.0 (/w/crates/ep9_cli)
";

    #[test]
    fn allow_crate_paths() {
        let t = parse_depth_tree(LIBM_TREE).unwrap();
        assert_eq!(t[0], (0, "libm".to_string()));
        assert_eq!(t.len(), 14);
        let via = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Every path passes temporal_rs or calendrical_calculations, or ep9_temporal (the
        // direct force-soft-floats edge).
        assert!(
            paths_not_via(
                &t,
                &via(&["temporal_rs", "calendrical_calculations", "ep9_temporal"])
            )
            .is_empty()
        );
        // Control: without ep9_temporal in `via`, the direct edge fails, and the failure
        // names the path.
        let bad = paths_not_via(&t, &via(&["temporal_rs", "calendrical_calculations"]));
        assert_eq!(
            bad,
            [via(&["libm", "ep9_temporal", "ep9_builtins", "ep9_cli"])]
        );
        // Control: a `via` that no path passes fails every path.
        assert_eq!(paths_not_via(&t, &via(&["serde"])).len(), 3);
        assert!(parse_depth_tree("libm v1").is_err());
    }

    #[test]
    fn repo_allow_file_parses() {
        let p = repo_root().join("expectations/fp-check-allow.toml");
        let f = parse_allow_file(&fs::read_to_string(&p).unwrap()).unwrap();
        assert!(f.len() <= MAX_ALLOW_ENTRIES);
        assert_eq!(f.crates.len(), 1);
        assert_eq!(f.crates[0].crates, ["core_maths", "libm"]);
        let v = parse_allow(&fs::read_to_string(p).unwrap()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].symbols, ["fmod", "pow"]);
        assert_eq!(
            v[0].function,
            "oxc_semantic::ts_enum::eval::evaluate_expression"
        );
        assert_eq!(v[0].targets, None);
    }

    // The B0 shape of `cargo tree -p ep9_cli -e normal,build --target all -i libm --no-dedupe
    // --prefix depth -f {p}` (every path goes through ep9_temporal), plus one new path that
    // does not: icu_calendar from another crate (an INTL crate, MODES-CHECK-w29-B0 MC-w29-B0-1).
    const LIBM_TREE_B0_PLUS_INTL: &str = "0libm v0.2.16
1core_maths v0.1.1
2calendrical_calculations v0.2.4
3icu_calendar v2.3.0
4temporal_rs v0.2.6
5ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
6ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
7ep9_cli v0.0.0 (/w/crates/ep9_cli)
4ep9_intl v0.0.0 (/w/crates/ep9_intl)
5ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
6ep9_cli v0.0.0 (/w/crates/ep9_cli)
3temporal_rs v0.2.6
4ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
5ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
6ep9_cli v0.0.0 (/w/crates/ep9_cli)
3zoneinfo64 v0.3.0
4ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
5ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
6ep9_cli v0.0.0 (/w/crates/ep9_cli)
4timezone_provider v0.2.6
5ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
6ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
7ep9_cli v0.0.0 (/w/crates/ep9_cli)
5temporal_rs v0.2.6
6ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
7ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
8ep9_cli v0.0.0 (/w/crates/ep9_cli)
2temporal_rs v0.2.6
3ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
4ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
5ep9_cli v0.0.0 (/w/crates/ep9_cli)
1ep9_temporal v0.0.0 (/w/crates/ep9_temporal)
2ep9_builtins v0.0.0 (/w/crates/ep9_builtins)
3ep9_cli v0.0.0 (/w/crates/ep9_cli)
";

    #[test]
    fn allow_crate_via_is_only_ep9_temporal() {
        let p = repo_root().join("expectations/fp-check-allow.toml");
        let f = parse_allow_file(&fs::read_to_string(p).unwrap()).unwrap();
        let via = &f.crates[0].via;
        assert_eq!(via, &["ep9_temporal"]);
        let t = parse_depth_tree(LIBM_TREE_B0_PLUS_INTL).unwrap();
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // The B0 paths pass: drop the ep9_intl lines (8, 9, 10) and every path is covered.
        let b0: Vec<_> = t
            .iter()
            .enumerate()
            .filter(|(k, _)| !(8..=10).contains(k))
            .map(|(_, l)| l.clone())
            .collect();
        assert!(paths_not_via(&b0, via).is_empty());
        // The new icu_calendar path from another crate fails, and the failure names it.
        assert_eq!(
            paths_not_via(&t, via),
            [s(&[
                "libm",
                "core_maths",
                "calendrical_calculations",
                "icu_calendar",
                "ep9_intl",
                "ep9_builtins",
                "ep9_cli"
            ])]
        );
        // Control: the wide list of round 1 lets the same path pass (it goes through
        // calendrical_calculations), so the narrow list is what makes it fail.
        let wide = s(&["temporal_rs", "calendrical_calculations", "ep9_temporal"]);
        assert!(paths_not_via(&t, &wide).is_empty());
    }

    fn block(raw: &str, addr: u64, insns: &[(&str, Option<&str>)]) -> Block {
        Block {
            member: String::new(),
            raw: raw.into(),
            addr,
            insns: insns
                .iter()
                .enumerate()
                .map(|(k, (t, r))| Insn {
                    addr: addr + 4 * k as u64,
                    text: t.to_string(),
                    reloc: r.map(str::to_string),
                })
                .collect(),
        }
    }

    #[test]
    fn scan_names_functions() {
        // Mach-O: ltmp0 hides the function at address 0; .Lfunc_end hides the next function.
        let blocks = vec![
            block("ltmp0", 0, &[("bl\t0x0", Some("_fmod"))]),
            block("a::f", 0x40, &[("fmadd\td0, d1, d2, d3", None)]),
            block(".Lfunc_end_a", 0x80, &[("bl\t0x80", Some("_pow"))]),
            block(
                "ep9_math::exact::fma_exact",
                0xc0,
                &[("fmadd\td0, d0, d1, d2", None)],
            ),
            block("$ehgcr_1_2", 0x100, &[("bl\t0x100", Some("_sin"))]),
        ];
        let mut at = BTreeMap::new();
        at.insert(0, "x::start".to_string());
        at.insert(0x80, "b::g".to_string());
        let names = function_names(&blocks, &at);
        assert_eq!(
            names,
            [
                "x::start",
                "a::f",
                "b::g",
                "ep9_math::exact::fma_exact",
                "ep9_math::exact::fma_exact"
            ]
        );
        let undefined: BTreeSet<String> = ["fmod", "pow", "sin", "cbrt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = scan_blocks("mach-o arm64", &blocks, &names, &undefined).unwrap();
        assert_eq!(
            s.banned,
            [
                ("fmod".to_string(), "x::start".to_string()),
                ("pow".to_string(), "b::g".to_string()),
                ("sin".to_string(), "ep9_math::exact::fma_exact".to_string()),
                (
                    "cbrt".to_string(),
                    "<no code reference: data or unknown>".to_string()
                ),
            ]
        );
        assert_eq!(s.fma_outside.len(), 1);
        assert_eq!(s.fma_outside[0].0, "a::f");
        assert_eq!(s.fma_inside.len(), 1);
    }

    #[test]
    fn nm_and_tree_parsing() {
        let nm = "0000000000000000 T _ep9_spill_aapcs64\n0000000000000000 t ltmp0\n\
                  0000000000000040 t core::ptr::drop_glue::<a b>\n0000000000000040 T g\n\
                  0000000000000080 t .Lfunc_end0\n";
        let m = parse_nm_functions(nm);
        assert_eq!(m[&0], "_ep9_spill_aapcs64");
        assert_eq!(m[&0x40], "g");
        assert!(!m.contains_key(&0x80));
        let u =
            parse_nm_undefined("                 U fmod\n                 w getrandom\nU _pow\n");
        assert!(u.contains("fmod") && u.contains("getrandom") && u.contains("_pow"));
        let tree = "ep9_cli v0.0.0 (/x)\nlibm v0.2.16\nlibc v0.2.1 (*)\n";
        let p = parse_tree_packages(tree);
        assert!(p.contains("libm") && p.contains("ep9_cli") && p.contains("libc"));
        let dice = "f.o:\nData in code table (2 entries)\noffset     length kind\n\
                    0x0000c7a4    268 JUMP_TABLE32\n0x0000ed90     76 JUMP_TABLE32\n";
        assert_eq!(parse_data_in_code(dice), [(0xc7a4, 268), (0xed90, 76)]);
    }

    // x86-64 Mach-O from the `jump-table-fma` fixture (tests/fpcheck): a jump table at 0x14..0x1c
    // (`.data_region jt32`), then a `vfmadd213sd` at 0x1c. `-d` decodes across the table end:
    // `addb %al, %ah` at 0x1b takes the byte 0xc4 of the FMA.
    const JT_FIRST: &str = "fx.o:\tfile format mach-o 64-bit x86-64

0000000000000000 <_fx_table_fma>:
       0:      \tleaq\t0xd(%rip), %rcx         ## 0x14 <_fx_table_fma+0x14>
       7:      \tandl\t$0x1, %edi
       a:      \tmovslq\t(%rcx,%rdi,4), %rax
       e:      \taddq\t%rcx, %rax
      11:      \tjmpq\t*%rax
      13:      \tnop
      14:      \torb\t%al, (%rax)
      16:      \taddb\t%al, (%rax)
      18:      \t<unknown>
      19:      \taddb\t%al, (%rax)
      1b:      \taddb\t%al, %ah
      1d:      \tloop\t0x10 <_fx_table_fma+0x10>
      1f:      \ttestl\t$0xfbc5c3c2, %eax       ## imm = 0xFBC5C3C2
      24:      \tpopq\t%rcx
      25:      \troll\t$0xfb, %ebp
      28:      \tpopq\t%rax
      29:      \tretq\t$0xfc3                  ## imm = 0xFC3
      2c:      \t<unknown>
      2d:      \taddb\t%r8b, (%rax)

0000000000000030 <core::ptr::drop_glue::<std::env::Args>>:
      30:      \tpushq\t%rbp
";
    // The same file with `--start-address=0x1c --stop-address=0x30`.
    const JT_AFTER: &str = "fx.o:\tfile format mach-o 64-bit x86-64

0000000000000000 <_fx_table_fma>:
      1c:      \tvfmadd213sd\t%xmm2, %xmm1, %xmm0 ## xmm0 = (xmm1 * xmm0) + xmm2
      21:      \tretq
      22:      \tvmulsd\t%xmm1, %xmm0, %xmm0
      26:      \tvaddsd\t%xmm2, %xmm0, %xmm0
      2a:      \tretq
      2b:      \tnopl\t(%rax,%rax)
";

    #[test]
    fn macho_fma_after_jump_table() {
        let format = "mach-o 64-bit x86-64";
        let scan = |blocks: &[Block]| {
            let names = function_names(blocks, &BTreeMap::new());
            scan_blocks(format, blocks, &names, &BTreeSet::new()).unwrap()
        };
        // Negative control: dropping the table alone hides the FMA.
        let (_, mut blocks) = parse_disasm(JT_FIRST);
        drop_data(&mut blocks, 0, &[(0x14, 8)]);
        assert!(scan(&blocks).fma_outside.is_empty());

        // Decoded again from the table end up to the next symbol, the FMA is found.
        let (_, mut blocks) = parse_disasm(JT_FIRST);
        let mut calls = Vec::new();
        drop_macho_data(&mut blocks, &[(0x14, 8)], |start, stop| {
            calls.push((start, stop));
            Ok(parse_disasm(JT_AFTER).1)
        })
        .unwrap();
        assert_eq!(calls, [(0x1c, Some(0x30))]);
        assert_eq!(
            scan(&blocks).fma_outside,
            [(
                "_fx_table_fma".to_string(),
                "vfmadd213sd %xmm2, %xmm1, %xmm0".to_string()
            )]
        );
        assert_eq!(blocks[0].insns.last().unwrap().addr, 0x2b);

        // A table that ends where the next symbol starts needs no second decode.
        let (_, mut blocks) = parse_disasm(JT_FIRST);
        drop_macho_data(&mut blocks, &[(0x28, 8)], |_, _| {
            panic!("no decode after a table at a symbol end")
        })
        .unwrap();
        assert_eq!(blocks[0].insns.last().unwrap().addr, 0x25);
    }

    #[test]
    fn allow_sites_are_named() {
        let src = "//! `#[allow(clippy::disallowed_methods)]` in a doc comment is not a site\n\
                   pub struct S;\n\
                   impl S {\n\
                   \x20   #[inline]\n\
                   \x20   #[allow(clippy::disallowed_methods)]\n\
                   \x20   pub fn f(x: f64) -> f64 {\n\
                   \x20       x.sin()\n\
                   \x20   }\n\
                   }\n\
                   mod inner {\n\
                   \x20   #[cfg_attr(test, allow(clippy::disallowed_methods))]\n\
                   \x20   fn g() {}\n\
                   }\n\
                   macro_rules! m { () => { #[allow(clippy::disallowed_methods)] fn h() {} } }\n";
        let s = file_sites("crates/fx_engine/src/num/mod.rs", src);
        let got: Vec<(usize, &str)> = s.iter().map(|x| (x.line, x.item.as_str())).collect();
        assert_eq!(
            got,
            [
                (5, "fx_engine::num::S::f"),
                (11, "fx_engine::num::inner::g"),
                (14, "fx_engine::num::m!"),
            ]
        );
        assert_eq!(
            file_sites(
                "crates/c/src/lib.rs",
                "#![allow(clippy::disallowed_methods)]\n"
            )[0]
            .item,
            "c (module level)"
        );
        assert_eq!(module_path("crates/ep9_vm/src/lib.rs"), "ep9_vm");
        assert_eq!(
            module_path("crates/ep9_vm/src/interp/ops.rs"),
            "ep9_vm::interp::ops"
        );
    }

    #[test]
    fn spec_and_options() {
        assert_eq!(
            Spec::parse("x86_64-unknown-linux-gnu@x86-64-v3").unwrap(),
            Spec {
                target: "x86_64-unknown-linux-gnu".into(),
                cpu: Some("x86-64-v3".into())
            }
        );
        assert!(Spec::parse("x@").is_err());
        let o = parse_opts(&[]).unwrap();
        assert_eq!(o.specs.len(), 6);
        assert_eq!(o.specs[5].label(), V3_SPEC);
        assert!(parse_opts(&["--bogus".into()]).is_err());
    }

    /// Today's tree has no site outside exact.rs (H36 Rules row 1).
    #[test]
    fn repo_has_no_allow_site_outside_exact_rs() {
        assert_eq!(scan_allow_sites(&repo_root()).unwrap(), []);
    }
}
