//! Scope pass step 1: one walk over the AST collects scopes, bindings, references and direct
//! eval sites. Every recursion cycle passes through `visit_expression`, `visit_statement`,
//! `visit_assignment_target` or `decl_pattern`, and each of them checks the depth (layer 3).

use std::collections::HashMap;

use oxc_ast::ast::BlockStatement;
use oxc_ast::ast::{
    ArrowFunctionExpression, AssignmentTarget, BindingIdentifier, BindingPattern, CallExpression,
    CatchClause, Class, ClassElement, ClassType, ExportAllDeclaration, ExportDeclaration,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExportFromDeclaration,
    ExportNamedDeclaration, Expression, ForInStatement, ForOfStatement, ForStatement,
    ForStatementInit, ForStatementLeft, FormalParameters, Function, FunctionType,
    IdentifierReference, IfStatement, ImportDeclaration, ImportDeclarationSpecifier,
    MethodDefinitionKind, ModuleExportName, NewTarget, ObjectProperty, PrivateFieldExpression,
    PrivateInExpression, Program, PropertyKey, PropertyKind, Statement, Super, SwitchStatement,
    ThisExpression, VariableDeclaration, VariableDeclarationKind, WithStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::{ScopeFlags, Scoping};
use oxc_span::{GetSpan, Span};

use super::{
    Binding, BindingId, BindingKind, BindingLocation, ClassInfo, FunctionId, FunctionInfo,
    FunctionKind, ImplicitRef, NONE, NameId, Param, PerIteration, PrivateKind, RefId, RefKind,
    Reference, Resolution, ScopeAnalysis, ScopeId, ScopeInfo, ScopeKind,
};
use crate::{ByteSpan, FrontEnd, Goal};

/// A direct `eval(...)` call.
#[derive(Clone, Copy)]
pub(super) struct EvalSite {
    pub(super) scope: ScopeId,
    pub(super) function: FunctionId,
    /// The scope that receives `var` declarations of sloppy eval code.
    pub(super) var_scope: ScopeId,
    /// The eval code is strict (the caller is strict).
    pub(super) strict: bool,
}

/// Build-time data per scope.
#[derive(Default)]
pub(super) struct ScopeExtra {
    /// Name lookup. Never iterated, so the hash order cannot reach the output (H2).
    pub(super) names: HashMap<NameId, BindingId>,
    /// A catch scope with a destructuring parameter (Annex B.3.4 does not apply).
    pub(super) catch_pattern: bool,
}

/// Build-time data per function.
#[derive(Default)]
pub(super) struct FunctionExtra {
    /// Sloppy plain function declarations in blocks: Annex B.3.2 candidates.
    pub(super) block_functions: Vec<BindingId>,
    /// Parameter bindings and the end of their `FormalParameter` (TDZ for non-simple lists).
    pub(super) param_ends: Vec<(BindingId, u32)>,
    /// ECMA-262 FunctionDeclarationInstantiation `argumentsObjectNeeded`, before use.
    pub(super) arguments_needed: bool,
    /// A `var arguments` in the function scope: it holds the arguments object.
    pub(super) arguments_var: Option<BindingId>,
}

#[derive(Clone, Copy)]
enum Decl {
    /// `var`: hoisted to the var scope. `assign`: the declarator writes a value.
    Var { assign: bool },
    /// `let`, `const`, `using`: in the current scope, initialized at `init_pos`.
    Lexical { kind: BindingKind, init_pos: u32 },
    /// A formal parameter that ends at `end`.
    Param { end: u32 },
    /// A catch parameter.
    Catch,
}

/// Walk state saved around a function.
struct Saved {
    scope: ScopeId,
    function: FunctionId,
    var_scope: ScopeId,
    strict: bool,
}

pub(super) struct Builder<'a> {
    scoping: &'a Scoping,
    module: bool,
    pub(super) names: Vec<Box<str>>,
    name_ids: HashMap<&'a str, NameId>,
    private_ids: HashMap<&'a str, NameId>,
    pub(super) functions: Vec<FunctionInfo>,
    pub(super) fn_extra: Vec<FunctionExtra>,
    pub(super) scopes: Vec<ScopeInfo>,
    pub(super) scope_extra: Vec<ScopeExtra>,
    pub(super) bindings: Vec<Binding>,
    /// Number of declarations per binding (sloppy duplicates share one binding).
    pub(super) decls: Vec<u32>,
    pub(super) references: Vec<Reference>,
    pub(super) eval_sites: Vec<EvalSite>,
    by_oxc_reference: Vec<u32>,
    by_oxc_scope: Vec<u32>,
    declared_at: Vec<(u32, BindingId)>,
    implicit: Vec<(u32, ImplicitRef, RefId)>,
    function_at: Vec<(u32, FunctionId)>,
    synthetic_blocks: Vec<(u32, ScopeId)>,
    pub(super) n_this: NameId,
    pub(super) n_new_target: NameId,
    pub(super) n_home_object: NameId,
    pub(super) n_callee: NameId,
    pub(super) n_arguments: NameId,
    pub(super) n_eval_ext: NameId,
    n_default: NameId,
    cur_scope: ScopeId,
    cur_fn: FunctionId,
    var_scope: ScopeId,
    strict: bool,
    exporting: bool,
    pattern_expressions: bool,
    depth: u32,
    cap: u32,
    /// Start of the first node deeper than the cap.
    pub(super) too_deep: Option<u32>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(fe: &FrontEnd<'a>, cap: u32) -> Self {
        let scoping = fe.semantic.scoping();
        let mut builder = Self {
            scoping,
            module: fe.opts.goal() == Goal::Module,
            names: Vec::new(),
            name_ids: HashMap::new(),
            private_ids: HashMap::new(),
            functions: Vec::new(),
            fn_extra: Vec::new(),
            scopes: Vec::new(),
            scope_extra: Vec::new(),
            bindings: Vec::new(),
            decls: Vec::new(),
            references: Vec::new(),
            eval_sites: Vec::new(),
            by_oxc_reference: vec![NONE; scoping.references_len()],
            by_oxc_scope: vec![NONE; scoping.scopes_len()],
            declared_at: Vec::new(),
            implicit: Vec::new(),
            function_at: Vec::new(),
            synthetic_blocks: Vec::new(),
            n_this: NameId(0),
            n_new_target: NameId(0),
            n_home_object: NameId(0),
            n_callee: NameId(0),
            n_arguments: NameId(0),
            n_eval_ext: NameId(0),
            n_default: NameId(0),
            cur_scope: ScopeId(0),
            cur_fn: FunctionId(0),
            var_scope: ScopeId(0),
            strict: false,
            exporting: false,
            pattern_expressions: false,
            depth: 0,
            cap,
            too_deep: None,
        };
        // Fixed names first, so their ids do not depend on the source.
        builder.n_this = builder.name("this");
        builder.n_new_target = builder.name("new.target");
        builder.n_home_object = builder.name(".home_object");
        builder.n_callee = builder.name(".callee");
        builder.n_arguments = builder.name("arguments");
        builder.n_default = builder.name("*default*");
        builder.n_eval_ext = builder.name(ep9_bytecode::EVAL_EXT_BINDING);
        builder
    }

    pub(super) fn collect(&mut self, program: &Program<'a>) {
        self.visit_program(program);
    }

    pub(super) fn finish(mut self) -> ScopeAnalysis {
        self.declared_at.sort_by_key(|&(start, _)| start);
        self.declared_at.dedup_by_key(|&mut (start, _)| start);
        self.implicit.sort_by_key(|&(start, kind, _)| (start, kind));
        self.function_at.sort_by_key(|&(start, _)| start);
        self.synthetic_blocks.sort_by_key(|&(start, _)| start);
        ScopeAnalysis {
            names: self.names,
            functions: self.functions,
            scopes: self.scopes,
            bindings: self.bindings,
            references: self.references,
            by_oxc_reference: self.by_oxc_reference,
            by_oxc_scope: self.by_oxc_scope,
            declared_at: self.declared_at,
            implicit: self.implicit,
            function_at: self.function_at,
            synthetic_blocks: self.synthetic_blocks,
        }
    }

    pub(super) fn name(&mut self, text: &'a str) -> NameId {
        if let Some(&id) = self.name_ids.get(text) {
            return id;
        }
        let id = NameId::of(self.names.len());
        self.names.push(text.into());
        self.name_ids.insert(text, id);
        id
    }

    fn private_name(&mut self, text: &'a str) -> NameId {
        if let Some(&id) = self.private_ids.get(text) {
            return id;
        }
        let id = NameId::of(self.names.len());
        self.names.push(format!("#{text}").into());
        self.private_ids.insert(text, id);
        id
    }

    /// Depth check at a cycle point. Always pair with [`Self::leave`].
    #[inline]
    fn enter(&mut self, span: Span) -> bool {
        self.depth += 1;
        if self.too_deep.is_some() {
            return false;
        }
        if self.depth > self.cap {
            self.too_deep = Some(span.start);
            return false;
        }
        true
    }

    #[inline]
    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn new_scope(
        &mut self,
        kind: ScopeKind,
        parent: Option<ScopeId>,
        function: FunctionId,
        span: Span,
        strict: bool,
        oxc: Option<oxc_semantic::ScopeId>,
    ) -> ScopeId {
        let id = ScopeId::of(self.scopes.len());
        self.scopes.push(ScopeInfo {
            id,
            kind,
            parent,
            function,
            span: ByteSpan::of(span),
            strict,
            bindings: Vec::new(),
            context_slots: Vec::new(),
            needs_context: false,
            sloppy_eval: false,
            contains_eval: false,
            per_iteration: PerIteration::No,
            hoisted_functions: Vec::new(),
            class: None,
        });
        self.scope_extra.push(ScopeExtra::default());
        if let Some(slot) = oxc.and_then(|o| self.by_oxc_scope.get_mut(o.index())) {
            *slot = id.0;
        }
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn new_function(
        &mut self,
        id: FunctionId,
        kind: FunctionKind,
        span: Span,
        strict: bool,
        outer_scope: Option<ScopeId>,
        scope: ScopeId,
        name_scope: Option<(ScopeId, BindingId)>,
    ) {
        assert_eq!(
            id.index(),
            self.functions.len(),
            "function ids are preorder"
        );
        self.functions.push(FunctionInfo {
            id,
            kind,
            is_async: false,
            is_generator: false,
            is_static: false,
            parent: (id.0 > 0).then_some(self.cur_fn),
            span: ByteSpan::of(span),
            strict,
            outer_scope,
            scope,
            body_scope: scope,
            name_scope: name_scope.map(|(s, _)| s),
            params: Vec::new(),
            simple_params: true,
            has_parameter_expressions: false,
            this: None,
            new_target: None,
            arguments: None,
            mapped_arguments: false,
            home_object: None,
            callee: None,
            self_name: name_scope.map(|(_, b)| b),
            calls_eval: false,
            calls_sloppy_eval: false,
            register_count: 0,
        });
        self.fn_extra.push(FunctionExtra::default());
    }

    /// Declare `name` in `scope`. Var-like redeclarations share one binding.
    pub(super) fn declare(
        &mut self,
        scope: ScopeId,
        name: NameId,
        kind: BindingKind,
        span: Option<Span>,
    ) -> BindingId {
        self.declare_full(scope, name, kind, span, true)
    }

    pub(super) fn declare_full(
        &mut self,
        scope: ScopeId,
        name: NameId,
        kind: BindingKind,
        span: Option<Span>,
        record_site: bool,
    ) -> BindingId {
        use BindingKind as K;
        if let Some(&existing) = self.scope_extra[scope.index()].names.get(&name) {
            let old = self.bindings[existing.index()].kind;
            let merged = match (old, kind) {
                (K::Var | K::Function | K::Parameter | K::Arguments, K::Var)
                | (K::Function | K::Parameter, K::Function)
                | (K::BlockFunction, K::BlockFunction)
                | (K::Parameter, K::Parameter) => Some(old),
                (K::Var, K::Function) => Some(K::Function),
                (K::PrivateName { kind: a, is_static }, K::PrivateName { kind: b, .. }) => {
                    Some(K::PrivateName {
                        kind: match (a, b) {
                            (PrivateKind::Getter, PrivateKind::Setter)
                            | (PrivateKind::Setter, PrivateKind::Getter) => {
                                PrivateKind::GetterSetter
                            }
                            _ => a,
                        },
                        is_static,
                    })
                }
                // Other redeclarations are early errors; oxc reports them before this pass.
                _ => None,
            };
            if let Some(merged) = merged {
                self.bindings[existing.index()].kind = merged;
                self.decls[existing.index()] += 1;
                if let (Some(span), true) = (span, record_site) {
                    self.declared_at.push((span.start, existing));
                }
                return existing;
            }
        }
        let id = BindingId::of(self.bindings.len());
        self.bindings.push(Binding {
            name,
            kind,
            scope,
            span: span.map(ByteSpan::of),
            location: BindingLocation::Register(0),
            context_reason: None,
            tdz: kind.has_tdz(),
            init_pos: 0,
            captured: false,
            referenced: false,
            exported: false,
            annex_b_var: None,
            init_from: None,
        });
        self.decls.push(1);
        self.scopes[scope.index()].bindings.push(id);
        self.scope_extra[scope.index()]
            .names
            .entry(name)
            .or_insert(id);
        if let (Some(span), true) = (span, record_site) {
            self.declared_at.push((span.start, id));
        }
        id
    }

    fn mark_exported(&mut self, binding: BindingId, scope: ScopeId) {
        if self.exporting && self.module && scope.0 == 0 {
            self.bindings[binding.index()].exported = true;
        }
    }

    fn push_reference(&mut self, name: NameId, kind: RefKind, span: Span) -> RefId {
        let id = RefId::of(self.references.len());
        self.references.push(Reference {
            name,
            kind,
            scope: self.cur_scope,
            function: self.cur_fn,
            span: ByteSpan::of(span),
            resolution: Resolution::Global,
        });
        id
    }

    fn implicit_ref(&mut self, span: Span, key: ImplicitRef, name: NameId, kind: RefKind) {
        let r = self.push_reference(name, kind, span);
        self.implicit.push((span.start, key, r));
    }

    fn identifier(&mut self, it: &IdentifierReference<'a>, export: bool) {
        let name = self.name(it.name.as_str());
        let oxc = it.reference_id.get();
        let kind = if export {
            RefKind::Export
        } else {
            let flags = oxc.map(|id| self.scoping.get_reference(id).flags());
            RefKind::Identifier {
                read: flags.is_none_or(|f| f.is_read()),
                write: flags.is_some_and(|f| f.is_write()),
            }
        };
        let r = self.push_reference(name, kind, it.span);
        if let Some(slot) = oxc.and_then(|id| self.by_oxc_reference.get_mut(id.index())) {
            *slot = r.0;
        }
    }

    fn decl_ident(&mut self, id: &BindingIdentifier<'a>, decl: Decl) -> BindingId {
        let name = self.name(id.name.as_str());
        match decl {
            Decl::Var { assign } => {
                let scope = self.var_scope;
                let b = self.declare(scope, name, BindingKind::Var, Some(id.span));
                self.mark_exported(b, scope);
                if assign {
                    self.implicit_ref(id.span, ImplicitRef::VarInit, name, RefKind::VarInit);
                }
                b
            }
            Decl::Lexical { kind, init_pos } => {
                let scope = self.cur_scope;
                let b = self.declare(scope, name, kind, Some(id.span));
                self.bindings[b.index()].init_pos = init_pos;
                self.mark_exported(b, scope);
                b
            }
            Decl::Param { end } => {
                let b = self.declare(self.cur_scope, name, BindingKind::Parameter, Some(id.span));
                self.fn_extra[self.cur_fn.index()].param_ends.push((b, end));
                b
            }
            Decl::Catch => self.declare(
                self.cur_scope,
                name,
                BindingKind::CatchParameter,
                Some(id.span),
            ),
        }
    }

    /// Declare every name in a binding pattern and visit its expressions (defaults, computed
    /// keys) in the current scope.
    fn decl_pattern(&mut self, pattern: &BindingPattern<'a>, decl: Decl) {
        if self.enter(pattern.span()) {
            match pattern {
                BindingPattern::BindingIdentifier(id) => {
                    self.decl_ident(id, decl);
                }
                BindingPattern::ObjectPattern(object) => {
                    for property in &object.properties {
                        if property.computed {
                            self.pattern_expressions = true;
                            self.visit_property_key(&property.key);
                        }
                        self.decl_pattern(&property.value, decl);
                    }
                    if let Some(rest) = &object.rest {
                        self.decl_pattern(&rest.argument, decl);
                    }
                }
                BindingPattern::ArrayPattern(array) => {
                    for element in array.elements.iter().flatten() {
                        self.decl_pattern(element, decl);
                    }
                    if let Some(rest) = &array.rest {
                        self.decl_pattern(&rest.argument, decl);
                    }
                }
                BindingPattern::AssignmentPattern(assign) => {
                    self.decl_pattern(&assign.left, decl);
                    self.pattern_expressions = true;
                    self.visit_expression(&assign.right);
                }
            }
        }
        self.leave();
    }

    fn enter_function(&mut self, f: FunctionId, scope: ScopeId, strict: bool) -> Saved {
        let saved = Saved {
            scope: self.cur_scope,
            function: self.cur_fn,
            var_scope: self.var_scope,
            strict: self.strict,
        };
        self.cur_fn = f;
        self.cur_scope = scope;
        self.var_scope = scope;
        self.strict = strict;
        saved
    }

    fn restore(&mut self, saved: Saved) {
        self.cur_scope = saved.scope;
        self.cur_fn = saved.function;
        self.var_scope = saved.var_scope;
        self.strict = saved.strict;
    }

    /// Parameters, then the split body scope if a parameter has an expression.
    fn params(&mut self, params: &FormalParameters<'a>, body_span: Span) {
        let f = self.cur_fn;
        let outer_flag = std::mem::replace(&mut self.pattern_expressions, false);
        let mut simple = params.rest.is_none();
        let mut list = Vec::with_capacity(params.items.len() + 1);
        for item in &params.items {
            let end = item.span.end;
            let binding = match &item.pattern {
                BindingPattern::BindingIdentifier(id) => {
                    Some(self.decl_ident(id, Decl::Param { end }))
                }
                pattern => {
                    simple = false;
                    self.decl_pattern(pattern, Decl::Param { end });
                    None
                }
            };
            if let Some(init) = &item.initializer {
                simple = false;
                self.pattern_expressions = true;
                self.visit_expression(init);
            }
            list.push(Param {
                span: ByteSpan::of(item.span),
                binding,
                rest: false,
            });
        }
        if let Some(rest) = &params.rest {
            let end = rest.span.end;
            let binding = match &rest.rest.argument {
                BindingPattern::BindingIdentifier(id) => {
                    Some(self.decl_ident(id, Decl::Param { end }))
                }
                pattern => {
                    self.decl_pattern(pattern, Decl::Param { end });
                    None
                }
            };
            list.push(Param {
                span: ByteSpan::of(rest.span),
                binding,
                rest: true,
            });
        }
        // Sloppy duplicates: only the last parameter of a name binds it.
        for i in 0..list.len() {
            if let Some(b) = list[i].binding
                && list[i + 1..].iter().any(|p| p.binding == Some(b))
            {
                list[i].binding = None;
            }
        }
        let has_expressions = std::mem::replace(&mut self.pattern_expressions, outer_flag);
        let info = &mut self.functions[f.index()];
        info.params = list;
        info.simple_params = simple;
        info.has_parameter_expressions = has_expressions;
        if has_expressions {
            let (scope, strict) = (info.scope, info.strict);
            let body = self.new_scope(
                ScopeKind::FunctionBody,
                Some(scope),
                f,
                body_span,
                strict,
                None,
            );
            self.functions[f.index()].body_scope = body;
            self.cur_scope = body;
            self.var_scope = body;
        }
    }

    fn function(&mut self, it: &Function<'a>, kind: FunctionKind, is_static: bool) {
        let fid = FunctionId::of(self.functions.len());
        let outer = self.cur_scope;
        if it.r#type == FunctionType::FunctionDeclaration {
            let name = match &it.id {
                Some(id) => self.name(id.name.as_str()),
                None => self.n_default,
            };
            let at_top = self.cur_scope == self.var_scope;
            let (scope, kind) = if at_top {
                (self.var_scope, BindingKind::Function)
            } else {
                (self.cur_scope, BindingKind::BlockFunction)
            };
            let b = self.declare(scope, name, kind, it.id.as_ref().map(|id| id.span));
            self.mark_exported(b, scope);
            self.scopes[scope.index()].hoisted_functions.push((b, fid));
            if kind == BindingKind::BlockFunction
                && !self.strict
                && !it.r#async
                && !it.generator
                && self.decls[b.index()] == 1
            {
                self.fn_extra[self.cur_fn.index()].block_functions.push(b);
            }
        }
        let strict = self.strict
            || it
                .body
                .as_ref()
                .is_some_and(|b| b.has_use_strict_directive());
        let name_scope = match (&it.id, it.r#type) {
            (Some(id), FunctionType::FunctionExpression) => {
                let s = self.new_scope(
                    ScopeKind::FunctionName,
                    Some(outer),
                    fid,
                    it.span,
                    strict,
                    None,
                );
                let name = self.name(id.name.as_str());
                let b = self.declare(s, name, BindingKind::FunctionName, Some(id.span));
                Some((s, b))
            }
            _ => None,
        };
        let parent = name_scope.map_or(outer, |(s, _)| s);
        let scope = self.new_scope(
            ScopeKind::Function,
            Some(parent),
            fid,
            it.span,
            strict,
            it.scope_id.get(),
        );
        self.new_function(fid, kind, it.span, strict, Some(outer), scope, name_scope);
        let info = &mut self.functions[fid.index()];
        info.is_async = it.r#async;
        info.is_generator = it.generator;
        info.is_static = is_static;
        self.function_at.push((it.span.start, fid));

        let saved = self.enter_function(fid, scope, strict);
        let body_span = it.body.as_ref().map_or(it.span, |b| b.span);
        self.params(&it.params, body_span);
        if let Some(body) = &it.body {
            self.visit_statements(&body.statements);
        }
        self.finish_function(fid);
        self.restore(saved);
    }

    /// A synthetic class function (default constructor, member initializers).
    fn synthetic(
        &mut self,
        kind: FunctionKind,
        class_scope: ScopeId,
        span: Span,
        is_static: bool,
    ) -> FunctionId {
        let fid = FunctionId::of(self.functions.len());
        let scope = self.new_scope(
            ScopeKind::Function,
            Some(class_scope),
            fid,
            span,
            true,
            None,
        );
        self.new_function(fid, kind, span, true, Some(class_scope), scope, None);
        self.functions[fid.index()].is_static = is_static;
        fid
    }

    /// Run `visit` with a synthetic function as the current function.
    fn in_synthetic(&mut self, f: Option<FunctionId>, visit: impl FnOnce(&mut Self)) {
        let f = f.expect("the class needs this synthetic function");
        let scope = self.functions[f.index()].scope;
        let saved = self.enter_function(f, scope, true);
        visit(self);
        self.restore(saved);
    }

    fn for_in_of(
        &mut self,
        span: Span,
        oxc: Option<oxc_semantic::ScopeId>,
        left: &ForStatementLeft<'a>,
        right: &Expression<'a>,
        body: &Statement<'a>,
    ) {
        let outer = self.cur_scope;
        let scope = self.new_scope(
            ScopeKind::For,
            Some(outer),
            self.cur_fn,
            span,
            self.strict,
            oxc,
        );
        self.cur_scope = scope;
        match left {
            ForStatementLeft::VariableDeclaration(decl) => {
                let lexical = lexical_kind(decl.kind);
                if lexical.is_some() {
                    self.scopes[scope.index()].per_iteration = PerIteration::Fresh;
                }
                for declarator in &decl.declarations {
                    let d = match lexical {
                        None => Decl::Var { assign: true },
                        Some(kind) => Decl::Lexical {
                            kind,
                            init_pos: right.span().end,
                        },
                    };
                    self.decl_pattern(&declarator.id, d);
                    if let Some(init) = &declarator.init {
                        self.visit_expression(init);
                    }
                }
            }
            target => self.visit_assignment_target(target.to_assignment_target()),
        }
        self.visit_expression(right);
        self.visit_statement(body);
        self.cur_scope = outer;
    }

    /// `if (x) function f() {}` (Annex B.3.3): the function is in a synthetic block.
    fn if_branch(&mut self, statement: &Statement<'a>) {
        if let Statement::FunctionDeclaration(f) = statement {
            let outer = self.cur_scope;
            let block = self.new_scope(
                ScopeKind::Block,
                Some(outer),
                self.cur_fn,
                f.span,
                self.strict,
                None,
            );
            self.synthetic_blocks.push((f.span.start, block));
            self.cur_scope = block;
            self.visit_statement(statement);
            self.cur_scope = outer;
        } else {
            self.visit_statement(statement);
        }
    }
}

fn lexical_kind(kind: VariableDeclarationKind) -> Option<BindingKind> {
    match kind {
        VariableDeclarationKind::Var => None,
        VariableDeclarationKind::Let => Some(BindingKind::Let),
        VariableDeclarationKind::Const => Some(BindingKind::Const),
        VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => {
            Some(BindingKind::Using)
        }
    }
}

/// A direct eval: `eval(...)` or `(eval)(...)`, not `eval?.(...)` (ECMA-262 EvaluateCall).
fn is_direct_eval(call: &CallExpression<'_>) -> bool {
    if call.optional {
        return false;
    }
    let mut callee = &call.callee;
    while let Expression::ParenthesizedExpression(p) = callee {
        callee = &p.expression;
    }
    matches!(callee, Expression::Identifier(id) if id.name == "eval")
}

impl<'a> Visit<'a> for Builder<'a> {
    // Cycle points: depth check (layer 3).

    fn visit_expression(&mut self, it: &Expression<'a>) {
        if self.enter(it.span()) {
            walk::walk_expression(self, it);
        }
        self.leave();
    }

    fn visit_statement(&mut self, it: &Statement<'a>) {
        if self.enter(it.span()) {
            walk::walk_statement(self, it);
        }
        self.leave();
    }

    fn visit_assignment_target(&mut self, it: &AssignmentTarget<'a>) {
        if self.enter(it.span()) {
            walk::walk_assignment_target(self, it);
        }
        self.leave();
    }

    fn visit_binding_pattern(&mut self, it: &BindingPattern<'a>) {
        // Every binding pattern is declared through `decl_pattern`; this path only keeps a
        // future AST change from recursing unchecked.
        if self.enter(it.span()) {
            walk::walk_binding_pattern(self, it);
        }
        self.leave();
    }

    // Scopes.

    fn visit_program(&mut self, it: &Program<'a>) {
        let strict = self.module || it.has_use_strict_directive();
        let kind = if self.module {
            ScopeKind::Module
        } else {
            ScopeKind::Script
        };
        let root = self.new_scope(
            kind,
            None,
            FunctionId(0),
            it.span,
            strict,
            it.scope_id.get(),
        );
        let fkind = if self.module {
            FunctionKind::Module
        } else {
            FunctionKind::Script
        };
        self.new_function(FunctionId(0), fkind, it.span, strict, None, root, None);
        self.cur_fn = FunctionId(0);
        self.cur_scope = root;
        self.var_scope = root;
        self.strict = strict;
        self.visit_statements(&it.body);
        self.finish_function(FunctionId(0));
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        let outer = self.cur_scope;
        self.cur_scope = self.new_scope(
            ScopeKind::Block,
            Some(outer),
            self.cur_fn,
            it.span,
            self.strict,
            it.scope_id.get(),
        );
        self.visit_statements(&it.body);
        self.cur_scope = outer;
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.visit_expression(&it.test);
        self.if_branch(&it.consequent);
        if let Some(alternate) = &it.alternate {
            self.if_branch(alternate);
        }
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        let outer = self.cur_scope;
        let scope = self.new_scope(
            ScopeKind::For,
            Some(outer),
            self.cur_fn,
            it.span,
            self.strict,
            it.scope_id.get(),
        );
        self.cur_scope = scope;
        if let Some(init) = &it.init {
            match init {
                ForStatementInit::VariableDeclaration(decl) => {
                    if decl.kind == VariableDeclarationKind::Let {
                        self.scopes[scope.index()].per_iteration = PerIteration::Copy;
                    }
                    self.visit_variable_declaration(decl);
                }
                expr => self.visit_expression(expr.to_expression()),
            }
        }
        if let Some(test) = &it.test {
            self.visit_expression(test);
        }
        if let Some(update) = &it.update {
            self.visit_expression(update);
        }
        self.visit_statement(&it.body);
        self.cur_scope = outer;
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.for_in_of(it.span, it.scope_id.get(), &it.left, &it.right, &it.body);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.for_in_of(it.span, it.scope_id.get(), &it.left, &it.right, &it.body);
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.visit_expression(&it.discriminant);
        let outer = self.cur_scope;
        self.cur_scope = self.new_scope(
            ScopeKind::Switch,
            Some(outer),
            self.cur_fn,
            it.span,
            self.strict,
            it.scope_id.get(),
        );
        for case in &it.cases {
            if let Some(test) = &case.test {
                self.visit_expression(test);
            }
            self.visit_statements(&case.consequent);
        }
        self.cur_scope = outer;
    }

    fn visit_with_statement(&mut self, it: &WithStatement<'a>) {
        self.visit_expression(&it.object);
        let outer = self.cur_scope;
        self.cur_scope = self.new_scope(
            ScopeKind::With,
            Some(outer),
            self.cur_fn,
            it.span,
            self.strict,
            it.scope_id.get(),
        );
        self.visit_statement(&it.body);
        self.cur_scope = outer;
    }

    fn visit_catch_clause(&mut self, it: &CatchClause<'a>) {
        let outer = self.cur_scope;
        let scope = self.new_scope(
            ScopeKind::Catch,
            Some(outer),
            self.cur_fn,
            it.span,
            self.strict,
            it.scope_id.get(),
        );
        self.cur_scope = scope;
        if let Some(param) = &it.param {
            let pattern = !matches!(param.pattern, BindingPattern::BindingIdentifier(_));
            self.scope_extra[scope.index()].catch_pattern = pattern;
            self.decl_pattern(&param.pattern, Decl::Catch);
            // A pattern initializes its names one by one, so a default value or a computed key
            // can read one before its initialization (ECMA-262 CatchClauseEvaluation: the
            // bindings are created uninitialized). Same rule as a non-simple parameter list.
            if pattern {
                let end = param.pattern.span().end;
                for &b in &self.scopes[scope.index()].bindings {
                    let binding = &mut self.bindings[b.index()];
                    binding.tdz = true;
                    binding.init_pos = end;
                }
            }
        }
        self.visit_block_statement(&it.body);
        self.cur_scope = outer;
    }

    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        self.function(it, FunctionKind::Normal, false);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let fid = FunctionId::of(self.functions.len());
        let outer = self.cur_scope;
        let strict = self.strict || it.has_use_strict_directive();
        let scope = self.new_scope(
            ScopeKind::Function,
            Some(outer),
            fid,
            it.span,
            strict,
            it.scope_id.get(),
        );
        self.new_function(
            fid,
            FunctionKind::Arrow,
            it.span,
            strict,
            Some(outer),
            scope,
            None,
        );
        self.functions[fid.index()].is_async = it.r#async;
        self.function_at.push((it.span.start, fid));
        let saved = self.enter_function(fid, scope, strict);
        let body_span = it.get_function_body().map_or(it.span, |b| b.span);
        self.params(&it.params, body_span);
        if let Some(body) = it.get_function_body() {
            self.visit_statements(&body.statements);
        } else if let Some(expr) = it.get_expression() {
            self.visit_expression(expr);
        }
        self.finish_function(fid);
        self.restore(saved);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let outer = self.cur_scope;
        self.visit_decorators(&it.decorators);
        if it.r#type == ClassType::ClassDeclaration {
            let name = match &it.id {
                Some(id) => self.name(id.name.as_str()),
                None => self.n_default,
            };
            let b = self.declare(
                outer,
                name,
                BindingKind::Class,
                it.id.as_ref().map(|id| id.span),
            );
            self.bindings[b.index()].init_pos = it.span.end;
            self.mark_exported(b, outer);
        }
        let class_scope = self.new_scope(
            ScopeKind::Class,
            Some(outer),
            self.cur_fn,
            it.span,
            true,
            it.scope_id.get(),
        );
        let declaration = it.r#type == ClassType::ClassDeclaration;
        let inner_name = it.id.as_ref().map(|id| {
            let name = self.name(id.name.as_str());
            // A class declaration's identifier maps to the outer binding in `declared_at`.
            let b = self.declare_full(
                class_scope,
                name,
                BindingKind::ClassInner,
                Some(id.span),
                !declaration,
            );
            self.bindings[b.index()].init_pos = it.span.end;
            b
        });
        let saved_strict = self.strict;
        self.cur_scope = class_scope;
        self.strict = true;
        if let Some(heritage) = &it.heritage {
            self.visit_expression(&heritage.expression);
        }

        // Private names are visible in the whole class body: declare them first.
        let elements = &it.body.body;
        for element in elements {
            let (key, kind, is_static) = match element {
                ClassElement::MethodDefinition(m) => {
                    let kind = match m.kind {
                        MethodDefinitionKind::Get => PrivateKind::Getter,
                        MethodDefinitionKind::Set => PrivateKind::Setter,
                        _ => PrivateKind::Method,
                    };
                    (&m.key, kind, m.r#static)
                }
                ClassElement::PropertyDefinition(p) => (&p.key, PrivateKind::Field, p.r#static),
                ClassElement::AccessorProperty(p) => (&p.key, PrivateKind::Accessor, p.r#static),
                _ => continue,
            };
            if let PropertyKey::PrivateIdentifier(id) = key {
                let name = self.private_name(id.name.as_str());
                self.declare(
                    class_scope,
                    name,
                    BindingKind::PrivateName { kind, is_static },
                    Some(id.span),
                );
            }
        }

        // Synthetic class-context bindings (P2-CLS-CG, D2). Their names start with `#.`, which
        // no source can spell, so codegen finds them by name in the class scope and a direct
        // eval never sees them. Like every private name they live in the class context.
        //   #.brand          instance brand (any instance private method or accessor)
        //   #.static-brand   static brand (any static private method or accessor)
        //   #.set-x          the setter of private accessor `#x` (`#x` holds the getter)
        //   #.class-field-N  the key of the N-th computed field, after ToPropertyKey
        fn synthetic_name(names: &mut Vec<Box<str>>, text: String) -> NameId {
            let id = NameId::of(names.len());
            names.push(text.into_boxed_str());
            id
        }
        let private_method = |e: &ClassElement<'a>, want_static: bool| {
            matches!(e, ClassElement::MethodDefinition(m)
                if m.r#static == want_static && matches!(m.key, PropertyKey::PrivateIdentifier(_)))
        };
        let brands = [(".brand", false), (".static-brand", true)];
        for (text, is_static) in brands {
            if elements.iter().any(|e| private_method(e, is_static)) {
                let name = self.private_name(text);
                self.declare(
                    class_scope,
                    name,
                    BindingKind::PrivateName {
                        kind: PrivateKind::Method,
                        is_static,
                    },
                    None,
                );
            }
        }
        let accessors: Vec<(NameId, bool)> = self.scopes[class_scope.index()]
            .bindings
            .iter()
            .filter_map(|b| match self.bindings[b.index()].kind {
                BindingKind::PrivateName {
                    kind: PrivateKind::Getter | PrivateKind::Setter | PrivateKind::GetterSetter,
                    is_static,
                } => Some((self.bindings[b.index()].name, is_static)),
                _ => None,
            })
            .collect();
        for (accessor, is_static) in accessors {
            let text = format!("#.set-{}", &self.names[accessor.index()][1..]);
            let name = synthetic_name(&mut self.names, text);
            self.declare(
                class_scope,
                name,
                BindingKind::PrivateName {
                    kind: PrivateKind::Method,
                    is_static,
                },
                None,
            );
        }
        let computed_fields = elements
            .iter()
            .filter_map(|e| match e {
                ClassElement::PropertyDefinition(p) if p.computed => Some(p.r#static),
                _ => None,
            })
            .enumerate()
            .collect::<Vec<_>>();
        for (n, is_static) in computed_fields {
            let name = synthetic_name(&mut self.names, format!("#.class-field-{n}"));
            self.declare(
                class_scope,
                name,
                BindingKind::PrivateName {
                    kind: PrivateKind::Field,
                    is_static,
                },
                None,
            );
        }

        // Synthetic functions: ids at the start of the class body, in this fixed order.
        let derived = it.heritage.is_some();
        let has_constructor = elements.iter().any(|e| {
            matches!(e, ClassElement::MethodDefinition(m) if m.kind == MethodDefinitionKind::Constructor)
        });
        let needs_instance = elements.iter().any(|e| match e {
            ClassElement::PropertyDefinition(p) => !p.r#static,
            ClassElement::AccessorProperty(p) => !p.r#static,
            ClassElement::MethodDefinition(m) => {
                !m.r#static && matches!(m.key, PropertyKey::PrivateIdentifier(_))
            }
            _ => false,
        });
        let needs_static = elements.iter().any(|e| match e {
            ClassElement::PropertyDefinition(p) => p.r#static,
            ClassElement::AccessorProperty(p) => p.r#static,
            ClassElement::StaticBlock(_) => true,
            _ => false,
        });
        let body_span = it.body.span;
        let default_constructor = (!has_constructor).then(|| {
            let kind = if derived {
                FunctionKind::DefaultDerivedConstructor
            } else {
                FunctionKind::DefaultConstructor
            };
            self.synthetic(kind, class_scope, body_span, false)
        });
        let instance = needs_instance.then(|| {
            self.synthetic(
                FunctionKind::InstanceMembersInitializer,
                class_scope,
                body_span,
                false,
            )
        });
        let statics = needs_static.then(|| {
            self.synthetic(
                FunctionKind::StaticMembersInitializer,
                class_scope,
                body_span,
                true,
            )
        });

        let mut constructor = default_constructor;
        for element in elements {
            match element {
                ClassElement::MethodDefinition(m) => {
                    self.visit_decorators(&m.decorators);
                    if m.computed {
                        self.visit_property_key(&m.key);
                    }
                    let kind = match m.kind {
                        MethodDefinitionKind::Constructor if derived => {
                            FunctionKind::DerivedConstructor
                        }
                        MethodDefinitionKind::Constructor => FunctionKind::ClassConstructor,
                        MethodDefinitionKind::Method => FunctionKind::Method,
                        MethodDefinitionKind::Get => FunctionKind::Getter,
                        MethodDefinitionKind::Set => FunctionKind::Setter,
                    };
                    if m.kind == MethodDefinitionKind::Constructor {
                        constructor = Some(FunctionId::of(self.functions.len()));
                    }
                    self.function(&m.value, kind, m.r#static);
                }
                ClassElement::PropertyDefinition(p) => {
                    self.visit_decorators(&p.decorators);
                    if p.computed {
                        self.visit_property_key(&p.key);
                    }
                    if let Some(value) = &p.value {
                        let f = if p.r#static { statics } else { instance };
                        self.in_synthetic(f, |s| s.visit_expression(value));
                    }
                }
                ClassElement::AccessorProperty(p) => {
                    self.visit_decorators(&p.decorators);
                    if p.computed {
                        self.visit_property_key(&p.key);
                    }
                    if let Some(value) = &p.value {
                        let f = if p.r#static { statics } else { instance };
                        self.in_synthetic(f, |s| s.visit_expression(value));
                    }
                }
                ClassElement::StaticBlock(block) => {
                    self.in_synthetic(statics, |s| {
                        let parent = s.cur_scope;
                        let scope = s.new_scope(
                            ScopeKind::StaticBlock,
                            Some(parent),
                            s.cur_fn,
                            block.span,
                            true,
                            block.scope_id.get(),
                        );
                        s.cur_scope = scope;
                        s.var_scope = scope;
                        s.visit_statements(&block.body);
                    });
                }
                ClassElement::TSIndexSignature(_) => {}
            }
        }
        for f in [default_constructor, instance, statics]
            .into_iter()
            .flatten()
        {
            self.finish_function(f);
        }
        self.scopes[class_scope.index()].class = Some(ClassInfo {
            derived,
            inner_name,
            constructor,
            default_constructor,
            instance_initializer: instance,
            static_initializer: statics,
        });
        self.cur_scope = outer;
        self.strict = saved_strict;
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        let kind = match it.kind {
            PropertyKind::Get => Some(FunctionKind::Getter),
            PropertyKind::Set => Some(FunctionKind::Setter),
            PropertyKind::Init if it.method => Some(FunctionKind::Method),
            PropertyKind::Init => None,
        };
        match (kind, &it.value) {
            (Some(kind), Expression::FunctionExpression(f)) => {
                if it.computed {
                    self.visit_property_key(&it.key);
                }
                self.function(f, kind, false);
            }
            _ => walk::walk_object_property(self, it),
        }
    }

    // Declarations.

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        let lexical = lexical_kind(it.kind);
        for declarator in &it.declarations {
            let decl = match lexical {
                None => Decl::Var {
                    assign: declarator.init.is_some(),
                },
                Some(kind) => Decl::Lexical {
                    kind,
                    init_pos: declarator.span.end,
                },
            };
            self.decl_pattern(&declarator.id, decl);
            if let Some(init) = &declarator.init {
                self.visit_expression(init);
            }
        }
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let Some(specifiers) = &it.specifiers else {
            return;
        };
        for specifier in specifiers {
            let (local, namespace) = match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(s) => (&s.local, false),
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => (&s.local, false),
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (&s.local, true),
            };
            let name = self.name(local.name.as_str());
            let b = self.declare(self.cur_scope, name, BindingKind::Import, Some(local.span));
            // A namespace object is initialized when the module environment is made.
            self.bindings[b.index()].tdz = !namespace;
        }
    }

    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        self.exporting = true;
        self.visit_declaration(&it.declaration);
        self.exporting = false;
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        for specifier in &it.specifiers {
            if let ModuleExportName::IdentifierReference(local) = &specifier.local {
                self.identifier(local, true);
            }
        }
    }

    fn visit_export_from_declaration(&mut self, _it: &ExportFromDeclaration<'a>) {}

    fn visit_export_all_declaration(&mut self, _it: &ExportAllDeclaration<'a>) {}

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        self.exporting = true;
        match &it.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                self.function(f, FunctionKind::Normal, false);
            }
            ExportDefaultDeclarationKind::ClassDeclaration(c) => self.visit_class(c),
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
            other => {
                if let Some(expr) = other.as_expression() {
                    let scope = self.cur_scope;
                    let b = self.declare(scope, self.n_default, BindingKind::Let, None);
                    self.bindings[b.index()].init_pos = it.span.end;
                    self.mark_exported(b, scope);
                    self.visit_expression(expr);
                }
            }
        }
        self.exporting = false;
    }

    // References.

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.identifier(it, false);
    }

    fn visit_this_expression(&mut self, it: &ThisExpression) {
        self.implicit_ref(it.span, ImplicitRef::This, self.n_this, RefKind::This);
    }

    fn visit_new_target(&mut self, it: &NewTarget) {
        self.implicit_ref(
            it.span,
            ImplicitRef::NewTarget,
            self.n_new_target,
            RefKind::NewTarget,
        );
    }

    /// `super.x` and `super[x]`: `super()` is handled in `visit_call_expression`.
    fn visit_super(&mut self, it: &Super) {
        self.implicit_ref(
            it.span,
            ImplicitRef::HomeObject,
            self.n_home_object,
            RefKind::HomeObject,
        );
        self.implicit_ref(it.span, ImplicitRef::This, self.n_this, RefKind::This);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Super(s) = &it.callee {
            self.implicit_ref(s.span, ImplicitRef::Callee, self.n_callee, RefKind::Callee);
            self.implicit_ref(
                s.span,
                ImplicitRef::NewTarget,
                self.n_new_target,
                RefKind::NewTarget,
            );
            self.implicit_ref(s.span, ImplicitRef::This, self.n_this, RefKind::This);
            self.visit_arguments(&it.arguments);
            return;
        }
        if is_direct_eval(it) {
            self.eval_sites.push(EvalSite {
                scope: self.cur_scope,
                function: self.cur_fn,
                var_scope: self.var_scope,
                strict: self.strict,
            });
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        self.visit_expression(&it.object);
        let name = self.private_name(it.field.name.as_str());
        self.implicit_ref(
            it.field.span,
            ImplicitRef::Private,
            name,
            RefKind::PrivateName,
        );
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        let name = self.private_name(it.left.name.as_str());
        self.implicit_ref(
            it.left.span,
            ImplicitRef::Private,
            name,
            RefKind::PrivateName,
        );
        self.visit_expression(&it.right);
    }
}
