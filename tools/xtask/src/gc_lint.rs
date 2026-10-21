//! `cargo xtask gc-lint` (H10-Q2, H20 rule R2): a stack-only GC type must stay on the native
//! stack, where the conservative scan sees it. The lint parses `crates/ep9_vm/src` and
//! `crates/ep9_builtins/src` with `syn` and flags every place that would put one elsewhere.
//!
//! ```text
//! parse every .rs file
//!   pass 1  type defs, aliases, `use .. as` renames, fn return and field types (all files)
//!           fixed point: the stack-only types (the base list + every type that holds one by
//!           value); per generic param of a local type: held by value? put in heap memory?
//!   pass 2  per file: every type and path, `static`, `thread_local!`, boxed closures
//!           -> findings `file:line`; `// gc-lint: allow(<reason>)` skips the next item
//! ```
//!
//! | Finding | Example |
//! |---|---|
//! | a stack-only type inside a heap container | `Vec<Local>`, `HashMap<_, PropertyDescriptor>`, `Box<[JsStr]>`, `Vec::<Local>::new()`, `Vec<Holder>` where `struct Holder { l: Local }` |
//! | a `static` that holds one (through references too) | `static L: Option<Local<'static>>` |
//! | a `thread_local!` that holds one | `static C: Cell<Option<Local<'static>>>` in `thread_local!` |
//! | a closure given to `Box::new`, `Rc::new` or `Arc::new` that uses a binding of a stack-only type | `Box::new(move \|\| l.bits())` with `l: Local` |
//!
//! - Stack-only types: [`STACK_ONLY`], any struct, enum, union or alias that holds one by
//!   value, and any `use .. as` name of one. Names are matched by their last path segment.
//! - Heap containers: [`HEAP_CONTAINERS`], and a local generic type whose field puts the
//!   parameter in one (`struct Stack<T> { items: Vec<T> }`).
//! - Not a finding: a reference or raw pointer (the value stays where it is), `PhantomData`,
//!   a `fn` pointer, a trait object or `impl Trait` (`HostFn` tables, `Box<dyn Fn(Local)>`).
//! - Closures: the type of a binding comes from its annotation, a fn or closure parameter, or a
//!   simple initializer (a call or method call whose return type the scanned files declare, a
//!   struct literal, `?`, a field, a cast).
//! - Macro bodies: `thread_local!` is parsed as items, any other macro as comma-separated
//!   expressions when it parses (`vec![..]`, `assert!(..)`); `macro_rules!` bodies are skipped.
//! - Known gaps (H10 risk R3): inferred types (`let v: Vec<_> = it.collect()`), values moved
//!   into a container by a call (`v.push(l)`), and macro bodies that do not parse. The debug
//!   on-stack check in `Local::bits` and the GC-stress runs are the backstop.
//!
//! Allow: `// gc-lint: allow(<reason>)` on its own line, right before an item, field, variant
//! or statement (its attributes and doc comment included), skips every finding inside that
//! node; when allows nest, the innermost one takes the finding. The tool lists every allow.
//! An allow that is not followed by such a node, or that covers no finding, is itself a
//! finding.
//!
//! Exit codes: 0 no finding, 1 a finding, 2 usage error or a file that does not parse.
//! Paths print relative to the root with `/` on every OS. Fixture tests:
//! `tools/xtask/tests/gc_lint.rs` and the cases in `tools/xtask/tests/gc_lint/`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Expr, FnArg, GenericArgument, Item, Pat, PathArguments, ReturnType, Type, TypeParamBound,
};

use crate::disasm::repo_root;

const USAGE: &str = "\
usage: cargo xtask gc-lint [--root DIR] [-v] [PATH...]
  --root DIR   tree to check (default: this repo); paths print relative to it, with `/`
  -v           also print the stack-only types and the local heap containers it found
  PATH...      .rs files or directories under the root (default: crates/ep9_vm/src
               crates/ep9_builtins/src)";

/// Types whose values must live on the native stack (H20 rule R2): a `Local` and the types
/// that hold `Local`s or views into heap cells.
pub(crate) const STACK_ONLY: &[&str] = &[
    "Local",
    "Args",
    "Rest",
    "ArgsBuilder",
    "ScratchList",
    "PropertyDescriptor",
    "JsStr",
    "FlatStr",
];

/// Types that put their type arguments in Rust heap memory (`RefCell` too: H20 lists it).
pub(crate) const HEAP_CONTAINERS: &[&str] = &[
    "Vec",
    "VecDeque",
    "LinkedList",
    "BinaryHeap",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "Box",
    "Rc",
    "Arc",
    "Weak",
    "RefCell",
    "SmallVec",
    "Cow",
];

/// Generic types that hold no value of their type arguments.
const OPAQUE: &[&str] = &["PhantomData"];

/// `<X>::new` and `<X>::pin` move their argument into heap memory.
const BOXING: &[&str] = &["Box", "Rc", "Arc"];

const DEFAULT_PATHS: &[&str] = &["crates/ep9_vm/src", "crates/ep9_builtins/src"];

const ALLOW_PREFIX: &str = "gc-lint:";

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
    if paths.is_empty() {
        paths = DEFAULT_PATHS.iter().map(|s| s.to_string()).collect();
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("--root {}: {e}", root.display()))?;
    Ok(Opts {
        root,
        paths,
        verbose,
    })
}

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let r = run(&o.root, &o.paths)?;
    if o.verbose {
        for (name, base) in &r.stack_only {
            println!("gc-lint: stack-only type {name} (holds {base})");
        }
        for name in &r.containers {
            println!("gc-lint: local heap container {name}");
        }
    }
    for f in &r.findings {
        println!("{}:{}: {}", f.file, f.line, f.msg);
    }
    for a in &r.allows {
        println!(
            "gc-lint: allow {}:{}: {} (lines {}-{}, {} finding{})",
            a.file,
            a.line,
            a.reason,
            a.start,
            a.end,
            a.used,
            if a.used == 1 { "" } else { "s" }
        );
    }
    println!(
        "gc-lint: {} file{} under {}: {} finding{}, {} allow{}",
        r.files,
        if r.files == 1 { "" } else { "s" },
        o.paths.join(" "),
        r.findings.len(),
        if r.findings.len() == 1 { "" } else { "s" },
        r.allows.len(),
        if r.allows.len() == 1 { "" } else { "s" },
    );
    Ok(r.findings.is_empty())
}

// ---- results --------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Finding {
    /// Path from the root, with `/`.
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) msg: String,
}

/// An allow that covers at least one finding.
#[derive(Clone, Debug)]
pub(crate) struct AllowUse {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) reason: String,
    /// The node it covers: lines `start..=end`.
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) used: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Report {
    pub(crate) files: usize,
    /// Every finding that no allow covers, and every bad allow; sorted.
    pub(crate) findings: Vec<Finding>,
    pub(crate) allows: Vec<AllowUse>,
    /// Every stack-only type -> the base type it holds.
    pub(crate) stack_only: BTreeMap<String, String>,
    /// Local generic types that put a type parameter in heap memory.
    pub(crate) containers: BTreeSet<String>,
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
    let mut parsed = Vec::new();
    for f in &files {
        let rel = rel_path(root, f);
        let src = fs::read_to_string(f).map_err(|e| format!("{rel}: {e}"))?;
        let ast = syn::parse_file(&src).map_err(|e| {
            let at = e.span().start();
            format!("{rel}:{}:{}: does not parse: {e}", at.line, at.column + 1)
        })?;
        parsed.push((rel, src, ast));
    }
    let model = Model::build(parsed.iter().map(|(_, _, ast)| ast));
    let mut report = Report {
        files: parsed.len(),
        stack_only: model.stack_only.clone(),
        containers: model
            .params
            .iter()
            .filter(|(_, r)| r.iter().any(|p| p.heap))
            .map(|(n, _)| n.clone())
            .collect(),
        ..Report::default()
    };
    for (rel, src, ast) in &parsed {
        lint_file(&model, rel, src, ast, &mut report);
    }
    report.findings.sort();
    report.findings.dedup();
    Ok(report)
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

fn lint_file(model: &Model, rel: &str, src: &str, ast: &syn::File, report: &mut Report) {
    let mut l = Linter {
        m: model,
        findings: Vec::new(),
        scopes: Vec::new(),
        impl_self: Vec::new(),
    };
    l.visit_file(ast);
    let mut nodes = NodeSpans::default();
    nodes.visit_file(ast);
    let (mut allows, bad) = parse_allows(src);
    for (line, msg) in bad {
        report.findings.push(Finding {
            file: rel.into(),
            line,
            msg,
        });
    }
    for a in &mut allows {
        // The outermost node that starts on the next line.
        a.range = nodes
            .spans
            .iter()
            .filter(|(s, _)| *s == a.line + 1)
            .max_by_key(|(_, e)| *e)
            .copied();
    }
    for (line, msg) in l.findings {
        // The innermost allow that covers the line.
        let cover = allows
            .iter_mut()
            .filter(|a| a.range.is_some_and(|(s, e)| (s..=e).contains(&line)))
            .min_by_key(|a| a.range.map(|(s, e)| e - s));
        match cover {
            Some(a) => a.used += 1,
            None => report.findings.push(Finding {
                file: rel.into(),
                line,
                msg,
            }),
        }
    }
    for a in allows {
        match a.range {
            None => report.findings.push(Finding {
                file: rel.into(),
                line: a.line,
                msg: "this gc-lint allow is not followed by an item, field, variant or statement \
                      on the next line"
                    .into(),
            }),
            Some(_) if a.used == 0 => report.findings.push(Finding {
                file: rel.into(),
                line: a.line,
                msg: "this gc-lint allow covers no finding: remove it".into(),
            }),
            Some((start, end)) => report.allows.push(AllowUse {
                file: rel.into(),
                line: a.line,
                reason: a.reason,
                start,
                end,
                used: a.used,
            }),
        }
    }
}

// ---- allows ---------------------------------------------------------------------------------

struct Allow {
    line: usize,
    reason: String,
    range: Option<(usize, usize)>,
    used: usize,
}

/// The `// gc-lint: allow(<reason>)` lines of `src`, and the malformed `// gc-lint:` lines.
fn parse_allows(src: &str) -> (Vec<Allow>, Vec<(usize, String)>) {
    let mut allows = Vec::new();
    let mut bad = Vec::new();
    for (i, text) in src.lines().enumerate() {
        let line = i + 1;
        let Some(c) = text.trim_start().strip_prefix("//") else {
            continue;
        };
        // Doc comments (`///`, `//!`) may show the syntax.
        if c.starts_with('/') || c.starts_with('!') {
            continue;
        }
        let Some(rest) = c.trim_start().strip_prefix(ALLOW_PREFIX) else {
            continue;
        };
        let reason = rest
            .trim()
            .strip_prefix("allow(")
            .and_then(|r| r.strip_suffix(')'))
            .map(str::trim);
        match reason {
            Some(r) if !r.is_empty() => allows.push(Allow {
                line,
                reason: r.to_string(),
                range: None,
                used: 0,
            }),
            _ => bad.push((
                line,
                "malformed gc-lint comment: write `// gc-lint: allow(<reason>)`".to_string(),
            )),
        }
    }
    (allows, bad)
}

/// Lines `(start, end)` of every item, impl or trait item, field, variant and statement.
#[derive(Default)]
struct NodeSpans {
    spans: Vec<(usize, usize)>,
}

impl NodeSpans {
    fn add(&mut self, s: proc_macro2::Span) {
        self.spans.push((s.start().line, s.end().line));
    }
}

impl<'ast> Visit<'ast> for NodeSpans {
    fn visit_item(&mut self, i: &'ast Item) {
        self.add(i.span());
        visit::visit_item(self, i);
    }
    fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
        self.add(i.span());
        visit::visit_impl_item(self, i);
    }
    fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
        self.add(i.span());
        visit::visit_trait_item(self, i);
    }
    fn visit_foreign_item(&mut self, i: &'ast syn::ForeignItem) {
        self.add(i.span());
        visit::visit_foreign_item(self, i);
    }
    fn visit_field(&mut self, i: &'ast syn::Field) {
        self.add(i.span());
        visit::visit_field(self, i);
    }
    fn visit_variant(&mut self, i: &'ast syn::Variant) {
        self.add(i.span());
        visit::visit_variant(self, i);
    }
    fn visit_stmt(&mut self, i: &'ast syn::Stmt) {
        self.add(i.span());
        visit::visit_stmt(self, i);
    }
}

// ---- the model (pass 1) ---------------------------------------------------------------------

/// A struct, enum, union or alias: its type parameters and the types it holds.
struct Def {
    name: String,
    params: Vec<String>,
    tys: Vec<Type>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ParamRole {
    /// A field holds the parameter by value.
    held: bool,
    /// A field puts it in heap memory.
    heap: bool,
}

/// What a value of a type is, as far as the closure check needs.
#[derive(Clone, Debug, Default)]
struct Summary {
    /// The type's name (last path segment; references stripped).
    name: Option<String>,
    /// The stack-only type it holds by value.
    holds: Option<String>,
    /// Its first type argument (`?` on a `Result` or `Option`).
    first: Option<Box<Summary>>,
}

#[derive(Default)]
struct Model {
    /// Stack-only type -> the base type it holds (itself for a base type).
    stack_only: BTreeMap<String, String>,
    /// `use a::B as C`: C -> B.
    renames: HashMap<String, String>,
    /// Local generic types: the role of each type parameter.
    params: HashMap<String, Vec<ParamRole>>,
    /// (impl type, or "" for a free fn; fn name) -> return types.
    fns: HashMap<(String, String), Vec<Summary>>,
    /// (type, field) -> field types.
    fields: HashMap<(String, String), Vec<Summary>>,
}

/// Pass 1 collector.
#[derive(Default)]
struct Collect {
    defs: Vec<Def>,
    renames: HashMap<String, String>,
    /// (impl type name, or "" for a free fn; fn name; return type).
    fns: Vec<(String, String, Type)>,
    /// (type, field, field type).
    fields: Vec<(String, String, Type)>,
    impl_self: Vec<Option<String>>,
}

fn type_params(g: &syn::Generics) -> Vec<String> {
    g.params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The name of a path type: its last segment.
fn last_ident(p: &syn::Path) -> Option<String> {
    p.segments.last().map(|s| s.ident.to_string())
}

fn self_type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) if tp.qself.is_none() => last_ident(&tp.path),
        Type::Paren(p) => self_type_name(&p.elem),
        Type::Group(g) => self_type_name(&g.elem),
        _ => None,
    }
}

impl Collect {
    fn fields(&mut self, name: &str, fields: &syn::Fields, tys: &mut Vec<Type>) {
        for (i, f) in fields.iter().enumerate() {
            tys.push(f.ty.clone());
            let fname = f.ident.as_ref().map_or(i.to_string(), |id| id.to_string());
            self.fields.push((name.to_string(), fname, f.ty.clone()));
        }
    }

    fn use_tree(&mut self, t: &syn::UseTree) {
        match t {
            syn::UseTree::Path(p) => self.use_tree(&p.tree),
            syn::UseTree::Rename(r) if r.ident != r.rename => {
                self.renames
                    .insert(r.rename.to_string(), r.ident.to_string());
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|i| self.use_tree(i)),
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for Collect {
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        let name = i.ident.to_string();
        let mut tys = Vec::new();
        self.fields(&name, &i.fields, &mut tys);
        self.defs.push(Def {
            name,
            params: type_params(&i.generics),
            tys,
        });
        visit::visit_item_struct(self, i);
    }
    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        let name = i.ident.to_string();
        let tys = i
            .variants
            .iter()
            .flat_map(|v| v.fields.iter().map(|f| f.ty.clone()))
            .collect();
        self.defs.push(Def {
            name,
            params: type_params(&i.generics),
            tys,
        });
        visit::visit_item_enum(self, i);
    }
    fn visit_item_union(&mut self, i: &'ast syn::ItemUnion) {
        let name = i.ident.to_string();
        let mut tys = Vec::new();
        self.fields(&name, &syn::Fields::Named(i.fields.clone()), &mut tys);
        self.defs.push(Def {
            name,
            params: type_params(&i.generics),
            tys,
        });
        visit::visit_item_union(self, i);
    }
    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        self.defs.push(Def {
            name: i.ident.to_string(),
            params: type_params(&i.generics),
            tys: vec![(*i.ty).clone()],
        });
        visit::visit_item_type(self, i);
    }
    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        self.use_tree(&i.tree);
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        self.impl_self.push(self_type_name(&i.self_ty));
        visit::visit_item_impl(self, i);
        self.impl_self.pop();
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        self.impl_self.push(None);
        visit::visit_item_trait(self, i);
        self.impl_self.pop();
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if let (Some(Some(owner)), ReturnType::Type(_, t)) = (self.impl_self.last(), &i.sig.output)
        {
            self.fns
                .push((owner.clone(), i.sig.ident.to_string(), (**t).clone()));
        }
        visit::visit_impl_item_fn(self, i);
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if let ReturnType::Type(_, t) = &i.sig.output {
            self.fns
                .push((String::new(), i.sig.ident.to_string(), (**t).clone()));
        }
        // Items inside a fn body are not methods of an enclosing impl.
        let saved = std::mem::take(&mut self.impl_self);
        visit::visit_item_fn(self, i);
        self.impl_self = saved;
    }
}

/// Where a type walk stands.
#[derive(Clone, Copy)]
struct At<'p> {
    /// The container path whose type argument holds the current type.
    heap: Option<&'p syn::Path>,
    /// Reached from the root by value (no reference, pointer, fn pointer or trait object).
    by_value: bool,
}

const CUT: At<'static> = At {
    heap: None,
    by_value: false,
};
const ROOT: At<'static> = At {
    heap: None,
    by_value: true,
};

/// Whether references and raw pointers hold their target (a `static` holds what its
/// references point to; a value on the stack does not).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Value,
    Static,
}

enum Role {
    Heap,
    Held,
    Cut,
}

type PathFn<'a, 'p> = dyn FnMut(&'p syn::Path, At<'p>) + 'a;

impl Model {
    fn build<'f>(files: impl Iterator<Item = &'f syn::File>) -> Model {
        let mut c = Collect::default();
        for f in files {
            c.visit_file(f);
        }
        let mut m = Model {
            stack_only: STACK_ONLY
                .iter()
                .map(|s| (s.to_string(), s.to_string()))
                .collect(),
            renames: c.renames,
            ..Model::default()
        };
        // Fixed point: both sets only grow.
        loop {
            let mut changed = false;
            for d in &c.defs {
                if !m.stack_only.contains_key(&d.name)
                    && let Some(h) = d.tys.iter().find_map(|t| m.holds(t, Reach::Value, None))
                {
                    let base = m.stack_only[&h].clone();
                    m.stack_only.insert(d.name.clone(), base);
                    changed = true;
                }
                if d.params.is_empty() {
                    continue;
                }
                let mut roles = m
                    .params
                    .get(&d.name)
                    .cloned()
                    .unwrap_or_else(|| vec![ParamRole::default(); d.params.len()]);
                roles.resize(roles.len().max(d.params.len()), ParamRole::default());
                let before = roles.clone();
                for t in &d.tys {
                    m.walk_type(t, Reach::Value, ROOT, &mut |p, at| {
                        if p.leading_colon.is_none()
                            && p.segments.len() == 1
                            && let Some(i) = d.params.iter().position(|x| p.segments[0].ident == x)
                        {
                            roles[i].held |= at.by_value;
                            roles[i].heap |= at.heap.is_some();
                        }
                    });
                }
                if roles != before {
                    m.params.insert(d.name.clone(), roles);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut fns: HashMap<(String, String), Vec<Summary>> = HashMap::new();
        for (owner, name, ty) in &c.fns {
            let self_name = (!owner.is_empty()).then_some(owner.as_str());
            let s = m.summary(ty, self_name);
            fns.entry((m.resolve(owner).to_string(), name.clone()))
                .or_default()
                .push(s);
        }
        let mut fields: HashMap<(String, String), Vec<Summary>> = HashMap::new();
        for (owner, name, ty) in &c.fields {
            let s = m.summary(ty, Some(owner));
            fields
                .entry((owner.clone(), name.clone()))
                .or_default()
                .push(s);
        }
        m.fns = fns;
        m.fields = fields;
        m
    }

    /// Follows `use .. as` renames.
    fn resolve<'a>(&'a self, name: &'a str) -> &'a str {
        let mut n = name;
        for _ in 0..8 {
            match self.renames.get(n) {
                Some(next) => n = next,
                None => break,
            }
        }
        n
    }

    /// The stack-only type that `p` names, if any (`Self` is `self_name`).
    fn stack_only_name(&self, p: &syn::Path, self_name: Option<&str>) -> Option<&str> {
        let id = p.segments.last()?.ident.to_string();
        let n = match (id.as_str(), self_name) {
            ("Self", Some(s)) => s,
            _ => id.as_str(),
        };
        let n = self.resolve(n);
        self.stack_only.get_key_value(n).map(|(k, _)| k.as_str())
    }

    fn role(&self, name: &str, i: usize) -> Role {
        if HEAP_CONTAINERS.contains(&name) {
            return Role::Heap;
        }
        if OPAQUE.contains(&name) {
            return Role::Cut;
        }
        match self.params.get(name).and_then(|r| r.get(i)) {
            Some(r) if r.heap => Role::Heap,
            Some(r) if r.held => Role::Held,
            Some(_) => Role::Cut,
            // A type that is not local, or an argument past the declared ones.
            None => Role::Held,
        }
    }

    /// Calls `f` for every path type in `ty` with where it stands.
    fn walk_type<'p>(&self, ty: &'p Type, reach: Reach, at: At<'p>, f: &mut PathFn<'_, 'p>) {
        match ty {
            Type::Path(tp) => match &tp.qself {
                Some(q) => {
                    self.walk_type(&q.ty, reach, CUT, f);
                    self.walk_path(&tp.path, reach, CUT, f);
                }
                None => self.walk_path(&tp.path, reach, at, f),
            },
            Type::Reference(r) => {
                let next = if reach == Reach::Static { at } else { CUT };
                self.walk_type(&r.elem, reach, next, f);
            }
            Type::Ptr(p) => {
                let next = if reach == Reach::Static { at } else { CUT };
                self.walk_type(&p.elem, reach, next, f);
            }
            Type::Array(a) => self.walk_type(&a.elem, reach, at, f),
            Type::Slice(s) => self.walk_type(&s.elem, reach, at, f),
            Type::Tuple(t) => t.elems.iter().for_each(|e| self.walk_type(e, reach, at, f)),
            Type::Paren(p) => self.walk_type(&p.elem, reach, at, f),
            Type::Group(g) => self.walk_type(&g.elem, reach, at, f),
            Type::BareFn(b) => {
                for a in &b.inputs {
                    self.walk_type(&a.ty, reach, CUT, f);
                }
                if let ReturnType::Type(_, t) = &b.output {
                    self.walk_type(t, reach, CUT, f);
                }
            }
            Type::TraitObject(t) => self.walk_bounds(t.bounds.iter(), reach, f),
            Type::ImplTrait(t) => self.walk_bounds(t.bounds.iter(), reach, f),
            _ => {}
        }
    }

    fn walk_bounds<'p>(
        &self,
        bounds: impl Iterator<Item = &'p TypeParamBound>,
        reach: Reach,
        f: &mut PathFn<'_, 'p>,
    ) {
        for b in bounds {
            if let TypeParamBound::Trait(t) = b {
                self.walk_path(&t.path, reach, CUT, f);
            }
        }
    }

    /// A path in a type or an expression (`Vec::<Local>::new`): its generic arguments, each
    /// with the role its segment gives it.
    fn walk_path<'p>(&self, path: &'p syn::Path, reach: Reach, at: At<'p>, f: &mut PathFn<'_, 'p>) {
        f(path, at);
        // The marker of `Local<'s, JsObject>` is not a value.
        if self.stack_only_name(path, None).is_some() {
            return;
        }
        for seg in &path.segments {
            let id = seg.ident.to_string();
            let name = self.resolve(&id);
            match &seg.arguments {
                PathArguments::None => {}
                PathArguments::Parenthesized(p) => {
                    for t in &p.inputs {
                        self.walk_type(t, reach, CUT, f);
                    }
                    if let ReturnType::Type(_, t) = &p.output {
                        self.walk_type(t, reach, CUT, f);
                    }
                }
                PathArguments::AngleBracketed(a) => {
                    let mut i = 0;
                    for arg in &a.args {
                        match arg {
                            GenericArgument::Type(t) => {
                                let next = match self.role(name, i) {
                                    Role::Heap => At {
                                        heap: Some(path),
                                        by_value: at.by_value,
                                    },
                                    Role::Held => at,
                                    Role::Cut => CUT,
                                };
                                i += 1;
                                self.walk_type(t, reach, next, f);
                            }
                            GenericArgument::AssocType(a) => self.walk_type(&a.ty, reach, CUT, f),
                            GenericArgument::Constraint(c) => {
                                self.walk_bounds(c.bounds.iter(), reach, f)
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// The stack-only type that a value of `ty` holds.
    fn holds(&self, ty: &Type, reach: Reach, self_name: Option<&str>) -> Option<String> {
        let mut found = None;
        self.walk_type(ty, reach, ROOT, &mut |p, at| {
            if found.is_none() && at.by_value {
                found = self.stack_only_name(p, self_name).map(str::to_string);
            }
        });
        found
    }

    fn summary(&self, ty: &Type, self_name: Option<&str>) -> Summary {
        match ty {
            Type::Reference(r) => Summary {
                name: self.summary(&r.elem, self_name).name,
                ..Summary::default()
            },
            Type::Paren(p) => self.summary(&p.elem, self_name),
            Type::Group(g) => self.summary(&g.elem, self_name),
            Type::Path(tp) if tp.qself.is_none() => {
                let id = last_ident(&tp.path);
                let name = match (id.as_deref(), self_name) {
                    (Some("Self"), Some(s)) => Some(s.to_string()),
                    _ => id.map(|n| self.resolve(&n).to_string()),
                };
                let first = tp.path.segments.last().and_then(|s| match &s.arguments {
                    PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| match g {
                        GenericArgument::Type(t) => Some(Box::new(self.summary(t, self_name))),
                        _ => None,
                    }),
                    _ => None,
                });
                Summary {
                    name,
                    holds: self.holds(ty, Reach::Value, self_name),
                    first,
                }
            }
            _ => Summary {
                holds: self.holds(ty, Reach::Value, self_name),
                ..Summary::default()
            },
        }
    }

    /// The summary of a value of the type named `name` (a struct literal, a constructor).
    fn named(&self, name: &str) -> Summary {
        let n = self.resolve(name);
        Summary {
            name: Some(n.to_string()),
            holds: self.stack_only.contains_key(n).then(|| n.to_string()),
            first: None,
        }
    }

    /// The return type of `owner::name` (`owner` "" for a free fn); with several candidates,
    /// one that holds a stack-only type wins.
    fn returned(&self, owner: &str, name: &str) -> Option<Summary> {
        pick(self.fns.get(&(owner.to_string(), name.to_string())))
    }

    fn field(&self, owner: &str, name: &str) -> Option<Summary> {
        pick(self.fields.get(&(owner.to_string(), name.to_string())))
    }
}

fn pick(c: Option<&Vec<Summary>>) -> Option<Summary> {
    let c = c?;
    c.iter()
        .find(|s| s.holds.is_some())
        .or_else(|| c.first())
        .cloned()
}

// ---- the lint (pass 2) ----------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Binding {
    ty: Summary,
    /// A closure binding: the stack-only bindings it captures (name, type).
    captures: Vec<(String, String)>,
}

struct Linter<'m> {
    m: &'m Model,
    findings: Vec<(usize, String)>,
    /// Bindings of the fn being visited, innermost scope last.
    scopes: Vec<Vec<(String, Binding)>>,
    impl_self: Vec<Option<String>>,
}

/// Short type text: `Vec<Local<'s>>`.
fn text(t: &impl ToTokens) -> String {
    let mut s = t.to_token_stream().to_string();
    for (from, to) in [
        (" < ", "<"),
        ("< ", "<"),
        (" <", "<"),
        (" >", ">"),
        (" :: ", "::"),
        (":: ", "::"),
        (" ,", ","),
        ("& '", "&'"),
        ("& ", "&"),
        (" ;", ";"),
        ("[ ", "["),
        (" ]", "]"),
        ("( ", "("),
        (" )", ")"),
    ] {
        s = s.replace(from, to);
    }
    if s.chars().count() > 80 {
        s = s.chars().take(77).collect::<String>() + "...";
    }
    s
}

fn line_of(s: &impl Spanned) -> usize {
    s.span().start().line
}

/// Every identifier a pattern binds.
fn pat_idents(p: &Pat, out: &mut Vec<String>) {
    struct V<'o>(&'o mut Vec<String>);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
            self.0.push(i.ident.to_string());
            visit::visit_pat_ident(self, i);
        }
        // Types and expressions inside a pattern bind nothing.
        fn visit_type(&mut self, _: &'ast Type) {}
        fn visit_expr(&mut self, _: &'ast Expr) {}
    }
    V(out).visit_pat(p);
}

/// Identifiers a closure body uses and binds (an over-approximation of its free variables).
#[derive(Default)]
struct Free {
    used: Vec<String>,
    bound: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Free {
    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        if e.qself.is_none()
            && e.path.leading_colon.is_none()
            && e.path.segments.len() == 1
            && e.path.segments[0].arguments.is_none()
        {
            self.used.push(e.path.segments[0].ident.to_string());
        }
        visit::visit_expr_path(self, e);
    }
    fn visit_pat(&mut self, p: &'ast Pat) {
        let mut v = Vec::new();
        pat_idents(p, &mut v);
        self.bound.extend(v);
        visit::visit_pat(self, p);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        idents(&m.tokens, &mut self.used);
    }
}

fn idents(ts: &TokenStream, out: &mut Vec<String>) {
    for t in ts.clone() {
        match t {
            TokenTree::Ident(i) => out.push(i.to_string()),
            TokenTree::Group(g) => idents(&g.stream(), out),
            _ => {}
        }
    }
}

fn parse_items(input: ParseStream<'_>) -> syn::Result<Vec<Item>> {
    let mut v = Vec::new();
    while !input.is_empty() {
        v.push(input.parse()?);
    }
    Ok(v)
}

/// `e` without parentheses, groups and casts.
fn strip(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(p) => strip(&p.expr),
        Expr::Group(g) => strip(&g.expr),
        Expr::Cast(c) => strip(&c.expr),
        _ => e,
    }
}

/// `Box::new`, `Rc::pin`, ...: the container's name.
fn boxing_call(func: &Expr) -> Option<String> {
    let Expr::Path(p) = strip(func) else {
        return None;
    };
    let n = p.path.segments.len();
    if p.qself.is_some() || n < 2 {
        return None;
    }
    let f = p.path.segments[n - 1].ident.to_string();
    let t = p.path.segments[n - 2].ident.to_string();
    (BOXING.contains(&t.as_str()) && (f == "new" || f == "pin")).then(|| format!("{t}::{f}"))
}

impl Linter<'_> {
    fn finding(&mut self, line: usize, msg: String) {
        self.findings.push((line, msg));
    }

    fn self_name(&self) -> Option<String> {
        self.impl_self.last().cloned().flatten()
    }

    /// A stack-only type inside a heap container, anywhere in `ty` or in the path `p`.
    fn lint_walk(&mut self, ty: Option<&Type>, p: Option<&syn::Path>) {
        let m = self.m;
        let mut out = Vec::new();
        let mut f = |path: &syn::Path, at: At<'_>| {
            if let (Some(c), Some(n)) = (at.heap, m.stack_only_name(path, None)) {
                let container = c.segments.iter().rev().find_map(|s| {
                    let id = s.ident.to_string();
                    let r = m.resolve(&id);
                    (!matches!(s.arguments, PathArguments::None)).then(|| r.to_string())
                });
                let base = &m.stack_only[n];
                let what = if base == n {
                    format!("`{n}`")
                } else {
                    format!("`{n}` (it holds a `{base}`)")
                };
                out.push((
                    line_of(path),
                    format!(
                        "`{}` puts {what} in Rust heap memory (`{}`); a stack-only GC type \
                         must stay on the native stack (H20 R2)",
                        text(c),
                        container.unwrap_or_default()
                    ),
                ));
            }
        };
        if let Some(t) = ty {
            m.walk_type(t, Reach::Value, ROOT, &mut f);
        }
        if let Some(p) = p {
            m.walk_path(p, Reach::Value, ROOT, &mut f);
        }
        self.findings.extend(out);
    }

    fn static_holds(&mut self, kind: &str, s: &syn::ItemStatic) {
        if let Some(n) = self.m.holds(&s.ty, Reach::Static, None) {
            self.finding(
                line_of(&s.ident),
                format!(
                    "{kind} `{}: {}` holds a `{n}`; a stack-only GC type never lives in a \
                     {kind} (H20 R2)",
                    s.ident,
                    text(&s.ty)
                ),
            );
        }
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.iter().rev().find(|(n, _)| n == name).map(|(_, b)| b))
    }

    fn bind(&mut self, name: String, b: Binding) {
        if self.scopes.is_empty() {
            self.scopes.push(Vec::new());
        }
        self.scopes.last_mut().expect("a scope").push((name, b));
    }

    fn bind_unknown(&mut self, p: &Pat) {
        let mut v = Vec::new();
        pat_idents(p, &mut v);
        for n in v {
            self.bind(n, Binding::default());
        }
    }

    /// Binds `p` to a value of type `ty` (unknown for a destructuring pattern).
    fn bind_typed(&mut self, p: &Pat, ty: Summary) {
        match p {
            Pat::Ident(i) if i.subpat.is_none() => self.bind(
                i.ident.to_string(),
                Binding {
                    ty,
                    captures: Vec::new(),
                },
            ),
            Pat::Type(t) => {
                let s = self.m.summary(&t.ty, self.self_name().as_deref());
                self.bind_typed(&t.pat, s)
            }
            _ => self.bind_unknown(p),
        }
    }

    fn bind_inputs<'a>(&mut self, inputs: impl Iterator<Item = &'a FnArg>) {
        let self_name = self.self_name();
        for a in inputs {
            match a {
                FnArg::Receiver(r) => {
                    let s = self.m.summary(&r.ty, self_name.as_deref());
                    self.bind(
                        "self".into(),
                        Binding {
                            ty: s,
                            captures: Vec::new(),
                        },
                    );
                }
                FnArg::Typed(t) => {
                    let s = self.m.summary(&t.ty, self_name.as_deref());
                    self.bind_typed(&t.pat, s);
                }
            }
        }
    }

    /// What the value of `e` is, as far as the scanned declarations tell.
    fn expr_summary(&self, e: &Expr) -> Option<Summary> {
        match e {
            Expr::Path(p) if p.qself.is_none() => {
                let segs = &p.path.segments;
                if segs.len() == 1 && p.path.leading_colon.is_none() {
                    return self
                        .lookup(&segs[0].ident.to_string())
                        .map(|b| b.ty.clone());
                }
                None
            }
            Expr::Paren(p) => self.expr_summary(&p.expr),
            Expr::Group(g) => self.expr_summary(&g.expr),
            Expr::Reference(r) => Some(Summary {
                name: self.expr_summary(&r.expr)?.name,
                ..Summary::default()
            }),
            Expr::Cast(c) => Some(self.m.summary(&c.ty, self.self_name().as_deref())),
            Expr::Struct(s) => last_ident(&s.path).map(|n| self.m.named(&n)),
            Expr::Try(t) => self.expr_summary(&t.expr)?.first.map(|b| *b),
            Expr::Field(f) => {
                let owner = self.expr_summary(&f.base)?.name?;
                let field = match &f.member {
                    syn::Member::Named(i) => i.to_string(),
                    syn::Member::Unnamed(i) => i.index.to_string(),
                };
                self.m.field(&owner, &field)
            }
            Expr::MethodCall(c) => {
                let owner = self.expr_summary(&c.receiver)?.name?;
                self.m.returned(&owner, &c.method.to_string())
            }
            Expr::Call(c) => {
                let Expr::Path(p) = strip(&c.func) else {
                    return None;
                };
                let segs = &p.path.segments;
                match segs.len() {
                    0 => None,
                    1 => {
                        let n = segs[0].ident.to_string();
                        self.m.returned("", &n).or_else(|| {
                            // A tuple struct constructor.
                            let r = self.m.resolve(&n);
                            self.m.stack_only.contains_key(r).then(|| self.m.named(r))
                        })
                    }
                    k => {
                        let owner = segs[k - 2].ident.to_string();
                        let owner = match (owner.as_str(), self.self_name()) {
                            ("Self", Some(s)) => s,
                            _ => owner,
                        };
                        let f = segs[k - 1].ident.to_string();
                        self.m
                            .returned(self.m.resolve(&owner), &f)
                            .or_else(|| self.m.returned("", &f))
                    }
                }
            }
            _ => None,
        }
    }

    /// The stack-only bindings a closure captures: (name, type).
    fn captures(&self, c: &syn::ExprClosure) -> Vec<(String, String)> {
        let mut free = Free::default();
        for p in &c.inputs {
            free.visit_pat(p);
        }
        free.visit_expr(&c.body);
        let mut out: Vec<(String, String)> = Vec::new();
        let mut seen = BTreeSet::new();
        for n in free.used {
            if free.bound.contains(&n) || !seen.insert(n.clone()) {
                continue;
            }
            if let Some(b) = self.lookup(&n) {
                if let Some(h) = &b.ty.holds {
                    out.push((n.clone(), h.clone()));
                }
                out.extend(b.captures.iter().cloned());
            }
        }
        out
    }

    fn report_captures(&mut self, line: usize, call: &str, caps: &[(String, String)]) {
        if caps.is_empty() {
            return;
        }
        let list = caps
            .iter()
            .map(|(n, t)| format!("`{n}` (`{t}`)"))
            .collect::<Vec<_>>()
            .join(", ");
        self.finding(
            line,
            format!(
                "a closure given to `{call}` captures {list}: the capture lives in Rust heap \
                 memory; a stack-only GC type must stay on the native stack (H20 R2)"
            ),
        );
    }

    fn fn_body(&mut self, inputs: Vec<&FnArg>, f: impl FnOnce(&mut Self)) {
        let saved = std::mem::take(&mut self.scopes);
        self.scopes.push(Vec::new());
        self.bind_inputs(inputs.into_iter());
        f(self);
        self.scopes = saved;
    }
}

impl<'ast> Visit<'ast> for Linter<'_> {
    fn visit_type(&mut self, t: &'ast Type) {
        self.lint_walk(Some(t), None);
    }

    fn visit_path(&mut self, p: &'ast syn::Path) {
        self.lint_walk(None, Some(p));
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.static_holds("static", s);
        visit::visit_item_static(self, s);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = last_ident(&m.path).unwrap_or_default();
        match name.as_str() {
            "macro_rules" => {}
            "thread_local" => {
                if let Ok(items) = m.parse_body_with(parse_items) {
                    for i in &items {
                        match i {
                            // Not `self.visit_item_static`: one finding, as a thread_local.
                            Item::Static(s) => {
                                self.static_holds("thread_local", s);
                                visit::visit_item_static(self, s);
                            }
                            _ => Visit::visit_item(self, i),
                        }
                    }
                } else {
                    let mut ids = Vec::new();
                    idents(&m.tokens, &mut ids);
                    if let Some(n) = ids
                        .iter()
                        .find(|i| self.m.stack_only.contains_key(self.m.resolve(i)))
                    {
                        self.finding(
                            line_of(m),
                            format!(
                                "thread_local names `{n}`; a stack-only GC type never lives \
                                 in a thread_local (H20 R2)"
                            ),
                        );
                    }
                }
            }
            _ => {
                if let Ok(items) = m.parse_body_with(parse_items) {
                    for i in &items {
                        Visit::visit_item(self, i);
                    }
                } else if let Ok(exprs) =
                    m.parse_body_with(Punctuated::<Expr, syn::Token![,]>::parse_terminated)
                {
                    for e in &exprs {
                        Visit::visit_expr(self, e);
                    }
                }
            }
        }
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        self.impl_self.push(self_type_name(&i.self_ty));
        visit::visit_item_impl(self, i);
        self.impl_self.pop();
    }

    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        self.impl_self.push(None);
        visit::visit_item_trait(self, i);
        self.impl_self.pop();
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let saved = std::mem::take(&mut self.impl_self);
        self.fn_body(i.sig.inputs.iter().collect(), |s| {
            visit::visit_item_fn(s, i)
        });
        self.impl_self = saved;
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.fn_body(i.sig.inputs.iter().collect(), |s| {
            visit::visit_impl_item_fn(s, i)
        });
    }

    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        self.fn_body(i.sig.inputs.iter().collect(), |s| {
            visit::visit_trait_item_fn(s, i)
        });
    }

    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.scopes.push(Vec::new());
        visit::visit_block(self, b);
        self.scopes.pop();
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        // The initializer sees the bindings before this one.
        let mut binding = None;
        if let Some(init) = &l.init {
            self.visit_expr(&init.expr);
            if let Some((_, e)) = &init.diverge {
                self.visit_expr(e);
            }
            binding = Some(match strip(&init.expr) {
                Expr::Closure(c) => Binding {
                    ty: Summary::default(),
                    captures: self.captures(c),
                },
                e => Binding {
                    ty: self.expr_summary(e).unwrap_or_default(),
                    captures: Vec::new(),
                },
            });
        }
        self.visit_pat(&l.pat);
        match (&l.pat, binding) {
            (Pat::Ident(i), Some(b)) if i.subpat.is_none() => self.bind(i.ident.to_string(), b),
            (p, Some(b)) => self.bind_typed(p, b.ty),
            (p, None) => self.bind_typed(p, Summary::default()),
        }
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        self.scopes.push(Vec::new());
        for p in &c.inputs {
            self.bind_typed(p, Summary::default());
        }
        visit::visit_expr_closure(self, c);
        self.scopes.pop();
    }

    fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
        self.visit_expr(&f.expr);
        self.scopes.push(Vec::new());
        self.bind_unknown(&f.pat);
        self.visit_pat(&f.pat);
        self.visit_block(&f.body);
        self.scopes.pop();
    }

    fn visit_arm(&mut self, a: &'ast syn::Arm) {
        self.scopes.push(Vec::new());
        self.bind_unknown(&a.pat);
        visit::visit_arm(self, a);
        self.scopes.pop();
    }

    fn visit_expr_let(&mut self, l: &'ast syn::ExprLet) {
        self.visit_expr(&l.expr);
        self.bind_unknown(&l.pat);
        self.visit_pat(&l.pat);
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let Some(call) = boxing_call(&c.func) {
            for a in &c.args {
                match strip(a) {
                    Expr::Closure(cl) => {
                        let caps = self.captures(cl);
                        self.report_captures(line_of(cl), &call, &caps);
                    }
                    Expr::Path(p)
                        if p.qself.is_none()
                            && p.path.segments.len() == 1
                            && p.path.leading_colon.is_none() =>
                    {
                        let caps = self
                            .lookup(&p.path.segments[0].ident.to_string())
                            .map(|b| b.captures.clone())
                            .unwrap_or_default();
                        self.report_captures(line_of(p), &call, &caps);
                    }
                    _ => {}
                }
            }
        }
        visit::visit_expr_call(self, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint_src(src: &str) -> Vec<(usize, String)> {
        let ast = syn::parse_file(src).unwrap();
        let m = Model::build([&ast].into_iter());
        let mut r = Report::default();
        lint_file(&m, "t.rs", src, &ast, &mut r);
        r.findings.sort();
        r.findings.into_iter().map(|f| (f.line, f.msg)).collect()
    }

    fn lines(src: &str) -> Vec<usize> {
        let mut v: Vec<usize> = lint_src(src).into_iter().map(|(l, _)| l).collect();
        v.dedup();
        v
    }

    #[test]
    fn containers_and_cuts() {
        let src = "\
struct A<'s> { a: Vec<Local<'s>> }
struct B<'s> { b: Vec<&'s Local<'s>>, c: core::marker::PhantomData<Local<'s>> }
struct C { f: Vec<fn(Local<'static>) -> Local<'static>>, g: Box<dyn Fn(Local<'static>)> }
type D<'s> = std::collections::HashMap<u32, (u8, [PropertyDescriptor<'s>; 2])>;
fn e<'a>() -> &'a Vec<JsStr<'a>> { todo!() }
fn f() { let _v = Vec::<Local<'_>>::new(); }
";
        assert_eq!(lines(src), [1, 4, 5, 6]);
    }

    #[test]
    fn derived_and_renamed_types() {
        let src = "\
use crate::abi::Local as L;
struct Holder<'s> { l: Option<L<'s>> }
enum Pair<'s> { One(Holder<'s>), Two }
struct Stack<T> { items: Vec<T> }
struct Wrap<T> { t: T }
fn a(_: Vec<Pair<'_>>) {}
fn b(_: Stack<Local<'_>>, _: Wrap<Local<'_>>, _: Stack<u32>) {}
fn c(_: Vec<Wrap<Holder<'_>>>) {}
";
        let f = lint_src(src);
        assert_eq!(
            f.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            [6, 7, 8],
            "{f:#?}"
        );
        assert!(f[0].1.contains("`Pair` (it holds a `Local`)"), "{}", f[0].1);
    }

    #[test]
    fn statics_and_thread_locals() {
        let src = "\
static A: Option<Local<'static>> = None;
static B: &[Local<'static>] = &[];
static C: &[(&str, HostFn)] = &[];
type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>) -> Local<'s>;
thread_local! {
    static D: Cell<Option<Local<'static>>> = const { Cell::new(None) };
    static E: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
fn f() {
    thread_local!(static G: RefCell<Vec<u64>> = RefCell::new(Vec::new()));
    static H: Option<ScratchList<'static>> = None;
}
";
        let kinds: Vec<_> = lint_src(src)
            .into_iter()
            .map(|(l, m)| (l, m.split(' ').next().unwrap().to_string()))
            .collect();
        let want = [
            (1, "static"),
            (2, "static"),
            (6, "thread_local"),
            (11, "static"),
        ];
        assert_eq!(kinds, want.map(|(l, k)| (l, k.to_string())));
    }

    #[test]
    fn boxed_closures() {
        let src = "\
impl<'s> Args<'s> { fn get(&self, i: usize) -> Local<'s> { todo!() } }
impl<'s> Cx<'s> { fn to_object(&mut self) -> JsResult<Local<'s, JsObject>> { todo!() } }
fn f<'s>(cx: &mut Cx<'s>, args: Args<'s>, n: u32) -> JsResult<()> {
    let a = args.get(0);
    let o = cx.to_object()?;
    let r = &a;
    let _ = Box::new(move || a.bits());
    let _ = Rc::new(move || o);
    let _ = Box::new(move || r.bits() + n as u64);
    let c = move || args.len();
    let _: Box<dyn Fn() -> usize> = Box::new(c);
    let _ = Box::new(|| format!(\"{}\", a.bits()));
    let a = 5u32;
    let _ = Box::new(move || a);
    let _ = || a;
    Ok(())
}
";
        let f = lint_src(src);
        assert_eq!(
            f.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            [7, 8, 11, 12],
            "{f:#?}"
        );
        assert!(
            f[0].1.contains("`Box::new` captures `a` (`Local`)"),
            "{}",
            f[0].1
        );
        assert!(f[2].1.contains("`args` (`Args`)"), "{}", f[2].1);
    }

    #[test]
    fn allows() {
        let src = "\
// gc-lint: allow(test of the lint)
fn a(_: Vec<Local<'_>>) {}
/// Doc.
// gc-lint: allow(misplaced: after the doc comment)
fn b(_: Vec<Local<'_>>) {}
// gc-lint: allow(nothing to allow)
fn c() {}
// gc-lint: allow(dangling)

fn d() {
    // gc-lint: allow(a statement)
    let _v: Vec<Local<'_>> = Vec::new();
}
// gc-lint: allow()
// gc-lint: alow(typo)
/// `// gc-lint: allow(x)` in a doc comment is text.
fn e() {}
// gc-lint: allow(outer)
fn g(_: Vec<Local<'_>>) {
    // gc-lint: allow(inner)
    let _v: Vec<Local<'_>> = Vec::new();
}
";
        let ast = syn::parse_file(src).unwrap();
        let m = Model::build([&ast].into_iter());
        let mut r = Report::default();
        lint_file(&m, "t.rs", src, &ast, &mut r);
        r.findings.sort();
        let got: Vec<_> = r.findings.iter().map(|f| f.line).collect();
        // The allow after b's doc comment (4) is not before a node (b starts on 3 with its
        // doc), so b's finding (5) stays; the unused allow (6); the dangling one (8); the
        // malformed ones (14, 15).
        assert_eq!(got, [4, 5, 6, 8, 14, 15], "{:#?}", r.findings);
        let a: Vec<_> = r
            .allows
            .iter()
            .map(|a| (a.line, a.start, a.end, a.used))
            .collect();
        // Nested allows: each finding goes to the innermost one.
        assert_eq!(
            a,
            [
                (1, 2, 2, 1),
                (11, 12, 12, 1),
                (18, 19, 22, 1),
                (20, 21, 21, 1)
            ]
        );
    }

    #[test]
    fn text_is_short_and_tidy() {
        let t: Type = syn::parse_str("std::collections::HashMap<u32, &'a [Local<'s>]>").unwrap();
        assert_eq!(text(&t), "std::collections::HashMap<u32, &'a [Local<'s>]>");
    }
}
