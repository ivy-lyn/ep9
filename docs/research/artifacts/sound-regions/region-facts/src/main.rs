//! region-facts: type-check a tsconfig project with tsrs (one checker) and export the raw facts that the
//! "sound region" finder (analyze.py) needs: classes (fields, members, ctor leak scan, heritage), functions
//! (param/local types, casts, any-flows, r2 sites), value flows (source type -> contextual target type),
//! casts, member accesses, r3 hazards, r7 hazards, resolved call targets and function value references.
//!
//! Setup code (program, checker, file filter) is copied from the R13 oracle (tsrs-type-oracle/oracle).
//! usage: region_facts <tsconfig.json> <out.jsonl>
//! Output: line 1 {"files":[...]} (project files, index = id prefix); then one line per file; last line {"stats"}.
//! Ids: "<file index>:<utf8 offset of first token>" for project declarations (classes, interfaces, functions).

use std::collections::HashMap;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value, json};
use tsrs_ast::{self as ast, Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{self as checker, Checker, ContextFlags, ObjectFlags, Type, TypeFlags};
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

fn start_of(n: P<Node>) -> u32 {
    let Some(sf) = ast::get_source_file_of_node(n) else { return 0 };
    let Some(p) = n.pos().source_offset() else { return 0 };
    tsrs_scanner::skip_trivia(sf.text(), p)
}

fn end_of(n: P<Node>) -> u32 {
    n.end().source_offset().unwrap_or(0)
}

fn skip_parens(mut n: P<Node>) -> P<Node> {
    while n.kind() == Kind::ParenthesizedExpression {
        match n.expression() {
            Some(e) => n = e,
            None => break,
        }
    }
    n
}

fn name_text(nm: P<Node>) -> String {
    match nm.kind() {
        Kind::Identifier | Kind::PrivateIdentifier | Kind::StringLiteral | Kind::NumericLiteral | Kind::NoSubstitutionTemplateLiteral => {
            nm.text().to_string()
        }
        _ => "[computed]".to_string(),
    }
}

fn skip_outer(mut n: P<Node>) -> P<Node> {
    loop {
        match n.kind() {
            Kind::ParenthesizedExpression | Kind::AsExpression | Kind::TypeAssertionExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => {
                match n.expression() {
                    Some(e) => n = e,
                    None => return n,
                }
            }
            _ => return n,
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

fn is_fn_like_with_body(n: P<Node>) -> bool {
    matches!(
        n.kind(),
        Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
    ) && n.body().is_some()
}

fn is_this_boundary(k: Kind) -> bool {
    // nodes that bind their own `this`
    matches!(
        k,
        Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::ClassDeclaration
            | Kind::ClassExpression
            | Kind::ClassStaticBlockDeclaration
    )
}

/// Does the subtree mention `this` (looking through arrow functions, not through `this`-binding nodes)?
fn captures_this(n: P<Node>) -> bool {
    if n.kind() == Kind::ThisKeyword {
        return true;
    }
    let mut found = false;
    n.for_each_child(&mut |ch: P<Node>| {
        if found || is_this_boundary(ch.kind()) {
            return found;
        }
        if captures_this(ch) {
            found = true;
        }
        found
    });
    found
}

fn first_this(n: P<Node>) -> Option<P<Node>> {
    if n.kind() == Kind::ThisKeyword {
        return Some(n);
    }
    let mut found = None;
    n.for_each_child(&mut |ch: P<Node>| {
        if found.is_some() {
            return true;
        }
        if is_this_boundary(ch.kind()) {
            return false;
        }
        found = first_this(ch);
        found.is_some()
    });
    found
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
                | Kind::ImportDeclaration
                | Kind::ImportEqualsDeclaration
                | Kind::ExportDeclaration
        )
}

fn is_assign_op(k: Kind) -> bool {
    matches!(
        k,
        Kind::EqualsToken
            | Kind::BarBarEqualsToken
            | Kind::AmpersandAmpersandEqualsToken
            | Kind::QuestionQuestionEqualsToken
    )
}

fn this_prop_name(n: P<Node>) -> Option<&'static str> {
    let n = skip_parens(n);
    if n.kind() != Kind::PropertyAccessExpression {
        return None;
    }
    let e = skip_parens(n.expression()?);
    if e.kind() != Kind::ThisKeyword {
        return None;
    }
    let nm = n.name()?;
    if !matches!(nm.kind(), Kind::Identifier | Kind::PrivateIdentifier) {
        return None;
    }
    Some(nm.text())
}

#[derive(Default)]
struct Scan {
    assigned: Vec<String>,
    before_leak: Option<Vec<String>>,
    leak: Option<(String, u32)>,
    events: Vec<(String, u32, usize)>,
}

#[derive(Default)]
struct Func {
    id: String,
    kind: &'static str,
    name: String,
    s: u32,
    e: u32,
    bs: u32,
    be: u32,
    cls: Option<String>,
    st: bool,
    exp: bool,
    params: Vec<Value>,
    this_d: Option<Value>,
    locals: Vec<Value>,
    nested: usize,
    asrt: usize,
    upcast: usize,
    nonnull: usize,
    anyflow: usize,
    r2: usize,
    parent: Option<usize>,
    ctx: &'static str,
}

struct G {
    program: &'static Program,
    proj: HashMap<String, usize>,
}

struct Fw<'a> {
    g: &'a G,
    c: &'a mut Checker,
    fi: usize,
    funcs: Vec<Func>,
    classes: Vec<Value>,
    flows: Vec<Value>,
    ifl: Vec<Value>,
    casts: Vec<Value>,
    members: Vec<Value>,
    r3: Vec<Value>,
    r7: Vec<Value>,
    calls: Vec<Value>,
    vrefs: Vec<Value>,
    panics: usize,
    panic_ex: Vec<String>,
}

impl<'a> Fw<'a> {
    fn node_id(&self, n: P<Node>) -> Option<String> {
        let sf = ast::get_source_file_of_node(n)?;
        let idx = self.g.proj.get(sf.file_name())?;
        let off = tsrs_scanner::skip_trivia(sf.text(), n.pos().source_offset()?);
        Some(format!("{idx}:{off}"))
    }

    fn decl_is_foreign(&self, n: P<Node>) -> bool {
        match ast::get_source_file_of_node(n) {
            Some(sf) => !self.g.proj.contains_key(sf.file_name()),
            None => true,
        }
    }

    fn class_decl_of(sym: P<Symbol>) -> Option<P<Node>> {
        sym.declarations()
            .iter()
            .copied()
            .find(|d| matches!(d.kind(), Kind::ClassDeclaration | Kind::ClassExpression))
    }

    /// project class id of an instance type, or Err(name) for a foreign class
    fn class_id_of_type(&self, t: P<Type>) -> Result<String, String> {
        let Some(sym) = t.symbol() else { return Err("?".into()) };
        match Self::class_decl_of(sym) {
            Some(d) => self.node_id(d).ok_or_else(|| sym.name().to_string()),
            None => Err(sym.name().to_string()),
        }
    }

    fn type_decl_id(&self, t: P<Type>) -> Result<String, String> {
        let Some(sym) = t.symbol() else { return Err("?".into()) };
        match sym.declarations().first() {
            Some(&d) => self.node_id(d).ok_or_else(|| sym.name().to_string()),
            None => Err(sym.name().to_string()),
        }
    }

    /// Compact type descriptor.
    fn desc(&mut self, t: P<Type>, depth: u32) -> Value {
        let f = t.flags();
        let mut m = Map::new();
        let cat: &str = if f.intersects(TypeFlags::Any) {
            if t == self.c.get_error_type() { "error" } else { "any" }
        } else if f.intersects(TypeFlags::Unknown) {
            "unknown"
        } else if f.intersects(TypeFlags::Never) {
            "never"
        } else if f.intersects(TypeFlags::Union) {
            let members = t.types();
            let mut non_null = Vec::new();
            let mut nullish = 0;
            for &mt in members {
                if mt.flags().intersects(TypeFlags::Nullable | TypeFlags::Void) {
                    nullish += 1;
                } else {
                    non_null.push(mt);
                }
            }
            let prim = |p: TypeFlags| !non_null.is_empty() && non_null.iter().all(|m| m.flags().intersects(p));
            let pc = if prim(TypeFlags::NumberLike) {
                Some("number")
            } else if prim(TypeFlags::StringLike) {
                Some("string")
            } else if prim(TypeFlags::BooleanLike) {
                Some("boolean")
            } else if prim(TypeFlags::BigIntLike) {
                Some("bigint")
            } else {
                None
            };
            if let Some(pc) = pc {
                if nullish > 0 {
                    m.insert("i".into(), json!({"c": pc}));
                    "nullable"
                } else {
                    pc
                }
            } else if nullish > 0 && non_null.len() == 1 {
                let inner = if depth > 0 { self.desc(non_null[0], depth - 1) } else { json!({"c": "?"}) };
                m.insert("i".into(), inner);
                "nullable"
            } else {
                if nullish > 0 {
                    m.insert("nul".into(), json!(1));
                }
                if depth > 0 {
                    let ms: Vec<Value> = non_null.iter().take(16).map(|&x| self.desc(x, depth - 1)).collect();
                    m.insert("m".into(), json!(ms));
                }
                m.insert("n".into(), json!(non_null.len()));
                "union"
            }
        } else if f.intersects(TypeFlags::NumberLike) {
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
            let is_this = t.as_type_parameter().is_this_type();
            let cons = self.c.get_constraint_of_type_parameter(t);
            if is_this {
                if let Some(ct) = cons {
                    if depth > 0 {
                        let mut v = self.desc(ct, depth - 1);
                        if let Value::Object(vm) = &mut v {
                            vm.insert("this".into(), json!(1));
                        }
                        return v;
                    }
                }
            }
            if let Some(ct) = cons {
                if depth > 0 {
                    let v = self.desc(ct, depth - 1);
                    m.insert("i".into(), v);
                }
            }
            "tp"
        } else if f.intersects(TypeFlags::Intersection) {
            if depth > 0 {
                let ms: Vec<Value> = t.types().iter().take(4).map(|&x| self.desc(x, depth - 1)).collect();
                m.insert("m".into(), json!(ms));
            }
            "inter"
        } else if f.intersects(TypeFlags::NonPrimitive) {
            "object"
        } else if f.intersects(TypeFlags::Object) {
            let of = t.object_flags();
            let ta = |s: &mut Self, m: &mut Map<String, Value>| {
                if depth > 0 && of.intersects(ObjectFlags::Reference) {
                    let args = s.c.get_type_arguments(t);
                    if !args.is_empty() {
                        let v: Vec<Value> = args.iter().take(4).map(|&a| s.desc(a, depth - 1)).collect();
                        m.insert("ta".into(), json!(v));
                    }
                }
            };
            if self.c.is_array_type(t) {
                if depth > 0 {
                    let args = self.c.get_type_arguments(t);
                    if let Some(&e) = args.first() {
                        let v = self.desc(e, depth - 1);
                        m.insert("el".into(), v);
                    }
                }
                "array"
            } else if checker::is_tuple_type_exported(t) {
                ta(self, &mut m);
                "tuple"
            } else if is_class_instance(t) {
                match self.class_id_of_type(t) {
                    Ok(id) => {
                        m.insert("id".into(), json!(id));
                    }
                    Err(n) => {
                        m.insert("fn".into(), json!(n));
                    }
                }
                ta(self, &mut m);
                "inst"
            } else if of.intersects(ObjectFlags::Interface)
                || (of.intersects(ObjectFlags::Reference)
                    && t.target().is_some_and(|x| x.object_flags().intersects(ObjectFlags::Interface)))
            {
                match self.type_decl_id(t) {
                    Ok(id) => {
                        m.insert("id".into(), json!(id));
                    }
                    Err(n) => {
                        m.insert("fn".into(), json!(n));
                    }
                }
                ta(self, &mut m);
                "iface"
            } else if t.symbol().is_some_and(|s| s.flags().intersects(SymbolFlags::Class)) {
                if let Some(s) = t.symbol() {
                    if let Some(d) = Self::class_decl_of(s) {
                        if let Some(id) = self.node_id(d) {
                            m.insert("id".into(), json!(id));
                        }
                    }
                }
                "ctor"
            } else if t.symbol().is_some_and(|s| {
                s.flags().intersects(SymbolFlags::ValueModule | SymbolFlags::NamespaceModule | SymbolFlags::Enum)
            }) {
                "ns"
            } else if !self.c.get_call_signatures(t).is_empty() || !self.c.get_construct_signatures(t).is_empty() {
                "fn"
            } else {
                let nprops = self.c.get_properties_of_type(t).len();
                if nprops == 0 && !of.intersects(ObjectFlags::Mapped) {
                    "emptyobj"
                } else {
                    if let Ok(id) = self.type_decl_id(t) {
                        m.insert("id".into(), json!(id));
                    } else if let Some(s) = t.symbol() {
                        m.insert("fn".into(), json!(s.name()));
                    }
                    if of.intersects(ObjectFlags::ObjectLiteral) { "objlit" } else { "tlit" }
                }
            }
        } else {
            "other"
        };
        m.insert("c".into(), json!(cat));
        Value::Object(m)
    }

    /// project class ids referenced by a type: (direct value is a C, C nested in containers/type args)
    fn refs(&mut self, t: P<Type>, depth: u32, direct: bool, out_d: &mut Vec<String>, out_n: &mut Vec<String>) {
        self.refs_x(t, depth, direct, false, out_d, out_n);
    }

    /// deep = also follow property types of interfaces (used for targets: where can a nested C legally go)
    fn refs_x(&mut self, t: P<Type>, depth: u32, direct: bool, deep: bool, out_d: &mut Vec<String>, out_n: &mut Vec<String>) {
        if depth == 0 {
            return;
        }
        let f = t.flags();
        if f.intersects(TypeFlags::Union | TypeFlags::Intersection) {
            for &m in t.types().iter().take(16) {
                self.refs_x(m, depth - 1, direct, deep, out_d, out_n);
            }
            return;
        }
        if f.intersects(TypeFlags::TypeParameter) {
            if let Some(ct) = self.c.get_constraint_of_type_parameter(t) {
                self.refs_x(ct, depth - 1, direct, deep, out_d, out_n);
            }
            return;
        }
        if !f.intersects(TypeFlags::Object) {
            return;
        }
        let of = t.object_flags();
        if is_class_instance(t) {
            if let Ok(id) = self.class_id_of_type(t) {
                let o = if direct { &mut *out_d } else { &mut *out_n };
                if !o.contains(&id) {
                    o.push(id);
                }
            }
        }
        if of.intersects(ObjectFlags::Reference) {
            let args = self.c.get_type_arguments(t);
            for &a in args.iter().take(6) {
                self.refs_x(a, depth - 1, false, deep, out_d, out_n);
            }
        }
        let iface = of.intersects(ObjectFlags::Interface)
            || (of.intersects(ObjectFlags::Reference) && t.target().is_some_and(|x| x.object_flags().intersects(ObjectFlags::Interface)));
        if deep && iface && !self.c.is_array_type(t) {
            let props = self.c.get_properties_of_type(t);
            for &p in props.iter().take(32) {
                if p.flags().intersects(SymbolFlags::Property) {
                    let pt = self.c.get_type_of_symbol(p);
                    self.refs_x(pt, depth - 1, false, deep, out_d, out_n);
                }
            }
        }
        if of.intersects(ObjectFlags::Anonymous | ObjectFlags::ObjectLiteral)
            && !t.symbol().is_some_and(|s| s.flags().intersects(SymbolFlags::Class | SymbolFlags::Function | SymbolFlags::Method | SymbolFlags::ValueModule | SymbolFlags::Enum))
        {
            let props = self.c.get_properties_of_type(t);
            for &p in props.iter().take(32) {
                if p.flags().intersects(SymbolFlags::Property) {
                    let pt = self.c.get_type_of_symbol(p);
                    self.refs_x(pt, depth - 1, false, deep, out_d, out_n);
                }
            }
        }
        // index signature value types (Record<string, C>, { [k: string]: C })
        if of.intersects(ObjectFlags::Anonymous | ObjectFlags::Mapped | ObjectFlags::ObjectLiteral | ObjectFlags::Interface) {
            let infos = self.c.get_index_infos_of_type_exported(t);
            for info in infos.iter().take(2) {
                if let Some(vt) = info.value_type.get() {
                    self.refs_x(vt, depth - 1, false, deep, out_d, out_n);
                }
            }
        }
    }

    fn refs_all(&mut self, t: P<Type>) -> (Vec<String>, Vec<String>) {
        let mut d = Vec::new();
        let mut n = Vec::new();
        self.refs(t, 4, true, &mut d, &mut n);
        (d, n)
    }

    fn tdesc_at(&mut self, n: P<Node>) -> Value {
        let t = self.c.get_type_at_location(n);
        self.desc(t, 2)
    }

    fn resolve_sym(&mut self, s: P<Symbol>) -> P<Symbol> {
        if s.flags().intersects(SymbolFlags::Alias) { self.c.get_aliased_symbol(s) } else { s }
    }

    /// declaration with a body for a function/method symbol (implementation of overloads)
    fn body_decl(sym: P<Symbol>) -> Option<P<Node>> {
        sym.declarations().iter().copied().find(|d| is_fn_like_with_body(*d))
    }

    fn fn_id_of_decl(&self, d: P<Node>) -> Option<String> {
        let d = if is_fn_like_with_body(d) {
            d
        } else {
            match d.symbol().and_then(Self::body_decl) {
                Some(b) => b,
                None => {
                    // property/variable initialized with a function expression
                    if !matches!(d.kind(), Kind::VariableDeclaration | Kind::PropertyDeclaration | Kind::PropertyAssignment) {
                        return None;
                    }
                    let init = d.initializer().map(skip_parens);
                    match init {
                        Some(i) if is_fn_like_with_body(i) => i,
                        _ => return None,
                    }
                }
            }
        };
        self.node_id(d)
    }

    // ---------------------------------------------------------------- flows
    fn flow(&mut self, e: P<Node>, cur: Option<usize>, force_any: bool) {
        self.flow_g(e, cur, force_any, None);
    }

    fn has_tp(v: &Value) -> bool {
        if v.get("c").and_then(Value::as_str) == Some("tp") {
            return true;
        }
        for k in ["i", "el"] {
            if v.get(k).is_some_and(Self::has_tp) {
                return true;
            }
        }
        for k in ["m", "ta"] {
            if let Some(a) = v.get(k).and_then(Value::as_array) {
                if a.iter().any(Self::has_tp) {
                    return true;
                }
            }
        }
        false
    }

    fn flow_g(&mut self, e: P<Node>, cur: Option<usize>, force_any: bool, gt: Option<Value>) {
        self.flow_c(e, e, cur, force_any, gt);
    }

    /// value of `e` flows to the contextual type of `ctx` (ctx != e only for spread arguments)
    fn flow_c(&mut self, e: P<Node>, ctx: P<Node>, cur: Option<usize>, force_any: bool, gt: Option<Value>) {
        let st = self.c.get_type_at_location(e);
        let sf = st.flags();
        let s_any = if sf.intersects(TypeFlags::Any) {
            if st == self.c.get_error_type() { Some("error") } else { Some("any") }
        } else if sf.intersects(TypeFlags::Unknown) {
            Some("unknown")
        } else {
            None
        };
        let tt: Option<P<Type>> = if force_any { None } else { self.c.get_contextual_type(ctx, ContextFlags::None) };
        let (rs_d, rs_n) = self.refs_all(st);
        let target_desc = |s: &mut Self| -> Value {
            if force_any {
                json!({"c": "any", "throw": 1})
            } else {
                match tt {
                    Some(t) => s.desc(t, 3),
                    None => json!({"c": "none"}),
                }
            }
        };
        if let Some(sa) = s_any {
            if let Some(t) = tt {
                let tf = t.flags();
                if !tf.intersects(TypeFlags::Any | TypeFlags::Unknown) {
                    let (td, tn) = self.refs_all(t);
                    let tdsc = self.desc(t, 1);
                    self.flows.push(json!({"k": "any", "sa": sa, "f": cur, "td": td, "tn": tn, "t": tdsc, "s": start_of(e)}));
                    if let Some(i) = cur {
                        self.funcs[i].anyflow += 1;
                    }
                }
            }
            return;
        }
        if !rs_d.is_empty() || !rs_n.is_empty() {
            let (td, tn) = match tt {
                Some(t) => {
                    let mut d = Vec::new();
                    let mut n = Vec::new();
                    self.refs_x(t, 4, true, true, &mut d, &mut n);
                    (d, n)
                }
                None => (Vec::new(), Vec::new()),
            };
            let tdsc = target_desc(self);
            let sdsc = self.desc(st, 2);
            let lit = matches!(skip_parens(e).kind(), Kind::ObjectLiteralExpression | Kind::ArrayLiteralExpression);
            let mut fr = json!({"k": "cls", "f": cur, "sd": rs_d, "sn": rs_n, "td": td, "tn": tn, "t": tdsc, "sdesc": sdsc, "lit": lit, "s": start_of(e)});
            if let Some(g) = gt {
                fr.as_object_mut().unwrap().insert("gt".into(), g);
            }
            self.flows.push(fr);
            return;
        }
        // non-instance object values flowing into a class-typed target (structural fake instance)
        if let Some(t) = tt {
            let (td, _) = self.refs_all(t);
            if !td.is_empty() {
                let sdsc = self.desc(st, 1);
                let sc = sdsc.get("c").and_then(Value::as_str).unwrap_or("");
                if matches!(sc, "objlit" | "tlit" | "iface" | "inter" | "union" | "object" | "emptyobj" | "tp" | "fn" | "other") {
                    self.flows.push(json!({"k": "fake", "f": cur, "td": td, "sdesc": sdsc, "s": start_of(e)}));
                }
            }
        }
        // flows of non-class values into project interfaces / structural types (for interface sharing)
        if let Some(t) = tt {
            let tdsc = self.desc(t, 2);
            if Self::has_project_struct(&tdsc) {
                let sdsc = self.desc(st, 2);
                self.ifl.push(json!({"t": tdsc, "sdesc": sdsc, "s": start_of(e)}));
            }
        }
    }

    fn has_project_struct(v: &Value) -> bool {
        let c = v.get("c").and_then(Value::as_str).unwrap_or("");
        match c {
            "iface" | "tlit" | "objlit" => v.get("id").is_some(),
            "nullable" => v.get("i").is_some_and(Self::has_project_struct),
            "array" => v.get("el").is_some_and(Self::has_project_struct),
            "union" | "inter" => v.get("m").and_then(Value::as_array).is_some_and(|a| a.iter().any(Self::has_project_struct)),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- classes
    fn is_accessor_access(&mut self, pa: P<Node>) -> bool {
        let Some(nm) = pa.name() else { return false };
        match self.c.get_symbol_at_location_exported(nm) {
            Some(s) => s.flags().intersects(SymbolFlags::GetAccessor | SymbolFlags::SetAccessor),
            None => false,
        }
    }

    fn leak(st: &mut Scan, kind: &str, n: P<Node>) {
        if st.events.len() < 64 {
            st.events.push((kind.to_string(), start_of(n), st.assigned.len()));
        }
        if st.leak.is_none() {
            st.leak = Some((kind.to_string(), start_of(n)));
            st.before_leak = Some(st.assigned.clone());
        }
    }

    fn scan(&mut self, n: P<Node>, st: &mut Scan) {
        let k = n.kind();
        if (k >= Kind::FirstTypeNode && k <= Kind::LastTypeNode) || is_this_boundary(k) {
            return;
        }
        match k {
            Kind::ArrowFunction => {
                if captures_this(n) {
                    Self::leak(st, "closure", n);
                }
                return;
            }
            Kind::ThisKeyword => {
                Self::leak(st, "this-value", n);
                return;
            }
            Kind::BinaryExpression => {
                let be = n.as_binary_expression();
                let op = be.operator_token.kind();
                if is_assign_op(op) {
                    if let Some(name) = this_prop_name(be.left()) {
                        let r = be.right();
                        let rk = skip_parens(r).kind();
                        if !(rk == Kind::ArrowFunction || rk == Kind::FunctionExpression) {
                            self.scan(r, st);
                        }
                        if self.is_accessor_access(skip_parens(be.left())) {
                            Self::leak(st, "accessor", n);
                        }
                        if op == Kind::EqualsToken && !st.assigned.iter().any(|x| x == name) {
                            st.assigned.push(name.to_string());
                        }
                        return;
                    }
                }
            }
            Kind::CallExpression | Kind::NewExpression => {
                if let Some(callee) = n.expression() {
                    let cs = skip_parens(callee);
                    if cs.kind() == Kind::SuperKeyword {
                        for &a in n.arguments() {
                            self.scan(a, st);
                        }
                        return;
                    }
                    if matches!(cs.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                        if let Some(obj) = cs.expression() {
                            let ok = skip_parens(obj).kind();
                            if ok == Kind::ThisKeyword || ok == Kind::SuperKeyword {
                                if cs.kind() == Kind::ElementAccessExpression {
                                    self.scan(cs.as_element_access_expression().argument_expression, st);
                                }
                                for &a in n.arguments() {
                                    self.scan(a, st);
                                }
                                Self::leak(st, "method-call", n);
                                return;
                            }
                        }
                    }
                }
            }
            Kind::PropertyAccessExpression => {
                if let Some(e) = n.expression() {
                    let ek = skip_parens(e).kind();
                    if ek == Kind::ThisKeyword || ek == Kind::SuperKeyword {
                        if self.is_accessor_access(n) {
                            Self::leak(st, "accessor", n);
                        }
                        return;
                    }
                }
            }
            Kind::ElementAccessExpression => {
                if let Some(e) = n.expression() {
                    if skip_parens(e).kind() == Kind::ThisKeyword {
                        self.scan(n.as_element_access_expression().argument_expression, st);
                        return;
                    }
                }
            }
            _ => {}
        }
        for ch in n.iter_children() {
            self.scan(ch, st);
        }
    }

    fn class_rec(&mut self, n: P<Node>) -> Value {
        let id = self.node_id(n).unwrap_or_default();
        let name = n
            .name()
            .map(name_text)
            .or_else(|| n.parent().filter(|p| p.kind() == Kind::VariableDeclaration).and_then(|p| p.name()).map(name_text))
            .unwrap_or_else(|| "<anon>".into());
        let mf = ast::get_combined_modifier_flags(n);
        let mut rec = Map::new();
        rec.insert("id".into(), json!(id));
        rec.insert("name".into(), json!(name));
        rec.insert("s".into(), json!(start_of(n)));
        rec.insert("e".into(), json!(end_of(n)));
        rec.insert("expr".into(), json!(n.kind() == Kind::ClassExpression));
        rec.insert("exp".into(), json!(mf.intersects(ModifierFlags::Export)));
        rec.insert("abs".into(), json!(mf.intersects(ModifierFlags::Abstract)));
        rec.insert("dec".into(), json!(n.decorators().len()));
        let mut mdec = 0usize;
        let mut pdec = 0usize;
        let mut index = 0usize;
        let mut members = Vec::new();
        let mut ctor: Option<P<Node>> = None;
        for &m in n.members() {
            mdec += m.decorators().len();
            let st = ast::is_static(m);
            let mname = m.name().map(name_text).unwrap_or_default();
            match m.kind() {
                Kind::Constructor => {
                    if m.body().is_some() {
                        ctor = Some(m);
                    }
                    for &p in m.parameters() {
                        pdec += p.decorators().len();
                    }
                }
                Kind::MethodDeclaration => {
                    for &p in m.parameters() {
                        pdec += p.decorators().len();
                    }
                    members.push(json!({"n": mname, "st": st, "k": "m"}));
                }
                Kind::GetAccessor => members.push(json!({"n": mname, "st": st, "k": "get"})),
                Kind::SetAccessor => members.push(json!({"n": mname, "st": st, "k": "set"})),
                Kind::PropertyDeclaration => {
                    let acc = ast::has_accessor_modifier(m);
                    members.push(json!({"n": mname, "st": st, "k": if acc { "acc" } else { "f" }}));
                }
                Kind::IndexSignature => index += 1,
                _ => {}
            }
        }
        rec.insert("mdec".into(), json!(mdec));
        rec.insert("pdec".into(), json!(pdec));
        rec.insert("index".into(), json!(index));
        rec.insert("members".into(), json!(members));
        // heritage
        let sym = n.symbol();
        let mut ext = Value::Null;
        if let Some(h) = ast::get_class_extends_heritage_element(n) {
            let ek = h.expression().map(|e| format!("{:?}", e.kind())).unwrap_or_default();
            let mut em = Map::new();
            em.insert("ek".into(), json!(ek));
            if let Some(sym) = sym {
                let dt = self.c.get_declared_type_of_symbol(sym);
                let bases = self.c.get_base_types(dt);
                match bases.first() {
                    Some(&b) => {
                        if is_class_instance(b) {
                            match self.class_id_of_type(b) {
                                Ok(bid) => {
                                    em.insert("id".into(), json!(bid));
                                }
                                Err(nm) => {
                                    em.insert("foreign".into(), json!(nm));
                                }
                            }
                        } else {
                            let d = self.desc(b, 1);
                            em.insert("foreign".into(), json!(format!("non-class:{}", d.get("c").and_then(Value::as_str).unwrap_or("?"))));
                        }
                    }
                    None => {
                        em.insert("foreign".into(), json!("unresolved"));
                    }
                }
            }
            ext = Value::Object(em);
        }
        rec.insert("ext".into(), ext);
        let mut impls = Vec::new();
        {
            {
                for &ty in ast::get_heritage_elements(n, Kind::ImplementsKeyword) {
                    let t = self.c.get_type_at_location(ty);
                    match self.type_decl_id(t) {
                        Ok(iid) => impls.push(json!(iid)),
                        Err(nm) => impls.push(json!(format!("dts:{nm}"))),
                    }
                }
            }
        }
        rec.insert("impl".into(), json!(impls));
        // fields from the checker (own instance properties)
        let mut fields = Vec::new();
        let mut merged = false;
        if let Some(sym) = sym {
            merged = sym.declarations().iter().any(|d| d.kind() == Kind::InterfaceDeclaration);
            let dt = self.c.get_declared_type_of_symbol(sym);
            let props = self.c.get_properties_of_type(dt);
            for &p in props {
                if !p.flags().intersects(SymbolFlags::Property) {
                    continue;
                }
                let own = p.parent().map(|pp| pp == sym).unwrap_or(true)
                    || p.declarations().iter().any(|d| ast::get_containing_class(*d) == Some(n));
                if !own {
                    continue;
                }
                let mut fr = Map::new();
                fr.insert("n".into(), json!(p.name()));
                fr.insert("opt".into(), json!(p.flags().intersects(SymbolFlags::Optional)));
                let mut dk = Vec::new();
                for &d in p.declarations() {
                    match d.kind() {
                        Kind::PropertyDeclaration => {
                            let pf = ast::get_combined_modifier_flags(d);
                            if d.initializer().is_some() {
                                fr.insert("init".into(), json!(true));
                            }
                            if d.postfix_token().is_some_and(|t| t.kind() == Kind::ExclamationToken) {
                                fr.insert("def".into(), json!(true));
                            }
                            if pf.intersects(ModifierFlags::Ambient) {
                                fr.insert("declare".into(), json!(true));
                            }
                            if pf.intersects(ModifierFlags::Abstract) {
                                fr.insert("abstract".into(), json!(true));
                            }
                            if ast::has_accessor_modifier(d) {
                                fr.insert("acc".into(), json!(true));
                            }
                            dk.push("prop");
                        }
                        Kind::Parameter => dk.push("pp"),
                        Kind::BinaryExpression | Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                            let inctor = ast::get_containing_function(d).is_some_and(|f| f.kind() == Kind::Constructor);
                            dk.push(if inctor { "js-ctor" } else { "js-late" });
                        }
                        Kind::PropertySignature => dk.push("merged"),
                        _ => dk.push("other"),
                    }
                }
                fr.insert("dk".into(), json!(dk));
                let ft = self.c.get_type_of_symbol(p);
                let d = self.desc(ft, 2);
                fr.insert("t".into(), d);
                fields.push(Value::Object(fr));
            }
        }
        rec.insert("merged".into(), json!(merged));
        rec.insert("fields".into(), json!(fields));
        // ctor scan: param properties, then instance field initializers in order, then the ctor body
        let mut st = Scan::default();
        if let Some(c) = ctor {
            for &p in c.parameters() {
                if ast::get_combined_modifier_flags(p).intersects(ModifierFlags::ParameterPropertyModifier) {
                    if let Some(nm) = p.name() {
                        st.assigned.push(name_text(nm));
                    }
                }
            }
        }
        for &m in n.members() {
            if m.kind() == Kind::PropertyDeclaration && !ast::is_static(m) {
                if let Some(init) = m.initializer() {
                    let ik = skip_parens(init).kind();
                    if !(ik == Kind::ArrowFunction || ik == Kind::FunctionExpression) {
                        self.scan(init, &mut st);
                    }
                    if let Some(nm) = m.name() {
                        st.assigned.push(name_text(nm));
                    }
                }
            }
        }
        if let Some(c) = ctor {
            if let Some(b) = c.body() {
                self.scan(b, &mut st);
            }
        }
        let bl = st.before_leak.clone().unwrap_or_else(|| st.assigned.clone());
        rec.insert("bl".into(), json!(bl));
        rec.insert("aa".into(), json!(st.assigned));
        rec.insert("ev".into(), json!(st.events.iter().map(|(k, s, c)| json!([k, s, c])).collect::<Vec<_>>()));
        rec.insert("leak".into(), match &st.leak {
            Some((k, s)) => json!({"k": k, "s": s}),
            None => Value::Null,
        });
        Value::Object(rec)
    }

    // ---------------------------------------------------------------- functions
    fn func_name(n: P<Node>) -> String {
        if let Some(nm) = n.name() {
            let t = name_text(nm);
            if !t.is_empty() {
                return t;
            }
        }
        if let Some(p) = n.parent() {
            match p.kind() {
                Kind::VariableDeclaration | Kind::PropertyAssignment | Kind::PropertyDeclaration => {
                    if let Some(nm) = p.name() {
                        return name_text(nm);
                    }
                }
                Kind::BinaryExpression => {
                    let l = skip_parens(p.as_binary_expression().left());
                    if l.kind() == Kind::PropertyAccessExpression {
                        if let Some(nm) = l.name() {
                            return format!(".{}", name_text(nm));
                        }
                    }
                }
                _ => {}
            }
        }
        "<anon>".into()
    }

    fn begin_func(&mut self, n: P<Node>, cur: Option<usize>) -> usize {
        let k = n.kind();
        let mut f = Func {
            id: self.node_id(n).unwrap_or_default(),
            kind: match k {
                Kind::FunctionDeclaration => "fn",
                Kind::FunctionExpression => "fnexpr",
                Kind::ArrowFunction => "arrow",
                Kind::MethodDeclaration => "method",
                Kind::Constructor => "ctor",
                Kind::GetAccessor => "get",
                Kind::SetAccessor => "set",
                _ => "?",
            },
            name: Self::func_name(n),
            s: start_of(n),
            e: end_of(n),
            parent: cur,
            ..Default::default()
        };
        if let Some(b) = n.body() {
            f.bs = start_of(b);
            f.be = end_of(b);
        }
        let parent = n.parent();
        // class member?
        if let Some(p) = parent {
            if matches!(p.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
                && matches!(k, Kind::MethodDeclaration | Kind::Constructor | Kind::GetAccessor | Kind::SetAccessor)
            {
                f.cls = self.node_id(p);
                f.st = ast::is_static(n);
                let cf = ast::get_combined_modifier_flags(p);
                let mf = ast::get_combined_modifier_flags(n);
                f.exp = cf.intersects(ModifierFlags::Export) && !mf.intersects(ModifierFlags::Private) && n.name().is_none_or(|nm| nm.kind() != Kind::PrivateIdentifier);
            }
        }
        if k == Kind::FunctionDeclaration {
            f.exp = ast::get_combined_modifier_flags(n).intersects(ModifierFlags::Export);
        }
        f.ctx = match parent.map(|p| (p, p.kind())) {
            Some((p, Kind::CallExpression | Kind::NewExpression)) => {
                if p.expression().map(skip_parens) == Some(n) { "iife" } else { "arg" }
            }
            Some((p, Kind::ParenthesizedExpression)) => {
                let mut q = p;
                while q.kind() == Kind::ParenthesizedExpression {
                    match q.parent() {
                        Some(x) => q = x,
                        None => break,
                    }
                }
                if matches!(q.kind(), Kind::CallExpression) && q.expression().map(skip_parens) == Some(n) { "iife" } else { "other" }
            }
            Some((_, Kind::VariableDeclaration)) => "var",
            Some((_, Kind::PropertyAssignment)) => "prop",
            Some((_, Kind::PropertyDeclaration)) => "field",
            Some((_, Kind::ReturnStatement)) => "ret",
            Some((_, Kind::BinaryExpression)) => "assign",
            Some((_, Kind::ObjectLiteralExpression)) => "objmethod",
            Some((_, Kind::ClassDeclaration | Kind::ClassExpression)) => "member",
            _ => "other",
        };
        for &p in n.parameters() {
            let Some(nm) = p.name() else { continue };
            if nm.kind() == Kind::Identifier && name_text(nm) == "this" {
                // explicit this parameter
                let d = self.tdesc_at(nm);
                f.this_d = Some(d);
                continue;
            }
            let d = self.tdesc_at(nm);
            f.params.push(d);
        }
        // `this` for non-arrow, non-member functions that use it
        if f.cls.is_none() && k != Kind::ArrowFunction && f.this_d.is_none() {
            if let Some(b) = n.body() {
                if let Some(th) = first_this(b) {
                    let d = self.tdesc_at(th);
                    f.this_d = Some(d);
                }
            }
        }
        if let Some(c) = cur {
            if k == Kind::FunctionDeclaration {
                self.funcs[c].locals.push(json!({"c": "fn", "decl": 1}));
            }
            self.funcs[c].nested += 1;
        }
        self.funcs.push(f);
        self.funcs.len() - 1
    }

    fn binding_names(n: P<Node>, out: &mut Vec<P<Node>>) {
        match n.kind() {
            Kind::Identifier => out.push(n),
            Kind::ObjectBindingPattern | Kind::ArrayBindingPattern => {
                for ch in n.iter_children() {
                    Self::binding_names(ch, out);
                }
            }
            Kind::BindingElement => {
                if let Some(nm) = n.name() {
                    Self::binding_names(nm, out);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- walk
    fn guarded(&mut self, n: P<Node>, f: impl FnOnce(&mut Self)) {
        let r = catch_unwind(AssertUnwindSafe(|| f(self)));
        if let Err(e) = r {
            self.panics += 1;
            if self.panic_ex.len() < 5 {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_default();
                self.panic_ex.push(format!("{:?}@{}: {}", n.kind(), start_of(n), msg));
            }
        }
    }

    fn visit(&mut self, n: P<Node>, cur: Option<usize>) {
        let k = n.kind();
        if is_type_only_container(k) {
            return;
        }
        if k == Kind::HeritageClause && n.as_heritage_clause().token == Kind::ImplementsKeyword {
            return;
        }
        if matches!(k, Kind::ClassDeclaration | Kind::ModuleDeclaration | Kind::FunctionDeclaration | Kind::VariableStatement | Kind::EnumDeclaration)
            && ast::get_combined_modifier_flags(n).intersects(ModifierFlags::Ambient)
        {
            return;
        }
        let mut next = cur;
        self.guarded(n, |s| s.handle(n, cur, &mut next));
        if matches!(k, Kind::ClassDeclaration | Kind::ClassExpression) {
            // members: methods create their own funcs; field initializers run at construction (no func)
            for ch in n.iter_children() {
                let ck = ch.kind();
                let c2 = if ck == Kind::PropertyDeclaration || ck == Kind::ClassStaticBlockDeclaration { None } else { cur };
                self.visit(ch, c2);
            }
            return;
        }
        for ch in n.iter_children() {
            self.visit(ch, next);
        }
    }

    fn handle(&mut self, n: P<Node>, cur: Option<usize>, next: &mut Option<usize>) {
        let k = n.kind();
        if is_fn_like_with_body(n) {
            let i = self.begin_func(n, cur);
            *next = Some(i);
            // expression-bodied arrow: the body is a return flow
            if k == Kind::ArrowFunction {
                if let Some(b) = n.body() {
                    if b.kind() != Kind::Block {
                        self.flow(b, Some(i), false);
                    }
                }
            }
            for &p in n.parameters() {
                let destructure = p.name().is_some_and(|nm| matches!(nm.kind(), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern));
                if let Some(init) = p.initializer() {
                    if !destructure {
                        self.flow(init, Some(i), false);
                    }
                }
            }
            return;
        }
        match k {
            Kind::ClassDeclaration | Kind::ClassExpression => {
                let r = self.class_rec(n);
                self.classes.push(r);
                if let Some(c) = cur {
                    self.funcs[c].locals.push(json!({"c": "ctor", "decl": 1}));
                }
            }
            Kind::VariableDeclaration => {
                let destructure = n.name().is_some_and(|nm| matches!(nm.kind(), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern));
                if let Some(c) = cur {
                    let mut names = Vec::new();
                    if let Some(nm) = n.name() {
                        Self::binding_names(nm, &mut names);
                    }
                    for nm in names {
                        let d = self.tdesc_at(nm);
                        self.funcs[c].locals.push(d);
                    }
                }
                if let Some(init) = n.initializer() {
                    if !destructure {
                        self.flow(init, cur, false);
                    }
                }
            }
            Kind::PropertyDeclaration => {
                if let Some(init) = n.initializer() {
                    self.flow(init, cur, false);
                }
            }
            Kind::ReturnStatement | Kind::YieldExpression => {
                if let Some(e) = n.expression() {
                    self.flow(e, cur, false);
                }
            }
            Kind::ThrowStatement => {
                if let Some(e) = n.expression() {
                    self.flow(e, cur, true);
                }
            }
            Kind::PropertyAssignment => {
                if let Some(init) = n.initializer() {
                    self.flow(init, cur, false);
                }
            }
            Kind::ShorthandPropertyAssignment => {
                if let Some(nm) = n.name() {
                    self.flow(nm, cur, false);
                }
            }
            Kind::ArrayLiteralExpression => {
                for &el in n.elements() {
                    if !matches!(el.kind(), Kind::SpreadElement | Kind::OmittedExpression) {
                        self.flow(el, cur, false);
                    }
                }
            }
            Kind::AsExpression | Kind::TypeAssertionExpression | Kind::NonNullExpression => {
                if k != Kind::NonNullExpression && ast::is_const_assertion(n) {
                    return;
                }
                let Some(inner) = n.expression() else { return };
                let s_t = self.c.get_type_at_location(inner);
                let t_t = self.c.get_type_at_location(n);
                let (sd, sn) = self.refs_all(s_t);
                let (td, tn) = self.refs_all(t_t);
                let up = self.c.is_type_assignable_to(s_t, t_t);
                let sc = self.desc(s_t, 0);
                let tc = self.desc(t_t, 0);
                let kind = if k == Kind::NonNullExpression { "nn" } else { "as" };
                if let Some(c) = cur {
                    if kind == "nn" {
                        self.funcs[c].nonnull += 1;
                    } else if up {
                        self.funcs[c].upcast += 1;
                    } else {
                        self.funcs[c].asrt += 1;
                    }
                }
                self.casts.push(json!({"k": kind, "f": cur, "sd": sd, "sn": sn, "td": td, "tn": tn, "up": up,
                    "sc": sc.get("c"), "tc": tc.get("c"), "rep": n.flags().intersects(NodeFlags::Reparsed), "s": start_of(n)}));
            }
            Kind::PropertyAccessExpression => {
                let Some(recv) = n.expression() else { return };
                let rt = self.c.get_type_at_location(recv);
                let rd = self.desc(rt, 1);
                let (dd, _) = self.refs_all(rt);
                let name = n.name().map(|x| x.text()).unwrap_or("");
                let mut sym_kind = "";
                let is_callee = n.parent().is_some_and(|p| {
                    matches!(p.kind(), Kind::CallExpression | Kind::NewExpression) && p.expression() == Some(n)
                });
                if let Some(nm) = n.name() {
                    if let Some(s) = self.c.get_symbol_at_location_exported(nm) {
                        let fl = s.flags();
                        sym_kind = if fl.intersects(SymbolFlags::Method) {
                            "method"
                        } else if fl.intersects(SymbolFlags::GetAccessor | SymbolFlags::SetAccessor) {
                            "accessor"
                        } else if fl.intersects(SymbolFlags::Property) {
                            "field"
                        } else {
                            "other"
                        };
                        if !is_callee && fl.intersects(SymbolFlags::Method | SymbolFlags::Function) {
                            if let Some(d) = Self::body_decl(s) {
                                if let Some(fid) = self.node_id(d) {
                                    self.vrefs.push(json!([cur, fid, start_of(n)]));
                                }
                            }
                        }
                    }
                }
                self.members.push(json!({"f": cur, "rc": rd.get("c"), "this": rd.get("this").is_some() || skip_parens(recv).kind() == Kind::ThisKeyword,
                    "d": dd, "sk": sym_kind, "s": start_of(n)}));
                if name == "__proto__" && !dd.is_empty() {
                    self.r3.push(json!({"k": "__proto__", "inst": dd, "ctor": [], "s": start_of(n)}));
                }
            }
            Kind::ElementAccessExpression => {
                let ea = n.as_element_access_expression();
                let recv = ea.expression;
                let key = skip_parens(ea.argument_expression);
                let rt = self.c.get_type_at_location(recv);
                let rd = self.desc(rt, 1);
                let (dd, _) = self.refs_all(rt);
                let kt = self.c.get_type_at_location(key);
                let kf = kt.flags();
                let const_key = matches!(key.kind(), Kind::StringLiteral | Kind::NumericLiteral | Kind::NoSubstitutionTemplateLiteral)
                    || kf.intersects(TypeFlags::StringLiteral | TypeFlags::NumberLiteral | TypeFlags::UniqueESSymbol | TypeFlags::EnumLiteral | TypeFlags::BooleanLiteral);
                // r2 only matters on class-instance receivers (direct)
                let rc = rd.get("c").and_then(Value::as_str).unwrap_or("");
                let on_class = !dd.is_empty() && !matches!(rc, "array" | "tuple");
                if on_class && !const_key {
                    if let Some(c) = cur {
                        self.funcs[c].r2 += 1;
                    }
                }
                self.members.push(json!({"f": cur, "rc": rd.get("c"), "d": if matches!(rc, "array" | "tuple") { json!([]) } else { json!(dd) },
                    "el": 1, "ck": const_key, "s": start_of(n)}));
                if key.kind() == Kind::StringLiteral && key.text() == "__proto__" && !dd.is_empty() {
                    self.r3.push(json!({"k": "__proto__", "inst": dd, "ctor": [], "s": start_of(n)}));
                }
            }
            Kind::DeleteExpression => {
                if let Some(e) = n.expression() {
                    let e = skip_parens(e);
                    if matches!(e.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                        if let Some(r) = e.expression() {
                            let rt = self.c.get_type_at_location(r);
                            let (dd, _) = self.refs_all(rt);
                            let rd = self.desc(rt, 0);
                            self.r3.push(json!({"k": "delete", "inst": dd, "ctor": [], "rc": rd.get("c"), "f": cur, "s": start_of(n)}));
                        }
                    }
                }
            }
            Kind::WithStatement => {
                self.r7.push(json!({"k": "with", "s": start_of(n)}));
            }
            Kind::BinaryExpression => {
                let be = n.as_binary_expression();
                let op = be.operator_token.kind();
                if is_assign_op(op) {
                    if !matches!(skip_parens(be.left()).kind(), Kind::ObjectLiteralExpression | Kind::ArrayLiteralExpression) {
                        self.flow(be.right(), cur, false);
                    }
                    // X.prototype.y = ... / X.prototype = ...
                    let l = skip_outer(be.left());
                    let mut proto_owner: Option<P<Node>> = None;
                    if l.kind() == Kind::PropertyAccessExpression && l.name().is_some_and(|nm| nm.text() == "prototype") {
                        proto_owner = l.expression();
                    } else if matches!(l.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                        if let Some(o) = l.expression() {
                            let o = skip_outer(o);
                            if o.kind() == Kind::PropertyAccessExpression && o.name().is_some_and(|nm| nm.text() == "prototype") {
                                proto_owner = o.expression();
                            }
                        }
                    }
                    if let Some(po) = proto_owner {
                        let t = self.c.get_type_at_location(po);
                        let d = self.desc(t, 0);
                        let mut ctor_ids = Vec::new();
                        if let Some(id) = d.get("id") {
                            ctor_ids.push(id.clone());
                        }
                        self.r3.push(json!({"k": "proto-assign", "inst": [], "ctor": ctor_ids, "rc": d.get("c"), "f": cur, "s": start_of(n)}));
                    }
                }
            }
            Kind::CallExpression | Kind::NewExpression => {
                let Some(callee) = n.expression() else { return };
                let cs = skip_parens(callee);
                // r7
                if cs.kind() == Kind::Identifier {
                    let t = cs.text();
                    if t == "eval" && k == Kind::CallExpression {
                        self.r7.push(json!({"k": "eval", "s": start_of(n)}));
                    }
                    if t == "Function" {
                        self.r7.push(json!({"k": "new Function", "s": start_of(n)}));
                    }
                    if t == "Proxy" && k == Kind::NewExpression {
                        if let Some(&a0) = n.arguments().first() {
                            let at = self.c.get_type_at_location(a0);
                            let (dd, _) = self.refs_all(at);
                            self.r3.push(json!({"k": "Proxy", "inst": dd, "ctor": [], "f": cur, "s": start_of(n)}));
                        }
                    }
                }
                // r3 reflective calls
                if cs.kind() == Kind::PropertyAccessExpression {
                    if let (Some(o), Some(nm)) = (cs.expression(), cs.name()) {
                        let o = skip_parens(o);
                        if o.kind() == Kind::Identifier {
                            let ot = o.text();
                            let mt = nm.text();
                            let hit = (ot == "Object"
                                && matches!(mt, "defineProperty" | "defineProperties" | "setPrototypeOf" | "assign" | "freeze" | "seal" | "preventExtensions" | "create"))
                                || (ot == "Reflect" && matches!(mt, "set" | "defineProperty" | "deleteProperty" | "setPrototypeOf" | "preventExtensions"));
                            if hit {
                                if let Some(&a0) = n.arguments().first() {
                                    let at = self.c.get_type_at_location(a0);
                                    let (dd, _) = self.refs_all(at);
                                    let ad = self.desc(at, 0);
                                    let mut ctor_ids = Vec::new();
                                    if ad.get("c").and_then(Value::as_str) == Some("ctor") {
                                        if let Some(id) = ad.get("id") {
                                            ctor_ids.push(id.clone());
                                        }
                                    }
                                    self.r3.push(json!({"k": format!("{ot}.{mt}"), "inst": dd, "ctor": ctor_ids, "rc": ad.get("c"), "f": cur, "s": start_of(n)}));
                                }
                            }
                        }
                    }
                }
                // call target
                let sig = self.c.get_resolved_signature_exported(n);
                // declared (uninstantiated) parameter types of project generic callees
                let mut gts: Vec<Option<Value>> = Vec::new();
                if let Some(d) = sig.declaration.get() {
                    if !self.decl_is_foreign(d) && ast::is_function_like(Some(d)) {
                        let ps = d.parameters();
                        for i in 0..n.arguments().len() {
                            let g = match ps.get(i).and_then(|p| p.name()) {
                                Some(nm) if nm.kind() == Kind::Identifier => {
                                    let t = self.c.get_type_at_location(nm);
                                    let v = self.desc(t, 2);
                                    if Self::has_tp(&v) { Some(v) } else { None }
                                }
                                _ => None,
                            };
                            gts.push(g);
                        }
                    }
                }
                // flows into arguments
                for (i, &a) in n.arguments().iter().enumerate() {
                    if a.kind() != Kind::SpreadElement {
                        let g = gts.get(i).cloned().flatten();
                        self.flow_g(a, cur, false, g);
                    } else if let Some(inner) = a.expression() {
                        // spread argument: elements of `inner` flow to the parameter type at this position
                        self.flow_c(inner, a, cur, false, None);
                    }
                }
                let tgt = sig.declaration.get().and_then(|d| self.fn_id_of_decl(d));
                let foreign = sig.declaration.get().is_some_and(|d| self.decl_is_foreign(d));
                let is_super = cs.kind() == Kind::SuperKeyword;
                let mname = if cs.kind() == Kind::PropertyAccessExpression { cs.name().map(|x| x.text()) } else { None };
                // interface / type-literal method signature target: record the owner for dispatch attribution
                let ti = sig.declaration.get().filter(|d| matches!(d.kind(), Kind::MethodSignature | Kind::PropertySignature))
                    .and_then(|d| d.parent()).and_then(|p| self.node_id(p));
                self.calls.push(json!({"f": cur, "t": tgt, "ti": ti, "fx": foreign, "sup": is_super, "m": mname, "s": start_of(n)}));
            }
            Kind::Identifier => {
                // function value references (address taken): not callee, not declaration name
                let Some(p) = n.parent() else { return };
                if !ast::is_expression_node(n) && p.kind() != Kind::ShorthandPropertyAssignment {
                    return;
                }
                if matches!(p.kind(), Kind::CallExpression | Kind::NewExpression) && p.expression() == Some(n) {
                    return;
                }
                if p.kind() == Kind::PropertyAccessExpression && p.name() == Some(n) {
                    return;
                }
                if p.name() == Some(n) && p.kind() != Kind::ShorthandPropertyAssignment {
                    return;
                }
                let sym0 = if p.kind() == Kind::ShorthandPropertyAssignment {
                    self.c.get_shorthand_assignment_value_symbol(Some(p))
                } else {
                    self.c.get_symbol_at_location_exported(n)
                };
                if let Some(s) = sym0 {
                    let s = self.resolve_sym(s);
                    if s.flags().intersects(SymbolFlags::Function | SymbolFlags::Method) {
                        if let Some(d) = Self::body_decl(s) {
                            if let Some(fid) = self.node_id(d) {
                                self.vrefs.push(json!([cur, fid, start_of(n)]));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn out_line(self, sf: P<SourceFile>) -> String {
        let funcs: Vec<Value> = self
            .funcs
            .iter()
            .map(|f| {
                json!({"id": f.id, "k": f.kind, "n": f.name, "s": f.s, "e": f.e, "bs": f.bs, "be": f.be, "cls": f.cls, "st": f.st,
                    "exp": f.exp, "params": f.params, "this": f.this_d, "locals": f.locals, "nested": f.nested, "asrt": f.asrt,
                    "up": f.upcast, "nn": f.nonnull, "anyflow": f.anyflow, "r2": f.r2, "parent": f.parent, "ctx": f.ctx})
            })
            .collect();
        let v = json!({"file": sf.file_name(), "fi": self.fi, "js": ast::is_in_js_file(Some(sf.as_node())),
            "classes": self.classes, "funcs": funcs, "flows": self.flows, "ifl": self.ifl, "casts": self.casts,
            "members": self.members, "r3": self.r3, "r7": self.r7, "calls": self.calls, "vrefs": self.vrefs,
            "panics": self.panics, "panic_ex": self.panic_ex});
        let mut s = serde_json::to_string(&v).unwrap();
        s.push('\n');
        s
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: region_facts <tsconfig|dir> <out.jsonl>");
        std::process::exit(2);
    }
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
    let compiler_host = new_cached_fs_compiler_host(&cwd, fs, &bundled::embed::lib_path(), None, None, None);
    let mut opts = ProgramOptions::new(P::new(config.unwrap()), compiler_host);
    opts.single_threaded = Tristate::True;
    let program = new_program(opts);
    let diags = get_diagnostics_of_any_program(
        &Context::default(),
        program,
        None,
        false,
        &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
        &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
    );
    let mut diag_codes: std::collections::BTreeMap<i32, usize> = Default::default();
    for d in &diags {
        *diag_codes.entry(d.code()).or_default() += 1;
    }
    let t_check = t_start.elapsed();
    let mut files: Vec<P<SourceFile>> = Vec::new();
    for &sf in program.source_files() {
        if program.is_source_file_default_library(sf.path()) || sf.is_declaration_file() {
            continue;
        }
        if sf.file_name().contains("/node_modules/") {
            continue;
        }
        files.push(sf);
    }
    let mut proj = HashMap::new();
    for (i, sf) in files.iter().enumerate() {
        proj.insert(sf.file_name().to_string(), i);
    }
    let g = G { program, proj };
    let _ = g.program;
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    let names: Vec<&str> = files.iter().map(|f| f.file_name()).collect();
    serde_json::to_writer(&mut out, &json!({"files": names})).unwrap();
    out.write_all(b"\n").unwrap();
    let mut panics = 0usize;
    for (i, &sf) in files.iter().enumerate() {
        let mut handle = program.get_type_checker_for_file(&Context::default(), sf);
        let mut fw = Fw {
            g: &g,
            c: &mut handle,
            fi: i,
            funcs: Vec::new(),
            classes: Vec::new(),
            flows: Vec::new(),
            ifl: Vec::new(),
            casts: Vec::new(),
            members: Vec::new(),
            r3: Vec::new(),
            r7: Vec::new(),
            calls: Vec::new(),
            vrefs: Vec::new(),
            panics: 0,
            panic_ex: Vec::new(),
        };
        for ch in sf.as_node().iter_children() {
            fw.visit(ch, None);
        }
        panics += fw.panics;
        for p in &fw.panic_ex {
            eprintln!("panic {}: {}", sf.file_name(), p);
        }
        let line = fw.out_line(sf);
        out.write_all(line.as_bytes()).unwrap();
    }
    let stats = json!({"stats": {"config": config_path, "src_files": files.len(), "diagnostics": diags.len(),
        "diag_codes": diag_codes.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_,_>>(),
        "panics": panics, "ms_check": t_check.as_secs_f64() * 1e3, "ms_total": t_start.elapsed().as_secs_f64() * 1e3}});
    serde_json::to_writer(&mut out, &stats).unwrap();
    out.write_all(b"\n").unwrap();
    out.flush().unwrap();
    eprintln!("{}", serde_json::to_string(&stats).unwrap());
}
