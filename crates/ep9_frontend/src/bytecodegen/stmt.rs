//! Statements: blocks, declarations, control flow, loops (incl. for-in and for-of), switch,
//! try/catch/finally, with, labels.

use ep9_bytecode::{
    FeedbackSlotKind, HandlerKind, Label, Register, RegisterList, RuntimeId, UnitString,
};
use oxc_ast::ast::{
    BindingPattern, BlockStatement, CatchClause, Class, DoWhileStatement, ForInStatement,
    ForOfStatement, ForStatement, ForStatementInit, ForStatementLeft, Function, IfStatement,
    Statement, SwitchStatement, TryStatement, VariableDeclaration, VariableDeclarationKind,
    WhileStatement, WithStatement,
};
use oxc_span::GetSpan;

use super::expr::Prepared;
use super::func::{Command, Control, FnGen, Gen, TargetKind};
use super::pattern::BindMode;
use crate::scope::{BindingId, BindingLocation, ImplicitRef, PerIteration, Resolution, ScopeId};

impl<'a> FnGen<'a, '_> {
    pub(crate) fn statement(&mut self, s: &'a Statement<'a>) -> Gen {
        self.enter(s.span())?;
        let result = self.statement_inner(s);
        self.leave();
        result
    }

    fn statement_inner(&mut self, s: &'a Statement<'a>) -> Gen {
        let labels = std::mem::take(&mut self.pending_labels);
        let takes_labels = matches!(
            s,
            Statement::LabeledStatement(_)
                | Statement::WhileStatement(_)
                | Statement::DoWhileStatement(_)
                | Statement::ForStatement(_)
                | Statement::ForInStatement(_)
                | Statement::ForOfStatement(_)
                | Statement::SwitchStatement(_)
        );
        if !labels.is_empty() && !takes_labels {
            let exit = self.b.new_label();
            self.control.push(Control::Target {
                labels,
                kind: TargetKind::Labeled,
                break_to: exit,
                continue_to: None,
            });
            let result = self.statement_kind(s, Vec::new());
            self.control.pop();
            result?;
            self.b.bind(exit);
            return Ok(());
        }
        self.statement_kind(s, labels)
    }

    fn statement_kind(&mut self, s: &'a Statement<'a>, labels: Vec<&'a str>) -> Gen {
        match s {
            Statement::BlockStatement(b) => self.block(b),
            Statement::BreakStatement(b) => {
                self.stmt_pos(b.span);
                let label = b.label.as_ref().map(|l| l.name.as_str());
                let Some(i) = self.find_break(label) else {
                    return Err(super::internal(format!(
                        "no break target at {}",
                        b.span.start
                    )));
                };
                self.perform(Command::Break(i));
                Ok(())
            }
            Statement::ContinueStatement(c) => {
                self.stmt_pos(c.span);
                let label = c.label.as_ref().map(|l| l.name.as_str());
                let Some(i) = self.find_continue(label) else {
                    return Err(super::internal(format!(
                        "no continue target at {}",
                        c.span.start
                    )));
                };
                self.perform(Command::Continue(i));
                Ok(())
            }
            Statement::DebuggerStatement(d) => {
                self.stmt_pos(d.span);
                self.b.debugger();
                Ok(())
            }
            Statement::DoWhileStatement(d) => self.do_while(d, labels),
            Statement::EmptyStatement(_) => Ok(()),
            Statement::ExpressionStatement(e) => {
                self.stmt_pos(e.span);
                self.expr_effect(&e.expression)
            }
            Statement::ForInStatement(f) => self.for_in(f, labels),
            Statement::ForOfStatement(f) => self.for_of(f, labels),
            Statement::ForStatement(f) => self.for_loop(f, labels),
            Statement::IfStatement(i) => self.if_stmt(i),
            Statement::LabeledStatement(l) => {
                let mut labels = labels;
                labels.push(l.label.name.as_str());
                self.pending_labels = labels;
                self.statement(&l.body)
            }
            Statement::ReturnStatement(r) => {
                self.stmt_pos(r.span);
                match &r.argument {
                    Some(e) => self.expr(e)?,
                    None => {
                        self.b.lda_undefined();
                    }
                }
                self.perform(Command::Return);
                Ok(())
            }
            Statement::SwitchStatement(sw) => self.switch(sw, labels),
            Statement::ThrowStatement(t) => {
                self.stmt_pos(t.span);
                self.expr(&t.argument)?;
                self.expr_pos(t.span.start);
                self.b.throw();
                Ok(())
            }
            Statement::TryStatement(t) => self.try_stmt(t),
            Statement::WhileStatement(w) => self.while_loop(w, labels),
            Statement::WithStatement(w) => self.with_stmt(w),
            Statement::VariableDeclaration(v) => self.var_decl(v),
            Statement::FunctionDeclaration(f) => self.function_declaration(f),
            Statement::ClassDeclaration(c) => self.class_declaration(c, None),
            // Module items: bytecodegen/module.rs (routing stub, P0-D7; MODULE fills it).
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_) => self.module_declaration(s),
            Statement::ExportDeclaration(e) => self.export_declaration(e),
            Statement::ExportDefaultDeclaration(e) => self.export_default(e),
            Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSEnumDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSNamespaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
            | Statement::TSExportAssignment(_)
            | Statement::TSNamespaceExportDeclaration(_) => {
                self.unsupported("TypeScript declarations", s.span())
            }
        }
    }

    // ---- Blocks and declarations ----

    pub(crate) fn block(&mut self, b: &'a BlockStatement<'a>) -> Gen {
        let scope = self.scope_of(b.scope_id.get(), b.span)?;
        let pushed = self.enter_scope(scope);
        self.hoist_functions(scope)?;
        self.statements(&b.body)?;
        self.exit_scope(scope, pushed);
        Ok(())
    }

    pub(crate) fn var_decl(&mut self, decl: &'a VariableDeclaration<'a>) -> Gen {
        if is_using(decl.kind) {
            return self.using_decl(decl);
        }
        let lexical = decl.kind != VariableDeclarationKind::Var;
        for d in &decl.declarations {
            self.stmt_pos(d.span);
            if lexical {
                match &d.init {
                    Some(init) => self.named_value(init, super::func::pattern_name(&d.id))?,
                    None => {
                        self.b.lda_undefined();
                    }
                }
                self.bind_value_in_acc(&d.id, BindMode::Init)?;
            } else if let Some(init) = &d.init {
                self.var_initializer(&d.id, init)?;
            }
        }
        Ok(())
    }

    /// `var id = init` (also the Annex B.3.5 `for (var x = init in o)` head): the value of
    /// `init` (NamedEvaluation for an anonymous function), then PutValue to `id`. A dynamic
    /// `x` is resolved before `init` runs (§14.3.2.1 step 2, B.3.5 step 2; lookup.rs).
    fn var_initializer(
        &mut self,
        id: &'a BindingPattern<'a>,
        init: &'a oxc_ast::ast::Expression<'a>,
    ) -> Gen {
        if let BindingPattern::BindingIdentifier(b) = id
            && !super::lookup::runs_no_code(init)
        {
            let r = self.implicit(b.span.start, ImplicitRef::VarInit)?;
            if matches!(r.resolution, Resolution::Dynamic { .. }) {
                let m = self.mark();
                let p = self.resolve_dynamic(Prepared::Binding(r));
                self.named_value(init, super::func::pattern_name(id))?;
                self.put_prepared(&p, b.span.start, false)?;
                self.release(m);
                return Ok(());
            }
        }
        self.named_value(init, super::func::pattern_name(id))?;
        self.bind_value_in_acc(id, BindMode::Var)
    }

    /// Bind the accumulator to `pattern`.
    pub(crate) fn bind_value_in_acc(
        &mut self,
        pattern: &'a BindingPattern<'a>,
        mode: BindMode,
    ) -> Gen {
        match pattern {
            BindingPattern::BindingIdentifier(id) => self.bind_identifier(id, mode),
            _ => {
                let m = self.mark();
                let t = self.temp();
                self.b.star(t);
                self.bind_pattern(pattern, t, mode)?;
                self.release(m);
                Ok(())
            }
        }
    }

    /// Store the accumulator into the binding of `id`.
    pub(crate) fn bind_identifier(
        &mut self,
        id: &oxc_ast::ast::BindingIdentifier<'a>,
        mode: BindMode,
    ) -> Gen {
        match mode {
            BindMode::Init => {
                let b = self.declared(id)?;
                self.store_binding_init(b);
            }
            BindMode::Var => {
                let r = self.implicit(id.span.start, ImplicitRef::VarInit)?;
                self.store_ref(r, false);
            }
        }
        Ok(())
    }

    pub(crate) fn function_declaration(&mut self, f: &'a Function<'a>) -> Gen {
        let name = f.id.as_ref().map_or_else(
            || UnitString::new("default"),
            |id| UnitString::new(id.name.as_str()),
        );
        self.closure(f, Some(name))?;
        // Annex B.3.2: copy the block binding into the var binding when the declaration runs.
        if let Some(id) = &f.id {
            let b = self.declared(id)?;
            if let Some(var) = self.binding(b).annex_b_var {
                self.stmt_pos(f.span);
                self.load_binding_here(b, false);
                self.store_annex_b_var(var);
            }
        }
        Ok(())
    }

    /// genv.SetMutableBinding(F, fobj, false) for an Annex B var binding.
    fn store_annex_b_var(&mut self, var: BindingId) {
        if let Some(&(_, flag)) = self.eval_annex_b.iter().find(|&&(v, _)| v == var) {
            // Sloppy eval code (B.3.2.3 step ii.2.g.ii): the var binding is in the caller's
            // variable environment, and only if DeclareEvalAnnexBFunction made it (`flag`).
            let m = self.mark();
            let args = self.temps(2);
            let skip = self.b.new_label();
            self.b.star(args.get(1));
            self.b.ldar(flag).jump_if_false(skip);
            let k = self.name_const(self.binding(var).name);
            self.b.lda_constant(k).star(args.get(0));
            self.runtime(RuntimeId::StoreEvalAnnexBFunction, args);
            self.b.bind(skip);
            self.release(m);
            return;
        }
        match self.binding(var).location {
            BindingLocation::Global | BindingLocation::GlobalLexical => {
                let name = self.binding(var).name;
                let k = self.name_const(name);
                let s = self.global_slot(FeedbackSlotKind::StoreGlobal, k);
                self.b.sta_global(k, s);
            }
            _ => self.store_binding_init(var),
        }
    }

    pub(crate) fn class_declaration(
        &mut self,
        c: &'a Class<'a>,
        binding: Option<BindingId>,
    ) -> Gen {
        self.stmt_pos(c.span);
        let name = c.id.as_ref().map_or_else(
            || UnitString::new("default"),
            |id| UnitString::new(id.name.as_str()),
        );
        self.class(c, Some(name), None)?;
        let b = match (binding, &c.id) {
            (Some(b), _) => b,
            (None, Some(id)) => self.declared(id)?,
            (None, None) => self.default_binding()?,
        };
        self.store_binding_init(b);
        Ok(())
    }

    // ---- if / loops ----

    fn if_stmt(&mut self, i: &'a IfStatement<'a>) -> Gen {
        self.stmt_pos(i.span);
        let else_label = self.b.new_label();
        self.branch_if_false(&i.test, else_label)?;
        self.if_branch(&i.consequent)?;
        match &i.alternate {
            Some(alt) => {
                let end = self.b.new_label();
                self.b.jump(end);
                self.b.bind(else_label);
                self.if_branch(alt)?;
                self.b.bind(end);
            }
            None => self.b.bind(else_label),
        }
        Ok(())
    }

    /// `if (x) function f() {}` (Annex B.3.3): the function is in a synthetic block.
    fn if_branch(&mut self, s: &'a Statement<'a>) -> Gen {
        if let Statement::FunctionDeclaration(f) = s {
            let block = self.sa.synthetic_block_at(f.span.start).ok_or_else(|| {
                super::internal(format!("no synthetic block at {}", f.span.start))
            })?;
            let pushed = self.enter_scope(block);
            self.hoist_functions(block)?;
            self.statement(s)?;
            self.exit_scope(block, pushed);
            Ok(())
        } else {
            self.statement(s)
        }
    }

    fn loop_target(&mut self, labels: Vec<&'a str>, exit: Label, cont: Label) {
        self.control.push(Control::Target {
            labels,
            kind: TargetKind::Loop,
            break_to: exit,
            continue_to: Some(cont),
        });
    }

    fn while_loop(&mut self, w: &'a WhileStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(w.span);
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let (exit, cont) = (self.b.new_label(), self.b.new_label());
        self.branch_if_false(&w.test, exit)?;
        self.loop_target(labels, exit, cont);
        let result = self.statement(&w.body);
        self.control.pop();
        result?;
        self.b.bind(cont);
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;
        Ok(())
    }

    fn do_while(&mut self, d: &'a DoWhileStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(d.span);
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let (exit, cont) = (self.b.new_label(), self.b.new_label());
        self.loop_target(labels, exit, cont);
        let result = self.statement(&d.body);
        self.control.pop();
        result?;
        self.b.bind(cont);
        self.branch_if_false(&d.test, exit)?;
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;
        Ok(())
    }

    fn for_loop(&mut self, f: &'a ForStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(f.span);
        let scope = self.scope_of(f.scope_id.get(), f.span)?;
        let pushed = self.enter_scope(scope);
        if let Some(ForStatementInit::VariableDeclaration(v)) = &f.init
            && is_using(v.kind)
        {
            // The head's resources: disposed after the whole loop (ForStatement step 10).
            let mut kinds = Vec::new();
            push_using_kinds(v, &mut kinds);
            let suspends = using_head_suspends(v);
            self.using_region(&kinds, suspends, v.span, |g| {
                g.for_loop_tail(f, labels, scope, pushed)
            })?;
            self.exit_scope(scope, pushed);
            return Ok(());
        }
        self.for_loop_tail(f, labels, scope, pushed)?;
        self.exit_scope(scope, pushed);
        Ok(())
    }

    /// A `for` statement from its head declaration on, in the loop head scope.
    fn for_loop_tail(
        &mut self,
        f: &'a ForStatement<'a>,
        labels: Vec<&'a str>,
        scope: ScopeId,
        pushed: bool,
    ) -> Gen {
        match &f.init {
            Some(ForStatementInit::VariableDeclaration(v)) => self.var_decl(v)?,
            Some(init) => {
                let e = init.to_expression();
                self.expr_effect(e)?;
            }
            None => {}
        }
        let copy = pushed && self.per_iteration(scope) == PerIteration::Copy;
        if copy {
            self.copy_iteration_context(scope);
        }
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let (exit, cont) = (self.b.new_label(), self.b.new_label());
        if let Some(test) = &f.test {
            self.branch_if_false(test, exit)?;
        }
        self.loop_target(labels, exit, cont);
        let result = self.statement(&f.body);
        self.control.pop();
        result?;
        self.b.bind(cont);
        if copy {
            self.copy_iteration_context(scope);
        }
        if let Some(update) = &f.update {
            self.expr_effect(update)?;
        }
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;
        Ok(())
    }

    /// CreatePerIterationEnvironment: replace the loop head context by a copy (G7).
    fn copy_iteration_context(&mut self, scope: ScopeId) {
        let m = self.mark();
        let old = self.temp();
        let saved = self.contexts.last().expect("the loop head context").saved;
        self.b.lda_current_context().star(old);
        self.b.pop_context(saved);
        let k = self.b.scope_constant(self.cx.unit_scope(scope));
        self.b.create_block_context(k).push_context(saved);
        let n = self.sa.scope(scope).context_slots.len() as u32;
        for slot in 0..n {
            self.b
                .lda_context_slot(old, slot, 0)
                .sta_current_context_slot(slot);
        }
        self.release(m);
    }

    /// The head of a for-in/of: the right side runs in a TDZ scope of the head names.
    fn for_head_rhs(&mut self, scope: ScopeId, right: &'a oxc_ast::ast::Expression<'a>) -> Gen {
        let pushed = self.enter_scope(scope);
        self.expr(right)?;
        self.exit_scope(scope, pushed);
        Ok(())
    }

    /// Assign the value in `value` to the head of a for-in/of (in the iteration scope).
    fn for_head_assign(&mut self, left: &'a ForStatementLeft<'a>, value: Register) -> Gen {
        match left {
            ForStatementLeft::VariableDeclaration(v) => {
                if is_using(v.kind) {
                    return self.using_head(v, value);
                }
                let mode = if v.kind == VariableDeclarationKind::Var {
                    BindMode::Var
                } else {
                    BindMode::Init
                };
                let d = &v.declarations[0];
                self.b.ldar(value);
                self.bind_value_in_acc(&d.id, mode)
            }
            target => {
                let target = target.to_assignment_target();
                self.assign_to_target(target, value)
            }
        }
    }

    /// `for (var x = init in o)` (Annex B.3.5): the initializer runs first, before the
    /// right side; `x` is resolved before the initializer (step 2), as in `var_decl`.
    fn for_in_var_initializer(&mut self, left: &'a ForStatementLeft<'a>) -> Gen {
        if let ForStatementLeft::VariableDeclaration(v) = left
            && let Some(d) = v.declarations.first()
            && let Some(init) = &d.init
        {
            self.var_initializer(&d.id, init)?;
        }
        Ok(())
    }

    fn for_in(&mut self, f: &'a ForInStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(f.span);
        let scope = self.scope_of(f.scope_id.get(), f.span)?;
        // The initializer's own scopes (a class) are children of the for scope. A `var` head
        // binds nothing in that scope, so it needs no context (Annex B.3.5).
        let outer = self.scope;
        self.scope = scope;
        let init = self.for_in_var_initializer(&f.left);
        self.scope = outer;
        init?;
        let m = self.mark();
        let recv = self.temp();
        let cache = self.temps(3);
        let index = self.temp();
        let key = self.temp();
        self.for_head_rhs(scope, &f.right)?;
        let exit = self.b.new_label();
        self.b.jump_if_undefined_or_null(exit);
        self.expr_pos(f.right.span().start);
        self.b.to_object(recv);
        self.b.for_in_enumerate(recv);
        let s = self.slot(FeedbackSlotKind::ForIn);
        self.b.for_in_prepare(cache, s);
        self.b.lda_zero().star(index);
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let cont = self.b.new_label();
        self.b.jump_if_for_in_done(exit, index, cache.get(2));
        self.b.for_in_next(
            recv,
            index,
            RegisterList::new(cache.get(0).local_index().unwrap(), 2),
            s,
        );
        self.b.jump_if_undefined(cont);
        self.b.star(key);
        self.loop_target(labels, exit, cont);
        let result = (|| -> Gen {
            let pushed = self.enter_scope(scope);
            self.for_head_assign(&f.left, key)?;
            self.statement(&f.body)?;
            self.exit_scope(scope, pushed);
            Ok(())
        })();
        self.control.pop();
        result?;
        self.b.bind(cont);
        self.b.for_in_step(index);
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;
        self.release(m);
        Ok(())
    }

    fn for_of(&mut self, f: &'a ForOfStatement<'a>, labels: Vec<&'a str>) -> Gen {
        if f.r#await {
            return self.for_await_of(f, labels);
        }
        self.stmt_pos(f.span);
        let scope = self.scope_of(f.scope_id.get(), f.span)?;
        let m = self.mark();
        let iter = self.temp();
        let next = self.temp();
        let done = self.temp();
        let value = self.temp();
        self.for_head_rhs(scope, &f.right)?;
        self.get_iterator(iter, next, f.right.span().start);
        self.try_finally(
            |g| {
                g.loop_depth += 1;
                let header = g.b.bound_label();
                let (exit, cont) = (g.b.new_label(), g.b.new_label());
                g.b.lda_true().star(done);
                g.iterator_step(iter, next, value, exit, f.span.start);
                g.b.lda_false().star(done);
                g.loop_target(labels, exit, cont);
                let result = g.for_of_iteration(f, scope, value);
                g.control.pop();
                result?;
                g.b.bind(cont);
                g.jump_loop(header);
                g.b.bind(exit);
                g.loop_depth -= 1;
                Ok(())
            },
            |g, token| {
                let skip = g.b.new_label();
                g.b.ldar(done).jump_if_true(skip);
                g.close_iterator(iter, token);
                g.b.bind(skip);
                Ok(())
            },
        )?;
        self.release(m);
        Ok(())
    }

    /// `for await (lhs of rhs) body` (§14.7.5.6 ForIn/OfHeadEvaluation and §14.7.5.7
    /// ForIn/OfBodyEvaluation with iteratorKind async; M3-P2 ASYNCGEN), in an async function
    /// or an async generator body: the `for-of` loop with GetIterator(rhs, async)
    /// (`async_gen.rs` AGF5), an Await of each `next` result, and AsyncIteratorClose on an
    /// abrupt exit. At a module top level it is top-level await, so it stays unsupported with
    /// the cause of a top-level `await` ("await", AF6; M3-P3 IMPORT-TLA; review RV-P2-AG1).
    fn for_await_of(&mut self, f: &'a ForOfStatement<'a>, labels: Vec<&'a str>) -> Gen {
        if self.await_target().is_none() {
            return self.unsupported("await", f.span);
        }
        self.stmt_pos(f.span);
        let at = f.span.start;
        let scope = self.scope_of(f.scope_id.get(), f.span)?;
        let m = self.mark();
        let iter = self.temp();
        let next = self.temp();
        let done = self.temp();
        let value = self.temp();
        self.for_head_rhs(scope, &f.right)?;
        self.get_async_iterator(iter, next, f.right.span().start);
        self.try_finally(
            |g| {
                g.loop_depth += 1;
                let header = g.b.bound_label();
                let (exit, cont) = (g.b.new_label(), g.b.new_label());
                // A throw of `next`, of its Await or of the result checks does not close the
                // iterator (step 6.a-d).
                g.b.lda_true().star(done);
                let call = g.slot(FeedbackSlotKind::Call);
                g.expr_pos(at);
                g.b.call_property0(next, iter, call);
                g.await_acc(at)?;
                g.b.star(value);
                let ok = g.b.new_label();
                g.b.jump_if_js_receiver(ok);
                g.expr_pos(at);
                g.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[value]);
                g.b.bind(ok);
                let k_done = g.str_const("done");
                let s = g.slot(FeedbackSlotKind::LoadNamed);
                g.b.get_named_property(value, k_done, s);
                g.b.jump_if_to_boolean_true(exit);
                let k_value = g.str_const("value");
                let s = g.slot(FeedbackSlotKind::LoadNamed);
                g.b.get_named_property(value, k_value, s).star(value);
                g.b.lda_false().star(done);
                g.loop_target(labels, exit, cont);
                let result = g.for_of_iteration(f, scope, value);
                g.control.pop();
                result?;
                g.b.bind(cont);
                g.jump_loop(header);
                g.b.bind(exit);
                g.loop_depth -= 1;
                Ok(())
            },
            |g, token| {
                let skip = g.b.new_label();
                g.b.ldar(done).jump_if_true(skip);
                g.close_async_iterator(iter, token, at)?;
                g.b.bind(skip);
                Ok(())
            },
        )?;
        self.release(m);
        Ok(())
    }

    /// One iteration of a `for-of` or `for await` loop: the head binding of `value`, then the
    /// body, in the iteration scope; a `using` head makes the iteration a using region (its
    /// resource is disposed at the end of each iteration, before the iterator closes).
    fn for_of_iteration(
        &mut self,
        f: &'a ForOfStatement<'a>,
        scope: ScopeId,
        value: Register,
    ) -> Gen {
        let pushed = self.enter_scope(scope);
        if let ForStatementLeft::VariableDeclaration(v) = &f.left
            && is_using(v.kind)
        {
            let mut kinds = Vec::new();
            push_using_kinds(v, &mut kinds);
            self.using_region(&kinds, using_head_suspends(v), v.span, |g| {
                g.for_head_assign(&f.left, value)?;
                g.statement(&f.body)
            })?;
        } else {
            self.for_head_assign(&f.left, value)?;
            self.statement(&f.body)?;
        }
        self.exit_scope(scope, pushed);
        Ok(())
    }

    /// `iter = GetIterator(acc)`, `next = iter.next`.
    pub(crate) fn get_iterator(&mut self, iter: Register, next: Register, at: u32) {
        let m = self.mark();
        let obj = self.temp();
        self.b.star(obj);
        let load = self.slot(FeedbackSlotKind::LoadNamed);
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.get_iterator(obj, load, call).star(iter);
        let k = self.str_const("next");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.b.get_named_property(iter, k, s).star(next);
        self.release(m);
    }

    /// One IteratorStep: `value = next value`, or jump to `done` when the iterator is done.
    pub(crate) fn iterator_step(
        &mut self,
        iter: Register,
        next: Register,
        value: Register,
        done: Label,
        at: u32,
    ) {
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.call_property0(next, iter, call).star(value);
        let ok = self.b.new_label();
        self.b.jump_if_js_receiver(ok);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[value]);
        self.b.bind(ok);
        let k_done = self.str_const("done");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.b.get_named_property(value, k_done, s);
        self.b.jump_if_to_boolean_true(done);
        let k_value = self.str_const("value");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.b.get_named_property(value, k_value, s).star(value);
    }

    /// IteratorClose (ECMA-262 7.4.11; V8 BuildFinalizeIteration): token 0 means a throw
    /// completion, so errors from the `return` lookup and from `return()` are ignored (the
    /// original exception wins); otherwise they propagate and the result must be an object.
    pub(crate) fn close_iterator(&mut self, iter: Register, token: Register) {
        let m = self.mark();
        let method = self.temp();
        let done = self.b.new_label();
        let k = self.str_const("return");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        let call = self.slot(FeedbackSlotKind::Call);
        let throw_path = self.b.new_label();
        self.b
            .lda_zero()
            .test_reference_equal(token)
            .jump_if_true(throw_path);
        // Normal completion: the result must be an object.
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        self.b.call_property0(method, iter, call);
        self.b.jump_if_js_receiver(done);
        let r = self.temp();
        self.b.star(r);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[r]);
        self.b.jump(done);
        // Throw completion: look up and call, ignore any exception (a `return` getter too).
        self.b.bind(throw_path);
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler) = (self.b.new_label(), self.b.new_label(), self.b.new_label());
        self.b.bind(start);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        self.b.call_property0(method, iter, call);
        self.b.bind(end);
        self.b
            .add_handler(start, end, handler, ctx, HandlerKind::Caught);
        self.b.jump(done);
        self.b.bind(handler);
        self.b.bind(done);
        self.release(m);
    }

    // ---- switch, try, with ----

    fn switch(&mut self, sw: &'a SwitchStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(sw.span);
        let m = self.mark();
        let tag = self.temp();
        self.expr(&sw.discriminant)?;
        self.b.star(tag);
        let scope = self.scope_of(sw.scope_id.get(), sw.span)?;
        let pushed = self.enter_scope(scope);
        self.hoist_functions(scope)?;
        let exit = self.b.new_label();
        self.control.push(Control::Target {
            labels,
            kind: TargetKind::Switch,
            break_to: exit,
            continue_to: None,
        });
        let result = (|| -> Gen {
            let case_labels: Vec<Label> = sw.cases.iter().map(|_| self.b.new_label()).collect();
            let mut default = None;
            for (i, case) in sw.cases.iter().enumerate() {
                match &case.test {
                    Some(test) => {
                        self.expr(test)?;
                        let s = self.slot(FeedbackSlotKind::CompareOp);
                        self.b
                            .test_equal_strict(tag, s)
                            .jump_if_true(case_labels[i]);
                    }
                    None => default = Some(i),
                }
            }
            self.b.jump(default.map_or(exit, |d| case_labels[d]));
            for (i, case) in sw.cases.iter().enumerate() {
                self.b.bind(case_labels[i]);
                self.statements(&case.consequent)?;
            }
            Ok(())
        })();
        self.control.pop();
        result?;
        self.b.bind(exit);
        self.exit_scope(scope, pushed);
        self.release(m);
        Ok(())
    }

    fn try_stmt(&mut self, t: &'a TryStatement<'a>) -> Gen {
        self.stmt_pos(t.span);
        match (&t.handler, &t.finalizer) {
            (Some(h), None) => self.try_catch(&t.block, h),
            (None, Some(f)) => self.try_finally(|g| g.block(&t.block), |g, _| g.block(f)),
            (Some(h), Some(f)) => self.try_finally(|g| g.try_catch(&t.block, h), |g, _| g.block(f)),
            (None, None) => self.block(&t.block),
        }
    }

    fn try_catch(&mut self, block: &'a BlockStatement<'a>, h: &'a CatchClause<'a>) -> Gen {
        let m = self.mark();
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler, done) = (
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
        );
        let before = self.b.instruction_count();
        self.b.bind(start);
        self.block(block)?;
        self.b.bind(end);
        if self.b.instruction_count() > before {
            self.b
                .add_handler(start, end, handler, ctx, HandlerKind::Caught);
        }
        self.b.jump(done);
        self.b.bind(handler);
        let exc = self.temp();
        // Clear the pending message (V8): the exception is handled.
        self.b.star(exc).lda_the_hole().set_pending_message();
        self.stmt_pos(h.span);
        let scope = self.scope_of(h.scope_id.get(), h.span)?;
        self.scope = scope;
        let needs_context = self.sa.scope(scope).needs_context;
        match h.param.as_ref().map(|p| &p.pattern) {
            None => {
                if needs_context {
                    self.push_context(scope, |g| {
                        let k = g.b.scope_constant(g.cx.unit_scope(scope));
                        g.b.create_block_context(k);
                    });
                }
            }
            Some(BindingPattern::BindingIdentifier(id)) => {
                let b = self.declared(id)?;
                if needs_context {
                    self.push_context(scope, |g| {
                        let k = g.b.scope_constant(g.cx.unit_scope(scope));
                        g.b.create_catch_context(exc, k);
                    });
                    debug_assert!(matches!(
                        self.binding(b).location,
                        BindingLocation::Context { slot: 0, .. }
                    ));
                } else {
                    self.b.ldar(exc);
                    self.store_binding_init(b);
                }
            }
            Some(pattern) => {
                if needs_context {
                    self.push_context(scope, |g| {
                        let k = g.b.scope_constant(g.cx.unit_scope(scope));
                        g.b.create_block_context(k);
                    });
                }
                // The pattern's names are in TDZ until it binds them (also on a later run of
                // this catch: a register still holds the last value).
                self.init_holes(scope);
                self.bind_pattern(pattern, exc, BindMode::Init)?;
            }
        }
        self.block(&h.body)?;
        self.exit_scope(scope, needs_context);
        self.b.bind(done);
        self.release(m);
        Ok(())
    }

    fn with_stmt(&mut self, w: &'a WithStatement<'a>) -> Gen {
        self.stmt_pos(w.span);
        let m = self.mark();
        let obj = self.temp();
        self.expr(&w.object)?;
        self.expr_pos(w.object.span().start);
        self.b.to_object(obj);
        let scope = self.scope_of(w.scope_id.get(), w.span)?;
        self.scope = scope;
        self.push_context(scope, |g| {
            let k = g.b.scope_constant(g.cx.unit_scope(scope));
            g.b.create_with_context(obj, k);
        });
        self.statement(&w.body)?;
        self.exit_scope(scope, true);
        self.release(m);
        Ok(())
    }
}

// ---- using and await using (explicit resource management; M7-P3 SPEC-ERM) ----
//
// A statement list that holds `using` or `await using` declarations (a block, a function,
// generator, async or module body, a class static block), the head of a `for` statement with
// a `using` declaration, and each iteration of a `for-of` with a `using` head are one
// *using region*: the DisposeCapability of its environment (DisposeResources at its end).
//
// ```text
// region entry:   method_i = undefined (using) or the hole (await using)   not on the stack
// try {                                                                    G5 form, token/value
//   using x = e:  value_i = e
//                 [value_i not undefined/null] method_i = GetDisposeMethod(value_i, 0)   (48)
//   await using:  [undefined/null]  method_i = undefined (on the stack, no method)
//                 [else] method_i = GetDisposeMethod(value_i, 1); if undefined:
//                        method_i = GetDisposeMethod(value_i, 0), fallback_i = true
//                 x = value_i                                            InitializeBinding
// } finally {     DisposeResources(capability, completion), resources in reverse order:
//   using:        [method_i] (step 3.d await) try { method_i.call(value_i) } catch (e) R(e)
//   await using:  [method_i hole] skip; [undefined] needsAwait = true
//                 try { r = fallback_i ? (sync call, keep its throw; undefined)
//                                      : method_i.call(value_i)
//                       hasAwaited = true; Await(r); rethrow the kept throw } catch (e) R(e)
//   end:          (step 4) if needsAwait and not hasAwaited: Await(undefined)
//   R(e):         completion is a throw ? value = CreateSuppressedError(e, value) (49)
//                                       : value = e, token = throw
// }
// ```
//
// | # | Decision | Why |
// |---|---|---|
// | U1 | The resources of a region live in registers, 2 per `using` and 3 per `await using` binding; the region knows them all before its body | The bindings of a region are fixed by the source (a `using` is never a statement of an `if` or a loop body, nor of a case clause: early errors); no DisposableStack object per region (V8 keeps a stack object) |
// | U2 | A region is the G5 `try`/`finally` with its token and value registers; the finalizer may replace the completion by a throw | `break`, `continue` and `return` through a region dispose first, as through `finally` |
// | U3 | The sync method of an `await using` value without @@asyncDispose is called directly, then undefined is awaited, then its throw (if any) is the result | The same ticks and results as the promise closure of GetDisposeMethod step 1.b.ii, which user code cannot see; no new builtin |
// | U4 | An `await using` region has `n + 1` await points for `n` bindings: one per `await using` binding, one step-3.d check per `using` binding, and the step-4 check ([`using_suspends`]) | AF3/AGF2: the counters of the body count them before the body |
// | U5 | Code with no `using` emits the same bytecode as before | [`using_kinds`] finds no binding, and no region is made |

/// One resource register set of a using region (U1).
#[derive(Clone, Copy, Debug)]
pub(crate) struct UsingResource {
    /// The value of the binding.
    value: Register,
    /// Its dispose method: undefined (a `using` resource) or the hole (an `await using` one)
    /// while it is not on the stack.
    method: Register,
    /// An `await using` resource: true when `method` is the sync @@dispose method (U3).
    fallback: Option<Register>,
}

/// The using regions being generated (innermost last): the resources and the next one that a
/// binding takes.
#[derive(Debug, Default)]
pub(crate) struct UsingRegion {
    resources: Vec<UsingResource>,
    next: usize,
}

/// True for the kinds of a `using` or `await using` declaration.
fn is_using(kind: VariableDeclarationKind) -> bool {
    matches!(
        kind,
        VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
    )
}

/// The resources that the `using` declarations of `list` itself make, in source order: true
/// for an `await using` binding. Empty when `list` has no `using` declaration (U5).
pub(crate) fn using_kinds(list: &[Statement<'_>]) -> Vec<bool> {
    let mut kinds = Vec::new();
    for s in list {
        if let Statement::VariableDeclaration(v) = s {
            push_using_kinds(v, &mut kinds);
        }
    }
    kinds
}

fn push_using_kinds(v: &VariableDeclaration<'_>, kinds: &mut Vec<bool>) {
    if is_using(v.kind) {
        let a = v.kind == VariableDeclarationKind::AwaitUsing;
        kinds.extend(std::iter::repeat_n(a, v.declarations.len()));
    }
}

/// The await points of the disposal of one region with these resources (U4).
fn suspends_of(kinds: &[bool]) -> usize {
    if kinds.iter().any(|&a| a) {
        kinds.len() + 1
    } else {
        0
    }
}

/// The await points that the using region of the statement list `list` adds (U4), for the
/// suspend counters of async code (AF3, AGF2, the module's AwaitPoints): 0 when `list` has no
/// `await using` declaration.
pub(crate) fn using_suspends(list: &[Statement<'_>]) -> usize {
    suspends_of(&using_kinds(list))
}

/// The await points that the using region of a `for` or `for-of` head declaration adds (U4).
pub(crate) fn using_head_suspends(v: &VariableDeclaration<'_>) -> usize {
    let mut kinds = Vec::new();
    push_using_kinds(v, &mut kinds);
    suspends_of(&kinds)
}

impl<'a> FnGen<'a, '_> {
    /// A using region (U1, U2) with the resources `kinds` around `body`. `span` is the source
    /// range of the region's first declaration (positions and errors). `suspends` is what
    /// [`using_suspends`] or [`using_head_suspends`] counts for it: the disposal checks that
    /// it emits that many await points (U4).
    pub(crate) fn using_region(
        &mut self,
        kinds: &[bool],
        suspends: usize,
        span: oxc_span::Span,
        body: impl FnOnce(&mut Self) -> Gen,
    ) -> Gen {
        let any_async = kinds.iter().any(|&a| a);
        if any_async && self.await_target().is_none() {
            return self.unsupported("await using", span);
        }
        let at = span.start;
        let m = self.mark();
        let token = self.temp();
        let value = self.temp();
        let ctx = self.temp();
        let mut resources = Vec::with_capacity(kinds.len());
        for &a in kinds {
            let value = self.temp();
            let method = self.temp();
            let fallback = if a { Some(self.temp()) } else { None };
            resources.push(UsingResource {
                value,
                method,
                fallback,
            });
        }
        // DisposeResources needsAwait and hasAwaited (an async region only).
        let flags = if any_async {
            Some((self.temp(), self.temp()))
        } else {
            None
        };
        // No resource is on the stack yet (also when a loop enters the region again).
        for r in &resources {
            if r.fallback.is_some() {
                self.b.lda_the_hole();
            } else {
                self.b.lda_undefined();
            }
            self.b.star(r.method);
        }
        self.b.lda_current_context().star(ctx);
        let (start, end, handler, entry) = (
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
            self.b.new_label(),
        );
        self.control.push(Control::Finally {
            token,
            value,
            entry,
            commands: Vec::new(),
        });
        self.using.push(UsingRegion {
            resources: resources.clone(),
            next: 0,
        });
        let before = self.b.instruction_count();
        self.b.bind(start);
        // On an error the unit is abandoned: the control stack no longer matters.
        body(self)?;
        let region = self.using.pop().expect("using regions out of order");
        if region.next != resources.len() {
            return Err(super::internal(format!(
                "using region at {at}: {} of {} resources bound",
                region.next,
                resources.len()
            )));
        }
        let Some(Control::Finally { commands, .. }) = self.control.pop() else {
            panic!("control stack out of order");
        };
        self.b.bind(end);
        if self.b.instruction_count() > before {
            self.b
                .add_handler(start, end, handler, ctx, HandlerKind::Uncaught);
        }
        self.b.lda_smi(-1).star(token).jump(entry);
        self.b.bind(handler);
        self.b.star(value).lda_zero().star(token);
        self.b.bind(entry);
        let message = self.temp();
        self.b.lda_the_hole().set_pending_message().star(message);
        let completion = Completion {
            token,
            value,
            message,
            ctx,
        };
        let first = self.generator.as_ref().map_or(0, |g| g.next);
        self.dispose_resources(&resources, completion, flags, at)?;
        let emitted = self.generator.as_ref().map_or(0, |g| g.next) - first;
        if emitted != suspends || suspends != suspends_of(kinds) {
            return Err(super::internal(format!(
                "using region at {at}: {emitted} await points emitted, {suspends} counted"
            )));
        }
        self.b.ldar(message).set_pending_message();
        let (table, labels) = self.b.jump_table(1 + commands.len() as u32);
        let after = self.b.new_label();
        self.b.ldar(token).switch_on_smi(table, 0);
        self.b.jump(after);
        self.b.bind(labels[0]);
        self.b.ldar(value).re_throw();
        for (i, &cmd) in commands.iter().enumerate() {
            self.b.bind(labels[i + 1]);
            if cmd == Command::Return {
                self.b.ldar(value);
            }
            self.perform_now(cmd);
        }
        self.b.bind(after);
        self.release(m);
        Ok(())
    }

    /// The statements of `list` as one using region (U1): `kinds` is `using_kinds(list)`, not
    /// empty.
    pub(crate) fn using_statements(
        &mut self,
        list: &'a oxc_allocator::Vec<'a, Statement<'a>>,
        kinds: &[bool],
    ) -> Gen {
        let span = list
            .iter()
            .find(|s| matches!(s, Statement::VariableDeclaration(v) if is_using(v.kind)))
            .map_or_else(oxc_span::Span::default, GetSpan::span);
        self.using_region(kinds, using_suspends(list), span, |g| {
            for s in list {
                g.statement(s)?;
            }
            Ok(())
        })
    }

    /// The next resource of the innermost using region, for a binding at `span`.
    fn next_using_resource(&mut self, span: oxc_span::Span) -> Gen<UsingResource> {
        let Some(region) = self.using.last_mut() else {
            return Err(super::internal(format!(
                "using binding outside a using region at {}",
                span.start
            )));
        };
        let Some(&r) = region.resources.get(region.next) else {
            return Err(super::internal(format!(
                "using binding at {} has no resource",
                span.start
            )));
        };
        region.next += 1;
        Ok(r)
    }

    /// `using x = e, ...` and `await using x = e, ...` (BindingEvaluation with the hint):
    /// each value, its resource, then its binding.
    fn using_decl(&mut self, decl: &'a VariableDeclaration<'a>) -> Gen {
        for d in &decl.declarations {
            self.stmt_pos(d.span);
            let Some(init) = &d.init else {
                return Err(super::internal(format!(
                    "using binding without an initializer at {}",
                    d.span.start
                )));
            };
            let r = self.next_using_resource(d.span)?;
            self.named_value(init, super::func::pattern_name(&d.id))?;
            self.add_resource(r, d.span.start);
            self.b.ldar(r.value);
            self.bind_value_in_acc(&d.id, BindMode::Init)?;
        }
        Ok(())
    }

    /// The `using` head of a `for-of` iteration: the value in `value` is its binding's value.
    fn using_head(&mut self, decl: &'a VariableDeclaration<'a>, value: Register) -> Gen {
        let d = &decl.declarations[0];
        let r = self.next_using_resource(d.span)?;
        self.b.ldar(value);
        self.add_resource(r, d.span.start);
        self.b.ldar(r.value);
        self.bind_value_in_acc(&d.id, BindMode::Init)
    }

    /// AddDisposableResource(capability, acc, hint) (CreateDisposableResource): stores the
    /// value in `r.value` and, unless a sync hint has undefined or null, its method.
    fn add_resource(&mut self, r: UsingResource, at: u32) {
        self.b.star(r.value);
        let m = self.mark();
        let args = self.temps(2);
        match r.fallback {
            None => {
                let done = self.b.new_label();
                self.b.jump_if_undefined_or_null(done);
                self.b.mov(r.value, args.get(0));
                self.b.lda_zero().star(args.get(1));
                self.expr_pos(at);
                self.runtime(RuntimeId::GetDisposeMethod, args);
                self.b.star(r.method);
                self.b.bind(done);
            }
            Some(fallback) => {
                let (null_case, have, done) =
                    (self.b.new_label(), self.b.new_label(), self.b.new_label());
                self.b.jump_if_undefined_or_null(null_case);
                self.b.mov(r.value, args.get(0));
                self.b.lda_smi(1).star(args.get(1));
                self.expr_pos(at);
                self.runtime(RuntimeId::GetDisposeMethod, args);
                self.b.jump_if_not_undefined(have);
                self.b.lda_zero().star(args.get(1));
                self.expr_pos(at);
                self.runtime(RuntimeId::GetDisposeMethod, args);
                self.b.star(r.method).lda_true().star(fallback).jump(done);
                self.b.bind(have);
                self.b.star(r.method).lda_false().star(fallback).jump(done);
                // On the stack with no method: the disposal only awaits (step 1.a.i-ii).
                self.b.bind(null_case);
                self.b.lda_undefined().star(r.method);
                self.b.lda_false().star(fallback);
                self.b.bind(done);
            }
        }
        self.release(m);
    }

    /// DisposeResources(capability, completion): the finalizer of a using region (U2, U3).
    fn dispose_resources(
        &mut self,
        resources: &[UsingResource],
        c: Completion,
        flags: Option<(Register, Register)>,
        at: u32,
    ) -> Gen {
        if let Some((needs_await, has_awaited)) = flags {
            self.b.lda_false().star(needs_await);
            self.b.lda_false().star(has_awaited);
        }
        for r in resources.iter().rev() {
            let next = self.b.new_label();
            match (r.fallback, flags) {
                (Some(fallback), Some((needs_await, has_awaited))) => {
                    let m = self.mark();
                    let kept = self.temp();
                    let threw = self.temp();
                    let result = self.temp();
                    let no_method = self.b.new_label();
                    self.b
                        .lda_the_hole()
                        .test_reference_equal(r.method)
                        .jump_if_true(next);
                    self.b.ldar(r.method).jump_if_undefined(no_method);
                    let (start, end, handler) =
                        (self.b.new_label(), self.b.new_label(), self.b.new_label());
                    let (sync_call, await_it, ok) =
                        (self.b.new_label(), self.b.new_label(), self.b.new_label());
                    self.b.bind(start);
                    self.b.lda_false().star(threw);
                    self.b.ldar(fallback).jump_if_true(sync_call);
                    let s = self.slot(FeedbackSlotKind::Call);
                    self.expr_pos(at);
                    self.b.call_property0(r.method, r.value, s);
                    self.b.jump(await_it);
                    // U3: the sync method; its throw is kept until after the Await.
                    self.b.bind(sync_call);
                    let (s2, e2, h2, d2) = (
                        self.b.new_label(),
                        self.b.new_label(),
                        self.b.new_label(),
                        self.b.new_label(),
                    );
                    self.b.bind(s2);
                    let s = self.slot(FeedbackSlotKind::Call);
                    self.expr_pos(at);
                    self.b.call_property0(r.method, r.value, s);
                    self.b.bind(e2);
                    self.b.add_handler(s2, e2, h2, c.ctx, HandlerKind::Caught);
                    self.b.jump(d2);
                    self.b.bind(h2);
                    self.b.star(kept).lda_the_hole().set_pending_message();
                    self.b.lda_true().star(threw);
                    self.b.bind(d2);
                    self.b.lda_undefined();
                    // Step 3.e.ii: Await(result); hasAwaited (nothing reads it in between).
                    self.b.bind(await_it);
                    self.b
                        .star(result)
                        .lda_true()
                        .star(has_awaited)
                        .ldar(result);
                    self.await_acc(at)?;
                    self.b.ldar(threw).jump_if_false(ok);
                    self.b.ldar(kept);
                    self.expr_pos(at);
                    self.b.throw();
                    self.b.bind(ok);
                    self.b.bind(end);
                    self.b
                        .add_handler(start, end, handler, c.ctx, HandlerKind::Caught);
                    self.b.jump(next);
                    self.b.bind(handler);
                    self.record_dispose_error(c);
                    self.b.jump(next);
                    // Step 3.f: an undefined or null value of an `await using`.
                    self.b.bind(no_method);
                    self.b.lda_true().star(needs_await);
                    self.release(m);
                }
                _ => {
                    self.b.ldar(r.method).jump_if_undefined(next);
                    if let Some((needs_await, has_awaited)) = flags {
                        self.await_if_needed(needs_await, has_awaited, at)?;
                    }
                    let (start, end, handler) =
                        (self.b.new_label(), self.b.new_label(), self.b.new_label());
                    self.b.bind(start);
                    let s = self.slot(FeedbackSlotKind::Call);
                    self.expr_pos(at);
                    self.b.call_property0(r.method, r.value, s);
                    self.b.bind(end);
                    self.b
                        .add_handler(start, end, handler, c.ctx, HandlerKind::Caught);
                    self.b.jump(next);
                    self.b.bind(handler);
                    self.record_dispose_error(c);
                }
            }
            self.b.bind(next);
        }
        if let Some((needs_await, has_awaited)) = flags {
            self.await_if_needed(needs_await, has_awaited, at)?;
        }
        Ok(())
    }

    /// DisposeResources steps 3.d and 4: if needsAwait and not hasAwaited, Await(undefined)
    /// and clear needsAwait.
    fn await_if_needed(&mut self, needs_await: Register, has_awaited: Register, at: u32) -> Gen {
        let go = self.b.new_label();
        self.b.ldar(needs_await).jump_if_false(go);
        self.b.ldar(has_awaited).jump_if_true(go);
        self.b.lda_undefined();
        self.await_acc(at)?;
        self.b.lda_false().star(needs_await);
        self.b.bind(go);
        Ok(())
    }

    /// DisposeResources step 3.e.iii with the throw value in the accumulator: a throw
    /// completion becomes `CreateSuppressedError(e, its value)`; any other becomes throw `e`.
    fn record_dispose_error(&mut self, c: Completion) {
        let m = self.mark();
        let args = self.temps(2);
        // Clear the pending message (V8): the exception is handled here.
        self.b
            .star(args.get(0))
            .lda_the_hole()
            .set_pending_message();
        self.b.lda_the_hole().star(c.message);
        let (was_throw, done) = (self.b.new_label(), self.b.new_label());
        self.b
            .lda_zero()
            .test_reference_equal(c.token)
            .jump_if_true(was_throw);
        self.b.ldar(args.get(0)).star(c.value);
        self.b.lda_zero().star(c.token).jump(done);
        self.b.bind(was_throw);
        self.b.mov(c.value, args.get(1));
        self.runtime(RuntimeId::CreateSuppressedError, args);
        self.b.star(c.value);
        self.b.bind(done);
        self.release(m);
    }
}

/// The completion registers of a using region's finalizer (G5).
#[derive(Clone, Copy)]
struct Completion {
    token: Register,
    value: Register,
    message: Register,
    ctx: Register,
}
