//! `eval` code (ECMA-262 PerformEval, §19.2.1.1): [`compile_eval`] turns the eval text and the
//! caller's [`EvalContext`] into a [`VerifiedUnit`] of [`CodeKind::Eval`](ep9_bytecode::CodeKind)
//! (C3, C4). The VM calls it through its compiler hook (`IsolateConfig::compiler`, C2): ep9_vm
//! has no edge to this crate.
//!
//! ```text
//! caller thread: prescan (layer 0)
//! front-end worker (own job: crate::parse / crate::semantic return on the first diagnostic):
//!   oxc_parser, Script goal -------------- fatal, or an error not in the allowed set --> Syntax
//!   depth::check_ast_depth (layer 2) ----- too deep --------------------------------> Range
//!   allowed set: the spans of the new.target, super.x / super[x], super(...) and #x nodes
//!     that the caller permits (EvalContext); parser and semantic diagnostics whose label
//!     span is one of them are dropped (and only those)
//!   completion rewrite (V8 Rewriter model): `.result = <value>` stores, see below
//!   caller strict: force_strict (a synthetic "use strict" directive)
//!   oxc_semantic (new_compiler) ---------- an error not in the allowed set --------> Syntax
//!   post-checks: `arguments` in a field initializer; every #x declared here or in
//!     EvalContext::private_names ------------------------------------------------> Syntax
//!   super(...) allowed and present, a #x of the caller's class,
//!     Annex B function in sloppy eval ------------------------------------------> Unsupported
//!   scope::analyze_eval (ScopeKind::Eval root; direct: free names Dynamic, indirect: Global)
//!   bytecodegen::generate (CodeKind::Eval; gen_eval_program)  --> VerifiedUnit
//! ```
//!
//! # Decisions
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | E1 | No wrapper parse: the text is parsed as a Script. A recoverable oxc error whose label span equals an allowed node is dropped | OXC-VERIFY: oxc builds the whole AST for the 4 rows (docs/upstream/oxc-eval-parse-options.md); a wrapper accepts `return 1` |
//! | E2 | The label spans (oxc 0.155, `SemanticBuilder::new_compiler()`): `new.target` = the NewTarget node (parser error); `super.x`, `super[x]` = the Super node; `super()` = the whole CallExpression; `#x` = the PrivateIdentifier (`#x`, with the `#`) | Measured, the table in `tests` and the oxc doc |
//! | E3 | `new.target` and `super` are allowed only at the eval code's own function level (also inside arrows), never inside a function, method, field initializer or static block of the eval code | Those have their own `new.target` and home object; oxc reports nothing there when they are valid |
//! | E4 | For indirect eval the context flags and `private_names` are ignored (treated as false and empty) | PerformEval: indirect eval is global code |
//! | E5 | The completion value comes from an AST rewrite before oxc_semantic: value-producing statements store into the synthetic root binding `.result` (V8 `Rewriter`, src/parsing/rewriter.cc, ported statement by statement), and function 0 returns it. A function or class value is stored through a one-element sequence, so it gets no name | The statement generators (stmt.rs) have no completion register; the rewrite keeps the change inside this file. `eval("(function(){})").name` is `""` (node 24) |
//! | E6 | `super()` in eval code is Unsupported in P2 | `super_call` (expr.rs) needs a static `this` binding; the caller's `this` is only reachable by name (C4) |
//! | E7 | An Annex B.3.2.3 function in sloppy eval code is Unsupported in P2 | Whether it gets a var binding depends on the caller's environments at run time (EvalDeclarationInstantiation B.3.2.3 steps i.1-7); the front end alone cannot decide it |
//! | E8 | Lone surrogates never reach this function in M0 (the hook takes `&str`) | D8: the shim is milestone-M3 work (01-front-end.md:159) |
//! | E9 | A `#x` that only the caller's class declares (`EvalContext::private_names`) is Unsupported in P2; the scope pass still resolves it `Dynamic` | The private-name codegen (bytecodegen/private.rs) needs a static binding; without this check such code would reach it as an internal error. The check goes when private.rs gets a by-name path (P3-EVAL-VM) |

use ep9_bytecode::{EvalContext, VerifiedUnit};
use oxc_allocator::{Allocator, CloneIn, GetAllocator, TakeIn};
use oxc_ast::ast::{
    AccessorProperty, AssignmentOperator, AssignmentTarget, CallExpression, Class, ClassElement,
    Expression, Function, IdentifierReference, NewTarget, PrivateFieldExpression,
    PrivateInExpression, Program, PropertyDefinition, PropertyKey, Statement, StaticBlock, Super,
};
use oxc_ast::builder::AstBuilder;
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::{ScopeFlags, SemanticBuilder};
use oxc_span::{GetSpan, SPAN, SourceType, Span};

use crate::bytecodegen::{self, CodegenError, CompileOptions, EVAL_RESULT, Unsupported};
use crate::scope::{self, EvalKind, ScopeId};
use crate::{
    AST_CAP, ByteSpan, CompileError, DepthLayer, EarlyErrors, FrontEnd, Goal, LineMap, PRESCAN_CAP,
    RangeError, RangeErrorKind, SourceOpts, Strictness, SyntaxError,
};

/// Compile `source` as eval code for the caller described by `ctx`.
///
/// The unit's function 0 returns the completion value of the code. For direct eval the VM runs
/// it as a closure over the caller's current context (free names are looked up by name, C4);
/// for indirect eval as global code with the global this as receiver. Sloppy eval code calls
/// `DeclareEvalFunction` and `DeclareEvalVar` for its top-level declarations (see
/// `bytecodegen::gen_eval_program`).
///
/// # Errors
///
/// See [`CodegenError`]: early errors (also the eval-only ones of PerformEval), depth limits,
/// and [`CodegenError::Unsupported`] for `super()` in eval code, for a `#x` of the caller's
/// class, for an Annex B function in sloppy eval code, and for every construct that BytecodeGen v0 does not support.
pub fn compile_eval(
    source: &str,
    ctx: &EvalContext,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    let estimate = crate::prescan::estimate(source.as_bytes(), false);
    if estimate.depth > PRESCAN_CAP {
        return Err(too_deep(DepthLayer::PreScan, source, estimate.at));
    }
    let owned = source.to_owned();
    let allow = Allow::from_context(ctx);
    let job = move || {
        on_worker(&owned, &allow, |fe, analysis| {
            bytecodegen::generate(fe, analysis, options)
        })
    };
    match crate::worker::run(crate::stack::WORKER_STACK_BYTES, job) {
        Ok(result) => result,
        Err(spawn) => Err(CompileError::Range(RangeError {
            kind: RangeErrorKind::StackUnavailable,
            message: spawn.message,
            span: None,
            position: None,
        })
        .into()),
    }
}

/// What the caller permits (PerformEval steps 4-11), with the indirect-eval rule E4.
#[derive(Clone, Debug)]
struct Allow {
    direct: bool,
    strict: bool,
    new_target: bool,
    super_property: bool,
    super_call: bool,
    in_class_field_init: bool,
    /// Without the `#`. The synthetic names of class codegen (`.brand`, ...) are left out:
    /// no source text can spell them.
    private_names: Vec<String>,
}

impl Allow {
    fn from_context(ctx: &EvalContext) -> Self {
        let d = ctx.direct;
        Self {
            direct: d,
            strict: ctx.strict,
            new_target: d && ctx.new_target_ok,
            super_property: d && ctx.super_property_ok,
            super_call: d && ctx.super_call_ok,
            in_class_field_init: d && ctx.in_class_field_init,
            private_names: if d {
                ctx.private_names
                    .iter()
                    .filter(|n| !n.starts_with('.'))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    fn private_name_ok(&self, name: &str) -> bool {
        self.private_names.iter().any(|n| n == name)
    }
}

/// Everything after the pre-scan, on the front-end worker: the checks and the scope pass,
/// then `codegen`.
fn on_worker<R>(
    source: &str,
    allow: &Allow,
    codegen: impl FnOnce(&FrontEnd<'_>, &scope::ScopeAnalysis) -> Result<R, CodegenError>,
) -> Result<R, CodegenError> {
    let allocator = Allocator::default();
    let parse_options = ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    };
    let mut parsed = Parser::new(&allocator, source, SourceType::script())
        .with_options(parse_options)
        .parse();
    let has_parse_errors = parsed.diagnostics.has_errors();
    if parsed.fatal_error {
        return Err(syntax(source, parsed.diagnostics.errors()));
    }
    // Layer 2 before any walk of the AST (the walks below recurse).
    if let Err(deep) = crate::depth::check_ast_depth(&parsed.program, AST_CAP) {
        if has_parse_errors {
            return Err(syntax(source, parsed.diagnostics.errors()));
        }
        return Err(too_deep(DepthLayer::Ast, source, deep.span.start));
    }
    let allowed = Allowed::collect(&parsed.program, allow);
    if has_parse_errors {
        let kept: Vec<&OxcDiagnostic> = parsed
            .diagnostics
            .errors()
            .filter(|d| !allowed.drops(d))
            .collect();
        if !kept.is_empty() {
            return Err(syntax(source, kept.into_iter()));
        }
    }

    let ast = AstBuilder::new(&allocator);
    rewrite_completion(&ast, &mut parsed.program);
    if allow.strict {
        crate::force_strict(&allocator, &mut parsed.program);
    }
    // The rewrite adds at most 2 levels (an assignment and a sequence) around a statement's
    // expression: check again, so the worker stack stays within its budget.
    if let Err(deep) = crate::depth::check_ast_depth(&parsed.program, AST_CAP) {
        return Err(too_deep(DepthLayer::Ast, source, deep.span.start));
    }

    let program = &parsed.program;
    let built = SemanticBuilder::new_compiler().build(program);
    if built.diagnostics.has_errors() {
        let kept: Vec<&OxcDiagnostic> = built
            .diagnostics
            .errors()
            .filter(|d| !allowed.drops(d))
            .collect();
        if !kept.is_empty() {
            return Err(syntax(source, kept.into_iter()));
        }
    }
    let caller_private = post_checks(source, program, allow)?;
    if let Some(&call) = allowed.super_calls.first() {
        return Err(unsupported(source, "super() in eval code", call));
    }
    if let Some(at) = caller_private {
        return Err(unsupported(
            source,
            "a private name of the caller's class in eval code",
            at,
        ));
    }

    let strictness = if allow.strict {
        Strictness::Strict
    } else {
        Strictness::Sloppy
    };
    let fe = FrontEnd {
        source,
        opts: SourceOpts::script(strictness),
        program,
        semantic: &built.semantic,
    };
    debug_assert_eq!(fe.opts.goal(), Goal::Script);
    let kind = if allow.direct {
        EvalKind::Direct
    } else {
        EvalKind::Indirect
    };
    let analysis = scope::analyze_eval(&fe, kind)?;
    check_annex_b(source, &analysis)?;
    codegen(&fe, &analysis)
}

fn too_deep(layer: DepthLayer, source: &str, at: u32) -> CodegenError {
    CodegenError::Compile(CompileError::Range(RangeError::too_deep(
        layer,
        source,
        Some(at),
    )))
}

fn syntax<'d>(source: &str, diagnostics: impl Iterator<Item = &'d OxcDiagnostic>) -> CodegenError {
    CodegenError::Compile(CompileError::Syntax(crate::early_errors(
        source,
        diagnostics,
    )))
}

fn syntax_at(source: &str, message: String, span: Span) -> CodegenError {
    let span = ByteSpan::of(span);
    CodegenError::Compile(CompileError::Syntax(EarlyErrors {
        errors: vec![SyntaxError {
            message,
            span: Some(span),
            position: Some(LineMap::new(source.as_bytes()).position(span.start)),
        }],
    }))
}

fn unsupported(source: &str, feature: &'static str, span: Span) -> CodegenError {
    CodegenError::Unsupported(Unsupported {
        feature,
        span: ByteSpan::of(span),
        position: LineMap::new(source.as_bytes()).position(span.start),
    })
}

// ---------------------------------------------------------------------------------------------
// The allowed set (E1-E3)

/// The spans of the nodes whose oxc diagnostics are dropped.
#[derive(Default)]
struct Allowed {
    /// Sorted, deduplicated `(start, end)`.
    spans: Vec<(u32, u32)>,
    /// The allowed `super(...)` calls (E6).
    super_calls: Vec<Span>,
}

impl Allowed {
    fn collect(program: &Program<'_>, allow: &Allow) -> Self {
        let mut walk = AllowWalk {
            allow,
            functions: 0,
            out: Allowed::default(),
        };
        walk.visit_program(program);
        let mut out = walk.out;
        out.spans.sort_unstable();
        out.spans.dedup();
        out
    }

    /// The diagnostic's label (primary, else the first) is exactly an allowed node.
    fn drops(&self, d: &OxcDiagnostic) -> bool {
        let Some(label) = d
            .labels
            .iter()
            .find(|l| l.primary())
            .or_else(|| d.labels.first())
        else {
            return false;
        };
        let start = label.offset();
        self.spans
            .binary_search(&(start, start.saturating_add(label.len())))
            .is_ok()
    }
}

struct AllowWalk<'s> {
    allow: &'s Allow,
    /// Functions, methods, field initializers and static blocks of the eval code around the
    /// current node (arrows do not count): `new.target` and `super` there are not the
    /// caller's.
    functions: u32,
    out: Allowed,
}

impl AllowWalk<'_> {
    fn add(&mut self, span: Span) {
        self.out.spans.push((span.start, span.end));
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self)) {
        self.functions += 1;
        f(self);
        self.functions -= 1;
    }
}

impl<'a> Visit<'a> for AllowWalk<'_> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.nested(|w| walk::walk_function(w, it, flags));
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.computed {
            self.visit_property_key(&it.key);
        }
        if let Some(value) = &it.value {
            self.nested(|w| w.visit_expression(value));
        }
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        if it.computed {
            self.visit_property_key(&it.key);
        }
        if let Some(value) = &it.value {
            self.nested(|w| w.visit_expression(value));
        }
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.nested(|w| walk::walk_static_block(w, it));
    }

    fn visit_new_target(&mut self, it: &NewTarget) {
        if self.functions == 0 && self.allow.new_target {
            self.add(it.span);
        }
    }

    /// `super.x` and `super[x]` (the callee of `super()` does not come here).
    fn visit_super(&mut self, it: &Super) {
        if self.functions == 0 && self.allow.super_property {
            self.add(it.span);
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if matches!(it.callee, Expression::Super(_)) {
            if self.functions == 0 && self.allow.super_call {
                self.add(it.span);
                self.out.super_calls.push(it.span);
            }
            self.visit_arguments(&it.arguments);
            return;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        if self.allow.private_name_ok(it.field.name.as_str()) {
            self.add(it.field.span);
        }
        walk::walk_private_field_expression(self, it);
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        if self.allow.private_name_ok(it.left.name.as_str()) {
            self.add(it.left.span);
        }
        walk::walk_private_in_expression(self, it);
    }
}

// ---------------------------------------------------------------------------------------------
// Post-checks (PerformEval steps 10-11 and AllPrivateIdentifiersValid)

/// The checks; `Ok` holds the first `#x` that only the caller's class declares (E9).
fn post_checks(
    source: &str,
    program: &Program<'_>,
    allow: &Allow,
) -> Result<Option<Span>, CodegenError> {
    let mut walk = PostWalk {
        allow,
        functions: 0,
        classes: Vec::new(),
        error: None,
        caller_private: None,
    };
    walk.visit_program(program);
    match walk.error {
        None => Ok(walk.caller_private),
        Some((message, span)) => Err(syntax_at(source, message, span)),
    }
}

struct PostWalk<'s> {
    allow: &'s Allow,
    /// Non-arrow functions around the node (ContainsArguments does not look inside them).
    functions: u32,
    /// The private names declared by the classes of the eval code around the node.
    classes: Vec<Vec<String>>,
    /// The first error (source order of the walk).
    error: Option<(String, Span)>,
    /// The first `#x` that only the caller's class declares (E9).
    caller_private: Option<Span>,
}

impl PostWalk<'_> {
    fn fail(&mut self, message: String, span: Span) {
        if self.error.is_none() {
            self.error = Some((message, span));
        }
    }

    fn private_ref(&mut self, name: &str, span: Span) {
        if self.classes.iter().any(|c| c.iter().any(|n| n == name)) {
            return;
        }
        if self.allow.private_name_ok(name) {
            self.caller_private.get_or_insert(span);
        } else {
            self.fail(
                format!("Private field '#{name}' must be declared in an enclosing class"),
                span,
            );
        }
    }
}

impl<'a> Visit<'a> for PostWalk<'_> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.functions += 1;
        walk::walk_function(self, it, flags);
        self.functions -= 1;
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        // PerformEval step 10.b / 11: inClassFieldInitializer and ContainsArguments.
        if self.allow.in_class_field_init && self.functions == 0 && it.name == "arguments" {
            self.fail(
                "'arguments' is not allowed in class field initializer or static \
                 initialization block"
                    .to_owned(),
                it.span,
            );
        }
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        // The heritage sees the outer private names only (ClassTail AllPrivateIdentifiersValid).
        if let Some(heritage) = &it.heritage {
            self.visit_expression(&heritage.expression);
        }
        let declared = it
            .body
            .body
            .iter()
            .filter_map(|e| match e {
                ClassElement::MethodDefinition(m) => Some(&m.key),
                ClassElement::PropertyDefinition(p) => Some(&p.key),
                ClassElement::AccessorProperty(p) => Some(&p.key),
                _ => None,
            })
            .filter_map(|k| match k {
                PropertyKey::PrivateIdentifier(id) => Some(id.name.as_str().to_owned()),
                _ => None,
            })
            .collect();
        self.classes.push(declared);
        self.visit_class_body(&it.body);
        self.classes.pop();
    }

    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        self.private_ref(it.field.name.as_str(), it.field.span);
        walk::walk_private_field_expression(self, it);
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        self.private_ref(it.left.name.as_str(), it.left.span);
        walk::walk_private_in_expression(self, it);
    }
}

/// E7: a block function of sloppy eval code whose Annex B var binding is an eval var.
fn check_annex_b(source: &str, sa: &scope::ScopeAnalysis) -> Result<(), CodegenError> {
    let root = sa.scope(ScopeId(0));
    if root.strict {
        return Ok(());
    }
    for b in sa.bindings() {
        if let Some(var) = b.annex_b_var
            && sa.binding(var).scope == root.id
        {
            let span = b.span.map_or(SPAN, |s| Span::new(s.start, s.end));
            return Err(unsupported(
                source,
                "Annex B function declarations in sloppy eval code",
                span,
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The completion value (E5): V8 `Processor` (src/parsing/rewriter.cc), on the oxc AST

/// Rewrite `program` so that it stores its completion value (ECMA-262 UpdateEmpty rules) into
/// `.result`. Walks statements backwards; `is_set` = every path from here on stores a value
/// first, so the statement need not store; `breakable` = inside a loop, switch or labeled
/// statement, where a `break` or `continue` can skip the later stores.
fn rewrite_completion<'a>(ast: &AstBuilder<'a>, program: &mut Program<'a>) {
    let mut r = Rewriter {
        ast,
        is_set: false,
        breakable: false,
        backups: 0,
    };
    r.process(&mut program.body);
    // Directives are ExpressionStatements too (`eval("'a'")` is "a"); they run first, so the
    // last one is the value only when no statement stores one.
    if !r.is_set
        && let Some(last) = program.directives.last()
    {
        let lit = last.expression.clone_in(ast.allocator());
        let value = Expression::StringLiteral(oxc_allocator::Box::new_in(lit, ast));
        let store = r.store(value, last.span);
        program.body.insert(0, store);
    }
}

struct Rewriter<'r, 'a> {
    ast: &'r AstBuilder<'a>,
    is_set: bool,
    breakable: bool,
    /// Backup bindings made so far (`.result_backup0`, ...).
    backups: u32,
}

impl<'a> Rewriter<'_, 'a> {
    fn name(&self, text: &str) -> &'a str {
        self.ast.allocator().alloc_str(text)
    }

    /// `.result = value` (or `.x = value` for a backup) as an expression statement.
    fn assign(&self, target: &'a str, value: Expression<'a>, span: Span) -> Statement<'a> {
        let left = AssignmentTarget::new_assignment_target_identifier(
            Span::empty(span.start),
            target,
            self.ast,
        );
        let expr = Expression::new_assignment_expression(
            span,
            AssignmentOperator::Assign,
            left,
            value,
            self.ast,
        );
        Statement::new_expression_statement(span, expr, self.ast)
    }

    /// `.result = value`. A function or class value goes through a one-element sequence, so
    /// NamedEvaluation does not name it `.result`.
    fn store(&self, value: Expression<'a>, span: Span) -> Statement<'a> {
        let value = if is_function_like(&value) {
            let list = oxc_allocator::Vec::from_array_in([value], self.ast);
            Expression::new_sequence_expression(span, list, self.ast)
        } else {
            value
        };
        self.assign(self.name(EVAL_RESULT), value, span)
    }

    fn store_undefined(&self, span: Span) -> Statement<'a> {
        let at = Span::empty(span.start);
        self.store(Expression::new_void_0(at, self.ast), at)
    }

    /// `{ .result = void 0; s }` in place of `s` (a single-statement position).
    fn wrap_undefined_before(&self, s: &mut Statement<'a>) {
        let span = s.span();
        let inner = s.take_in(self.ast);
        let list = oxc_allocator::Vec::from_array_in([self.store_undefined(span), inner], self.ast);
        *s = Statement::new_block_statement(span, list, self.ast);
    }

    fn single(&mut self, s: &mut Statement<'a>) {
        if self.visit(s) {
            self.wrap_undefined_before(s);
        }
    }

    /// V8 `Process`: backwards, and only as far as a store can still matter.
    fn process(&mut self, list: &mut oxc_allocator::Vec<'a, Statement<'a>>) {
        let mut i = list.len();
        while i > 0 && (self.breakable || !self.is_set) {
            i -= 1;
            if self.visit(&mut list[i]) {
                let store = self.store_undefined(list[i].span());
                list.insert(i, store);
            }
        }
    }

    fn breakable(&mut self, f: impl FnOnce(&mut Self)) {
        let saved = self.breakable;
        self.breakable = true;
        f(self);
        self.breakable = saved;
    }

    /// Rewrites `s`. Returns true if `.result = undefined` must run just before it (V8
    /// `AssignUndefinedBefore`).
    fn visit(&mut self, s: &mut Statement<'a>) -> bool {
        match s {
            Statement::BlockStatement(b) => {
                self.process(&mut b.body);
                false
            }
            Statement::ExpressionStatement(e) => {
                if !self.is_set {
                    let span = e.expression.span();
                    let value = e.expression.take_in(self.ast);
                    let Statement::ExpressionStatement(store) = self.store(value, span) else {
                        unreachable!("store makes an expression statement")
                    };
                    e.expression = store.unbox().expression;
                    self.is_set = true;
                }
                false
            }
            Statement::IfStatement(i) => {
                let set_after = self.is_set;
                self.single(&mut i.consequent);
                let set_in_then = self.is_set;
                self.is_set = set_after;
                if let Some(alternate) = &mut i.alternate {
                    self.single(alternate);
                }
                let before = !(set_in_then && self.is_set);
                self.is_set = true;
                before
            }
            Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_) => {
                self.breakable(|r| r.single(loop_body(s)));
                self.is_set = true;
                true
            }
            Statement::TryStatement(t) => {
                let t = &mut **t;
                let Some(finalizer) = &mut t.finalizer else {
                    let handler = t.handler.as_mut().expect("a try has a catch or a finally");
                    return self.try_catch(&mut t.block.body, &mut handler.body.body);
                };
                if self.breakable {
                    // A break or continue in the finally block replaces the completion.
                    self.is_set = true;
                    self.process(&mut finalizer.body);
                    if !self.is_set {
                        let store = self.store_undefined(finalizer.span);
                        finalizer.body.insert(0, store);
                    }
                    // On a normal completion the finally block keeps the value it found.
                    let backup = self.name(&format!("{EVAL_RESULT}_backup{}", self.backups));
                    self.backups += 1;
                    let at = Span::empty(finalizer.span.start);
                    let end = Span::empty(finalizer.span.end);
                    let result = self.name(EVAL_RESULT);
                    let read = Expression::new_identifier(at, result, self.ast);
                    let save = self.assign(backup, read, at);
                    let read_backup = Expression::new_identifier(end, backup, self.ast);
                    let restore = self.assign(result, read_backup, end);
                    finalizer.body.insert(0, save);
                    finalizer.body.push(restore);
                    self.is_set = false;
                }
                let before = match &mut t.handler {
                    Some(handler) => self.try_catch(&mut t.block.body, &mut handler.body.body),
                    None => {
                        self.process(&mut t.block.body);
                        !self.is_set
                    }
                };
                self.is_set = true;
                before
            }
            Statement::SwitchStatement(sw) => {
                self.breakable(|r| {
                    for case in sw.cases.iter_mut().rev() {
                        r.process(&mut case.consequent);
                    }
                });
                self.is_set = true;
                true
            }
            Statement::BreakStatement(_) | Statement::ContinueStatement(_) => {
                self.is_set = false;
                false
            }
            Statement::WithStatement(w) => {
                self.single(&mut w.body);
                let before = !self.is_set;
                self.is_set = true;
                before
            }
            // A labeled statement is a break target. Its `.result = undefined` goes before the
            // label: a labeled loop must stay a loop (`continue l`).
            Statement::LabeledStatement(l) => {
                let mut before = false;
                self.breakable(|r| before = r.visit(&mut l.body));
                before
            }
            // `throw` never completes; `return` is an early error in eval code.
            Statement::ThrowStatement(_) | Statement::ReturnStatement(_) => {
                self.is_set = true;
                false
            }
            // Declarations, `;` and `debugger` produce no value.
            _ => false,
        }
    }

    /// V8 `VisitTryCatchStatement`.
    fn try_catch(
        &mut self,
        block: &mut oxc_allocator::Vec<'a, Statement<'a>>,
        catch: &mut oxc_allocator::Vec<'a, Statement<'a>>,
    ) -> bool {
        let set_after = self.is_set;
        self.process(block);
        let set_in_try = self.is_set;
        self.is_set = set_after;
        self.process(catch);
        if !self.is_set {
            let at = catch.first().map_or(SPAN, GetSpan::span);
            let store = self.store_undefined(at);
            catch.insert(0, store);
        }
        self.is_set = true;
        !set_in_try
    }
}

/// The body of a loop statement.
fn loop_body<'s, 'a>(s: &'s mut Statement<'a>) -> &'s mut Statement<'a> {
    match s {
        Statement::ForStatement(f) => &mut f.body,
        Statement::ForInStatement(f) => &mut f.body,
        Statement::ForOfStatement(f) => &mut f.body,
        Statement::WhileStatement(w) => &mut w.body,
        Statement::DoWhileStatement(d) => &mut d.body,
        _ => unreachable!("not a loop"),
    }
}

/// A function, arrow or class expression, maybe in parentheses: an assignment would name it.
fn is_function_like(e: &Expression<'_>) -> bool {
    match e {
        Expression::ParenthesizedExpression(p) => is_function_like(&p.expression),
        Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::ClassExpression(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A direct eval in a plain function of a sloppy script.
    fn direct() -> EvalContext {
        EvalContext {
            direct: true,
            in_function: true,
            new_target_ok: true,
            ..EvalContext::default()
        }
    }

    /// The front-end answer for `src` (the checks and the scope pass, no BytecodeGen: class
    /// fields and private names are P2-CLS-CG's codegen).
    fn outcome(src: &str, ctx: &EvalContext) -> String {
        let allow = Allow::from_context(ctx);
        let owned = src.to_owned();
        let r = crate::worker::run(crate::stack::WORKER_STACK_BYTES, move || {
            on_worker(&owned, &allow, |_, _| Ok(()))
        })
        .expect("a worker");
        match r {
            Ok(()) => "ok".to_owned(),
            Err(CodegenError::Compile(CompileError::Syntax(e))) => {
                format!("syntax: {}", e.first().message)
            }
            Err(CodegenError::Unsupported(u)) => format!("unsupported: {}", u.feature),
            Err(e) => format!("other: {e}"),
        }
    }

    fn disasm(src: &str, ctx: &EvalContext) -> String {
        let unit = compile_eval(src, ctx, CompileOptions::default()).expect("compiles");
        ep9_bytecode::disassemble(unit.unit())
    }

    /// The 6 rows of docs/upstream/oxc-eval-parse-options.md, each in a context that allows
    /// the construct and in one that does not (the spec answer; node v24 agrees).
    #[test]
    fn the_six_oxc_rows() {
        let none = EvalContext {
            direct: true,
            ..EvalContext::default()
        };
        let method = EvalContext {
            in_method: true,
            super_property_ok: true,
            ..direct()
        };
        let derived = EvalContext {
            in_derived_ctor: true,
            super_property_ok: true,
            super_call_ok: true,
            ..direct()
        };
        let private = EvalContext {
            private_names: vec!["x".to_owned()],
            ..direct()
        };
        let field = EvalContext {
            in_class_field_init: true,
            ..direct()
        };
        let strict = EvalContext {
            strict: true,
            ..direct()
        };
        let rows: [(&str, &EvalContext, &str); 12] = [
            ("new.target", &direct(), "ok"),
            (
                "new.target",
                &none,
                "syntax: Unexpected new.target expression",
            ),
            ("super.x", &method, "ok"),
            (
                "super.x",
                &direct(),
                "syntax: 'super' can only be referenced in members of derived classes or object \
                 literal expressions.",
            ),
            ("super()", &derived, "unsupported: super() in eval code"),
            (
                "super()",
                &method,
                "syntax: Super calls are not permitted outside constructors or in nested \
                 functions inside constructors.",
            ),
            (
                "this.#x",
                &private,
                "unsupported: a private name of the caller's class in eval code",
            ),
            (
                "this.#x",
                &direct(),
                "syntax: Private identifier '#x' is not allowed outside class bodies",
            ),
            (
                "arguments",
                &field,
                "syntax: 'arguments' is not allowed in class field initializer or static \
                 initialization block",
            ),
            ("arguments", &direct(), "ok"),
            (
                "with (o) {}",
                &strict,
                "syntax: 'with' statements are not allowed",
            ),
            ("with (o) {}", &direct(), "ok"),
        ];
        for (src, ctx, want) in rows {
            assert_eq!(outcome(src, ctx), want, "{src} in {ctx:?}");
        }
    }

    #[test]
    fn script_only_errors_stay_errors() {
        let indirect = EvalContext::default();
        assert_eq!(
            outcome("return 1", &direct()),
            "syntax: A 'return' statement can only be used within a function body."
        );
        assert_eq!(
            outcome("new.target", &indirect),
            "syntax: Unexpected new.target expression"
        );
        // Indirect eval ignores the caller flags (E4).
        let flags = EvalContext {
            direct: false,
            new_target_ok: true,
            super_property_ok: true,
            private_names: vec!["x".to_owned()],
            ..EvalContext::default()
        };
        assert!(outcome("new.target", &flags).starts_with("syntax: "));
        assert!(outcome("super.x", &flags).starts_with("syntax: "));
        assert!(outcome("this.#x", &flags).starts_with("syntax: "));
        // An allowed node does not hide another error.
        assert_eq!(
            outcome("new.target; return", &direct()),
            "syntax: A 'return' statement can only be used within a function body."
        );
        // `yield` and `await` are identifiers in a sloppy Script.
        assert_eq!(
            outcome("var yield = 1, await = 2; yield + await", &direct()),
            "ok"
        );
    }

    /// E3: only the eval code's own function level (and arrows in it) sees the caller's
    /// `new.target` and `super`.
    #[test]
    fn nested_functions_have_their_own_new_target_and_super() {
        let method = EvalContext {
            super_property_ok: true,
            ..direct()
        };
        assert_eq!(outcome("() => super.x", &method), "ok");
        assert_eq!(outcome("(() => new.target)()", &direct()), "ok");
        assert!(outcome("function f() { super.x }", &method).starts_with("syntax: "));
        assert_eq!(
            outcome("function f() { new.target }", &EvalContext::default()),
            "ok"
        );
        assert_eq!(
            outcome("({ m() { super.x } })", &EvalContext::default()),
            "ok"
        );
        assert_eq!(
            outcome("class C { x = new.target }", &EvalContext::default()),
            "ok"
        );
        let field = EvalContext {
            in_class_field_init: true,
            ..direct()
        };
        // ContainsArguments does not look into functions, but does look into arrows.
        assert_eq!(outcome("function f() { arguments }", &field), "ok");
        assert!(outcome("() => arguments", &field).starts_with("syntax: "));
        assert!(outcome("({ [arguments]() {} })", &field).starts_with("syntax: "));
    }

    #[test]
    fn private_names_follow_the_caller_and_the_eval_classes() {
        let names = EvalContext {
            private_names: vec!["x".to_owned(), ".brand".to_owned()],
            ..direct()
        };
        // E9: a caller name passes the checks and then is Unsupported.
        let caller = "unsupported: a private name of the caller's class in eval code";
        assert_eq!(outcome("#x in this", &names), caller);
        assert_eq!(outcome("this?.#x", &names), caller);
        assert_eq!(
            outcome("class C { #y; m() { return this.#y + this.#x } }", &names),
            caller
        );
        assert_eq!(
            outcome("class C { #y; m() { return this.#y } }", &names),
            "ok"
        );
        // A syntax error wins over E9.
        assert_eq!(
            outcome("this.#x; class C { m() { this.#y } }", &names),
            "syntax: Private field '#y' must be declared in an enclosing class"
        );
        assert_eq!(
            outcome("class C { m() { this.#y } }", &names),
            "syntax: Private field '#y' must be declared in an enclosing class"
        );
        // The heritage sees only the outer names.
        assert_eq!(
            outcome("class C extends (this.#x, Object) { #x }", &names),
            caller
        );
        assert_eq!(
            outcome("class C extends Object { #x; m() { this.#x } }", &names),
            "ok"
        );
        assert!(
            outcome("class C extends (this.#z, Object) { #z }", &names).starts_with("syntax: ")
        );
    }

    #[test]
    fn eval_units_have_the_eval_code_kind() {
        let text = disasm("1", &EvalContext::default());
        assert!(text.contains("unit kind=eval strict=false"), "{text}");
        assert!(text.contains("function f0 eval "), "{text}");
        let strict = EvalContext {
            strict: true,
            ..direct()
        };
        let text = disasm("1", &strict);
        assert!(text.contains("unit kind=eval strict=true"), "{text}");
        let text = disasm("'use strict'; var a", &EvalContext::default());
        assert!(text.contains("unit kind=eval strict=true"), "{text}");
    }

    /// Free names: Dynamic in direct eval (C4), Global in indirect eval. Sloppy declarations
    /// go through DeclareEvalVar / DeclareEvalFunction; strict ones are local.
    #[test]
    fn free_names_and_declarations() {
        let has = |text: &str, op: &str, name: &str| {
            text.lines()
                .any(|l| l.contains(op) && l.contains(&format!("\"{name}\"")))
        };
        let text = disasm("var a = 1; function f() { return a + b; } f()", &direct());
        assert!(text.contains("CallRuntime DeclareEvalFunction"), "{text}");
        for (op, name) in [
            ("StaLookupSlot", "a"),
            ("LdaLookupSlot", "a"),
            ("LdaLookupSlot", "b"),
            ("LdaConstant", "f"),
        ] {
            assert!(has(&text, op, name), "missing {op} {name} in\n{text}");
        }
        // The callee `f` of `f()` is a dynamic callee: its value and receiver come from one
        // CallRuntimeForPair LoadLookupSlotForCall (bytecodegen/lookup.rs, P2-LOOKUP).
        assert!(
            text.contains("CallRuntimeForPair LoadLookupSlotForCall"),
            "{text}"
        );
        assert!(text.contains("CallRuntime DeclareEvalVar"), "{text}");
        assert!(
            text.find("DeclareEvalFunction") < text.find("DeclareEvalVar"),
            "functions first:\n{text}"
        );
        let text = disasm("var a = 1; a + b", &EvalContext::default());
        assert!(text.contains("CallRuntime DeclareEvalVar"), "{text}");
        assert!(text.contains("StaGlobal"), "{text}");
        assert!(text.contains("LdaGlobal"), "{text}");
        assert!(!text.contains("LookupSlot"), "{text}");
        let strict = EvalContext {
            strict: true,
            ..direct()
        };
        let text = disasm("var a = 1; function f() { return a; } a + b", &strict);
        assert!(!text.contains("DeclareEval"), "{text}");
        assert!(text.contains("CreateEvalContext"), "{text}");
        assert!(text.contains("LdaLookupSlot k"), "b is dynamic:\n{text}");
        // A lexical declaration is local in sloppy eval code too.
        let text = disasm("let c = 1; c", &direct());
        assert!(
            !text.contains("DeclareEval") && !text.contains("LookupSlot"),
            "{text}"
        );
    }

    /// The caller's `this`, `new.target`, home object and `arguments` are found by name in
    /// direct eval code (also from an arrow); indirect eval code uses its receiver.
    #[test]
    fn this_and_friends() {
        let method = EvalContext {
            super_property_ok: true,
            ..direct()
        };
        let text = disasm("() => [this, new.target, super.x, arguments]", &method);
        for name in [
            "\"this\"",
            "\"new.target\"",
            "\".home_object\"",
            "\"arguments\"",
        ] {
            assert!(
                text.lines()
                    .any(|l| l.contains("LdaLookupSlot") && l.contains(name)),
                "{name} in\n{text}"
            );
        }
        assert!(!text.contains("<this>"), "{text}");
        let text = disasm("x => this", &EvalContext::default());
        assert!(text.contains("Ldar <this>"), "{text}");
        assert!(text.contains("CreateEvalContext"), "{text}");
    }

    #[test]
    fn annex_b_functions_in_sloppy_eval_are_unsupported() {
        assert_eq!(
            outcome("{ function f() {} }", &direct()),
            "unsupported: Annex B function declarations in sloppy eval code"
        );
        assert_eq!(
            outcome("if (1) function f() {}", &EvalContext::default()),
            "unsupported: Annex B function declarations in sloppy eval code"
        );
        let strict = EvalContext {
            strict: true,
            ..direct()
        };
        assert_eq!(outcome("{ function f() {} }", &strict), "ok");
        // Inside a function of the eval code, Annex B is the function's own business.
        assert_eq!(
            outcome("function g() { { function f() {} } }", &direct()),
            "ok"
        );
    }

    #[test]
    fn two_compiles_give_the_same_bytes() {
        let src = "var a = 1; function f() { return a + b; } let c = this; try { f() } finally {}";
        let ctx = direct();
        let a = compile_eval(src, &ctx, CompileOptions::default()).unwrap();
        let owned = (src.to_owned(), ctx.clone());
        let b = std::thread::spawn(move || {
            compile_eval(&owned.0, &owned.1, CompileOptions::default()).unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_eq!(a.content_hash(), b.content_hash());
    }

    // ---- The completion rewrite (E5) ----

    /// The rewritten program as text: `.result` stores in the statement structure, with the
    /// source text of the other expressions.
    fn rewritten(src: &str) -> String {
        let allocator = Allocator::default();
        let mut parsed = Parser::new(&allocator, src, SourceType::script()).parse();
        assert!(!parsed.diagnostics.has_errors(), "{src}");
        let ast = AstBuilder::new(&allocator);
        rewrite_completion(&ast, &mut parsed.program);
        let mut out = String::new();
        for s in &parsed.program.body {
            print_statement(src, s, &mut out);
        }
        out.trim_end().to_owned()
    }

    fn print_statement(src: &str, s: &Statement<'_>, out: &mut String) {
        let text = |span: Span| &src[span.start as usize..span.end as usize];
        match s {
            Statement::ExpressionStatement(e) => match &e.expression {
                Expression::AssignmentExpression(a)
                    if matches!(&a.left, AssignmentTarget::AssignmentTargetIdentifier(id)
                        if id.name.as_str().starts_with(EVAL_RESULT)) =>
                {
                    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                        unreachable!()
                    };
                    let value = match &a.right {
                        Expression::UnaryExpression(_) if a.right.span().is_empty() => {
                            "undefined".to_owned()
                        }
                        Expression::Identifier(r) => r.name.as_str().to_owned(),
                        Expression::SequenceExpression(q) => {
                            format!("({})", text(q.expressions[0].span()))
                        }
                        v => text(v.span()).to_owned(),
                    };
                    out.push_str(&format!("{} = {value}; ", id.name.as_str()));
                }
                other => out.push_str(&format!("{}; ", text(other.span()))),
            },
            Statement::BlockStatement(b) => {
                out.push_str("{ ");
                for s in &b.body {
                    print_statement(src, s, out);
                }
                out.push_str("} ");
            }
            Statement::IfStatement(i) => {
                out.push_str(&format!("if ({}) ", text(i.test.span())));
                print_statement(src, &i.consequent, out);
                if let Some(a) = &i.alternate {
                    out.push_str("else ");
                    print_statement(src, a, out);
                }
            }
            Statement::DoWhileStatement(d) => {
                out.push_str("do ");
                print_statement(src, &d.body, out);
                out.push_str(&format!("while ({}); ", text(d.test.span())));
            }
            Statement::WhileStatement(w) => {
                out.push_str(&format!("while ({}) ", text(w.test.span())));
                print_statement(src, &w.body, out);
            }
            Statement::TryStatement(t) => {
                out.push_str("try { ");
                for s in &t.block.body {
                    print_statement(src, s, out);
                }
                out.push_str("} ");
                if let Some(h) = &t.handler {
                    out.push_str("catch { ");
                    for s in &h.body.body {
                        print_statement(src, s, out);
                    }
                    out.push_str("} ");
                }
                if let Some(f) = &t.finalizer {
                    out.push_str("finally { ");
                    for s in &f.body {
                        print_statement(src, s, out);
                    }
                    out.push_str("} ");
                }
            }
            Statement::LabeledStatement(l) => {
                out.push_str(&format!("{}: ", l.label.name.as_str()));
                print_statement(src, &l.body, out);
            }
            other => {
                let t = text(other.span());
                let semi = if t.ends_with(';') { "" } else { ";" };
                out.push_str(&format!("{t}{semi} "));
            }
        }
    }

    #[test]
    fn completion_rewrite() {
        let cases = [
            ("1; 2", "1; .result = 2;"),
            ("1; var x;", ".result = 1; var x;"),
            ("'a'; 'b'", ".result = 'b';"),
            ("'a'; var x", ".result = 'a'; var x;"),
            ("(function () {})", ".result = ((function () {}));"),
            (
                "1; if (a) 2;",
                "1; .result = undefined; if (a) .result = 2;",
            ),
            (
                "1; if (a) 2; else 3;",
                "1; if (a) .result = 2; else .result = 3;",
            ),
            (
                "1; do { 2; break; 3; } while (0)",
                "1; .result = undefined; do { .result = 2; break; .result = 3; } while (0);",
            ),
            ("1; l: { 2; break l; }", "1; l: { .result = 2; break l; }"),
            (
                "1; l: while (a) { 2; continue l; }",
                "1; .result = undefined; l: while (a) { .result = 2; continue l; }",
            ),
            (
                "1; try { 2 } catch { 3 }",
                "1; try { .result = 2; } catch { .result = 3; }",
            ),
            (
                "1; try { } catch { }",
                "1; .result = undefined; try { } catch { .result = undefined; }",
            ),
            (
                "1; try { 2 } finally { 3 }",
                "1; try { .result = 2; } finally { 3; }",
            ),
            (
                "do { try { 1 } finally { 2; break } } while (0)",
                ".result = undefined; do { try { .result = 1; } finally { \
                 .result_backup0 = .result; .result = 2; break; .result = .result_backup0; } } \
                 while (0);",
            ),
        ];
        for (src, want) in cases {
            assert_eq!(rewritten(src), want, "{src}");
        }
    }
}
