//! `cargo xtask extref-lint` (C-26, plan v10 V10-A7; H25-Q2 as H25 words it): a native word
//! (an off-heap address, H14 encoding) reaches a heap cell only through `ExternalRef<T>`
//! (`crates/ep9_vm/src/external_ref.rs`). The lint parses `crates/*/src` with `syn` and reports
//! two kinds of finding:
//!
//! ```text
//! parse every .rs file; tokens (macro bodies too) + regions from the AST
//!   encoding  `native_of`, `native_ptr`, `native_addr`, `frame::native` outside frame.rs,
//!             external_ref.rs, test code and the VM stack allow list [`VM_STACK`]
//!   field     a native-word field ([`FIELDS`]: `fun::INFO`, `fv::INFO`, `ctxf::SCOPE`,
//!             `modf::UNIT`, their `layout::` exports) named in a statement that does not
//!             name `ExternalRef`: a read or a write that skips the type, such as a native
//!             word copied from one cell field into another as a bare u64 (the FEEDBACK Call
//!             key half once copied the FUNCTION +24 word this way, MC-w13-1)
//!   hand      the encoding offset ([`OFFSETS`]: `DOUBLE_ENCODE_OFFSET`, `NATIVE_WORD_OFFSET`)
//!             outside the encoding crate ep9_value, frame.rs, external_ref.rs, layout.rs,
//!             test code and the double-Value allow list [`DOUBLE_VALUE`]: a native word
//!             made or decoded by hand (`w - DOUBLE_ENCODE_OFFSET`; M5-P3 reviewer A F3)
//! ```
//!
//! | Finding | Example |
//! |---|---|
//! | encoding | `init(c, fv::INFO, native_of(fi))`; `&*native_ptr::<FunctionInfo>(w)`; `frame::native(a)` |
//! | field | `let w = rd(c, fun::INFO);`; `init(v, k, rd(cell(f), fun::INFO))` |
//! | import | `use crate::frame::native_of as nw;` (a renamed encoding function); `use crate::object::fun::INFO;` (a bare field name); `use crate::object::fun as f;` (a renamed field or `frame` module); `use crate::layout::*;`; `use crate::frame::native;` |
//! | macro path | `fun::$f`, `crate::frame::$name` (a protected module, then a metavariable); `$m::INFO` |
//! | hand | `w.checked_sub(DOUBLE_ENCODE_OFFSET)` (the FEEDBACK Call key read of `facts_export.rs` until M5-P3 fixer round 2); `(w - layout::NATIVE_WORD_OFFSET) as usize`; `use ep9_value::encoding::*;` |
//! | allow | a [`VM_STACK`] or [`DOUBLE_VALUE`] entry with no hit in a scanned file (remove it) |
//!
//! - Allowed without a finding: `crates/ep9_vm/src/frame.rs` (the encoding and the VM stack),
//!   `crates/ep9_vm/src/external_ref.rs` (the type), `crates/ep9_vm/src/layout.rs` for fields
//!   (it exports the offsets, it reads no word), `#[cfg(test)]` items and the test files
//!   (`src/tests/**`, `tests.rs`), and the [`VM_STACK`] functions: code that reads or writes a
//!   VM stack frame slot (caller fp, return pc, FunctionInfo; H25 "The other `native_of`
//!   stores write VM stack frame slots"). The VM stack is not a heap cell. The list names a
//!   function, not a line, so it stays true when lines move; `-v` prints each allowed use with
//!   its line.
//! - Known gaps: a statement that names `ExternalRef` and also copies a bare field word; a
//!   raw offset (`rd(c, 24)`) in place of the field name; the offset as a literal (`1 << 49`)
//!   in place of its name; a macro path whose module and field both come from metavariables
//!   (`$m::$f`). Review and the H25 inventory cover them.
//! - Test code is an item whose `cfg` is false in every build without `test` (`test`,
//!   `all(test, ..)`); `any(test, feature = "x")` compiles in production and is checked.
//!
//! Exit codes: 0 no finding, 1 a finding, 2 usage error or a file that does not parse. Paths
//! print relative to the root with `/` on every OS. Fixture tests: the `tests` module below.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{LineColumn, Spacing, TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::disasm::repo_root;

const USAGE: &str = "\
usage: cargo xtask extref-lint [--root DIR] [-v] [PATH...]
  --root DIR   tree to check (default: this repo); paths print relative to it, with `/`
  -v           also print every allowed use (VM stack, layout export) with its line
  PATH...      .rs files or directories under the root (default: every crates/*/src)";

/// The native-word encoding functions of `crates/ep9_vm/src/frame.rs` (matched by name, any
/// path, turbofish included). `frame::native` is matched as a two-segment path, because
/// `native` alone is a common word.
pub(crate) const ENCODING: &[&str] = &["native_of", "native_ptr", "native_addr"];

/// The heap-cell fields that hold a native word (the H25 inventory), as `module::CONST`
/// paths, with the `ep9_vm::layout` exports of their offsets. The FEEDBACK Call key half has
/// no field name of its own (a Call slot's key half holds a FunctionInfo word, the hole or
/// undefined); its source is the FUNCTION +24 word, `fun::INFO`.
pub(crate) const FIELDS: &[(&str, &str)] = &[
    ("fun", "INFO"),
    ("fv", "INFO"),
    ("ctxf", "SCOPE"),
    ("modf", "UNIT"),
    ("layout", "FUNCTION_INFO"),
    ("layout", "CONTEXT_SCOPE"),
    ("layout", "FEEDBACK_INFO"),
];

/// The native-word offset, by its two names (`ep9_value::encoding` and the `ep9_vm::layout`
/// export): a native word is `address + offset` (`frame::native`). A double Value is the
/// same sum, so [`DOUBLE_VALUE`] names the code that boxes or unboxes a double with it.
pub(crate) const OFFSETS: &[&str] = &["DOUBLE_ENCODE_OFFSET", "NATIVE_WORD_OFFSET"];

/// The Value encoding crate: it defines the offset and boxes every double with it.
const VALUE_CRATE: &str = "crates/ep9_value/src/";

/// The functions (file, name) that use [`OFFSETS`] on a double Value, not on a native word:
/// the double of a number Value (`interp/ops.rs` `number`, `profile.rs` `keyed_key`) and the
/// Value of a DOUBLE element (`object.rs` `double_value`). Read at the M5-P3 fixer round 2
/// head with this lint (`-v` lists the lines).
pub(crate) const DOUBLE_VALUE: &[(&str, &str)] = &[
    ("crates/ep9_vm/src/interp/ops.rs", "number"),
    ("crates/ep9_vm/src/profile.rs", "keyed_key"),
    ("crates/ep9_vm/src/object.rs", "double_value"),
];

/// The encoding and the VM stack frame code.
const FRAME_FILE: &str = "crates/ep9_vm/src/frame.rs";
/// `ExternalRef<T>` itself.
const TYPE_FILE: &str = "crates/ep9_vm/src/external_ref.rs";
/// The layout export: it names field offsets and reads no cell word.
const LAYOUT_FILE: &str = "crates/ep9_vm/src/layout.rs";

/// The VM stack code (file, function or `macro_rules!` name) that may use the encoding: each
/// reads or writes a frame slot (`frame::slot::CALLER_FP`, `RETURN_PC`, `FUNCTION_INFO`) of
/// the VM register stack, never a heap cell. Read at c45612e1 (the M5-P2 head) with this lint
/// (`-v` lists the lines); plan v10 V10-A7 named `call.rs`, `error.rs:42`, `interp/mod.rs`
/// and `generator.rs` from a grep that missed the turbofish form `native_ptr::<T>(..)`.
pub(crate) const VM_STACK: &[(&str, &str)] = &[
    // The frame push: caller fp, return pc and FunctionInfo slots (`push_frame`, and
    // `write_call_frame`, the x86-64 `call_fast` form of M5-P2 PERF3-FIX).
    ("crates/ep9_vm/src/call.rs", "push_frame"),
    ("crates/ep9_vm/src/call.rs", "write_call_frame"),
    // `FrameRef`: the FunctionInfo, caller fp and return pc of a frame (stack walks).
    ("crates/ep9_vm/src/error.rs", "info"),
    ("crates/ep9_vm/src/error.rs", "caller"),
    ("crates/ep9_vm/src/error.rs", "return_pc"),
    // The interpreter: the FunctionInfo slot of the entry frame and of `St`, the return to
    // the caller (a construct return, `Return`, a generator's yield, the call record step).
    ("crates/ep9_vm/src/interp/mod.rs", "run"),
    ("crates/ep9_vm/src/interp/mod.rs", "construct_return"),
    ("crates/ep9_vm/src/interp/mod.rs", "info"),
    ("crates/ep9_vm/src/interp/ops.rs", "op!(Return)"),
    ("crates/ep9_vm/src/interp/dsl.rs", "record_call_next_fn"),
    (
        "crates/ep9_vm/src/interp/slow/generator.rs",
        "suspend_generator",
    ),
    // Runtime calls: the FunctionInfo slot of the top frame.
    ("crates/ep9_vm/src/runtime.rs", "frame_top"),
    ("crates/ep9_vm/src/runtime.rs", "enter"),
    // A sampled call record after `call_fast` pushed the callee frame: the caller frame's
    // FunctionInfo slot (its key store goes through `ExternalRef`, `target_key`).
    ("crates/ep9_vm/src/profile.rs", "record_call_pushed"),
];

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let r = run(&o.root, &o.paths)?;
    if o.verbose {
        for a in &r.allowed {
            println!(
                "extref-lint: allowed {}:{}: `{}` in {} ({})",
                a.file, a.line, a.name, a.item, a.why
            );
        }
    }
    for f in &r.findings {
        println!("{}:{}: {}: {}", f.file, f.line, f.rule, f.msg);
    }
    println!(
        "extref-lint: {} file{} under {}: {} finding{}, {} allowed use{} ({} VM stack), {} in test code",
        r.files,
        plural(r.files),
        o.paths.join(" "),
        r.findings.len(),
        plural(r.findings.len()),
        r.allowed.len(),
        plural(r.allowed.len()),
        r.allowed.iter().filter(|a| a.why == WHY_STACK).count(),
        r.in_tests,
    );
    Ok(r.findings.is_empty())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

struct Opts {
    root: PathBuf,
    paths: Vec<String>,
    verbose: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut root = repo_root();
    let mut paths = Vec::new();
    let mut verbose = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => {
                root = PathBuf::from(
                    it.next()
                        .ok_or_else(|| format!("--root needs a value\n{USAGE}"))?,
                )
            }
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s => paths.push(s.to_string()),
        }
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("--root {}: {e}", root.display()))?;
    if paths.is_empty() {
        paths = crate_src_dirs(&root)?;
    }
    Ok(Opts {
        root,
        paths,
        verbose,
    })
}

/// `crates/<name>/src` for every crate dir of the root, sorted.
fn crate_src_dirs(root: &Path) -> Result<Vec<String>, String> {
    let crates = root.join("crates");
    let mut out = Vec::new();
    for e in fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let e = e.map_err(|e| format!("{}: {e}", crates.display()))?;
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().join("src").is_dir() {
            out.push(format!("crates/{name}/src"));
        }
    }
    out.sort();
    Ok(out)
}

// ---- results --------------------------------------------------------------------------------

const WHY_STACK: &str = "VM stack";
const WHY_DOUBLE: &str = "double Value";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Finding {
    /// Path from the root, with `/`.
    pub(crate) file: String,
    pub(crate) line: usize,
    /// `encoding`, `field`, `hand`, `import` or `allow`.
    pub(crate) rule: &'static str,
    pub(crate) msg: String,
}

/// A use that the lint accepts outside the type and the tests.
#[derive(Clone, Debug)]
pub(crate) struct Allowed {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) name: String,
    /// The function or macro it is in.
    pub(crate) item: String,
    pub(crate) why: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct Report {
    pub(crate) files: usize,
    /// Sorted.
    pub(crate) findings: Vec<Finding>,
    pub(crate) allowed: Vec<Allowed>,
    /// Hits in test code (not findings).
    pub(crate) in_tests: usize,
}

/// Lints `paths` (files or directories, relative to `root`).
pub(crate) fn run(root: &Path, paths: &[String]) -> Result<Report, String> {
    let mut files = Vec::new();
    for p in paths {
        // One component at a time: on Windows a canonical root is a `\\?\` path, in which `/`
        // is not a separator.
        let mut full = root.to_path_buf();
        full.extend(p.split(['/', '\\']).filter(|s| !s.is_empty() && *s != "."));
        if full.is_dir() {
            rs_files(&full, &mut files)?;
        } else if full.is_file() {
            files.push(full);
        } else {
            return Err(format!("{}: no such file or directory", full.display()));
        }
    }
    files.sort();
    files.dedup();
    let mut report = Report::default();
    let mut scanned = Vec::new();
    for f in &files {
        let rel = rel_path(root, f);
        let src = fs::read_to_string(f).map_err(|e| format!("{rel}: {e}"))?;
        lint_source(&rel, &src, &mut report)?;
        scanned.push(rel);
    }
    report.files = scanned.len();
    unused_allows(&scanned, &mut report);
    report.findings.sort();
    report.findings.dedup();
    Ok(report)
}

/// A [`VM_STACK`] or [`DOUBLE_VALUE`] entry of a scanned file that no hit used is a finding:
/// each list stays the list of the code that needs it.
fn unused_allows(scanned: &[String], report: &mut Report) {
    let lists = [
        (VM_STACK, WHY_STACK, "VM_STACK", "encoding"),
        (DOUBLE_VALUE, WHY_DOUBLE, "DOUBLE_VALUE", "offset"),
    ];
    for (list, why, name, what) in lists {
        for (file, item) in list {
            if !scanned.iter().any(|s| s == file) {
                continue;
            }
            let used = report
                .allowed
                .iter()
                .any(|a| a.file == *file && a.item == *item && a.why == why);
            if !used {
                report.findings.push(Finding {
                    file: file.to_string(),
                    line: 1,
                    rule: "allow",
                    msg: format!(
                        "the {why} allow-list entry `{item}` has no {what} use in this file: \
                         remove it from extref_lint.rs {name}"
                    ),
                });
            }
        }
    }
}

fn rel_path(root: &Path, f: &Path) -> String {
    let rel = f.strip_prefix(root).unwrap_or(f);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
            if !name.starts_with('.') && name != "target" {
                rs_files(&p, out)?;
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// A test file: under a `src/tests` dir, or a `tests.rs` (both are `#[cfg(test)]` modules).
fn is_test_file(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.last() == Some(&"tests.rs") || parts.windows(2).any(|w| w == ["src", "tests"])
}

// ---- one file -------------------------------------------------------------------------------

type Pos = LineColumn;
type Range = (Pos, Pos);

fn key(p: Pos) -> (usize, usize) {
    (p.line, p.column)
}

fn contains(r: &Range, p: Pos) -> bool {
    key(r.0) <= key(p) && key(p) < key(r.1)
}

/// The smallest range of `rs` that holds `p`.
fn innermost<T>(rs: &[(Range, T)], p: Pos) -> Option<&(Range, T)> {
    rs.iter()
        .filter(|(r, _)| contains(r, p))
        .min_by_key(|(r, _)| (std::cmp::Reverse(key(r.0)), key(r.1)))
}

/// The regions of one file, from its AST.
#[derive(Default)]
struct Regions {
    /// `#[cfg(test)]` items.
    tests: Vec<(Range, ())>,
    /// Functions (name) and `macro_rules!` items (name).
    items: Vec<(Range, String)>,
    /// `use` items.
    uses: Vec<(Range, ())>,
    /// Statements and the bodies of `const` and `static` items: the unit of the field rule.
    stmts: Vec<(Range, ())>,
    /// `use` findings: (line, rule message).
    imports: Vec<(usize, String)>,
    /// Lines of a glob import of `encoding` (`ep9_value::encoding::*`): a finding outside
    /// [`VALUE_CRATE`], because it hides the [`OFFSETS`] names.
    encoding_globs: Vec<usize>,
}

fn range_of(s: proc_macro2::Span) -> Range {
    (s.start(), s.end())
}

/// An item is test code only when its `cfg` predicate is false in every build without `test`:
/// `test`, `all(.., test, ..)`, `any(..)` whose every arm needs `test`. `any(test, feature =
/// "x")`, `feature = "test-hooks"` and `not(..)` compile in a production build.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && a.parse_args::<syn::Meta>().is_ok_and(|m| needs_test(&m))
    })
}

fn needs_test(m: &syn::Meta) -> bool {
    match m {
        syn::Meta::Path(p) => p.is_ident("test"),
        syn::Meta::NameValue(_) => false,
        syn::Meta::List(l) => {
            let Ok(args) = l.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return false;
            };
            if l.path.is_ident("all") {
                args.iter().any(needs_test)
            } else if l.path.is_ident("any") {
                !args.is_empty() && args.iter().all(needs_test)
            } else {
                false // `not(..)` and anything else
            }
        }
    }
}

impl<'ast> Visit<'ast> for Regions {
    fn visit_item(&mut self, i: &'ast syn::Item) {
        let attrs: &[syn::Attribute] = match i {
            syn::Item::Fn(x) => &x.attrs,
            syn::Item::Mod(x) => &x.attrs,
            syn::Item::Impl(x) => &x.attrs,
            syn::Item::Use(x) => &x.attrs,
            syn::Item::Const(x) => &x.attrs,
            syn::Item::Static(x) => &x.attrs,
            syn::Item::Struct(x) => &x.attrs,
            syn::Item::Enum(x) => &x.attrs,
            syn::Item::Trait(x) => &x.attrs,
            syn::Item::Macro(x) => &x.attrs,
            _ => &[],
        };
        if is_cfg_test(attrs) {
            self.tests.push((range_of(i.span()), ()));
            return;
        }
        match i {
            syn::Item::Fn(f) => self
                .items
                .push((range_of(i.span()), f.sig.ident.to_string())),
            syn::Item::Macro(m) => {
                // `macro_rules! name`, or an item-level call `op!(Return(..) ..)`, named
                // `op!(Return)` by its first identifier.
                let name = match &m.ident {
                    Some(name) => Some(name.to_string()),
                    None => m.mac.path.segments.last().map(|seg| {
                        let first = m.mac.tokens.clone().into_iter().find_map(|t| match t {
                            TokenTree::Ident(id) => Some(id.to_string()),
                            _ => None,
                        });
                        format!("{}!({})", seg.ident, first.unwrap_or_default())
                    }),
                };
                if let Some(name) = name {
                    self.items.push((range_of(i.span()), name));
                }
            }
            syn::Item::Use(u) => {
                self.uses.push((range_of(i.span()), ()));
                self.use_tree(&u.tree, None, u.span().start().line);
            }
            syn::Item::Const(_) | syn::Item::Static(_) => {
                self.stmts.push((range_of(i.span()), ()));
            }
            _ => {}
        }
        visit::visit_item(self, i);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if is_cfg_test(&f.attrs) {
            self.tests.push((range_of(f.span()), ()));
            return;
        }
        self.items
            .push((range_of(f.span()), f.sig.ident.to_string()));
        visit::visit_impl_item_fn(self, f);
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        self.items
            .push((range_of(f.span()), f.sig.ident.to_string()));
        visit::visit_trait_item_fn(self, f);
    }

    fn visit_stmt(&mut self, s: &'ast syn::Stmt) {
        if let syn::Stmt::Item(_) = s {
            // Its own item rules apply.
        } else {
            self.stmts.push((range_of(s.span()), ()));
        }
        visit::visit_stmt(self, s);
    }
}

impl Regions {
    /// `use` trees: a renamed encoding function, and an imported field const, are findings
    /// (each would hide a name the token rules match).
    fn use_tree(&mut self, t: &syn::UseTree, parent: Option<&str>, line: usize) {
        match t {
            syn::UseTree::Path(p) => {
                let seg = p.ident.to_string();
                self.use_tree(&p.tree, Some(&seg), line);
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(t, parent, line);
                }
            }
            syn::UseTree::Rename(r) => {
                // `m::{self as x}` renames `m` itself (Codex M5-P3 r2): check it as `m as x`.
                let ident = r.ident.to_string();
                let name = match parent {
                    Some(p) if ident == "self" => p.to_string(),
                    _ => ident,
                };
                if ENCODING.contains(&name.as_str())
                    || OFFSETS.contains(&name.as_str())
                    || (parent == Some("frame") && name == "native")
                {
                    self.imports.push((
                        line,
                        format!(
                            "`{name}` is imported as `{}`: use the name itself",
                            r.rename
                        ),
                    ));
                }
                if is_protected_module(&name) {
                    self.imports.push((
                        line,
                        format!(
                            "the module `{name}` is imported as `{}`: the field and encoding \
                             names under it would hide; use `{name}` itself",
                            r.rename
                        ),
                    ));
                }
                self.field_import(parent, &name, line);
            }
            syn::UseTree::Name(n) => {
                let name = n.ident.to_string();
                if parent == Some("frame") && name == "native" {
                    self.imports.push((
                        line,
                        "`frame::native` is imported by name: call it `frame::native`".into(),
                    ));
                }
                self.field_import(parent, &name, line);
            }
            syn::UseTree::Glob(_) => {
                if parent == Some("encoding") {
                    self.encoding_globs.push(line);
                }
                if let Some(p) = parent
                    && (FIELDS.iter().any(|(m, _)| *m == p) || p == "frame")
                {
                    self.imports.push((
                        line,
                        format!("a glob import of `{p}` hides the native-word field names"),
                    ));
                }
            }
        }
    }

    fn field_import(&mut self, parent: Option<&str>, name: &str, line: usize) {
        if let Some(p) = parent
            && FIELDS.contains(&(p, name))
        {
            self.imports.push((
                line,
                format!(
                    "`{p}::{name}` is imported by name: name it `{p}::{name}` where it is used"
                ),
            ));
        }
    }
}

/// A module whose names the token rules match: a FIELDS module, or `frame` (`frame::native`).
fn is_protected_module(name: &str) -> bool {
    name == "frame" || FIELDS.iter().any(|(m, _)| *m == name)
}

/// A token of the flat stream: an identifier, `::`, or anything else (a group bound too, so a
/// path never joins across a group).
enum Tok {
    Ident(String, Pos),
    Colon2,
    /// `$` (a `macro_rules!` metavariable follows).
    Dollar(Pos),
    Other,
}

fn flatten(ts: TokenStream, out: &mut Vec<Tok>) {
    let mut it = ts.into_iter().peekable();
    while let Some(t) = it.next() {
        match t {
            TokenTree::Ident(i) => out.push(Tok::Ident(i.to_string(), i.span().start())),
            TokenTree::Punct(p) if p.as_char() == ':' && p.spacing() == Spacing::Joint => {
                if matches!(it.peek(), Some(TokenTree::Punct(q)) if q.as_char() == ':') {
                    it.next();
                    out.push(Tok::Colon2);
                } else {
                    out.push(Tok::Other);
                }
            }
            TokenTree::Punct(p) if p.as_char() == '$' => out.push(Tok::Dollar(p.span().start())),
            TokenTree::Group(g) => {
                out.push(Tok::Other);
                flatten(g.stream(), out);
                out.push(Tok::Other);
            }
            _ => out.push(Tok::Other),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Encoding,
    Field,
    /// An [`OFFSETS`] name.
    Hand,
}

/// Lints one file's source; `rel` is its path from the root, with `/`.
pub(crate) fn lint_source(rel: &str, src: &str, report: &mut Report) -> Result<(), String> {
    let ast = syn::parse_file(src).map_err(|e| {
        let at = e.span().start();
        format!("{rel}:{}:{}: does not parse: {e}", at.line, at.column + 1)
    })?;
    let mut rg = Regions::default();
    rg.visit_file(&ast);
    let ts: TokenStream = src
        .parse()
        .map_err(|e| format!("{rel}: does not tokenize: {e:?}"))?;
    let mut toks = Vec::new();
    flatten(ts, &mut toks);

    let mut hits: Vec<(Kind, String, Pos)> = Vec::new();
    let mut extref: Vec<Pos> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        // A path that a macro completes: `fun::$f`, `crate::frame::$name` (any protected
        // module, then a metavariable), or `$m::INFO` (a metavariable, then a field const).
        // The lint does not expand macros, so it rejects both forms.
        if let Tok::Dollar(at) = t {
            if i >= 2
                && let (Tok::Ident(head, _), Tok::Colon2) = (&toks[i - 2], &toks[i - 1])
                && is_protected_module(head)
            {
                let kind = if head == "frame" {
                    Kind::Encoding
                } else {
                    Kind::Field
                };
                hits.push((kind, format!("{head}::$.. (a macro-built path)"), *at));
            }
            if let (Some(Tok::Ident(..)), Some(Tok::Colon2), Some(Tok::Ident(c, _))) =
                (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3))
                && FIELDS.iter().any(|(_, f)| f == c)
            {
                hits.push((Kind::Field, format!("$..::{c} (a macro-built path)"), *at));
            }
            continue;
        }
        let Tok::Ident(name, at) = t else { continue };
        if name == "ExternalRef" {
            extref.push(*at);
        }
        if ENCODING.contains(&name.as_str()) {
            hits.push((Kind::Encoding, name.clone(), *at));
            continue;
        }
        if OFFSETS.contains(&name.as_str()) {
            hits.push((Kind::Hand, name.clone(), *at));
            continue;
        }
        if i >= 2
            && let (Tok::Ident(head, _), Tok::Colon2) = (&toks[i - 2], &toks[i - 1])
        {
            if head == "frame" && name == "native" {
                hits.push((Kind::Encoding, "frame::native".into(), *at));
            } else if FIELDS.contains(&(head.as_str(), name.as_str())) {
                hits.push((Kind::Field, format!("{head}::{name}"), *at));
            }
        }
    }

    let test_file = is_test_file(rel);
    let value_crate = rel.starts_with(VALUE_CRATE);
    let globs = rg.encoding_globs.iter().filter(|_| !value_crate);
    let globs = globs.map(|l| {
        (
            *l,
            "a glob import of `encoding` hides the offset names".into(),
        )
    });
    let imports: Vec<(usize, String)> = rg.imports.iter().cloned().chain(globs).collect();
    for (line, msg) in &imports {
        if !test_file
            && !rg
                .tests
                .iter()
                .any(|(r, _)| r.0.line <= *line && *line <= r.1.line)
        {
            report.findings.push(Finding {
                file: rel.into(),
                line: *line,
                rule: "import",
                msg: msg.clone(),
            });
        }
    }
    for (kind, name, at) in hits {
        if innermost(&rg.uses, at).is_some() {
            continue; // an import: the use sites are checked
        }
        if test_file || innermost(&rg.tests, at).is_some() {
            report.in_tests += 1;
            continue;
        }
        if kind == Kind::Hand && value_crate {
            continue; // the Value encoding itself
        }
        let item = innermost(&rg.items, at).map(|(_, n)| n.clone());
        let allowed = |why: &'static str, item: Option<String>, report: &mut Report| {
            report.allowed.push(Allowed {
                file: rel.into(),
                line: at.line,
                name: name.clone(),
                item: item.unwrap_or_else(|| "(file scope)".into()),
                why,
            });
        };
        match kind {
            Kind::Encoding => {
                if rel == FRAME_FILE || rel == TYPE_FILE {
                    allowed("the encoding or the type", item, report);
                } else if let Some(it) = &item
                    && VM_STACK.contains(&(rel, it.as_str()))
                {
                    allowed(WHY_STACK, item, report);
                } else {
                    report.findings.push(Finding {
                        file: rel.into(),
                        line: at.line,
                        rule: "encoding",
                        msg: format!(
                            "`{name}` outside ExternalRef and the VM stack code{}: a heap-cell \
                             word goes through `ExternalRef::new` / `ExternalRef::from_word` (H25)",
                            item.map(|i| format!(" (in `{i}`)")).unwrap_or_default()
                        ),
                    });
                }
            }
            Kind::Hand => {
                if rel == FRAME_FILE || rel == TYPE_FILE || rel == LAYOUT_FILE {
                    allowed("the encoding, the type or the layout export", item, report);
                } else if let Some(it) = &item
                    && DOUBLE_VALUE.contains(&(rel, it.as_str()))
                {
                    allowed(WHY_DOUBLE, item, report);
                } else {
                    report.findings.push(Finding {
                        file: rel.into(),
                        line: at.line,
                        rule: "hand",
                        msg: format!(
                            "`{name}` outside the encoding and ExternalRef{}: a native word is \
                             made by `ExternalRef::new` and decoded by `ExternalRef::get` or \
                             `ExternalRef::addr` (H25), never by hand",
                            item.map(|i| format!(" (in `{i}`)")).unwrap_or_default()
                        ),
                    });
                }
            }
            Kind::Field => {
                if rel == LAYOUT_FILE {
                    allowed("the layout export", item, report);
                    continue;
                }
                let ok = innermost(&rg.stmts, at)
                    .is_some_and(|(r, _)| extref.iter().any(|p| contains(r, *p)));
                if !ok {
                    report.findings.push(Finding {
                        file: rel.into(),
                        line: at.line,
                        rule: "field",
                        msg: format!(
                            "`{name}` holds a native word, but this statement does not go \
                             through ExternalRef: read it with `ExternalRef::from_word`, write \
                             it from `ExternalRef::new(..).to_word()` (no bare u64 copy, H25-Q2)"
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Finding, Report, VM_STACK, lint_source};

    /// The findings of one fixture, as (line, rule).
    fn lint(rel: &str, src: &str) -> (Vec<(usize, &'static str)>, Report) {
        let mut r = Report::default();
        lint_source(rel, src, &mut r).expect("fixture parses");
        r.findings.sort();
        let f = r
            .findings
            .iter()
            .map(|f: &Finding| (f.line, f.rule))
            .collect();
        (f, r)
    }

    const VM: &str = "crates/ep9_vm/src/fixture.rs";

    #[test]
    fn a_bare_native_of_store_fails() {
        let src =
            "fn make(c: CellPtr, fi: &FunctionInfo) {\n    init(c, fv::INFO, native_of(fi));\n}\n";
        let (f, _) = lint(VM, src);
        // Line 2: the encoding call, and the field written from a bare word.
        assert_eq!(f, [(2, "encoding"), (2, "field")]);
    }

    #[test]
    fn a_bare_native_ptr_read_fails_in_every_form() {
        let src = "\
fn a(f: u64) -> &'static FunctionInfo {
    unsafe { &*native_ptr::<FunctionInfo>(w(f)) }
}
fn b(w: u64) -> *mut u8 {
    crate::frame::native_ptr(w)
}
fn c(w: u64) -> usize {
    frame::native_addr(w) + frame::native(8) as usize
}
";
        let (f, _) = lint(VM, src);
        assert_eq!(
            f,
            [
                (2, "encoding"),
                (5, "encoding"),
                (8, "encoding"),
                (8, "encoding")
            ]
        );
    }

    #[test]
    fn a_copied_native_word_fails() {
        // The FEEDBACK Call key half once copied the FUNCTION +24 word (MC-w13-1): no
        // encoding call, only a field read whose word is stored elsewhere.
        let src = "\
fn target_key(callee: u64) -> u64 {
    word(callee, object::fun::INFO).bits()
}
fn copy(v: u64, f: u64) {
    let w = rd(cell(f), fun::INFO);
    init(cell(v), 8, w);
    init(cell(v), fv::INFO, rd(cell(f), fun::INFO));
}
";
        let (f, _) = lint(VM, src);
        assert_eq!(f, [(2, "field"), (5, "field"), (7, "field"), (7, "field")]);
    }

    #[test]
    fn the_external_ref_forms_pass() {
        let src = "\
fn make(c: CellPtr, fi: &FunctionInfo, m: u64, lu: &LoadedUnit) {
    init(c, fv::INFO, ExternalRef::new(fi).to_word());
    object::record_set(rt, m, modf::UNIT, ExternalRef::new(lu).to_word());
}
fn read(f: u64) -> &'static FunctionInfo {
    unsafe { &*ExternalRef::<FunctionInfo>::from_word(rd(cell(f), fun::INFO)).get() }
}
fn key(callee: u64) -> u64 {
    crate::external_ref::ExternalRef::new(object::function_info(callee)).to_word()
}
";
        let (f, r) = lint(VM, src);
        assert_eq!(f, []);
        assert!(r.allowed.is_empty());
    }

    #[test]
    fn test_code_and_doc_text_are_not_findings() {
        let src = "\
/// `native_of(p)` and `fun::INFO` in a doc comment are text.
fn f() {}
// native_ptr in a comment.
#[cfg(test)]
mod tests {
    fn t() { let w = native_of(&1u8); init(c, fun::INFO, w); }
}
";
        let (f, r) = lint(VM, src);
        assert_eq!(f, []);
        assert_eq!(r.in_tests, 2);
        let (f, r) = lint(
            "crates/ep9_vm/src/tests/x.rs",
            "fn t() { native_of(&1u8); }",
        );
        assert_eq!((f, r.in_tests), (vec![], 1));
    }

    #[test]
    fn the_vm_stack_list_names_functions_of_one_file() {
        let src = "\
pub(crate) unsafe fn push_frame(fp: *mut u64, fi: *const FunctionInfo) {
    *fp.offset(slot::FUNCTION_INFO) = native_of(fi);
}
fn other(fp: *mut u64, fi: *const FunctionInfo) {
    *fp.offset(slot::FUNCTION_INFO) = native_of(fi);
}
";
        let (f, r) = lint("crates/ep9_vm/src/call.rs", src);
        assert_eq!(f, [(5, "encoding")]);
        assert_eq!(r.allowed.len(), 1);
        assert_eq!(
            (r.allowed[0].line, r.allowed[0].item.as_str()),
            (2, "push_frame")
        );
        // The same function in another file is not on the list.
        let (f, _) = lint(VM, src);
        assert_eq!(f, [(2, "encoding"), (5, "encoding")]);
    }

    #[test]
    fn macro_bodies_and_imports_are_checked() {
        let src = "\
use crate::frame::{native_of, native_ptr as np};
use crate::object::fun::INFO;
use crate::object::{fun, fv::*};
macro_rules! sneak {
    ($w:expr) => { crate::frame::native_ptr::<u8>($w) };
}
";
        let (f, _) = lint(VM, src);
        assert_eq!(
            f,
            [(1, "import"), (2, "import"), (3, "import"), (5, "encoding")]
        );
    }

    /// Codex M5-P3 r1: module aliases, `frame::native` by name, globs, macro-built paths and
    /// production cfgs that name `test` each hid a native word.
    #[test]
    fn aliases_macro_paths_and_production_cfgs_fail() {
        let alias = "\
use crate::object::fun as fields;
mod m { pub use crate::object::fun as f; }
use crate::frame as encoding;
use crate::layout::*;
use crate::frame::native;
use crate::frame::*;
";
        let (f, _) = lint(VM, alias);
        assert_eq!(
            f,
            [
                (1, "import"),
                (2, "import"),
                (3, "import"),
                (4, "import"),
                (5, "import"),
                (6, "import")
            ]
        );
        let mac = "\
macro_rules! read_word { ($v:expr, $f:ident) => { rd(cell($v), fun::$f) }; }
macro_rules! enc { ($n:ident, $p:expr) => { crate::frame::$n($p) }; }
macro_rules! rd_mod { ($m:ident, $c:expr) => { rd($c, $m::INFO) }; }
";
        let (f, _) = lint(VM, mac);
        assert_eq!(f, [(1, "field"), (2, "encoding"), (3, "field")]);
        let cfg = "\
#[cfg(any(test, target_pointer_width = \"64\"))]
fn a(c: CellPtr, fi: &FunctionInfo) { init(c, fun::INFO, native_of(fi)); }
#[cfg(feature = \"test-hooks\")]
fn b(c: CellPtr, fi: &FunctionInfo) { init(c, fun::INFO, native_of(fi)); }
#[cfg(all(test, not(miri)))]
fn c(c: CellPtr, fi: &FunctionInfo) { init(c, fun::INFO, native_of(fi)); }
#[cfg(any(test, all(test, unix)))]
fn d(c: CellPtr, fi: &FunctionInfo) { init(c, fun::INFO, native_of(fi)); }
";
        let (f, r) = lint(VM, cfg);
        assert_eq!(
            f,
            [(2, "encoding"), (2, "field"), (4, "encoding"), (4, "field")]
        );
        assert_eq!(r.in_tests, 4);
    }

    /// Codex M5-P3 r2 eng: `self as x` in a group renames the parent module, so `x::native`
    /// and `x::INFO` hid from the encoding and field rules. A bare `{self}` keeps the name.
    #[test]
    fn self_aliases_of_protected_modules_fail() {
        let src = "\
use crate::frame::{self as enc, UNDEFINED};
fn a(addr: usize) -> u64 { enc::native(addr) }
use crate::object::fun::{self as f, CONTEXT};
fn b(c: CellPtr) -> u64 { rd(c, f::INFO) }
use crate::layout::{self as l};
use crate::frame::{self};
use crate::object::{self as o};
";
        let (f, _) = lint(VM, src);
        assert_eq!(f, [(1, "import"), (3, "import"), (5, "import")]);
    }

    #[test]
    fn the_type_frame_and_layout_files_are_allowed() {
        let ty = "fn new(p: *const T) -> u64 { crate::frame::native_of(p) }";
        assert_eq!(lint("crates/ep9_vm/src/external_ref.rs", ty).0, []);
        let fr = "pub fn native_ptr<T>(b: u64) -> *mut T { with(native_addr(b)) }";
        assert_eq!(lint("crates/ep9_vm/src/frame.rs", fr).0, []);
        let lay = "pub const FUNCTION_INFO: usize = fun::INFO;";
        assert_eq!(lint("crates/ep9_vm/src/layout.rs", lay).0, []);
        // The same export anywhere else names a field outside ExternalRef.
        assert_eq!(lint(VM, lay).0, [(1, "field")]);
    }

    /// M5-P3 reviewer A F3: a native word decoded (or made) by hand with the offset, the form
    /// `facts_export.rs` used for the FEEDBACK Call key half (reviewer A's fixture extref-fx);
    /// an import that hides the offset names. The encoding files, the Value crate and the
    /// double-Value functions keep the offset.
    #[test]
    fn a_hand_made_or_decoded_native_word_fails() {
        let src = "\
use ep9_value::encoding::DOUBLE_ENCODE_OFFSET;
fn native_address(w: u64) -> Option<usize> {
    let addr = w.checked_sub(DOUBLE_ENCODE_OFFSET)?;
    (addr < 1 << 49).then_some(addr as usize)
}
fn info(w: u64) -> *const FunctionInfo {
    (w - crate::layout::NATIVE_WORD_OFFSET) as usize as *const FunctionInfo
}
fn word(a: usize) -> u64 { a as u64 + ep9_value::encoding::DOUBLE_ENCODE_OFFSET }
use ep9_value::encoding::DOUBLE_ENCODE_OFFSET as OFF;
use ep9_value::encoding::*;
";
        let (f, _) = lint("crates/ep9_vm/src/facts_export.rs", src);
        assert_eq!(
            f,
            [
                (3, "hand"),
                (7, "hand"),
                (9, "hand"),
                (10, "import"),
                (11, "import")
            ]
        );
        let ok = "pub fn native(a: usize) -> u64 { a as u64 + DOUBLE_ENCODE_OFFSET }";
        assert_eq!(lint("crates/ep9_vm/src/frame.rs", ok).0, []);
        assert_eq!(lint("crates/ep9_vm/src/external_ref.rs", ok).0, []);
        let lay = "pub const NATIVE_WORD_OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;";
        assert_eq!(lint("crates/ep9_vm/src/layout.rs", lay).0, []);
        let (f, r) = lint(
            "crates/ep9_value/src/value.rs",
            "use encoding::*;\n".to_owned().as_str(),
        );
        assert_eq!((f, r.allowed.len()), (vec![], 0));
        let (f, r) = lint("crates/ep9_value/src/value.rs", ok);
        assert_eq!((f, r.allowed.len()), (vec![], 0));
        let dbl =
            "fn number(b: u64) -> f64 { f64::from_bits(b.wrapping_sub(DOUBLE_ENCODE_OFFSET)) }";
        let (f, r) = lint("crates/ep9_vm/src/interp/ops.rs", dbl);
        assert_eq!((f, r.allowed[0].why), (vec![], "double Value"));
        // The same function in another file is not on the list.
        assert_eq!(lint(VM, dbl).0, [(1, "hand")]);
    }

    #[test]
    fn a_file_that_does_not_parse_is_an_error() {
        let mut r = Report::default();
        let e = lint_source(VM, "fn (", &mut r).unwrap_err();
        assert!(e.starts_with("crates/ep9_vm/src/fixture.rs:1:"), "{e}");
    }

    /// Each VM stack and double-Value entry is a (file, name) pair of a real VM file.
    #[test]
    fn the_vm_stack_list_is_vm_files() {
        for (file, item) in VM_STACK.iter().chain(super::DOUBLE_VALUE) {
            assert!(file.starts_with("crates/ep9_vm/src/"), "{file}");
            assert!(!item.is_empty());
        }
    }

    /// The tree: every native word in a heap cell goes through ExternalRef, and every
    /// VM stack entry is used (`cargo xtask extref-lint`, the acceptance of C-26).
    #[test]
    fn the_repo_tree_passes() {
        let root = crate::disasm::repo_root();
        let paths = super::crate_src_dirs(&root).expect("crates dir");
        let r = super::run(&root, &paths).expect("the tree parses");
        assert!(r.findings.is_empty(), "{:#?}", r.findings);
        assert!(r.files > 100, "{} files", r.files);
    }
}
