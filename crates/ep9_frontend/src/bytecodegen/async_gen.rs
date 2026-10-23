//! Async generators (`async function*`, ECMA-262 §15.6, §27.6) and the async iteration
//! helpers of `for await` (§14.7.5) and `yield*`, in the V8 model: an async generator body is a
//! generator body (`generator.rs`) with the generator opcodes 0x99-0x9b, in a function with
//! the `GENERATOR` and `ASYNC` flags, and the ABI 5 runtime ids 36-41 (M3-P2 ASYNCGEN).
//!
//! ```text
//! prologue:  LdaNewTarget -> r_gen; SwitchOnGeneratorState r_gen, T   (as a generator)
//!            FunctionDeclarationInstantiation     a throw here throws to the caller (§15.6.2)
//!            AsyncGeneratorEnter(closure, this) -> r_gen    id 36: object, record, queue
//!            initial suspend: acc = r_gen, suspend id 0     the first call returns the object
//!            try {                                          handler kind Promise
//!              body
//!              end / return v: AsyncGeneratorResolve(r_gen, v, true) (id 39); Return
//!            } catch (e) {
//!              AsyncGeneratorReject(r_gen, e) (id 40); Return
//!            }
//! await e:   AsyncGeneratorAwait(r_gen, e) (id 37), suspend, dispatch (`async_fn.rs` AF7)
//! yield e:   v = Await(e); AsyncGeneratorYield(r_gen, v) (id 38); suspend
//!              throw: throw input;  next: acc = input
//!              return: v = Await(input) (AsyncGeneratorUnwrapYieldResumption); return v
//! yield* e:  the loop below ([`FnGen::async_yield_star`])
//! for await: `stmt.rs`; GetIterator(e, async) is id 41 with the hole (AGF5)
//! ```
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | AGF1 | The body has the initial suspend of a generator (suspend id 0, state suspended-start); `AsyncGeneratorEnter` runs after FunctionDeclarationInstantiation, outside the handler | §15.6.2 EvaluateAsyncGeneratorBody: FunctionDeclarationInstantiation first, abrupt to the caller; the body starts at the first `next` (§27.6.3.2) |
//! | AGF2 | Suspend points per node: `await` 1, `yield` 3 (await the value, the yield, await a return resumption), `yield*` 5, `for await` 3 (AF8), `return Expression;` 1, plus the initial one; counted before the body ([`AsyncGenCounter`]) | GEN2 / AF3: the table length is known when the prologue switch is emitted |
//! | AGF3 | A `yield` with requests already queued still suspends; the VM runner resumes the frame at once (`interp/slow/async_gen.rs` AG2) | One code shape per `yield` |
//! | AGF4 | `return Expression;` awaits at the statement (§14.10.1 step 3), before the finally blocks; `return;`, a return resumption and a finally replay do not. `perform` knows the statement by the last statement position ([`FnGen::stmt_pos`]): the counter lists the start of every `return Expression;` of the body | The `return` arm of `stmt.rs` is not this role's (plan §4.2 owns only the for-await arms there); the start offset names one statement, and an expression evaluates no statement of its own function. test262 `return-undefined-implicit-and-explicit.js` pins the tick of each form |
//! | AGF5 | GetIterator(obj, async) is `CreateAsyncFromSyncIterator(obj, hole)` (VM AG3), then `next` is read from the result | ABI 5 has no symbol constant to load `@@asyncIterator` from bytecode, and a global `Symbol` lookup is observable |
//! | AGF6 | `yield*` in an async generator: one `Await` after the `next`/`throw`/`return` call; a return resumption awaits its value and a rejection there becomes a throw resumption of the loop (§15.5.5 with AsyncGeneratorUnwrapYieldResumption); a missing `return` awaits the value (step 7.c.iii.1); a `done` result of `return` is not awaited | node v24.22.1 (probe: a `then` getter on the value is read twice for a missing `return`, never for a done `return` result) |

use ep9_bytecode::{FeedbackSlotKind, HandlerKind, Register, RuntimeId};
use oxc_ast::ast::{
    ArrowFunctionExpression, AwaitExpression, Expression, ForOfStatement, Function,
    PropertyDefinition, ReturnStatement, StaticBlock, YieldExpression,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::ScopeFlags;
use oxc_span::{GetSpan, Span};

use super::async_fn::FOR_AWAIT_SUSPENDS;
use super::func::{Command, FnGen, Gen};
use super::generator::{GenKind, GenState, RESUME_NEXT, RESUME_RETURN, RESUME_THROW};

/// The suspend points of one `yield` in an async generator (AGF2).
const YIELD_SUSPENDS: usize = 3;
/// The suspend points of one `yield*` in an async generator (AGF2, AGF6).
const YIELD_STAR_SUSPENDS: usize = 5;

/// Counts the suspend points that an async generator body itself evaluates (AGF2) and lists
/// its `return Expression;` statements (AGF4). Nested functions, arrows, class element
/// bodies and static blocks are their own functions; class computed keys and heritage run in
/// this function (as `generator.rs` `SuspendCounter`).
struct AsyncGenCounter {
    count: usize,
    returns: Vec<u32>,
}

impl<'a> Visit<'a> for AsyncGenCounter {
    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.count += 1;
        walk::walk_await_expression(self, it);
    }

    fn visit_yield_expression(&mut self, it: &YieldExpression<'a>) {
        self.count += if it.delegate {
            YIELD_STAR_SUSPENDS
        } else {
            YIELD_SUSPENDS
        };
        walk::walk_yield_expression(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        if it.r#await {
            self.count += FOR_AWAIT_SUSPENDS;
        }
        walk::walk_for_of_statement(self, it);
    }

    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if it.argument.is_some() {
            self.count += 1;
            self.returns.push(it.span.start);
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_function(&mut self, _it: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.computed {
            self.visit_property_key(&it.key);
        }
    }

    fn visit_static_block(&mut self, _it: &StaticBlock<'a>) {}
}

impl<'a> FnGen<'a, '_> {
    /// The check before a closure is made for an async generator node (a declaration, an
    /// expression or a method): `Ok` lets the caller make it.
    pub(crate) fn async_generator_closure(&self, _span: Span) -> Gen {
        Ok(())
    }

    /// The async generator object register of an async generator body, else `None`.
    pub(crate) fn async_generator_object(&self) -> Option<Register> {
        self.generator
            .as_ref()
            .and_then(|g| (g.kind == GenKind::AsyncGenerator).then_some(g.object))
    }

    /// The body of an async generator function or method: the module comment's lowering.
    pub(crate) fn gen_async_generator(&mut self, node: &'a Function<'a>) -> Gen {
        let mut counter = AsyncGenCounter {
            count: 1,
            returns: Vec::new(),
        };
        counter.visit_formal_parameters(&node.params);
        if let Some(body) = &node.body {
            counter.visit_function_body(body);
        }
        let count =
            u32::try_from(counter.count).map_err(|_| super::internal("too many suspend points"))?;
        let mut returns = counter.returns;
        returns.sort_unstable();

        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        // Prologue: a resume jumps to its suspend point; the first call falls through.
        let object = self.temp();
        self.b.lda_new_target().star(object);
        let (table, resume) = self.b.jump_table(count);
        self.b.switch_on_generator_state(object, table);
        let mut state = GenState::new(object, resume, GenKind::AsyncGenerator);
        state.awaited_returns = returns;
        self.generator = Some(state);
        self.prologue_and_params(&node.params)?;
        // §15.6.2 steps 2-3 (AGF1): the object, after the parameters.
        let m = self.mark();
        let args = self.temps(2);
        self.b.lda_current_function().star(args.get(0));
        self.b.mov(Register::RECEIVER, args.get(1));
        self.expr_pos(node.span.start);
        self.runtime(RuntimeId::AsyncGeneratorEnter, args);
        self.release(m);
        self.b.star(object);
        // The initial suspend returns the object; the first `next` resumes it (its value
        // is dropped, §27.6.3.2).
        let m = self.mark();
        let input = self.temp();
        let live = input.local_index().expect("a local");
        self.b.ldar(object);
        self.suspend(live, node.span.start)?;
        self.release(m);

        // The handler that rejects the first request covers the body.
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler) = (self.b.new_label(), self.b.new_label(), self.b.new_label());
        self.b.bind(start);
        if let Some(body) = &node.body {
            self.statements(&body.statements)?;
        }
        // The implicit return reports the closing brace.
        self.b
            .set_throw_position(Some(node.span.end.saturating_sub(1)));
        self.b.lda_undefined();
        self.emit_return();
        self.b.bind(end);
        self.b
            .add_handler(start, end, handler, ctx, HandlerKind::Promise);
        self.b.bind(handler);
        // A throw completion of the body (§27.6.3.2 step 4): reject, then leave the frame.
        let m = self.mark();
        let args = self.temps(2);
        // Clear the pending message (V8): the exception is handled.
        self.b
            .star(args.get(1))
            .lda_the_hole()
            .set_pending_message();
        self.b.mov(object, args.get(0));
        self.runtime(RuntimeId::AsyncGeneratorReject, args);
        self.release(m);
        self.b.return_();

        let state = self.generator.take().expect("set above");
        if !state.complete() {
            return Err(super::internal(format!(
                "async generator at {}: {} suspend points emitted, {} counted",
                node.span.start,
                state.next,
                state.resume.len()
            )));
        }
        Ok(())
    }

    /// The return of an async generator body with the value in the accumulator: the body
    /// completed (AsyncGeneratorResolve with done = true), then the frame returns.
    pub(crate) fn async_generator_return(&mut self, object: Register) {
        let m = self.mark();
        let args = self.temps(3);
        self.b.star(args.get(1));
        self.b.mov(object, args.get(0));
        self.b.lda_true().star(args.get(2));
        self.runtime(RuntimeId::AsyncGeneratorResolve, args);
        self.release(m);
        self.b.return_();
    }

    /// AGF4: in an async generator, the `return Expression;` statement that starts at `at`
    /// awaits the value in the accumulator. A suspend point that was not counted marks the
    /// state (`perform` has no error path; the count check fails at the end of the body).
    pub(crate) fn await_return_statement(&mut self, at: u32) {
        let awaits = self.generator.as_ref().is_some_and(|g| {
            g.kind == GenKind::AsyncGenerator && g.awaited_returns.binary_search(&at).is_ok()
        });
        if awaits && self.await_acc(at).is_err() {
            if let Some(g) = self.generator.as_mut() {
                g.uncounted = true;
            }
        }
    }

    /// `yield` and `yield*` in an async generator (the module comment). Leaves the value of
    /// the expression in the accumulator.
    pub(crate) fn async_yield_expr(&mut self, y: &'a YieldExpression<'a>) -> Gen {
        let at = y.span.start;
        if y.delegate {
            let Some(arg) = &y.argument else {
                return Err(super::internal("yield* without an operand"));
            };
            return self.async_yield_star(arg, at);
        }
        let object = self.generator_object(y.span)?;
        match &y.argument {
            Some(e) => self.expr(e)?,
            None => {
                self.b.lda_undefined();
            }
        }
        // §27.5.3.7 Yield step 2 (async): AsyncGeneratorYield(? Await(value)).
        self.await_acc(at)?;
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(1));
        self.b.mov(object, args.get(0));
        self.expr_pos(at);
        self.runtime(RuntimeId::AsyncGeneratorYield, args);
        self.release(m);
        let m = self.mark();
        // The highest live register + 1: every register below `input` is saved.
        let input = self.temp();
        let live = input.local_index().expect("a local");
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        let (table, cases) = self.b.jump_table(2);
        self.b.switch_on_smi(table, RESUME_NEXT);
        // Throw (the fall-through).
        self.b.ldar(input);
        self.expr_pos(at);
        self.b.throw();
        // Return: AsyncGeneratorUnwrapYieldResumption awaits the value (§27.6.3.7), then
        // returns it through the finally blocks.
        self.b.bind(cases[1]);
        self.b.ldar(input);
        self.await_acc(at)?;
        self.perform_now(Command::Return);
        // Next.
        self.b.bind(cases[0]);
        self.b.ldar(input);
        self.release(m);
        Ok(())
    }

    /// `iter = GetIterator(acc, async)` (§7.4.3, AGF5), `next = iter.next`.
    pub(crate) fn get_async_iterator(&mut self, iter: Register, next: Register, at: u32) {
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(0));
        self.b.lda_the_hole().star(args.get(1));
        self.expr_pos(at);
        self.runtime(RuntimeId::CreateAsyncFromSyncIterator, args);
        self.b.star(iter);
        let k = self.str_const("next");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(iter, k, s).star(next);
        self.release(m);
    }

    /// AsyncIteratorClose(iter, completion) (§7.4.13) in a `for await` finalizer: token 0 is
    /// a throw completion (errors of the `return` lookup, its call and its awaited result are
    /// ignored; the original exception wins); otherwise they propagate and the awaited result
    /// must be an object. Two suspend points (AF8).
    pub(crate) fn close_async_iterator(&mut self, iter: Register, token: Register, at: u32) -> Gen {
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
        // Normal completion: the awaited result must be an object.
        self.expr_pos(at);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        self.expr_pos(at);
        self.b.call_property0(method, iter, call);
        self.await_acc(at)?;
        self.b.jump_if_js_receiver(done);
        let r = self.temp();
        self.b.star(r);
        self.expr_pos(at);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[r]);
        self.b.jump(done);
        // Throw completion: look up, call and await; ignore any exception.
        self.b.bind(throw_path);
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler) = (self.b.new_label(), self.b.new_label(), self.b.new_label());
        self.b.bind(start);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        self.b.call_property0(method, iter, call);
        self.await_acc(at)?;
        self.b.bind(end);
        self.b
            .add_handler(start, end, handler, ctx, HandlerKind::Caught);
        self.b.jump(done);
        self.b.bind(handler);
        self.b.bind(done);
        self.release(m);
        Ok(())
    }

    /// `yield* e` in an async generator (§15.5.5 with generatorKind async; AGF6):
    ///
    /// ```text
    /// iter, next = GetIterator(e, async); input = undefined; mode = next
    /// loop:
    ///   switch mode
    ///     next:   output = next.call(iter, input)
    ///     return: m = iter.return; if m is undefined or null: return Await(input)
    ///             output = m.call(iter, input)
    ///     throw:  m = iter.throw; if m is undefined or null:
    ///               AsyncIteratorClose(iter, normal); throw TypeError (ThrowThrowMethodMissing)
    ///             output = m.call(iter, input)
    ///   output = Await(output)
    ///   if output is not an object: throw TypeError
    ///   if output.done: break
    ///   AsyncGeneratorYield(output.value); input = suspend; mode = GeneratorGetResumeMode
    ///   if mode == return: input = Await(input), a rejection: mode = throw, input = reason
    /// value = output.value
    /// if mode == return: return value (through the finally blocks)
    /// acc = value
    /// ```
    fn async_yield_star(&mut self, arg: &'a Expression<'a>, at: u32) -> Gen {
        let object = self.generator_object(Span::new(at, at))?;
        let m = self.mark();
        let output = self.temp();
        let mode = self.temp();
        let iter = self.temp();
        let input = self.temp();
        let next = self.temp();
        let method = self.temp();
        self.expr(arg)?;
        self.get_async_iterator(iter, next, arg.span().start);
        self.b.lda_undefined().star(input);
        self.smi(RESUME_NEXT);
        self.b.star(mode);

        self.loop_depth += 1;
        let header = self.b.bound_label();
        let (after_switch, exit, back) =
            (self.b.new_label(), self.b.new_label(), self.b.new_label());
        let (table, cases) = self.b.jump_table(2);
        self.b.ldar(mode).switch_on_smi(table, RESUME_RETURN);
        // Next (the fall-through).
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.call_property1(next, iter, input, call);
        self.b.jump(after_switch);
        // Return.
        self.b.bind(cases[0]);
        let no_return = self.b.new_label();
        self.call_iterator_method(iter, input, method, "return", after_switch, no_return, at);
        self.b.bind(no_return);
        // Step 7.c.iii: the received value is awaited, then returned.
        self.b.ldar(input);
        self.await_acc(at)?;
        self.perform_now(Command::Return);
        // Throw.
        self.b.bind(cases[1]);
        let no_throw = self.b.new_label();
        self.call_iterator_method(iter, input, method, "throw", after_switch, no_throw, at);
        self.b.bind(no_throw);
        // Step 7.b.iii: AsyncIteratorClose(iteratorRecord, normal), then the TypeError.
        let closed = self.b.new_label();
        let k = self.str_const("return");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(closed);
        self.b.star(method);
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.call_property0(method, iter, call);
        self.await_acc(at)?;
        self.b.jump_if_js_receiver(closed);
        self.b.star(method);
        self.expr_pos(at);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[method]);
        self.b.bind(closed);
        self.expr_pos(at);
        self.runtime(RuntimeId::ThrowThrowMethodMissing, ep9_bytecode::RegisterList::EMPTY);
        // Await the inner result; it must be an object; stop when it is done.
        self.b.bind(after_switch);
        self.await_acc(at)?;
        self.b.star(output);
        let is_object = self.b.new_label();
        self.b.jump_if_js_receiver(is_object);
        self.expr_pos(at);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[output]);
        self.b.bind(is_object);
        let k_done = self.str_const("done");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(output, k_done, s);
        self.b.jump_if_to_boolean_true(exit);
        // AsyncGeneratorYield(? IteratorValue(output)) (no Await before it).
        let k_value = self.str_const("value");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(output, k_value, s);
        let m2 = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(1));
        self.b.mov(object, args.get(0));
        self.expr_pos(at);
        self.runtime(RuntimeId::AsyncGeneratorYield, args);
        self.release(m2);
        // Every register up to `method` is live.
        let live = method.local_index().expect("a local") + 1;
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        self.b.star(mode);
        self.smi(RESUME_RETURN);
        self.b.test_reference_equal(mode).jump_if_false(back);
        // A return resumption (AsyncGeneratorUnwrapYieldResumption): await the value; a
        // rejection is a throw resumption.
        let m2 = self.mark();
        let args = self.temps(2);
        self.b.ldar(input).star(args.get(1));
        self.b.mov(object, args.get(0));
        self.expr_pos(at);
        self.runtime(RuntimeId::AsyncGeneratorAwait, args);
        self.release(m2);
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        let (table, fulfilled) = self.b.jump_table(1);
        self.b.switch_on_smi(table, RESUME_NEXT);
        self.smi(RESUME_THROW);
        self.b.star(mode);
        self.b.bind(fulfilled[0]);
        self.b.bind(back);
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;

        // The value; a return completion returns it (not awaited, AGF6).
        let k_value = self.str_const("value");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(output, k_value, s).star(output);
        let is_value = self.b.new_label();
        self.smi(RESUME_RETURN);
        self.b.test_reference_equal(mode).jump_if_false(is_value);
        self.b.ldar(output);
        self.perform_now(Command::Return);
        self.b.bind(is_value);
        self.b.ldar(output);
        self.release(m);
        Ok(())
    }
}
