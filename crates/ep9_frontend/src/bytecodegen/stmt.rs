//! Statements: blocks, declarations, control flow, loops (incl. for-in and for-of), switch,
//! try/catch/finally, with, labels.

use ep9_bytecode::{
    FeedbackSlotKind, HandlerKind, Label, Register, RegisterList, RuntimeId, UnitString,
};
use oxc_ast::ast::{
    BindingPattern, BlockStatement, CatchClause, Class, DoWhileStatement,
    ExportDefaultDeclarationKind, ForInStatement, ForOfStatement, ForStatement, ForStatementInit,
    ForStatementLeft, Function, IfStatement, Statement, SwitchStatement, TryStatement,
    VariableDeclaration, VariableDeclarationKind, WhileStatement, WithStatement,
};
use oxc_span::GetSpan;

use super::func::{Command, Control, FnGen, Gen, TargetKind};
use super::pattern::BindMode;
use crate::scope::{BindingId, BindingLocation, ImplicitRef, PerIteration, ScopeId};

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
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportNamedDeclaration(_) => Ok(()),
            Statement::ExportDeclaration(e) => self.export_declaration(e),
            Statement::ExportDefaultDeclaration(e) => self.export_default(e),
            Statement::ExportFromDeclaration(_) => Ok(()),
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
        if matches!(
            decl.kind,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        ) {
            return self.unsupported("using declarations", decl.span);
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
                self.named_value(init, super::func::pattern_name(&d.id))?;
                self.bind_value_in_acc(&d.id, BindMode::Var)?;
            }
        }
        Ok(())
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

    fn function_declaration(&mut self, f: &'a Function<'a>) -> Gen {
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

    fn export_declaration(&mut self, e: &'a oxc_ast::ast::ExportDeclaration<'a>) -> Gen {
        use oxc_ast::ast::Declaration as D;
        match &e.declaration {
            D::VariableDeclaration(v) => self.var_decl(v),
            D::FunctionDeclaration(f) => self.function_declaration(f),
            D::ClassDeclaration(c) => self.class_declaration(c, None),
            _ => self.unsupported("TypeScript declarations", e.span),
        }
    }

    fn export_default(&mut self, e: &'a oxc_ast::ast::ExportDefaultDeclaration<'a>) -> Gen {
        match &e.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => self.function_declaration(f),
            ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                let default = c.id.is_none().then(|| self.default_binding()).transpose()?;
                self.class_declaration(c, default)
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => Ok(()),
            other => {
                let Some(expr) = other.as_expression() else {
                    return self.unsupported("export default form", e.span);
                };
                self.stmt_pos(e.span);
                self.named_value(expr, Some(UnitString::new("default")))?;
                let b = self.default_binding()?;
                self.store_binding_init(b);
                Ok(())
            }
        }
    }

    /// The `*default*` binding of a module.
    fn default_binding(&self) -> Gen<BindingId> {
        self.sa
            .scope(ScopeId(0))
            .bindings
            .iter()
            .copied()
            .find(|&b| self.sa.name(self.binding(b).name) == "*default*")
            .ok_or_else(|| super::internal("no *default* binding"))
    }

    fn class_declaration(&mut self, c: &'a Class<'a>, binding: Option<BindingId>) -> Gen {
        self.stmt_pos(c.span);
        let name = c.id.as_ref().map_or_else(
            || UnitString::new("default"),
            |id| UnitString::new(id.name.as_str()),
        );
        self.class(c, Some(name))?;
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
        self.exit_scope(scope, pushed);
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
                if matches!(
                    v.kind,
                    VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
                ) {
                    return self.unsupported("using declarations", v.span);
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

    /// `for (var x = init in o)` (Annex B): the initializer runs first.
    fn for_in_var_initializer(&mut self, left: &'a ForStatementLeft<'a>) -> Gen {
        if let ForStatementLeft::VariableDeclaration(v) = left
            && let Some(d) = v.declarations.first()
            && let Some(init) = &d.init
        {
            self.named_value(init, super::func::pattern_name(&d.id))?;
            self.bind_value_in_acc(&d.id, BindMode::Var)?;
        }
        Ok(())
    }

    fn for_in(&mut self, f: &'a ForInStatement<'a>, labels: Vec<&'a str>) -> Gen {
        self.stmt_pos(f.span);
        let scope = self.scope_of(f.scope_id.get(), f.span)?;
        self.for_in_var_initializer(&f.left)?;
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
            return self.unsupported("for await", f.span);
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
                let result = (|| -> Gen {
                    let pushed = g.enter_scope(scope);
                    g.for_head_assign(&f.left, value)?;
                    g.statement(&f.body)?;
                    g.exit_scope(scope, pushed);
                    Ok(())
                })();
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

    /// IteratorClose (V8 BuildFinalizeIteration): token 0 means a throw completion, so errors
    /// from `return()` are ignored; otherwise the result must be an object.
    pub(crate) fn close_iterator(&mut self, iter: Register, token: Register) {
        let m = self.mark();
        let method = self.temp();
        let done = self.b.new_label();
        let k = self.str_const("return");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        let call = self.slot(FeedbackSlotKind::Call);
        let throw_path = self.b.new_label();
        self.b
            .lda_zero()
            .test_reference_equal(token)
            .jump_if_true(throw_path);
        // Normal completion: the result must be an object.
        self.b.call_property0(method, iter, call);
        self.b.jump_if_js_receiver(done);
        let r = self.temp();
        self.b.star(r);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[r]);
        self.b.jump(done);
        // Throw completion: call, ignore any exception.
        self.b.bind(throw_path);
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler) = (self.b.new_label(), self.b.new_label(), self.b.new_label());
        self.b.bind(start);
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
