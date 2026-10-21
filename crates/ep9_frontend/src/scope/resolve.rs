//! Scope pass steps 2-4: function exit rules (parameters, `arguments`, Annex B), then
//! resolution of every reference, then allocation of every binding. No recursion: the loops
//! walk parent links, so deep nesting costs time, not stack.

use std::collections::HashMap;

use super::collect::{Builder, EvalSite};
use super::{
    BindingId, BindingKind, BindingLocation, ContextReason, FrameSlot, FunctionId, ImplicitRef,
    NameId, RefKind, Resolution, ScopeId, ScopeKind,
};

impl Builder<'_> {
    /// Rules that need the whole function: run when the walk leaves it.
    pub(super) fn finish_function(&mut self, f: FunctionId) {
        let info = &self.functions[f.index()];
        let (scope, body, simple, has_expressions, kind) = (
            info.scope,
            info.body_scope,
            info.simple_params,
            info.has_parameter_expressions,
            info.kind,
        );

        // 1. A non-simple parameter list initializes its bindings left to right, so a default
        //    value that reads a later parameter throws (ECMA-262 IteratorBindingInitialization).
        if !simple {
            for &(b, end) in &self.fn_extra[f.index()].param_ends {
                let binding = &mut self.bindings[b.index()];
                binding.tdz = true;
                binding.init_pos = end;
            }
        }

        // 2. argumentsObjectNeeded (ECMA-262 FunctionDeclarationInstantiation steps 15-18).
        let mut needed = kind.may_have_arguments();
        if needed && let Some(&b) = self.scope_extra[scope.index()].names.get(&self.n_arguments) {
            match self.bindings[b.index()].kind {
                BindingKind::Parameter => needed = false,
                // `var arguments` shares the binding with the arguments object.
                BindingKind::Var => self.fn_extra[f.index()].arguments_var = Some(b),
                // A function or lexical declaration (same scope only when not split).
                _ if !has_expressions => needed = false,
                _ => {}
            }
        }
        self.fn_extra[f.index()].arguments_needed = needed;

        // 3. Split body: a body `var` named like a parameter starts with the parameter's value
        //    (step 28.f.i.4). `var arguments` starts with the arguments object.
        if body != scope {
            for i in 0..self.scopes[body.index()].bindings.len() {
                let b = self.scopes[body.index()].bindings[i];
                if self.bindings[b.index()].kind != BindingKind::Var {
                    continue;
                }
                let name = self.bindings[b.index()].name;
                if let Some(&p) = self.scope_extra[scope.index()].names.get(&name) {
                    if self.bindings[p.index()].kind == BindingKind::Parameter {
                        self.bindings[b.index()].init_from = Some(p);
                    }
                } else if name == self.n_arguments && needed {
                    let a = self.arguments_binding(f);
                    self.bindings[b.index()].init_from = Some(a);
                    self.bindings[a.index()].referenced = true;
                }
            }
        }

        // 4. Annex B.3.2 block functions.
        let candidates = std::mem::take(&mut self.fn_extra[f.index()].block_functions);
        for b in candidates {
            self.annex_b(f, b);
        }
    }

    /// Annex B.3.2.1 / B.3.2.2: a sloppy block function also gets a var binding if replacing
    /// it with `var F` would not be an early error and `F` is not a parameter name.
    ///
    /// Two declarations of `F` in one block share one block binding and both copy it to the
    /// var binding. A literal reading of B.3.2.1 skips them (replacing one leaves a lexical `F`
    /// next to `var F`), but V8 and SpiderMonkey hoist them (node 24.22;
    /// test262 staging/sm/lexical-environment/block-scoped-functions-deprecated-redecl.js), and
    /// no main test262 test expects the skip.
    fn annex_b(&mut self, f: FunctionId, b: BindingId) {
        let name = self.bindings[b.index()].name;
        let info = &self.functions[f.index()];
        let (top, var_scope) = (info.scope, info.body_scope);
        if let Some(&p) = self.scope_extra[top.index()].names.get(&name)
            && self.bindings[p.index()].kind == BindingKind::Parameter
        {
            return;
        }
        if name == self.n_arguments && self.fn_extra[f.index()].arguments_needed {
            return;
        }
        let mut s = self.scopes[self.bindings[b.index()].scope.index()]
            .parent
            .expect("a block has a parent");
        loop {
            if let Some(&other) = self.scope_extra[s.index()].names.get(&name) {
                let conflict = match self.bindings[other.index()].kind {
                    BindingKind::Let
                    | BindingKind::Const
                    | BindingKind::Using
                    | BindingKind::Class
                    | BindingKind::BlockFunction => true,
                    // B.3.4: `var e` may redeclare a simple catch parameter.
                    BindingKind::CatchParameter => self.scope_extra[s.index()].catch_pattern,
                    _ => false,
                };
                if conflict {
                    return;
                }
            }
            if s == var_scope {
                break;
            }
            s = self.scopes[s.index()]
                .parent
                .expect("the var scope is an ancestor");
        }
        let var = match self.scope_extra[var_scope.index()].names.get(&name) {
            Some(&v) => v,
            None => {
                // Not a declaration site of its own: `declared_at` keeps the block binding.
                let span = self.bindings[b.index()]
                    .span
                    .map(|s| oxc_span::Span::new(s.start, s.end));
                self.declare_full(var_scope, name, BindingKind::Var, span, false)
            }
        };
        self.bindings[b.index()].annex_b_var = Some(var);
    }

    /// The binding that holds the arguments object of `f` (made on first use).
    fn arguments_binding(&mut self, f: FunctionId) -> BindingId {
        if let Some(b) = self.fn_extra[f.index()].arguments_var {
            return b;
        }
        let scope = self.functions[f.index()].scope;
        let b = self.declare(scope, self.n_arguments, BindingKind::Arguments, None);
        self.fn_extra[f.index()].arguments_var = Some(b);
        b
    }

    /// C6: binding 0 of a function var scope with a sloppy direct eval is `.eval_ext`, so it gets
    /// context slot 0 (`allocate` places a scope's bindings in order, and the eval puts every
    /// binding of the scope in its context). A script scope gets none: its eval vars go to the
    /// global object.
    fn eval_ext_binding(&mut self, scope: ScopeId) {
        if self.scopes[scope.index()].kind == ScopeKind::Script
            || self.scope_extra[scope.index()]
                .names
                .contains_key(&self.n_eval_ext)
        {
            return;
        }
        let b = self.declare(scope, self.n_eval_ext, BindingKind::EvalExt, None);
        let list = &mut self.scopes[scope.index()].bindings;
        debug_assert_eq!(list.last(), Some(&b), "declare appends the new binding");
        list.pop();
        list.insert(0, b);
        self.bindings[b.index()].referenced = true;
    }

    /// The nearest function with its own `this`: arrows use the enclosing one.
    fn receiver_function(&self, mut f: FunctionId) -> FunctionId {
        while self.functions[f.index()].kind == super::FunctionKind::Arrow {
            f = self.functions[f.index()]
                .parent
                .expect("an arrow has a parent function");
        }
        f
    }

    /// The implicit binding of `f` for `this`, `new.target`, the home object or the callee.
    fn pseudo(&mut self, f: FunctionId, which: ImplicitRef) -> BindingId {
        let info = &self.functions[f.index()];
        let existing = match which {
            ImplicitRef::This => info.this,
            ImplicitRef::NewTarget => info.new_target,
            ImplicitRef::HomeObject => info.home_object,
            ImplicitRef::Callee => info.callee,
            ImplicitRef::VarInit | ImplicitRef::Private => unreachable!("not a pseudo binding"),
        };
        if let Some(b) = existing {
            return b;
        }
        let (scope, derived) = (info.scope, info.kind.is_derived_constructor());
        let (name, kind) = match which {
            ImplicitRef::This => (self.n_this, BindingKind::This),
            ImplicitRef::NewTarget => (self.n_new_target, BindingKind::NewTarget),
            ImplicitRef::HomeObject => (self.n_home_object, BindingKind::HomeObject),
            _ => (self.n_callee, BindingKind::Callee),
        };
        let b = self.declare(scope, name, kind, None);
        // `this` of a derived constructor is uninitialized until `super()` returns.
        self.bindings[b.index()].tdz = kind == BindingKind::This && derived;
        let info = &mut self.functions[f.index()];
        match which {
            ImplicitRef::This => info.this = Some(b),
            ImplicitRef::NewTarget => info.new_target = Some(b),
            ImplicitRef::HomeObject => info.home_object = Some(b),
            _ => info.callee = Some(b),
        }
        b
    }

    /// Walk out from `from`. Returns the binding and whether a `with` object or a sloppy eval
    /// scope was passed on the way (then the result is only a fallback).
    fn lookup(
        &mut self,
        from: ScopeId,
        name: NameId,
        dynamic_scopes: bool,
    ) -> (Option<BindingId>, bool) {
        let mut s = from;
        let mut dynamic = false;
        loop {
            if let Some(&b) = self.scope_extra[s.index()].names.get(&name) {
                return (Some(b), dynamic);
            }
            let scope = &self.scopes[s.index()];
            if name == self.n_arguments && scope.kind == ScopeKind::Function {
                let f = scope.function;
                if self.functions[f.index()].scope == s && self.fn_extra[f.index()].arguments_needed
                {
                    return (Some(self.arguments_binding(f)), dynamic);
                }
            }
            // Global code: an eval `var` becomes a global property, which a global lookup finds.
            if dynamic_scopes
                && (scope.kind == ScopeKind::With
                    || (scope.sloppy_eval && scope.kind != ScopeKind::Script))
            {
                dynamic = true;
            }
            match scope.parent {
                Some(p) => s = p,
                None => return (None, dynamic),
            }
        }
    }

    fn hole_check(&self, b: BindingId, function: FunctionId, at: u32) -> bool {
        let binding = &self.bindings[b.index()];
        if !binding.tdz {
            return false;
        }
        if matches!(binding.kind, BindingKind::Import | BindingKind::This) {
            return true;
        }
        let scope = &self.scopes[binding.scope.index()];
        // V8 rule: skip the check only in the declaring function, in a linear scope, after the
        // initializer. A closure may run before the initialization; a switch case may skip it.
        scope.function != function || scope.kind == ScopeKind::Switch || at < binding.init_pos
    }

    pub(super) fn resolve(&mut self) {
        // 1. Direct eval sites.
        for i in 0..self.eval_sites.len() {
            let EvalSite {
                scope,
                function,
                var_scope,
                strict,
            } = self.eval_sites[i];
            let mut s = Some(scope);
            while let Some(id) = s {
                let info = &mut self.scopes[id.index()];
                if info.contains_eval {
                    break; // its ancestors are marked too
                }
                info.contains_eval = true;
                s = info.parent;
            }
            self.functions[function.index()].calls_eval = true;
            if !strict {
                self.scopes[var_scope.index()].sloppy_eval = true;
                let owner = self.scopes[var_scope.index()].function;
                self.functions[owner.index()].calls_sloppy_eval = true;
                self.eval_ext_binding(var_scope);
            }
            // The eval code can use this, new.target, super and arguments of the nearest
            // function with its own `this`.
            let nf = self.receiver_function(function);
            let kind = self.functions[nf.index()].kind;
            let mut used = vec![self.pseudo(nf, ImplicitRef::This)];
            if kind.has_new_target() {
                used.push(self.pseudo(nf, ImplicitRef::NewTarget));
            }
            if kind.has_home_object() {
                used.push(self.pseudo(nf, ImplicitRef::HomeObject));
            }
            if kind.is_derived_constructor() {
                used.push(self.pseudo(nf, ImplicitRef::Callee));
            }
            if self.fn_extra[nf.index()].arguments_needed {
                used.push(self.arguments_binding(nf));
            }
            for b in used {
                let binding = &mut self.bindings[b.index()];
                binding.referenced = true;
                binding.captured |= nf != function;
            }
        }

        // 2. References.
        for r in 0..self.references.len() {
            let reference = &self.references[r];
            let (name, kind, scope, function, at) = (
                reference.name,
                reference.kind,
                reference.scope,
                reference.function,
                reference.span.start,
            );
            let (binding, dynamic) = match kind {
                RefKind::This | RefKind::NewTarget | RefKind::HomeObject | RefKind::Callee => {
                    let which = match kind {
                        RefKind::This => ImplicitRef::This,
                        RefKind::NewTarget => ImplicitRef::NewTarget,
                        RefKind::HomeObject => ImplicitRef::HomeObject,
                        _ => ImplicitRef::Callee,
                    };
                    let nf = self.receiver_function(function);
                    (Some(self.pseudo(nf, which)), false)
                }
                // Private names are lexical: `with` and eval cannot shadow them.
                RefKind::PrivateName => self.lookup(scope, name, false),
                _ => self.lookup(scope, name, true),
            };
            let resolution = match binding {
                None if dynamic => Resolution::Dynamic { fallback: None },
                None => Resolution::Global,
                Some(b) => {
                    let owner = self.scopes[self.bindings[b.index()].scope.index()].function;
                    let binding = &mut self.bindings[b.index()];
                    binding.referenced = true;
                    binding.captured |= owner != function;
                    binding.exported |= kind == RefKind::Export;
                    if dynamic {
                        binding.context_reason = Some(ContextReason::DynamicLookup);
                        Resolution::Dynamic { fallback: Some(b) }
                    } else {
                        Resolution::Binding {
                            binding: b,
                            hops: 0,
                            hole_check: self.hole_check(b, function, at),
                        }
                    }
                }
            };
            self.references[r].resolution = resolution;
        }
    }

    pub(super) fn allocate(&mut self) {
        for f in 0..self.functions.len() {
            let used = self.fn_extra[f]
                .arguments_var
                .filter(|b| self.bindings[b.index()].referenced);
            let info = &mut self.functions[f];
            info.arguments = used;
            info.mapped_arguments = used.is_some() && !info.strict && info.simple_params;
        }
        let mut param_slot: HashMap<BindingId, u32> = HashMap::new();
        for info in &self.functions {
            if info.simple_params {
                for (i, p) in info.params.iter().enumerate() {
                    if let Some(b) = p.binding {
                        param_slot.insert(b, u32::try_from(i).expect("few parameters"));
                    }
                }
            }
        }
        for s in 0..self.scopes.len() {
            for i in 0..self.scopes[s].bindings.len() {
                let b = self.scopes[s].bindings[i];
                let (location, reason) = self.place(b, &param_slot);
                let binding = &mut self.bindings[b.index()];
                binding.location = location;
                binding.context_reason = reason;
            }
        }
        for scope in &mut self.scopes {
            scope.needs_context = !scope.context_slots.is_empty()
                || (scope.sloppy_eval && scope.kind != ScopeKind::Script)
                || scope.kind == ScopeKind::With;
        }
        // Contexts from the root down to each scope (inclusive). Parents have smaller ids.
        let mut depth = vec![0u32; self.scopes.len()];
        for s in 0..self.scopes.len() {
            let parent = self.scopes[s].parent.map_or(0, |p| depth[p.index()]);
            depth[s] = parent + u32::from(self.scopes[s].needs_context);
        }
        for r in 0..self.references.len() {
            let from = self.references[r].scope;
            if let Resolution::Binding { binding, hops, .. } = &mut self.references[r].resolution
                && let BindingLocation::Context { scope, .. } =
                    self.bindings[binding.index()].location
            {
                *hops = depth[from.index()] - depth[scope.index()];
            }
        }
    }

    fn place(
        &mut self,
        b: BindingId,
        param_slot: &HashMap<BindingId, u32>,
    ) -> (BindingLocation, Option<ContextReason>) {
        use BindingKind as K;
        let binding = &self.bindings[b.index()];
        let (kind, captured, dynamic, exported, s) = (
            binding.kind,
            binding.captured,
            binding.context_reason == Some(ContextReason::DynamicLookup),
            binding.exported,
            binding.scope,
        );
        let scope = &self.scopes[s.index()];
        let (scope_kind, contains_eval, fid) = (scope.kind, scope.contains_eval, scope.function);
        let f = &self.functions[fid.index()];
        let (mapped, simple, derived) = (
            f.mapped_arguments,
            f.simple_params,
            f.kind.is_derived_constructor(),
        );
        if s.0 == 0 && scope_kind == ScopeKind::Script {
            match kind {
                K::Var | K::Function => return (BindingLocation::Global, None),
                K::Let | K::Const | K::Class | K::Using => {
                    return (BindingLocation::GlobalLexical, None);
                }
                _ => {}
            }
        }
        if s.0 == 0 && scope_kind == ScopeKind::Module && (kind == K::Import || exported) {
            return (BindingLocation::Module, None);
        }
        let reason = if captured || matches!(kind, K::PrivateName { .. }) {
            // Private names are always in the class context: the member initializers and
            // the methods use them (V8 does the same).
            Some(ContextReason::Captured)
        } else if contains_eval {
            Some(ContextReason::Eval)
        } else if dynamic {
            Some(ContextReason::DynamicLookup)
        } else if kind == K::Parameter && mapped {
            Some(ContextReason::MappedArguments)
        } else {
            None
        };
        if reason.is_some() {
            let slots = &mut self.scopes[s.index()].context_slots;
            let slot = u32::try_from(slots.len()).expect("fewer than 2^32 slots");
            slots.push(b);
            return (BindingLocation::Context { scope: s, slot }, reason);
        }
        let location = match kind {
            K::Parameter if simple => match param_slot.get(&b) {
                Some(&i) => BindingLocation::Parameter(i),
                None => self.register(fid),
            },
            K::This if !derived => BindingLocation::Frame(FrameSlot::Receiver),
            K::NewTarget => BindingLocation::Frame(FrameSlot::NewTarget),
            K::Callee | K::FunctionName => BindingLocation::Frame(FrameSlot::Callee),
            K::HomeObject => BindingLocation::Frame(FrameSlot::HomeObject),
            _ => self.register(fid),
        };
        (location, None)
    }

    fn register(&mut self, f: FunctionId) -> BindingLocation {
        let info = &mut self.functions[f.index()];
        let r = info.register_count;
        info.register_count += 1;
        BindingLocation::Register(r)
    }
}
