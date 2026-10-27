//! Async functions, async arrows, async methods and `await` (ECMA-262 §15.8, §27.7), in the
//! V8 model: an async function body is a generator body (`generator.rs`) with the generator
//! opcodes 0x99-0x9b, in a function with the `ASYNC` flag, and the ABI 5 runtime ids 32-35.
//!
//! ```text
//! prologue:  LdaNewTarget -> r_obj          fp-3: undefined on the first call, the async
//!            SwitchOnGeneratorState r_obj, T   function object in a resumed frame (FRAME 2)
//!            (first call falls through; no switch when the body has no `await`)
//!            AsyncFunctionEnter(closure, this) -> r_obj   the object and its promise (id 32)
//!            try {                                        handler kind Promise
//!              FunctionDeclarationInstantiation           a throw here rejects (§15.8.4)
//!              body
//!              return v:  AsyncFunctionResolve(r_obj, v) (id 34); Return
//!            } catch (e) {
//!              AsyncFunctionReject(r_obj, e) (id 35); Return
//!            }
//! await e:   AsyncFunctionAwait(r_obj, e) (id 33): subscribes the resume
//!            SuspendGenerator r_obj, r0..rN, N, k    the caller gets the promise (the VM
//!                                                    returns it for an async function)
//!       T[k] ResumeGenerator r_obj, r0..rN, N        acc = the settled value
//!            input = acc; SwitchOnSmi GeneratorGetResumeMode(r_obj)
//!              throw (fall through): throw input     a rejection
//!              next:  acc = input                    a fulfillment
//! ```
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | AF1 | One GENERATOR cell in async-function mode holds the frame state and the promise (`genf::ASYNC`); no new opcode | ABI 5 adds ids 32-35 only (P0-D5); V8 lowers async functions on the generator bytecodes too (`BuildAsyncFunction*`) |
//! | AF2 | `AsyncFunctionEnter` runs before FunctionDeclarationInstantiation, inside no handler; the handler starts right after it | §15.8.4 EvaluateAsyncFunctionBody: the promise exists first, and an abrupt FunctionDeclarationInstantiation rejects it (test262 `dflt-params-abrupt`) |
//! | AF3 | The suspend ids are only the `await` nodes (no initial suspend); the table length is counted before the body ([`AwaitCounter`]) | The body runs at once up to the first `await` (§27.7.5.1 AsyncFunctionStart) |
//! | AF4 | `return v` resolves at the frame's real exit (after the finally blocks: `emit_return`), not at the `return` statement | §27.7.5.2 AsyncBlockStart: the completion of the whole body resolves the promise; `try { return 1 } finally { throw 2 }` rejects |
//! | AF5 | `return` in an async function does not await `v`; the resolve function takes a thenable job | §27.7.5.2 step 3.f calls `promiseCapability.[[Resolve]]` (node: 3 ticks for a returned promise) |
//! | AF6 | `await` at a module top level stays `unsupported: await` (M3-P3 IMPORT-TLA) | Plan M3-P1 ASYNC task 1 |
//! | AF7 | One `await` lowering ([`FnGen::await_acc`]) for every await point of async code: an `await` expression, `for await` and the awaits of an async generator. The id is the body's: `AsyncFunctionAwait` (33) in an async function, `AsyncGeneratorAwait` (37) in an async generator (M3-P2 ASYNCGEN, plan task 7) | An async generator's resume runs through its request queue, not through the async function closures |
//! | AF8 | `for await` in an async function counts 3 suspend points: the await of each step and the two awaits of AsyncIteratorClose (normal and throw completion; `async_gen.rs`) | AF3: the table length is known before the body |
//!
//! An async generator (`async function*`) goes to `async_gen.rs`, not here; `for await` is in
//! `stmt.rs` (its iterator helpers in `async_gen.rs`).

use ep9_bytecode::{HandlerKind, Register, RuntimeId};
use oxc_ast::ast::{
    ArrowFunctionExpression, AwaitExpression, ForOfStatement, FormalParameters, Function,
    PropertyDefinition, StaticBlock,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::ScopeFlags;
use oxc_span::Span;

use super::func::{FnGen, Gen};
use super::generator::{GenKind, GenState};

/// Resume mode next (`GeneratorGetResumeMode`, M2-P1-GEN-NUMBERS); a rejected `await`
/// resumes with throw (2). An async function is never resumed with return.
const RESUME_NEXT: i32 = 0;

/// The suspend points of one `for await` statement (AF8).
pub(crate) const FOR_AWAIT_SUSPENDS: usize = 3;

/// Counts the await points that the body itself evaluates (AF3, AF8), as `generator.rs`
/// `SuspendCounter` counts `yield`: nested functions, arrows, class element bodies and static
/// blocks are their own functions; class computed keys and heritage run in this function.
struct AwaitCounter(usize);

impl<'a> Visit<'a> for AwaitCounter {
    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.0 += 1;
        walk::walk_await_expression(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        if it.r#await {
            self.0 += FOR_AWAIT_SUSPENDS;
        }
        walk::walk_for_of_statement(self, it);
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

/// The body of an async function: a function body or an arrow's concise body.
enum Body<'a> {
    Function(&'a Function<'a>),
    Arrow(&'a ArrowFunctionExpression<'a>),
}

impl<'a> FnGen<'a, '_> {
    /// The check before a closure is made for an async function node that is not a generator
    /// (a declaration, an expression, a method, an arrow): `Ok` lets the caller make it.
    pub(crate) fn async_closure(&self, _span: Span) -> Gen {
        Ok(())
    }

    /// The body of an async function or async method (not a generator).
    pub(crate) fn gen_async_function(&mut self, node: &'a Function<'a>) -> Gen {
        self.async_body(&node.params, node.span, Body::Function(node))
    }

    /// The body of an async arrow function.
    pub(crate) fn gen_async_arrow(&mut self, node: &'a ArrowFunctionExpression<'a>) -> Gen {
        self.async_body(&node.params, node.span, Body::Arrow(node))
    }

    /// The async function object register of an async function body, else `None`.
    pub(crate) fn async_object(&self) -> Option<Register> {
        self.generator
            .as_ref()
            .and_then(|g| (g.kind == GenKind::AsyncFunction).then_some(g.object))
    }

    /// The object register and the `await` id of an async function or async generator body
    /// (AF7), else `None`.
    pub(crate) fn await_target(&self) -> Option<(Register, RuntimeId)> {
        let g = self.generator.as_ref()?;
        match g.kind {
            GenKind::AsyncFunction => Some((g.object, RuntimeId::AsyncFunctionAwait)),
            GenKind::AsyncGenerator => Some((g.object, RuntimeId::AsyncGeneratorAwait)),
            GenKind::Sync => None,
        }
    }

    /// The module comment's lowering: the prologue, `AsyncFunctionEnter`, then the
    /// parameters and the body inside the handler that rejects the promise.
    fn async_body(&mut self, params: &'a FormalParameters<'a>, span: Span, body: Body<'a>) -> Gen {
        let mut counter = AwaitCounter(0);
        match body {
            Body::Function(f) => {
                if let Some(b) = &f.body {
                    counter.visit_function_body(b);
                }
            }
            Body::Arrow(a) => counter.visit_arrow_function_body(&a.body),
        }
        let count =
            u32::try_from(counter.0).map_err(|_| super::internal("too many await points"))?;

        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        let object = self.temp();
        let resume = if count > 0 {
            // A resume jumps to its await point; the first call falls through.
            self.b.lda_new_target().star(object);
            let (table, resume) = self.b.jump_table(count);
            self.b.switch_on_generator_state(object, table);
            resume
        } else {
            Vec::new()
        };
        self.generator = Some(GenState::new(object, resume, GenKind::AsyncFunction));
        // §15.8.4 step 1 (AF2): the async function object and its promise.
        let m = self.mark();
        let args = self.temps(2);
        self.b.lda_current_function().star(args.get(0));
        self.b.mov(Register::RECEIVER, args.get(1));
        self.expr_pos(span.start);
        self.runtime(RuntimeId::AsyncFunctionEnter, args);
        self.release(m);
        self.b.star(object);

        // The handler that rejects the promise covers the parameters and the body.
        let ctx = self.temp();
        self.b.lda_current_context().star(ctx);
        let (start, end, handler) = (self.b.new_label(), self.b.new_label(), self.b.new_label());
        self.b.bind(start);
        self.prologue_and_params(params)?;
        match body {
            Body::Function(f) => {
                if let Some(b) = &f.body {
                    self.statements(&b.statements)?;
                }
                // The implicit return reports the closing brace.
                self.b.set_throw_position(Some(span.end.saturating_sub(1)));
                self.b.lda_undefined();
                self.emit_return();
            }
            Body::Arrow(a) => {
                if let Some(expr) = a.get_expression() {
                    self.b
                        .set_statement_position(oxc_span::GetSpan::span(expr).start);
                    self.expr(expr)?;
                } else if let Some(b) = a.get_function_body() {
                    self.statements(&b.statements)?;
                    self.b.lda_undefined();
                }
                self.emit_return();
            }
        }
        self.b.bind(end);
        self.b
            .add_handler(start, end, handler, ctx, HandlerKind::Promise);
        self.b.bind(handler);
        // A throw completion of the body (§27.7.5.2 step 3.g): reject, then leave the frame.
        let m = self.mark();
        let args = self.temps(2);
        // Clear the pending message (V8): the exception is handled.
        self.b
            .star(args.get(1))
            .lda_the_hole()
            .set_pending_message();
        self.b.mov(object, args.get(0));
        self.runtime(RuntimeId::AsyncFunctionReject, args);
        self.release(m);
        self.b.return_();

        let state = self.generator.take().expect("set above");
        if !state.complete() {
            return Err(super::internal(format!(
                "async function at {}: {} await points emitted, {} counted",
                span.start,
                state.next,
                state.resume.len()
            )));
        }
        Ok(())
    }

    /// The return of an async function body with the value in the accumulator (AF4, AF5):
    /// resolve the promise, then return it (the first call's caller gets it).
    pub(crate) fn async_return(&mut self, object: Register) {
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(1));
        self.b.mov(object, args.get(0));
        self.runtime(RuntimeId::AsyncFunctionResolve, args);
        self.release(m);
        self.b.return_();
    }

    /// `await e` (§27.7.5.3 Await), in an async function or async generator body. Leaves the
    /// settled value in the accumulator, or throws the rejection reason. At a module top level
    /// (or anywhere else outside async code) it stays unsupported (AF6).
    pub(crate) fn await_expr(&mut self, a: &'a AwaitExpression<'a>) -> Gen {
        if self.await_target().is_none() {
            return self.unsupported("await", a.span);
        }
        self.expr(&a.argument)?;
        self.await_acc(a.span.start)
    }

    /// Await(acc) (§6.2.3.1) at `at` (AF7): leaves the settled value in the accumulator, or
    /// throws the rejection reason there. One suspend point.
    pub(crate) fn await_acc(&mut self, at: u32) -> Gen {
        let Some((object, id)) = self.await_target() else {
            return Err(super::internal(format!("await outside async code at {at}")));
        };
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(1));
        self.b.mov(object, args.get(0));
        self.expr_pos(at);
        self.runtime(id, args);
        self.release(m);
        let m = self.mark();
        // The highest live register + 1: every register below `input` is saved.
        let input = self.temp();
        let live = input.local_index().expect("a local");
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        let (table, cases) = self.b.jump_table(1);
        self.b.switch_on_smi(table, RESUME_NEXT);
        // Throw (the fall-through): a rejected promise.
        self.b.ldar(input);
        self.expr_pos(at);
        self.b.throw();
        // Next: a fulfilled promise.
        self.b.bind(cases[0]);
        self.b.ldar(input);
        self.release(m);
        Ok(())
    }
}
