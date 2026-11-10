//! `cargo xtask api-list` (H20-Q6; plan v12 §8.1, M7-P5 TOOLS): the public API of `ep9_vm`
//! from compiler data, diffed against `crates/ep9_vm/tests/golden/public-api.txt`.
//!
//! ```text
//! cargo +nightly-2026-10-04 rustdoc -p ep9_vm --lib --features jit
//!       --target x86_64-unknown-linux-gnu -- -Z unstable-options --output-format json
//!       --document-hidden-items           (target dir <root>/target/api-list)
//!   -> ep9_vm.json (format_version 61) -> read with exit::json (no JSON crate)
//!   -> walk the public paths from the crate root (modules, `pub use`, globs)
//!   -> one line per item, field, variant, trait item, impl and pub impl item
//!   -> sorted, deduped; compare with the golden ('#' lines are comments)
//! ```
//!
//! The source-text golden `h20-core.txt` (`tests/h20_core.rs`) reads the forms of its list
//! and is a tripwire (H20 Decision, "The golden is a tripwire"). This listing is the
//! complete check that H20-Q6 asks for: rustdoc resolves every form that the compiler sees,
//! so the escape classes (a)-(f) of `tests/h20_core.rs` give lines here too (a macro-made
//! item, a `#[path]` module file anywhere, a trait impl in any file, an alias or a renaming
//! `use`, an impl in a body, an impl outside `abi/`). Class (f), the types of other crates,
//! shows as the paths that `ep9_vm` re-exports and the types its lines name; their own API
//! is not listed.
//!
//! The build: the nightly is the one pin of H26-Q3 (`fuzz/rust-toolchain.toml`), the target
//! is the CI lint runner's (one target, so one golden on every host; an item under a
//! `target_arch` or `target_os` cfg of another target gives no line), the features are those
//! of the ep9 binary (`jit`, the ep9_cli default; `spike-native`, `ic-stats` and
//! `fastdiff-hits` are off: dep-guard 11 keeps them out of the ep9 graph), and doc-hidden
//! items are read (H20-Q4: `Downcast::test` was a doc-hidden producer).
//!
//! What a line holds: the path, then the item: its kind, name, generics, bounds and where
//! clause; a function's header (`const`, `async`, `unsafe`, `extern`), its argument types
//! (not their names: a rename keeps the golden) and its return type; a const's or static's
//! type (not its value: a version bump keeps the golden); an enum variant's fields and
//! discriminant (ids are append-only, H20); the attributes `#[repr]`, `#[non_exhaustive]` and
//! `#[doc(hidden)]`. A type of this crate prints by its name (by its full path if two items
//! of the crate share the name); a type of another crate prints by its full path. Impls: every
//! impl of a listed type and of a listed trait, derived ones too, but not the blanket impls
//! of other crates (`impl<T> From<T> for T`) and of the auto traits only `Send` and `Sync`
//! (marked `(auto)`). The items of a local supertrait that has no public path (a sealed trait
//! in a private module) are listed under the trait that names it (`supertrait <Name> ::`); a
//! glob re-export of a local enum lists each variant at its path; an ABI with the unwind flag
//! prints as `extern "C-unwind"` (M7-P5 Codex T1, T3, T4).
//!
//! Usage: `cargo xtask api-list` (check), `--bless` (write the golden), `--print` (the
//! listing to stdout), `--json FILE` (read this rustdoc JSON, run no rustdoc: the fixture
//! tests and a JSON made elsewhere). Exit codes: 0 the listing equals the golden, 1 a line
//! differs (each printed with `+` for a new line and `-` for a gone one), 2 a usage error, a
//! rustdoc failure, a JSON that does not parse or a `format_version` other than
//! [`FORMAT_VERSION`]. A new `pub` item is a `+` line: bless it in the same commit, and the
//! review reads the golden diff (an H20 core item still needs its H20 row).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::exit::json::{Json, parse_json};

const USAGE: &str = "\
usage: cargo xtask api-list [--root DIR] [--json FILE] [--golden FILE] [--bless | --print]
  --root DIR     the tree (default: the git top dir of the current dir)
  --json FILE    read this rustdoc JSON; run no rustdoc
  --golden FILE  the golden (default: <root>/crates/ep9_vm/tests/golden/public-api.txt)
  --bless        write the listing to the golden
  --print        print the listing; compare nothing";

/// The rustdoc JSON format that this reader reads (rustdoc of [`TOOLCHAIN`]).
pub(crate) const FORMAT_VERSION: f64 = 61.0;
/// The one nightly pin (H26-Q3; `fuzz/rust-toolchain.toml`, the CI miri job).
pub(crate) const TOOLCHAIN: &str = "nightly-2026-10-04";
/// The CI lint runner's target: the golden is the listing of this build.
pub(crate) const TARGET: &str = "x86_64-unknown-linux-gnu";
/// The features of the ep9 binary's `ep9_vm` (the ep9_cli default).
pub(crate) const FEATURES: &str = "jit";
/// The golden, from the root.
pub(crate) const GOLDEN: &str = "crates/ep9_vm/tests/golden/public-api.txt";
/// The auto traits whose impls are listed (an embedder's thread rules); the others
/// (`Unpin`, `UnwindSafe`, `RefUnwindSafe`, `Freeze`) are not.
const AUTO_LISTED: &[&str] = &["Send", "Sync"];

const HEADER: &str = "\
# The public API of ep9_vm from rustdoc JSON (H20-Q6): `cargo xtask api-list` compares,
# `cargo xtask api-list --bless` writes this file (tools/xtask/src/api_list.rs has the line
# form). Build: nightly-2026-10-04, --target x86_64-unknown-linux-gnu, --features jit,
# doc-hidden items read, rustdoc JSON format_version 61. One line per item, sorted.
";

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let json_path = match &o.json {
        Some(p) => p.clone(),
        None => run_rustdoc(&o.root)?,
    };
    let text =
        fs::read_to_string(&json_path).map_err(|e| format!("{}: {e}", json_path.display()))?;
    let lines = listing(&text)?;
    if o.print {
        for l in &lines {
            println!("{l}");
        }
        return Ok(true);
    }
    let golden = o.golden.unwrap_or_else(|| o.root.join(GOLDEN));
    if o.bless {
        let mut out = String::from(HEADER);
        for l in &lines {
            out.push_str(l);
            out.push('\n');
        }
        fs::write(&golden, out).map_err(|e| format!("{}: {e}", golden.display()))?;
        println!(
            "api-list: wrote {} lines to {}",
            lines.len(),
            golden.display()
        );
        return Ok(true);
    }
    let want = fs::read_to_string(&golden).map_err(|e| format!("{}: {e}", golden.display()))?;
    let want: Vec<String> = want
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    let d = diff(&want, &lines);
    for l in &d {
        println!("{l}");
    }
    println!(
        "api-list: {} lines, golden {} lines: {} new, {} gone{}",
        lines.len(),
        want.len(),
        d.iter().filter(|l| l.starts_with('+')).count(),
        d.iter().filter(|l| l.starts_with('-')).count(),
        if d.is_empty() {
            String::new()
        } else {
            format!(
                " (a change of the public API of ep9_vm: if it is meant, run `cargo xtask \
                 api-list --bless` and commit {GOLDEN})"
            )
        }
    );
    Ok(d.is_empty())
}

struct Opts {
    root: PathBuf,
    json: Option<PathBuf>,
    golden: Option<PathBuf>,
    bless: bool,
    print: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        root: PathBuf::new(),
        json: None,
        golden: None,
        bless: false,
        print: false,
    };
    let mut root = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--root" => root = Some(val("--root")?),
            "--json" => o.json = Some(val("--json")?),
            "--golden" => o.golden = Some(val("--golden")?),
            "--bless" => o.bless = true,
            "--print" => o.print = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s => return Err(format!("unknown argument {s}\n{USAGE}")),
        }
    }
    if o.bless && o.print {
        return Err(format!("--bless and --print do not go together\n{USAGE}"));
    }
    o.root = match root {
        Some(r) => r,
        None => crate::exit::default_root()?,
    };
    Ok(o)
}

/// Runs rustdoc for the JSON and gives its path. Its own target dir, so a stable build of
/// the tree keeps its cache.
fn run_rustdoc(root: &Path) -> Result<PathBuf, String> {
    let target_dir = root.join("target").join("api-list");
    let status = Command::new("cargo")
        .current_dir(root)
        .arg(format!("+{TOOLCHAIN}"))
        .args(["rustdoc", "--locked", "-p", "ep9_vm", "--lib"])
        .args(["--features", FEATURES, "--target", TARGET])
        .args(["--", "-Z", "unstable-options", "--output-format", "json"])
        .arg("--document-hidden-items")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("RUSTDOCFLAGS")
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .env_remove("RUSTC_WRAPPER")
        .status()
        .map_err(|e| format!("cargo +{TOOLCHAIN} rustdoc: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo +{TOOLCHAIN} rustdoc failed ({status}); if the toolchain or its {TARGET} \
             std is missing: `rustup toolchain install {TOOLCHAIN} --profile minimal --target \
             {TARGET}`"
        ));
    }
    Ok(target_dir.join(TARGET).join("doc").join("ep9_vm.json"))
}

/// The lines of `want` that `got` lacks (`- `) and of `got` that `want` lacks (`+ `), in
/// line order. Both are sorted listings, so a set difference is the diff.
pub(crate) fn diff(want: &[String], got: &[String]) -> Vec<String> {
    let w: HashSet<&String> = want.iter().collect();
    let g: HashSet<&String> = got.iter().collect();
    let mut out: Vec<(String, char)> = want
        .iter()
        .filter(|l| !g.contains(l))
        .map(|l| (l.clone(), '-'))
        .chain(
            got.iter()
                .filter(|l| !w.contains(l))
                .map(|l| (l.clone(), '+')),
        )
        .collect();
    out.sort();
    out.into_iter().map(|(l, c)| format!("{c} {l}")).collect()
}

// ---- the listing ----------------------------------------------------------------------------

fn id_of(j: &Json) -> Option<u64> {
    j.num()
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as u64)
}

fn inner_kind(item: &Json) -> Option<(&str, &Json)> {
    match item.get("inner")? {
        Json::Obj(m) => m.first().map(|(k, v)| (k.as_str(), v)),
        Json::Str(s) => Some((s.as_str(), &Json::Null)),
        _ => None,
    }
}

struct Doc<'a> {
    index: HashMap<u64, &'a Json>,
    paths: HashMap<u64, &'a Json>,
    /// Names that two local type items share: such a type prints by its full path.
    ambiguous: HashSet<String>,
}

/// The listing of one rustdoc JSON text: sorted, deduped lines.
pub(crate) fn listing(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    let fv = root
        .get("format_version")
        .and_then(Json::num)
        .ok_or("rustdoc JSON: no format_version")?;
    if fv != FORMAT_VERSION {
        return Err(format!(
            "rustdoc JSON format_version {fv}; this reader reads {FORMAT_VERSION} (rustdoc of \
             {TOOLCHAIN}): a new nightly pin needs a reader update"
        ));
    }
    let map = |key: &str| -> Result<HashMap<u64, &Json>, String> {
        match root.get(key) {
            Some(Json::Obj(m)) => m
                .iter()
                .map(|(k, v)| {
                    k.parse::<u64>()
                        .map(|k| (k, v))
                        .map_err(|_| format!("rustdoc JSON: {key} key {k:?} is not an id"))
                })
                .collect(),
            _ => Err(format!("rustdoc JSON: no {key} object")),
        }
    };
    let index = map("index")?;
    let paths = map("paths")?;
    let mut by_name: HashMap<String, HashSet<u64>> = HashMap::new();
    for (id, p) in &paths {
        let local = p.get("crate_id").and_then(id_of) == Some(0);
        let kind = p.get("kind").and_then(Json::str).unwrap_or("");
        let typeish = matches!(kind, "struct" | "enum" | "union" | "trait" | "type_alias");
        if let (true, true, Some(name)) = (local, typeish, last_segment(p)) {
            by_name.entry(name).or_default().insert(*id);
        }
    }
    let ambiguous = by_name
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(n, _)| n)
        .collect();
    let doc = Doc {
        index,
        paths,
        ambiguous,
    };
    let root_id = root
        .get("root")
        .and_then(id_of)
        .ok_or("rustdoc JSON: no root id")?;
    let crate_name = doc
        .index
        .get(&root_id)
        .and_then(|i| i.get("name"))
        .and_then(Json::str)
        .ok_or("rustdoc JSON: the root item has no name")?
        .to_string();
    // Pass 1: every public path of every item.
    let mut public: Vec<(String, u64)> = Vec::new();
    let mut lines: BTreeSet<String> = BTreeSet::new();
    let mut seen_mods = HashSet::new();
    doc.walk(
        root_id,
        &crate_name,
        &mut public,
        &mut lines,
        &mut seen_mods,
    )?;
    // Pass 2: each item's line at each path; its members once, at its first path.
    public.sort();
    let mut canon: HashMap<u64, String> = HashMap::new();
    for (path, id) in &public {
        canon.entry(*id).or_insert_with(|| path.clone());
    }
    for (path, id) in &public {
        if let Some(item) = doc.index.get(id) {
            lines.insert(format!("{path} :: {}", doc.item_decl(item)));
        }
    }
    let mut impls: Vec<(u64, String)> = Vec::new();
    let listed: HashSet<u64> = canon.keys().copied().collect();
    let mut ids: Vec<(&String, &u64)> = canon.iter().map(|(id, p)| (p, id)).collect();
    ids.sort();
    for (path, id) in ids {
        let Some(item) = doc.index.get(id) else {
            continue;
        };
        doc.members(item, path, &listed, &mut lines, &mut impls);
    }
    // Impls: once each, under the listed self type, else under the trait.
    let mut done = HashSet::new();
    impls.sort();
    for (impl_id, owner) in impls {
        if !done.insert(impl_id) {
            continue;
        }
        let Some(item) = doc.index.get(&impl_id) else {
            continue;
        };
        let Some(("impl", imp)) = inner_kind(item) else {
            continue;
        };
        let owner = imp
            .path(&["for", "resolved_path", "id"])
            .and_then(id_of)
            .and_then(|i| canon.get(&i))
            .cloned()
            .unwrap_or(owner);
        doc.impl_lines(imp, &owner, &mut lines);
    }
    // An impl of this crate that no listed item links, such as `impl From<Local<'s>> for
    // Value` (a trait and a self type of other crates; H20 class (c)): under the crate path.
    let mut rest: Vec<u64> = doc
        .index
        .iter()
        .filter(|(id, it)| {
            !done.contains(*id)
                && it.get("crate_id").and_then(id_of) == Some(0)
                && matches!(inner_kind(it), Some(("impl", _)))
        })
        .map(|(id, _)| *id)
        .collect();
    rest.sort();
    for id in rest {
        if let Some(("impl", imp)) = doc.index.get(&id).and_then(|i| inner_kind(i)) {
            doc.impl_lines(imp, &crate_name, &mut lines);
        }
    }
    Ok(lines.into_iter().collect())
}

/// The `extern "..." ` part of a function or fn-pointer header (empty for the Rust ABI). An
/// ABI object with `unwind: true` (rustdoc's `C { unwind: true }`) prints with `-unwind`, as
/// the source spells it: `extern "C-unwind"` (M7-P5 Codex T4: the unwind flag is API).
fn extern_abi(hd: Option<&Json>) -> String {
    match hd.and_then(|h| h.get("abi")) {
        Some(Json::Str(s)) if s == "Rust" => String::new(),
        Some(Json::Str(s)) => format!("extern {s:?} "),
        Some(Json::Obj(m)) => {
            let Some((abi, v)) = m.first() else {
                return "extern \"?\" ".into();
            };
            let abi = if v.get("unwind").and_then(Json::bool) == Some(true) {
                format!("{abi}-unwind")
            } else {
                abi.clone()
            };
            format!("extern {abi:?} ")
        }
        _ => String::new(),
    }
}

fn last_segment(p: &Json) -> Option<String> {
    p.get("path")
        .and_then(Json::arr)
        .and_then(|a| a.last())
        .and_then(Json::str)
        .map(str::to_string)
}

fn join_path(p: &Json) -> Option<String> {
    let a = p.get("path").and_then(Json::arr)?;
    let parts: Option<Vec<&str>> = a.iter().map(Json::str).collect();
    parts.map(|v| v.join("::"))
}

impl Doc<'_> {
    fn name(&self, item: &Json) -> String {
        item.get("name")
            .and_then(Json::str)
            .unwrap_or("_")
            .to_string()
    }

    /// Pass 1: the items of module `id` under `prefix`.
    fn walk(
        &self,
        id: u64,
        prefix: &str,
        public: &mut Vec<(String, u64)>,
        lines: &mut BTreeSet<String>,
        seen: &mut HashSet<(u64, String)>,
    ) -> Result<(), String> {
        if !seen.insert((id, prefix.to_string())) {
            return Ok(());
        }
        let Some(item) = self.index.get(&id) else {
            return Ok(());
        };
        let Some(("module", m)) = inner_kind(item) else {
            return Err(format!("rustdoc JSON: item {id} is not a module"));
        };
        for child in m.get("items").and_then(Json::arr).unwrap_or(&[]) {
            let Some(cid) = id_of(child) else { continue };
            let Some(c) = self.index.get(&cid) else {
                continue;
            };
            if c.get("visibility").and_then(Json::str) != Some("public") {
                continue;
            }
            match inner_kind(c) {
                Some(("use", u)) => {
                    let name = u.get("name").and_then(Json::str).unwrap_or("_");
                    let source = u.get("source").and_then(Json::str).unwrap_or("?");
                    let target = u.get("id").and_then(id_of);
                    let glob = u.get("is_glob").and_then(Json::bool) == Some(true);
                    let local = target.filter(|t| self.index.contains_key(t));
                    match (glob, local) {
                        (true, Some(t)) if matches!(self.kind(t), Some("module")) => {
                            self.walk(t, prefix, public, lines, seen)?
                        }
                        (true, _) => {
                            let src = target
                                .and_then(|t| self.paths.get(&t))
                                .and_then(|p| join_path(p))
                                .unwrap_or_else(|| source.to_string());
                            lines.insert(format!("{prefix}::* :: pub use {src}::*"));
                            // A glob of a local enum puts its variants here (M7-P5 Codex T3).
                            if let Some(t) = local.filter(|t| self.kind(*t) == Some("enum")) {
                                for v in self.variants(t) {
                                    let vi = self.index[&v];
                                    public.push((format!("{prefix}::{}", self.name(vi)), v));
                                }
                            }
                        }
                        (false, Some(t)) => {
                            let path = format!("{prefix}::{name}");
                            public.push((path.clone(), t));
                            if matches!(self.kind(t), Some("module")) {
                                self.walk(t, &path, public, lines, seen)?;
                            }
                        }
                        (false, None) => {
                            let src = target
                                .and_then(|t| self.paths.get(&t))
                                .and_then(|p| join_path(p))
                                .unwrap_or_else(|| source.to_string());
                            lines.insert(format!("{prefix}::{name} :: pub use {src}"));
                        }
                    }
                }
                Some(("module", _)) => {
                    let path = format!("{prefix}::{}", self.name(c));
                    public.push((path.clone(), cid));
                    self.walk(cid, &path, public, lines, seen)?;
                }
                Some(_) => public.push((format!("{prefix}::{}", self.name(c)), cid)),
                None => {}
            }
        }
        Ok(())
    }

    /// The variant ids of the local enum `id` that the index has.
    fn variants(&self, id: u64) -> Vec<u64> {
        self.index
            .get(&id)
            .and_then(|e| e.path(&["inner", "enum", "variants"]))
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .filter_map(id_of)
            .filter(|v| self.index.contains_key(v))
            .collect()
    }

    fn kind(&self, id: u64) -> Option<&str> {
        self.index
            .get(&id)
            .and_then(|i| inner_kind(i))
            .map(|(k, _)| k)
    }

    /// The attributes that are API: `#[repr]`, `#[non_exhaustive]`, `#[doc(hidden)]`.
    fn attrs(&self, item: &Json) -> String {
        let mut out = String::new();
        for a in item.get("attrs").and_then(Json::arr).unwrap_or(&[]) {
            match a {
                Json::Str(s) if s == "non_exhaustive" => out.push_str("#[non_exhaustive] "),
                Json::Obj(_) => {
                    if let Some(r) = a.get("repr") {
                        let mut parts = Vec::new();
                        if let Some(k) = r.get("kind").and_then(Json::str)
                            && k != "rust"
                        {
                            parts.push(k.to_string());
                        }
                        for key in ["int", "align", "packed"] {
                            match r.get(key) {
                                Some(Json::Str(s)) => parts.push(s.clone()),
                                Some(Json::Num(n)) => parts.push(format!("{key}({n})")),
                                _ => {}
                            }
                        }
                        out.push_str(&format!("#[repr({})] ", parts.join(", ")));
                    } else if a.get("other").and_then(Json::str) == Some("#[doc(hidden)]") {
                        out.push_str("#[doc(hidden)] ");
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The one-line declaration of an item.
    fn item_decl(&self, item: &Json) -> String {
        let name = self.name(item);
        let at = self.attrs(item);
        let Some((kind, x)) = inner_kind(item) else {
            return format!("{at}? {name}");
        };
        let g = |x: &Json| self.generics(x.get("generics"));
        let w = |x: &Json| self.where_clause(x.get("generics"));
        match kind {
            "module" => format!("{at}mod {name}"),
            "struct" => {
                let shape = match x.get("kind") {
                    Some(Json::Str(s)) if s == "unit" => ";".to_string(),
                    Some(k) if k.get("tuple").is_some() => {
                        let f = k.get("tuple").and_then(Json::arr).unwrap_or(&[]);
                        let f: Vec<String> = f
                            .iter()
                            .map(|f| match id_of(f).and_then(|i| self.index.get(&i)) {
                                Some(fi) => self.ty(fi.path(&["inner", "struct_field"])),
                                None => "_".into(),
                            })
                            .collect();
                        format!("({})", f.join(", "))
                    }
                    Some(k) => {
                        let s = k
                            .path(&["plain", "has_stripped_fields"])
                            .and_then(Json::bool);
                        if s == Some(true) {
                            " { .. }".into()
                        } else {
                            " {}".into()
                        }
                    }
                    None => String::new(),
                };
                format!("{at}struct {name}{}{}{shape}", g(x), w(x))
            }
            "union" => format!("{at}union {name}{}{}", g(x), w(x)),
            "enum" => {
                let s = x.get("has_stripped_variants").and_then(Json::bool) == Some(true);
                let s = if s { " (stripped variants)" } else { "" };
                format!("{at}enum {name}{}{}{s}", g(x), w(x))
            }
            "trait" => {
                let mut h = String::new();
                if x.get("is_unsafe").and_then(Json::bool) == Some(true) {
                    h.push_str("unsafe ");
                }
                if x.get("is_auto").and_then(Json::bool) == Some(true) {
                    h.push_str("auto ");
                }
                let b = self.bounds(x.get("bounds"));
                let b = if b.is_empty() { b } else { format!(": {b}") };
                format!("{at}{h}trait {name}{}{b}{}", g(x), w(x))
            }
            "trait_alias" => format!("{at}trait {name}{} = ..", g(x)),
            "function" => format!("{at}{}", self.function(&name, x)),
            "constant" => format!("{at}const {name}: {}", self.ty(x.get("type"))),
            "static" => {
                let m = if x.get("is_mutable").and_then(Json::bool) == Some(true) {
                    "mut "
                } else {
                    ""
                };
                let u = if x.get("is_unsafe").and_then(Json::bool) == Some(true) {
                    "unsafe "
                } else {
                    ""
                };
                format!("{at}{u}static {m}{name}: {}", self.ty(x.get("type")))
            }
            "type_alias" => format!(
                "{at}type {name}{}{} = {}",
                g(x),
                w(x),
                self.ty(x.get("type"))
            ),
            "macro" => format!("{at}macro {name}!"),
            "proc_macro" => format!("{at}proc macro {name}"),
            "assoc_type" => {
                let b = self.bounds(x.get("bounds"));
                let b = if b.is_empty() { b } else { format!(": {b}") };
                let d = match x.get("type") {
                    Some(Json::Null) | None => String::new(),
                    t => format!(" = {}", self.ty(t)),
                };
                format!("{at}type {name}{}{b}{}{d}", g(x), w(x))
            }
            "assoc_const" => format!("{at}const {name}: {}", self.ty(x.get("type"))),
            "struct_field" => format!("{at}field {name}: {}", self.ty(Some(x))),
            "variant" => {
                let fields = match x.get("kind") {
                    Some(k) if k.get("tuple").is_some() => {
                        let f = k.get("tuple").and_then(Json::arr).unwrap_or(&[]);
                        let f: Vec<String> = f
                            .iter()
                            .map(|f| match id_of(f).and_then(|i| self.index.get(&i)) {
                                Some(fi) => self.ty(fi.path(&["inner", "struct_field"])),
                                None => "_".into(),
                            })
                            .collect();
                        format!("({})", f.join(", "))
                    }
                    Some(k) if k.get("struct").is_some() => {
                        let s = k.get("struct").unwrap();
                        let mut f: Vec<String> = s
                            .get("fields")
                            .and_then(Json::arr)
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|f| id_of(f).and_then(|i| self.index.get(&i)))
                            .map(|fi| {
                                format!(
                                    "{}: {}",
                                    self.name(fi),
                                    self.ty(fi.path(&["inner", "struct_field"]))
                                )
                            })
                            .collect();
                        if s.get("has_stripped_fields").and_then(Json::bool) == Some(true) {
                            f.push("..".into());
                        }
                        format!(" {{ {} }}", f.join(", "))
                    }
                    _ => String::new(),
                };
                let disc = x
                    .path(&["discriminant", "expr"])
                    .and_then(Json::str)
                    .map(|e| format!(" = {e}"))
                    .unwrap_or_default();
                format!("{at}variant {name}{fields}{disc}")
            }
            other => format!("{at}{other} {name}"),
        }
    }

    /// The member lines of an item at its first path, and its impls for the impl pass.
    /// `listed`: the items with a public path.
    fn members(
        &self,
        item: &Json,
        path: &str,
        listed: &HashSet<u64>,
        lines: &mut BTreeSet<String>,
        impls: &mut Vec<(u64, String)>,
    ) {
        let Some((kind, x)) = inner_kind(item) else {
            return;
        };
        let ids = |key: &[&str]| -> Vec<u64> {
            x.path(key)
                .and_then(Json::arr)
                .unwrap_or(&[])
                .iter()
                .filter_map(id_of)
                .collect()
        };
        let mut sub: Vec<u64> = Vec::new();
        match kind {
            "struct" => sub.extend(ids(&["kind", "plain", "fields"])),
            "union" => sub.extend(ids(&["fields"])),
            "enum" => sub.extend(ids(&["variants"])),
            "trait" => {
                sub.extend(ids(&["items"]));
                for i in ids(&["implementations"]) {
                    impls.push((i, path.to_string()));
                }
                self.supertrait_lines(x, path, listed, &mut HashSet::new(), lines);
            }
            _ => {}
        }
        for s in sub {
            if let Some(si) = self.index.get(&s) {
                // Fields of a struct or union are listed when public; variants and trait
                // items always (their visibility is the item's).
                let field = matches!(inner_kind(si), Some(("struct_field", _)));
                if field && si.get("visibility").and_then(Json::str) != Some("public") {
                    continue;
                }
                lines.insert(format!("{path} :: {}", self.item_decl(si)));
            }
        }
        if matches!(kind, "struct" | "enum" | "union" | "primitive") {
            for i in ids(&["impls"]) {
                impls.push((i, path.to_string()));
            }
        }
    }

    /// The items of each local supertrait of the trait `tr` that has no public path (a sealed
    /// trait in a private module), and of its supertraits: a caller of the listed trait calls
    /// them, so they are API (M7-P5 Codex T1). One line per item, under the listed trait:
    /// `<path> :: supertrait <Name> :: <item>`. A listed supertrait has its own lines. A
    /// supertrait is a bound (`trait T: S`) or a bound of `Self` in the where clause
    /// (`trait T where Self: S`; rustdoc keeps it in `generics.where_predicates`; M7-P5 Codex
    /// R3 finding 2).
    fn supertrait_lines(
        &self,
        tr: &Json,
        path: &str,
        listed: &HashSet<u64>,
        seen: &mut HashSet<u64>,
        lines: &mut BTreeSet<String>,
    ) {
        let where_self = tr
            .path(&["generics", "where_predicates"])
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| p.get("bound_predicate"))
            .filter(|p| p.path(&["type", "generic"]).and_then(Json::str) == Some("Self"))
            .flat_map(|p| p.get("bounds").and_then(Json::arr).unwrap_or(&[]));
        let own = tr.get("bounds").and_then(Json::arr).unwrap_or(&[]);
        for b in own.iter().chain(where_self) {
            let Some(t) = b.path(&["trait_bound", "trait"]) else {
                continue;
            };
            let Some(id) = t.get("id").and_then(id_of) else {
                continue;
            };
            if listed.contains(&id) || !seen.insert(id) {
                continue;
            }
            let Some(("trait", sx)) = self.index.get(&id).and_then(|i| inner_kind(i)) else {
                continue; // not a local trait
            };
            let name = self.path_ty(t);
            for i in sx.get("items").and_then(Json::arr).unwrap_or(&[]) {
                if let Some(it) = id_of(i).and_then(|i| self.index.get(&i)) {
                    let decl = self.item_decl(it);
                    lines.insert(format!("{path} :: supertrait {name} :: {decl}"));
                }
            }
            self.supertrait_lines(sx, path, listed, seen, lines);
        }
    }

    /// An impl's lines: its head, and its listed items.
    fn impl_lines(&self, imp: &Json, owner: &str, lines: &mut BTreeSet<String>) {
        if !matches!(imp.get("blanket_impl"), None | Some(Json::Null)) {
            return; // a blanket impl of another crate, instantiated for this type
        }
        let synthetic = imp.get("is_synthetic").and_then(Json::bool) == Some(true);
        let tr = imp.get("trait").filter(|t| !matches!(t, Json::Null));
        if synthetic {
            let name = tr
                .and_then(|t| t.get("path"))
                .and_then(Json::str)
                .unwrap_or("");
            let last = name.rsplit("::").next().unwrap_or(name);
            if !AUTO_LISTED.contains(&last) {
                return;
            }
        }
        let mut head = String::new();
        if imp.get("is_unsafe").and_then(Json::bool) == Some(true) {
            head.push_str("unsafe ");
        }
        head.push_str("impl");
        head.push_str(&self.generics(imp.get("generics")));
        head.push(' ');
        if let Some(t) = tr {
            if imp.get("is_negative").and_then(Json::bool) == Some(true) {
                head.push('!');
            }
            head.push_str(&self.path_ty(t));
            head.push_str(" for ");
        }
        head.push_str(&self.ty(imp.get("for")));
        head.push_str(&self.where_clause(imp.get("generics")));
        if synthetic {
            head.push_str(" (auto)");
        }
        lines.insert(format!("{owner} :: {head}"));
        for i in imp.get("items").and_then(Json::arr).unwrap_or(&[]) {
            let Some(it) = id_of(i).and_then(|i| self.index.get(&i)) else {
                continue;
            };
            let k = inner_kind(it).map(|(k, _)| k);
            let listed = if tr.is_some() {
                // A trait impl: its associated types and consts (the trait gives the fns).
                matches!(k, Some("assoc_type" | "assoc_const"))
            } else {
                it.get("visibility").and_then(Json::str) == Some("public")
            };
            if listed {
                lines.insert(format!("{owner} :: {head} :: {}", self.item_decl(it)));
            }
        }
    }

    fn function(&self, name: &str, f: &Json) -> String {
        let mut h = String::new();
        let hd = f.get("header");
        let flag = |k: &str| hd.and_then(|h| h.get(k)).and_then(Json::bool) == Some(true);
        if flag("is_const") {
            h.push_str("const ");
        }
        if flag("is_async") {
            h.push_str("async ");
        }
        if flag("is_unsafe") {
            h.push_str("unsafe ");
        }
        h.push_str(&extern_abi(hd));
        let sig = f.get("sig");
        let mut args: Vec<String> = Vec::new();
        for inp in sig
            .and_then(|s| s.get("inputs"))
            .and_then(Json::arr)
            .unwrap_or(&[])
        {
            let Some([n, t]) = inp.arr() else { continue };
            if n.str() == Some("self") {
                args.push(self.self_arg(t));
            } else {
                args.push(self.ty(Some(t)));
            }
        }
        if sig
            .and_then(|s| s.get("is_c_variadic"))
            .and_then(Json::bool)
            == Some(true)
        {
            args.push("...".into());
        }
        let ret = match sig.and_then(|s| s.get("output")) {
            None | Some(Json::Null) => String::new(),
            t => format!(" -> {}", self.ty(t)),
        };
        format!(
            "{h}fn {name}{}({}){ret}{}",
            self.generics(f.get("generics")),
            args.join(", "),
            self.where_clause(f.get("generics"))
        )
    }

    fn self_arg(&self, t: &Json) -> String {
        if t.get("generic").and_then(Json::str) == Some("Self") {
            return "self".into();
        }
        if let Some(r) = t.get("borrowed_ref")
            && r.path(&["type", "generic"]).and_then(Json::str) == Some("Self")
        {
            let lt = r
                .get("lifetime")
                .and_then(Json::str)
                .map(|l| format!("{l} "))
                .unwrap_or_default();
            let m = if r.get("is_mutable").and_then(Json::bool) == Some(true) {
                "mut "
            } else {
                ""
            };
            return format!("&{lt}{m}self");
        }
        format!("self: {}", self.ty(Some(t)))
    }

    /// `<..>` of a generics object, without the synthetic params of `impl Trait` arguments.
    fn generics(&self, g: Option<&Json>) -> String {
        let mut out = Vec::new();
        for p in g
            .and_then(|g| g.get("params"))
            .and_then(Json::arr)
            .unwrap_or(&[])
        {
            let name = p.get("name").and_then(Json::str).unwrap_or("_");
            let Some(k) = p.get("kind") else { continue };
            if let Some(lt) = k.get("lifetime") {
                let o: Vec<&str> = lt
                    .get("outlives")
                    .and_then(Json::arr)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Json::str)
                    .collect();
                if o.is_empty() {
                    out.push(name.to_string());
                } else {
                    out.push(format!("{name}: {}", o.join(" + ")));
                }
            } else if let Some(t) = k.get("type") {
                if t.get("is_synthetic").and_then(Json::bool) == Some(true) {
                    continue;
                }
                let b = self.bounds(t.get("bounds"));
                let mut s = name.to_string();
                if !b.is_empty() {
                    s.push_str(&format!(": {b}"));
                }
                if let Some(d) = t.get("default").filter(|d| !matches!(d, Json::Null)) {
                    s.push_str(&format!(" = {}", self.ty(Some(d))));
                }
                out.push(s);
            } else if let Some(c) = k.get("const") {
                let mut s = format!("const {name}: {}", self.ty(c.get("type")));
                if let Some(d) = c.get("default").and_then(Json::str) {
                    s.push_str(&format!(" = {d}"));
                }
                out.push(s);
            }
        }
        if out.is_empty() {
            String::new()
        } else {
            format!("<{}>", out.join(", "))
        }
    }

    fn where_clause(&self, g: Option<&Json>) -> String {
        let mut out = Vec::new();
        for p in g
            .and_then(|g| g.get("where_predicates"))
            .and_then(Json::arr)
            .unwrap_or(&[])
        {
            if let Some(b) = p.get("bound_predicate") {
                let hr = self.hrtb(b.get("generic_params"));
                out.push(format!(
                    "{hr}{}: {}",
                    self.ty(b.get("type")),
                    self.bounds(b.get("bounds"))
                ));
            } else if let Some(l) = p.get("lifetime_predicate") {
                let o: Vec<&str> = l
                    .get("outlives")
                    .and_then(Json::arr)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Json::str)
                    .collect();
                out.push(format!(
                    "{}: {}",
                    l.get("lifetime").and_then(Json::str).unwrap_or("'_"),
                    o.join(" + ")
                ));
            } else if let Some(e) = p.get("eq_predicate") {
                out.push(format!(
                    "{} == {}",
                    self.ty(e.get("lhs")),
                    self.term(e.get("rhs"))
                ));
            }
        }
        if out.is_empty() {
            String::new()
        } else {
            format!(" where {}", out.join(", "))
        }
    }

    fn hrtb(&self, params: Option<&Json>) -> String {
        let ps: Vec<&str> = params
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| p.get("name").and_then(Json::str))
            .collect();
        if ps.is_empty() {
            String::new()
        } else {
            format!("for<{}> ", ps.join(", "))
        }
    }

    fn bounds(&self, b: Option<&Json>) -> String {
        let v: Vec<String> = b
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .map(|b| self.bound(b))
            .collect();
        v.join(" + ")
    }

    fn bound(&self, b: &Json) -> String {
        if let Some(t) = b.get("trait_bound") {
            let m = match t.get("modifier").and_then(Json::str) {
                Some("maybe") => "?",
                Some("maybe_const") => "~const ",
                _ => "",
            };
            let hr = self.hrtb(t.get("generic_params"));
            let tr = t.get("trait").map(|p| self.path_ty(p)).unwrap_or_default();
            return format!("{hr}{m}{tr}");
        }
        if let Some(l) = b.get("outlives").and_then(Json::str) {
            return l.to_string();
        }
        if let Some(u) = b.get("use").and_then(Json::arr) {
            let v: Vec<String> = u
                .iter()
                .map(|x| match x {
                    Json::Str(s) => s.clone(),
                    o => o
                        .get("lifetime")
                        .or_else(|| o.get("param"))
                        .and_then(Json::str)
                        .unwrap_or("_")
                        .to_string(),
                })
                .collect();
            return format!("use<{}>", v.join(", "));
        }
        "?bound".into()
    }

    fn term(&self, t: Option<&Json>) -> String {
        match t {
            Some(t) if t.get("type").is_some() => self.ty(t.get("type")),
            Some(t) if t.get("constant").is_some() => t
                .path(&["constant", "expr"])
                .and_then(Json::str)
                .unwrap_or("_")
                .to_string(),
            _ => "_".into(),
        }
    }

    /// A path type (`{path, id, args}`): this crate's items by name (by full path if the
    /// name is [`Doc::ambiguous`]), other crates' items by full path.
    fn path_ty(&self, p: &Json) -> String {
        let written = p.get("path").and_then(Json::str).unwrap_or("?");
        let name = match p.get("id").and_then(id_of).and_then(|i| self.paths.get(&i)) {
            Some(s) => {
                let local = s.get("crate_id").and_then(id_of) == Some(0);
                match (local, last_segment(s)) {
                    (true, Some(n)) if !self.ambiguous.contains(&n) => n,
                    _ => join_path(s).unwrap_or_else(|| written.to_string()),
                }
            }
            None => written.to_string(),
        };
        format!("{name}{}", self.generic_args(p.get("args")))
    }

    fn generic_args(&self, a: Option<&Json>) -> String {
        let Some(a) = a.filter(|a| !matches!(a, Json::Null)) else {
            return String::new();
        };
        if let Some(ab) = a.get("angle_bracketed") {
            let mut v: Vec<String> = Vec::new();
            for x in ab.get("args").and_then(Json::arr).unwrap_or(&[]) {
                v.push(match x {
                    Json::Str(s) if s == "infer" => "_".into(),
                    o if o.get("lifetime").is_some() => o
                        .get("lifetime")
                        .and_then(Json::str)
                        .unwrap_or("'_")
                        .to_string(),
                    o if o.get("type").is_some() => self.ty(o.get("type")),
                    o if o.get("const").is_some() => o
                        .path(&["const", "expr"])
                        .and_then(Json::str)
                        .unwrap_or("_")
                        .to_string(),
                    _ => "?".into(),
                });
            }
            for c in ab.get("constraints").and_then(Json::arr).unwrap_or(&[]) {
                let n = c.get("name").and_then(Json::str).unwrap_or("?");
                let ga = self.generic_args(c.get("args"));
                match c.get("binding") {
                    Some(b) if b.get("equality").is_some() => {
                        v.push(format!("{n}{ga} = {}", self.term(b.get("equality"))));
                    }
                    Some(b) if b.get("constraint").is_some() => {
                        v.push(format!("{n}{ga}: {}", self.bounds(b.get("constraint"))));
                    }
                    _ => v.push(n.to_string()),
                }
            }
            if v.is_empty() {
                return String::new();
            }
            return format!("<{}>", v.join(", "));
        }
        if let Some(pa) = a.get("parenthesized") {
            let ins: Vec<String> = pa
                .get("inputs")
                .and_then(Json::arr)
                .unwrap_or(&[])
                .iter()
                .map(|t| self.ty(Some(t)))
                .collect();
            let out = match pa.get("output") {
                None | Some(Json::Null) => String::new(),
                t => format!(" -> {}", self.ty(t)),
            };
            return format!("({}){out}", ins.join(", "));
        }
        "(..)".into()
    }

    fn ty(&self, t: Option<&Json>) -> String {
        let Some(t) = t else { return "?".into() };
        let Json::Obj(m) = t else {
            return match t {
                Json::Str(s) if s == "infer" => "_".into(),
                _ => "?".into(),
            };
        };
        let Some((k, v)) = m.first() else {
            return "?".into();
        };
        match k.as_str() {
            "resolved_path" => self.path_ty(v),
            "generic" | "primitive" => v.str().unwrap_or("?").to_string(),
            "tuple" => {
                let e: Vec<String> = v
                    .arr()
                    .unwrap_or(&[])
                    .iter()
                    .map(|x| self.ty(Some(x)))
                    .collect();
                if e.len() == 1 {
                    format!("({},)", e[0])
                } else {
                    format!("({})", e.join(", "))
                }
            }
            "slice" => format!("[{}]", self.ty(Some(v))),
            "array" => format!(
                "[{}; {}]",
                self.ty(v.get("type")),
                v.get("len").and_then(Json::str).unwrap_or("_")
            ),
            "pat" => self.ty(v.get("type")),
            "infer" => "_".into(),
            "raw_pointer" => {
                let m = if v.get("is_mutable").and_then(Json::bool) == Some(true) {
                    "mut"
                } else {
                    "const"
                };
                format!("*{m} {}", self.ty(v.get("type")))
            }
            "borrowed_ref" => {
                let lt = v
                    .get("lifetime")
                    .and_then(Json::str)
                    .map(|l| format!("{l} "))
                    .unwrap_or_default();
                let m = if v.get("is_mutable").and_then(Json::bool) == Some(true) {
                    "mut "
                } else {
                    ""
                };
                format!("&{lt}{m}{}", self.ty(v.get("type")))
            }
            "impl_trait" => format!("impl {}", self.bounds(Some(v))),
            "dyn_trait" => {
                let mut parts: Vec<String> = v
                    .get("traits")
                    .and_then(Json::arr)
                    .unwrap_or(&[])
                    .iter()
                    .map(|p| {
                        format!(
                            "{}{}",
                            self.hrtb(p.get("generic_params")),
                            p.get("trait").map(|t| self.path_ty(t)).unwrap_or_default()
                        )
                    })
                    .collect();
                if let Some(l) = v.get("lifetime").and_then(Json::str) {
                    parts.push(l.to_string());
                }
                format!("dyn {}", parts.join(" + "))
            }
            "function_pointer" => {
                let hr = self.hrtb(v.get("generic_params"));
                let hd = v.get("header");
                let mut h = String::new();
                if hd.and_then(|h| h.get("is_unsafe")).and_then(Json::bool) == Some(true) {
                    h.push_str("unsafe ");
                }
                h.push_str(&extern_abi(hd));
                let sig = v.get("sig");
                let args: Vec<String> = sig
                    .and_then(|s| s.get("inputs"))
                    .and_then(Json::arr)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|i| i.arr().and_then(|a| a.get(1)))
                    .map(|t| self.ty(Some(t)))
                    .collect();
                let ret = match sig.and_then(|s| s.get("output")) {
                    None | Some(Json::Null) => String::new(),
                    t => format!(" -> {}", self.ty(t)),
                };
                format!("{hr}{h}fn({}){ret}", args.join(", "))
            }
            "qualified_path" => {
                let st = self.ty(v.get("self_type"));
                let tr = v
                    .get("trait")
                    .filter(|t| !matches!(t, Json::Null))
                    .map(|t| self.path_ty(t));
                let n = v.get("name").and_then(Json::str).unwrap_or("?");
                let ga = self.generic_args(v.get("args"));
                match tr {
                    Some(tr) => format!("<{st} as {tr}>::{n}{ga}"),
                    None => format!("{st}::{n}{ga}"),
                }
            }
            other => format!("?{other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{diff, listing};

    /// A rustdoc JSON text (format 61) with one module tree: the root re-exports `inner::Pub`
    /// under a new name, has a pub fn with a self type and a generic, and an enum.
    fn fixture(extra_fn: bool) -> String {
        let f = if extra_fn { ", 7" } else { "" };
        format!(
            r#"{{"root": 1, "format_version": 61, "index": {{
  "1": {{"name": "k", "visibility": "public", "attrs": [], "inner": {{"module": {{"items": [2, 3, 4{f}]}}}}}},
  "2": {{"name": null, "visibility": "public", "attrs": [], "inner": {{"use": {{"source": "inner::Pub", "name": "Renamed", "id": 5, "is_glob": false}}}}}},
  "3": {{"name": "E", "visibility": "public", "attrs": [{{"repr": {{"kind": "rust", "int": "u8"}}}}], "inner": {{"enum": {{"generics": {{"params": [], "where_predicates": []}}, "has_stripped_variants": false, "variants": [8], "impls": []}}}}}},
  "4": {{"name": "f", "visibility": "public", "attrs": [], "inner": {{"function": {{"sig": {{"inputs": [["x", {{"borrowed_ref": {{"lifetime": "'a", "is_mutable": false, "type": {{"resolved_path": {{"path": "inner::Pub", "id": 5, "args": null}}}}}}}}]], "output": {{"primitive": "u32"}}, "is_c_variadic": false}}, "generics": {{"params": [{{"name": "'a", "kind": {{"lifetime": {{"outlives": []}}}}}}], "where_predicates": []}}, "header": {{"is_const": false, "is_unsafe": true, "is_async": false, "abi": "Rust"}}, "has_body": true}}}}}},
  "5": {{"name": "Pub", "visibility": "public", "attrs": [], "inner": {{"struct": {{"kind": {{"plain": {{"fields": [6], "has_stripped_fields": true}}}}, "generics": {{"params": [], "where_predicates": []}}, "impls": [9]}}}}}},
  "6": {{"name": "a", "visibility": "public", "attrs": [], "inner": {{"struct_field": {{"primitive": "i32"}}}}}},
  "7": {{"name": "g", "visibility": "public", "attrs": [], "inner": {{"function": {{"sig": {{"inputs": [], "output": null, "is_c_variadic": false}}, "generics": {{"params": [], "where_predicates": []}}, "header": {{"is_const": true, "is_unsafe": false, "is_async": false, "abi": "Rust"}}, "has_body": true}}}}}},
  "8": {{"name": "A", "visibility": "default", "attrs": [], "inner": {{"variant": {{"kind": "plain", "discriminant": {{"expr": "3", "value": "3"}}}}}}}},
  "9": {{"name": null, "visibility": "default", "attrs": [], "inner": {{"impl": {{"is_unsafe": false, "generics": {{"params": [], "where_predicates": []}}, "trait": null, "for": {{"resolved_path": {{"path": "Pub", "id": 5, "args": null}}}}, "items": [10, 11], "is_negative": false, "is_synthetic": false, "blanket_impl": null}}}}}},
  "10": {{"name": "m", "visibility": "public", "attrs": [], "inner": {{"function": {{"sig": {{"inputs": [["self", {{"borrowed_ref": {{"lifetime": null, "is_mutable": true, "type": {{"generic": "Self"}}}}}}]], "output": null, "is_c_variadic": false}}, "generics": {{"params": [], "where_predicates": []}}, "header": {{"is_const": false, "is_unsafe": false, "is_async": false, "abi": "Rust"}}, "has_body": true}}}}}},
  "11": {{"name": "private", "visibility": "crate", "attrs": [], "inner": {{"function": {{"sig": {{"inputs": [], "output": null, "is_c_variadic": false}}, "generics": {{"params": [], "where_predicates": []}}, "header": {{"is_const": false, "is_unsafe": false, "is_async": false, "abi": "Rust"}}, "has_body": true}}}}}}
}}, "paths": {{"5": {{"crate_id": 0, "path": ["k", "inner", "Pub"], "kind": "struct"}}}}}}"#
        )
    }

    #[test]
    fn the_listing_reads_paths_items_members_and_impls() {
        let got = listing(&fixture(false)).unwrap();
        assert_eq!(
            got,
            [
                "k::E :: #[repr(u8)] enum E",
                "k::E :: variant A = 3",
                "k::Renamed :: field a: i32",
                "k::Renamed :: impl Pub",
                "k::Renamed :: impl Pub :: fn m(&mut self)",
                "k::Renamed :: struct Pub { .. }",
                "k::f :: unsafe fn f<'a>(&'a Pub) -> u32",
            ]
        );
    }

    /// The H20-Q6 check: a new `pub` item is a `+` line, and the diff fails.
    #[test]
    fn a_new_pub_item_fails_the_diff() {
        let base = listing(&fixture(false)).unwrap();
        let new = listing(&fixture(true)).unwrap();
        assert_eq!(diff(&base, &new), ["+ k::g :: const fn g()"]);
        assert_eq!(diff(&new, &base), ["- k::g :: const fn g()"]);
        assert!(diff(&base, &base).is_empty());
    }

    #[test]
    fn another_format_version_is_an_error() {
        let t = fixture(false).replace("\"format_version\": 61", "\"format_version\": 62");
        let e = listing(&t).unwrap_err();
        assert!(e.contains("format_version 62"), "{e}");
    }
}
