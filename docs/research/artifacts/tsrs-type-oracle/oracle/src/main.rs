//! ep9-typecheck prototype: type-check a tsconfig project with tsrs (one checker, single-threaded) and export a
//! side table of "typed IR hints" keyed by source span (UTF-8 byte offsets, token start .. end, no trivia).
//!
//! usage: ep9_typecheck <tsconfig.json|dir> <out.jsonl> [--no-check] [--files a,b]
//! Output: one JSON line per source file: {"file","js","dts","nodes":[...]} then a final {"stats":...} line.

use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value, json};
use tsrs_ast::{self as ast, Kind, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{self as checker, Checker, LiteralValue, ObjectFlags, Type, TypeFlags};
use tsrs_compiler::{
    Context, Program, ProgramOptions, get_diagnostics_of_any_program, new_cached_fs_compiler_host,
    new_program,
};
use tsrs_core::{P, Tristate, tspath};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{FS, bundled, osvfs};

struct OsParseConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for OsParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn max_rss_mib() -> f64 {
    // SAFETY: plain getrusage into a zeroed struct.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        // macOS reports bytes, Linux KiB.
        #[cfg(target_os = "macos")]
        return ru.ru_maxrss as f64 / (1024.0 * 1024.0);
        #[cfg(not(target_os = "macos"))]
        return ru.ru_maxrss as f64 / 1024.0;
    }
}

struct Ctx {
    program: &'static Program,
    depth_budget: u32,
}

fn src_pos(n: P<Node>) -> Option<u32> {
    n.pos().source_offset()
}

fn node_span(sf: P<SourceFile>, n: P<Node>) -> Option<(u32, u32)> {
    let pos = src_pos(n)?;
    let end = n.end().source_offset()?;
    Some((tsrs_scanner::skip_trivia(sf.text(), pos), end))
}

/// "file:offset" of a declaration, plus whether it comes from a .d.ts / the default lib.
fn decl_info(cx: &Ctx, decl: P<Node>) -> Value {
    let Some(sf) = ast::get_source_file_of_node(decl) else {
        return json!({"loc": "?"});
    };
    let off = src_pos(decl).map(|p| tsrs_scanner::skip_trivia(sf.text(), p)).unwrap_or(0);
    let name = tspath::get_base_file_name(sf.file_name());
    let mut m = Map::new();
    m.insert("loc".into(), json!(format!("{name}:{off}")));
    m.insert("dk".into(), json!(format!("{:?}", decl.kind())));
    if sf.is_declaration_file() {
        m.insert("dts".into(), json!(true));
    }
    if cx.program.is_source_file_default_library(sf.path()) {
        m.insert("lib".into(), json!(true));
    }
    Value::Object(m)
}

fn type_decl(cx: &Ctx, t: P<Type>, m: &mut Map<String, Value>) {
    if let Some(sym) = t.symbol() {
        m.insert("name".into(), json!(sym.name()));
        if let Some(&d) = sym.declarations().first() {
            if let Value::Object(dm) = decl_info(cx, d) {
                for (k, v) in dm {
                    if k != "dk" {
                        m.insert(k, v);
                    }
                }
            }
        }
    }
}

fn is_class_instance(t: P<Type>) -> bool {
    let of = t.object_flags();
    if of.intersects(ObjectFlags::Class) {
        return true;
    }
    if of.intersects(ObjectFlags::Reference) {
        if let Some(target) = t.target() {
            return target.object_flags().intersects(ObjectFlags::Class);
        }
    }
    false
}

/// Classify a type into a small lattice an AOT compiler can use.
fn classify(cx: &mut Ctx, c: &mut Checker, t: P<Type>) -> Value {
    let f = t.flags();
    let mut m = Map::new();
    let cat: &str = if f.intersects(TypeFlags::Any) {
        if t == c.get_error_type() { "error" } else { "any" }
    } else if f.intersects(TypeFlags::Unknown) {
        "unknown"
    } else if f.intersects(TypeFlags::Never) {
        "never"
    } else if f.intersects(TypeFlags::Union) {
        let members = t.types();
        let mut non_null = Vec::new();
        let mut nullish = Vec::new();
        for &mt in members {
            if mt.flags().intersects(TypeFlags::Nullable | TypeFlags::Void) {
                nullish.push(mt);
            } else {
                non_null.push(mt);
            }
        }
        let all_num = non_null.iter().all(|m| m.flags().intersects(TypeFlags::NumberLike));
        let all_str = non_null.iter().all(|m| m.flags().intersects(TypeFlags::StringLike));
        let all_bool = non_null.iter().all(|m| m.flags().intersects(TypeFlags::BooleanLike));
        let inner_cat = if non_null.is_empty() {
            None
        } else if all_num {
            Some("number")
        } else if all_str {
            Some("string")
        } else if all_bool {
            Some("boolean")
        } else {
            None
        };
        if !nullish.is_empty() && non_null.len() == 1 {
            if cx.depth_budget > 0 {
                cx.depth_budget -= 1;
                let inner = classify(cx, c, non_null[0]);
                cx.depth_budget += 1;
                m.insert("inner".into(), inner);
            }
            "nullable"
        } else if nullish.is_empty() && inner_cat.is_some() {
            inner_cat.unwrap()
        } else if let Some(ic) = inner_cat {
            m.insert("inner".into(), json!({"c": ic}));
            "nullable"
        } else {
            let mut cats: Vec<String> = Vec::new();
            if cx.depth_budget > 0 {
                cx.depth_budget -= 1;
                for &mt in members.iter().take(8) {
                    let v = classify(cx, c, mt);
                    let s = v.get("c").and_then(Value::as_str).unwrap_or("?").to_string();
                    let s = match v.get("name").and_then(Value::as_str) {
                        Some(n) => format!("{s}:{n}"),
                        None => s,
                    };
                    if !cats.contains(&s) {
                        cats.push(s);
                    }
                }
                cx.depth_budget += 1;
            }
            m.insert("n".into(), json!(members.len()));
            m.insert("members".into(), json!(cats));
            "union"
        }
    } else if f.intersects(TypeFlags::NumberLike) {
        if f.intersects(TypeFlags::NumberLiteral) {
            if let Some(LiteralValue::Number(n)) = t.as_literal_type().value() {
                m.insert("lit".into(), json!(n.0));
                if n.0.fract() == 0.0 && n.0.abs() < 2147483648.0 {
                    m.insert("int".into(), json!(true));
                }
            }
        }
        if f.intersects(TypeFlags::EnumLike) {
            m.insert("enum".into(), json!(true));
        }
        "number"
    } else if f.intersects(TypeFlags::StringLike) {
        "string"
    } else if f.intersects(TypeFlags::BooleanLike) {
        "boolean"
    } else if f.intersects(TypeFlags::BigIntLike) {
        "bigint"
    } else if f.intersects(TypeFlags::ESSymbolLike) {
        "symbol"
    } else if f.intersects(TypeFlags::Undefined) {
        "undefined"
    } else if f.intersects(TypeFlags::Null) {
        "null"
    } else if f.intersects(TypeFlags::Void) {
        "void"
    } else if f.intersects(TypeFlags::TypeParameter) {
        // polymorphic `this` (and constrained generics): classify through the constraint
        let is_this = t.as_type_parameter().is_this_type();
        let cons = c.get_constraint_of_type_parameter(t);
        match cons {
            Some(ct) if cx.depth_budget > 0 => {
                cx.depth_budget -= 1;
                let v = classify(cx, c, ct);
                cx.depth_budget += 1;
                if let Value::Object(vm) = v {
                    for (k, val) in vm {
                        m.insert(k, val);
                    }
                }
                m.insert(if is_this { "this".into() } else { "tp_constraint".into() }, json!(true));
                let cat = m.get("c").and_then(Value::as_str).unwrap_or("typeparam").to_string();
                m.insert("c".into(), json!(cat));
                return Value::Object(m);
            }
            _ => {}
        }
        "typeparam"
    } else if f.intersects(TypeFlags::Intersection) {
        "intersection"
    } else if f.intersects(TypeFlags::NonPrimitive) {
        "object"
    } else if f.intersects(TypeFlags::Object) {
        let of = t.object_flags();
        if c.is_array_type(t) {
            if cx.depth_budget > 0 {
                let args = c.get_type_arguments(t);
                if let Some(&e) = args.first() {
                    cx.depth_budget -= 1;
                    let inner = classify(cx, c, e);
                    cx.depth_budget += 1;
                    m.insert("elem".into(), inner);
                }
            }
            "array"
        } else if checker::is_tuple_type_exported(t) {
            "tuple"
        } else if is_class_instance(t) {
            type_decl(cx, t, &mut m);
            if of.intersects(ObjectFlags::Reference) && !of.intersects(ObjectFlags::Class) {
                m.insert("generic".into(), json!(true));
            }
            "instance"
        } else if of.intersects(ObjectFlags::Interface)
            || (of.intersects(ObjectFlags::Reference)
                && t.target().is_some_and(|x| x.object_flags().intersects(ObjectFlags::Interface)))
        {
            type_decl(cx, t, &mut m);
            "interface"
        } else if t.symbol().is_some_and(|s| s.flags().intersects(SymbolFlags::Class)) {
            type_decl(cx, t, &mut m);
            "class_ctor"
        } else if !c.get_call_signatures(t).is_empty() {
            let n = c.get_call_signatures(t).len();
            m.insert("sigs".into(), json!(n));
            "function"
        } else if of.intersects(ObjectFlags::ObjectLiteral) {
            "objlit"
        } else {
            if let Some(s) = t.symbol() {
                m.insert("name".into(), json!(s.name()));
            }
            "object"
        }
    } else {
        "generic"
    };
    m.insert("c".into(), json!(cat));
    Value::Object(m)
}

fn sym_info(cx: &Ctx, sym: P<Symbol>) -> Value {
    let f = sym.flags();
    let mut kinds = Vec::new();
    for (flag, name) in [
        (SymbolFlags::BlockScopedVariable, "let/const"),
        (SymbolFlags::FunctionScopedVariable, "var/param"),
        (SymbolFlags::Property, "prop"),
        (SymbolFlags::Method, "method"),
        (SymbolFlags::GetAccessor, "get"),
        (SymbolFlags::SetAccessor, "set"),
        (SymbolFlags::Function, "function"),
        (SymbolFlags::Class, "class"),
        (SymbolFlags::Interface, "interface"),
        (SymbolFlags::Enum, "enum"),
        (SymbolFlags::EnumMember, "enum_member"),
        (SymbolFlags::Alias, "alias"),
        (SymbolFlags::ValueModule, "module"),
        (SymbolFlags::Optional, "optional"),
    ] {
        if f.intersects(flag) {
            kinds.push(name);
        }
    }
    let decls = sym.declarations();
    let mut m = Map::new();
    m.insert("name".into(), json!(sym.name()));
    m.insert("f".into(), json!(kinds));
    m.insert("nd".into(), json!(decls.len()));
    if let Some(&d) = decls.first() {
        m.insert("decl".into(), decl_info(cx, d));
        if d.flags().intersects(NodeFlags::Const) || ast::is_var_const(d) {
            m.insert("const".into(), json!(true));
        }
    }
    if let Some(p) = sym.parent() {
        m.insert("owner".into(), json!(p.name()));
    }
    Value::Object(m)
}

/// Base-first list of instance *field* names of a class-like type (candidate slot order).
fn field_layout(c: &mut Checker, t: P<Type>, depth: u32) -> Vec<&'static str> {
    if depth > 16 {
        return Vec::new();
    }
    let target = if t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
        t
    } else if let Some(tt) = t.target() {
        tt
    } else {
        return Vec::new();
    };
    if !target.object_flags().intersects(ObjectFlags::ClassOrInterface) {
        return Vec::new();
    }
    let bases = c.get_base_types(target);
    let mut out = match bases.first() {
        Some(&b) => field_layout(c, b, depth + 1),
        None => Vec::new(),
    };
    let own_sym = target.symbol();
    for &p in c.get_properties_of_type(target) {
        if !p.flags().intersects(SymbolFlags::Property) {
            continue;
        }
        let own = match (p.parent(), own_sym) {
            (Some(pp), Some(os)) => pp == os,
            _ => true,
        };
        if own && !out.contains(&p.name()) {
            out.push(p.name());
        }
    }
    out
}

fn member_info(cx: &mut Ctx, c: &mut Checker, node: P<Node>) -> Value {
    let recv = node.expression().unwrap();
    let rt = c.get_type_at_location(recv);
    let nn = c.get_non_nullable_type(rt);
    let at = c.get_apparent_type(nn);
    let mut m = Map::new();
    m.insert("recv".into(), classify(cx, c, rt));
    let name_node = node.name().unwrap();
    let name = name_node.text();
    m.insert("name".into(), json!(name));
    if let Some(sym) = c.get_symbol_at_location_exported(name_node) {
        let si = sym_info(cx, sym);
        m.insert("sym".into(), si);
        let synthetic = sym.check_flags().intersects(ast::CheckFlags::SyntheticProperty);
        if synthetic {
            m.insert("poly".into(), json!(true));
        }
        if sym.flags().intersects(SymbolFlags::Property) && !synthetic && is_class_instance(at) {
            let layout = field_layout(c, at, 0);
            if let Some(i) = layout.iter().position(|n| *n == name) {
                m.insert("slot".into(), json!(i));
                m.insert("nslots".into(), json!(layout.len()));
            }
        }
    } else {
        m.insert("unresolved".into(), json!(true));
    }
    Value::Object(m)
}

fn call_info(cx: &mut Ctx, c: &mut Checker, node: P<Node>) -> Value {
    let mut m = Map::new();
    let callee = node.expression().unwrap();
    let ct = c.get_type_at_location(callee);
    let ncall = if node.kind() == Kind::NewExpression {
        c.get_construct_signatures(ct).len()
    } else {
        c.get_call_signatures(ct).len()
    };
    m.insert("ncand".into(), json!(ncall));
    let sig = c.get_resolved_signature_exported(node);
    match sig.declaration.get() {
        Some(d) => {
            m.insert("target".into(), decl_info(cx, d));
            if let Some(nm) = d.name() {
                if nm.kind() == Kind::Identifier || nm.kind() == Kind::PrivateIdentifier {
                    m.insert("fn".into(), json!(nm.text()));
                }
            }
        }
        None => {
            m.insert("target".into(), json!(null));
        }
    }
    let rt = c.get_return_type_of_signature(sig);
    m.insert("ret".into(), classify(cx, c, rt));
    // virtual-dispatch hint: callee is a method access on a class instance
    if ast::is_property_access_expression(callee) {
        let r = c.get_type_at_location(callee.expression().unwrap());
        let nn = c.get_non_nullable_type(r);
        let at = c.get_apparent_type(nn);
        if is_class_instance(at) {
            m.insert("method_on".into(), json!(at.symbol().map(|s| s.name())));
        }
    }
    Value::Object(m)
}

fn is_type_only_container(k: Kind) -> bool {
    (k >= Kind::FirstTypeNode && k <= Kind::LastTypeNode)
        || matches!(
            k,
            Kind::InterfaceDeclaration
                | Kind::TypeAliasDeclaration
                | Kind::JSTypeAliasDeclaration
                | Kind::TypeParameter
                | Kind::PropertySignature
                | Kind::MethodSignature
                | Kind::IndexSignature
                | Kind::CallSignature
                | Kind::ConstructSignature
        )
        || k == Kind::HeritageClause && false
}

fn collect(n: P<Node>, out: &mut Vec<P<Node>>) {
    let k = n.kind();
    if is_type_only_container(k) {
        return;
    }
    // `implements X` clauses are type-only
    if k == Kind::HeritageClause && n.as_heritage_clause().token == Kind::ImplementsKeyword {
        return;
    }
    let want = (ast::is_expression_node(n)
        || k == Kind::Identifier
        || k == Kind::PrivateIdentifier)
        && (!ast::is_part_of_type_node(n)
            // tsgo's isPartOfTypeNode says true for every `null` keyword; keep value-position ones
            || (k == Kind::NullKeyword && n.parent().is_some_and(|p| p.kind() != Kind::LiteralType)))
        && k != Kind::OmittedExpression;
    if want {
        out.push(n);
    }
    n.for_each_child(&mut |ch| {
        collect(ch, out);
        false
    });
}

#[derive(Default)]
struct Agg {
    nodes: usize,
    panics: usize,
    panic_examples: Vec<String>,
    cat_counts: std::collections::BTreeMap<String, usize>,
}

fn walk_file(program: &'static Program, c: &mut Checker, sf: P<SourceFile>, no_typestr: bool, agg: &mut Agg) -> String {
    let mut cx = Ctx { program, depth_budget: 2 };
        let mut nodes = Vec::new();
    collect(sf.as_node(), &mut nodes);
    let mut recs: Vec<String> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let Some((s, e)) = node_span(sf, n) else { continue };
        agg.nodes += 1;
        let k = n.kind();
        let mut rec = Map::new();
        rec.insert("s".into(), json!(s));
        rec.insert("e".into(), json!(e));
        rec.insert("k".into(), json!(format!("{k:?}")));
        if n.flags().intersects(NodeFlags::Reparsed) {
            rec.insert("reparsed".into(), json!(true));
        }
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut extra = Map::new();
            let t = c.get_type_at_location(n);
            if !no_typestr {
                let mut ts = c.type_to_string_exported(t);
                if ts.len() > 100 {
                    let mut cut = 97;
                    while !ts.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    ts.truncate(cut);
                    ts.push_str("...");
                }
                extra.insert("t".into(), json!(ts));
            }
            extra.insert("h".into(), classify(&mut cx, c, t));
            if k == Kind::Identifier || k == Kind::PrivateIdentifier {
                let parent = n.parent();
                let is_member_name = parent.is_some_and(|p| {
                    ast::is_property_access_expression(p) && p.name() == Some(n)
                });
                if !is_member_name {
                    if let Some(sym) = c.get_symbol_at_location_exported(n) {
                        extra.insert("sym".into(), sym_info(&cx, sym));
                    }
                }
            }
            if k == Kind::PropertyAccessExpression {
                extra.insert("member".into(), member_info(&mut cx, c, n));
            }
            if k == Kind::ElementAccessExpression {
                let recv = n.expression().unwrap();
                let rt = c.get_type_at_location(recv);
                extra.insert("recv".into(), classify(&mut cx, c, rt));
                if let Some(arg) = n.as_element_access_expression().argument_expression.into() {
                    let at: P<Type> = c.get_type_at_location(arg);
                    extra.insert("index".into(), classify(&mut cx, c, at));
                }
            }
            if k == Kind::CallExpression || k == Kind::NewExpression {
                extra.insert("call".into(), call_info(&mut cx, c, n));
            }
            extra
        }));
        match r {
            Ok(extra) => {
                if let Some(c) = extra.get("h").and_then(|h| h.get("c")).and_then(Value::as_str)
                {
                    *agg.cat_counts.entry(c.to_string()).or_default() += 1;
                }
                rec.extend(extra);
            }
            Err(e) => {
                agg.panics += 1;
                if agg.panic_examples.len() < 10 {
                    agg.panic_examples.push(format!(
                        "{}:{s} {k:?}: {}",
                        tspath::get_base_file_name(sf.file_name()),
                        panic_msg(&e)
                    ));
                }
                rec.insert("panic".into(), json!(true));
            }
        }
        recs.push(serde_json::to_string(&Value::Object(rec)).unwrap());
    }
    format!(
        "{{\"file\":{},\"js\":{},\"nodes\":[{}]}}\n",
        serde_json::to_string(sf.file_name()).unwrap(),
        ast::is_in_js_file(Some(sf.as_node())),
        recs.join(",")
    )
}

fn panic_msg(e: &Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "<panic>".into())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: ep9_typecheck <tsconfig|dir> <out.jsonl> [--no-check]");
        std::process::exit(2);
    }
    let no_check = args.iter().any(|a| a == "--no-check");
    let no_typestr = args.iter().any(|a| a == "--no-typestr");
    let parallel = args.iter().any(|a| a == "--parallel");
    let t_start = Instant::now();
    let fs: Arc<dyn FS> = Arc::new(bundled::embed::wrap_fs(osvfs::fs()));
    let cwd = tspath::normalize_path(&std::env::current_dir().unwrap().to_string_lossy());
    let host: &'static OsParseConfigHost =
        Box::leak(Box::new(OsParseConfigHost { fs: Arc::clone(&fs), cwd: cwd.clone() }));
    let mut config_path = tspath::get_normalized_absolute_path(&args[1], &cwd);
    if fs.directory_exists(&config_path) {
        config_path = tspath::combine_paths(&config_path, &["tsconfig.json"]);
    }
    let (config, errs) = tsoptions::get_parsed_command_line_of_config_file(
        &config_path,
        Some(&tsrs_core::CompilerOptions::default()),
        None,
        host,
        None,
    );
    if !errs.is_empty() || config.is_none() {
        eprintln!("config errors: {}", errs.len());
        std::process::exit(1);
    }
    let compiler_host =
        new_cached_fs_compiler_host(&cwd, fs, &bundled::embed::lib_path(), None, None, None);
    let mut opts = ProgramOptions::new(P::new(config.unwrap()), compiler_host);
    opts.single_threaded = if parallel { Tristate::False } else { Tristate::True };
    let t_prog0 = Instant::now();
    let program = new_program(opts);
    let t_prog = t_prog0.elapsed();
    let rss_prog = max_rss_mib();

    let t_check0 = Instant::now();
    let mut ndiag = 0usize;
    let mut diag_codes: std::collections::BTreeMap<i32, usize> = Default::default();
    if !no_check {
        let diags = get_diagnostics_of_any_program(
            &Context::default(),
            program,
            None,
            false,
            &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
            &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
        );
        ndiag = diags.len();
        for d in &diags {
            *diag_codes.entry(d.code()).or_default() += 1;
        }
        for d in diags.iter().take(8) {
            let f = d.file().map(|f| tspath::get_base_file_name(f.file_name())).unwrap_or_default();
            eprintln!("diag TS{} {}:{:?} {}", d.code(), f, d.pos(), tsrs_diagnostics::localize(d.message(), d.message_key(), d.message_args()));
        }
    }
    let t_check = t_check0.elapsed();
    let rss_check = max_rss_mib();
    let types_after_check = program.get_type_checker(&Context::default()).type_count;

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    let t_walk0 = Instant::now();
    let mut n_files = 0usize;
    let mut src_bytes = 0usize;
    let mut files: Vec<P<SourceFile>> = Vec::new();
    let all_files = program.source_files();
    let mut lib_files = 0usize;
    let mut dts_files = 0usize;
    for &sf in all_files {
        if program.is_source_file_default_library(sf.path()) {
            lib_files += 1;
            continue;
        }
        if sf.is_declaration_file() {
            dts_files += 1;
            continue;
        }
        if sf.file_name().contains("/node_modules/") {
            continue;
        }
        n_files += 1;
        src_bytes += sf.text().len();
        files.push(sf);
    }
    let results: std::sync::Mutex<Vec<Option<String>>> = std::sync::Mutex::new(vec![None; files.len()]);
    let aggm: std::sync::Mutex<Agg> = std::sync::Mutex::new(Agg::default());
    let ran_grouped = parallel
        && program.for_each_checker_group(&files, |c, i, sf| {
            let mut a = Agg::default();
            let line = walk_file(program, c, sf, no_typestr, &mut a);
            results.lock().unwrap()[i] = Some(line);
            let mut g = aggm.lock().unwrap();
            g.nodes += a.nodes;
            g.panics += a.panics;
            g.panic_examples.extend(a.panic_examples);
            for (k, v) in a.cat_counts {
                *g.cat_counts.entry(k).or_default() += v;
            }
        });
    if !ran_grouped {
        for (i, &sf) in files.iter().enumerate() {
            let mut handle = program.get_type_checker_for_file(&Context::default(), sf);
            let mut a = aggm.lock().unwrap();
            let line = walk_file(program, &mut handle, sf, no_typestr, &mut a);
            results.lock().unwrap()[i] = Some(line);
        }
    }
    for line in results.into_inner().unwrap().into_iter().flatten() {
        out.write_all(line.as_bytes()).unwrap();
    }
    let agg = aggm.into_inner().unwrap();
    let (total_nodes, panics, panic_examples, cat_counts) = (agg.nodes, agg.panics, agg.panic_examples, agg.cat_counts);
    let t_walk = t_walk0.elapsed();
    let (type_count, symbol_count, inst_count) = {
        let c = program.get_type_checker(&Context::default());
        (c.type_count, c.symbol_count, c.total_instantiation_count)
    };
    let stats = json!({"stats": {
        "config": config_path,
        "files_total": all_files.len(),
        "lib_files": lib_files,
        "dts_files": dts_files,
        "src_files": n_files,
        "src_bytes": src_bytes,
        "nodes": total_nodes,
        "panics": panics,
        "panic_examples": panic_examples,
        "diagnostics": ndiag,
        "diag_codes": diag_codes.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_,_>>(),
        "cat_counts": cat_counts,
        "ms_program": t_prog.as_secs_f64() * 1e3,
        "ms_check": t_check.as_secs_f64() * 1e3,
        "ms_walk": t_walk.as_secs_f64() * 1e3,
        "ms_total": t_start.elapsed().as_secs_f64() * 1e3,
        "rss_mib_after_program": rss_prog,
        "rss_mib_after_check": rss_check,
        "rss_mib_end": max_rss_mib(),
        "type_count": type_count,
        "type_count_after_check": types_after_check,
        "symbol_count": symbol_count,
        "instantiation_count": inst_count,
        "checker_count": program.checker_count(),
    }});
    serde_json::to_writer(&mut out, &stats).unwrap();
    out.write_all(b"\n").unwrap();
    out.flush().unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&stats).unwrap());
}
