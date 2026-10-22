//! Generators (`function*`, `yield`, `yield*`), in the V8 model (M2-P1-GEN-CG).
//!
//! ```text
//! prologue:  LdaNewTarget -> r_gen       fp-3: undefined on the first call, the generator object
//!                                        in a resumed frame (FRAME 2)
//!            SwitchOnGeneratorState r_gen, T     resume: jump to T[suspend id]
//!            (first call falls through)
//!            FunctionDeclarationInstantiation    params, arguments, hoisted functions
//!            CreateGeneratorObject(closure, this) -> r_gen    after the params (§15.5.2)
//!            initial suspend: acc = r_gen, suspend id 0 (the first call returns the object)
//! suspend k: SuspendGenerator r_gen, r0..rN, N, k     saves the N live registers
//!       T[k] ResumeGenerator r_gen, r0..rN, N         restores them; acc = the sent value
//!            input = acc; SwitchOnSmi GeneratorGetResumeMode(r_gen)
//!              throw (fall through): throw input
//!              return: input, then Command::Return (through the finally tokens, G5)
//!              next:   acc = input
//! yield v:   CreateIterResultObject(v, false), then suspend (the natives wrap only the final
//!            `{value, done: true}`: M2-P1-GEN-NUMBERS)
//! yield* e:  the V8 delegating loop (VisitYieldStar): the inner result object is passed
//!            through unwrapped
//! ```
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | GEN1 | Resume points are jump-table entries of the one `SwitchOnGeneratorState`, also inside loops. No loop-header `SwitchOnSmi` (plan task 5 not done, M2-P1-GEN-LOOPS-DIRECT) | Every entry is a forward jump from the function start, so H02 F6 holds as it is. V8 dropped the loop-header dispatch too: its bytecode binds each resume point straight into the prologue table (bytecode-generator.cc `BuildSuspendPoint`), and TurboFan rebuilds loop entries from `BytecodeAnalysis` |
//! | GEN2 | The table length is known before the body: 1 (the initial suspend) + the `yield` nodes of the body outside nested functions, counted by [`SuspendCounter`]. Each `yield` and `yield*` emits exactly one suspend; `gen_generator` checks the count at the end | `Builder::jump_table` needs the length when the prologue switch is emitted; BytecodeGen emits every AST node once and removes no dead code |
//! | GEN3 | A suspend saves `r0` up to the highest live register (all named locals and live temporaries; no liveness analysis in v0) | Registers above the allocation mark are dead (stack-allocated temporaries, G1) |
//! | GEN4 | The generator object is created after FunctionDeclarationInstantiation | §15.5.2 EvaluateGeneratorBody steps 1-2 (test262 `generator-created-after-decl-inst.js`; V8 creates it before the parameter defaults run there) |
//! | GEN5 | `new.target` in a generator body is `LdaUndefined` (func.rs `load_new_target`); the prologue `LdaNewTarget` is the only one | fp-3 of a resumed frame is the generator object |
//! | GEN6 | An async generator keeps the "generator functions" text | CAUSE rule: the async-generator files stay under that cause until month 3 |

use ep9_bytecode::{FeedbackSlotKind, Label, Register, RegisterList, RuntimeId};
use oxc_ast::ast::{
    ArrowFunctionExpression, Expression, Function, PropertyDefinition, StaticBlock, YieldExpression,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::ScopeFlags;
use oxc_span::{GetSpan, Span};

use super::func::{Command, FnGen, Gen};

/// Resume modes (`GeneratorGetResumeMode`, `genf::RESUME_MODE`; M2-P1-GEN-NUMBERS, V8 values).
const RESUME_NEXT: i32 = 0;
const RESUME_RETURN: i32 = 1;

/// The generator state of one generator function body.
pub(crate) struct GenState {
    /// The generator object (`undefined` until `CreateGeneratorObject` on the first call).
    object: Register,
    /// The resume point of each suspend id: the entries of the prologue jump table.
    resume: Vec<Label>,
    /// The next suspend id.
    next: usize,
}

/// Counts the suspend points of a generator body (GEN2): one per `yield` / `yield*` that the
/// body itself evaluates. Nested functions, arrows and class element bodies are their own
/// functions; class computed keys and heritage run in this function and are counted.
struct SuspendCounter(usize);

impl<'a> Visit<'a> for SuspendCounter {
    fn visit_yield_expression(&mut self, it: &YieldExpression<'a>) {
        self.0 += 1;
        walk::walk_yield_expression(self, it);
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
    /// The check before a closure is made for a generator function node (declaration,
    /// expression or method). An async generator keeps the "generator functions" text until
    /// async generators exist (month 3; GEN6).
    pub(crate) fn check_generator(&self, is_async: bool, span: Span) -> Gen {
        if is_async {
            return self.unsupported("generator functions", span);
        }
        Ok(())
    }

    /// The body of a generator function (an async generator is rejected, GEN6).
    pub(crate) fn gen_generator(&mut self, node: &'a Function<'a>) -> Gen {
        if node.r#async {
            return self.unsupported("generator functions", node.span);
        }
        let mut counter = SuspendCounter(1);
        counter.visit_formal_parameters(&node.params);
        if let Some(body) = &node.body {
            counter.visit_function_body(body);
        }
        let count =
            u32::try_from(counter.0).map_err(|_| super::internal("too many suspend points"))?;

        self.scope = self.info.outer_scope.unwrap_or(self.info.scope);
        // Prologue: a resume jumps to its suspend point; the first call falls through.
        let object = self.temp();
        self.b.lda_new_target().star(object);
        let (table, resume) = self.b.jump_table(count);
        self.b.switch_on_generator_state(object, table);
        self.generator = Some(GenState {
            object,
            resume,
            next: 0,
        });
        self.prologue_and_params(&node.params)?;
        // §15.5.2 steps 2-3: the generator object, after the parameters (GEN4).
        let m = self.mark();
        let args = self.temps(2);
        self.b.lda_current_function().star(args.get(0));
        self.b.mov(Register::RECEIVER, args.get(1));
        self.expr_pos(node.span.start);
        self.runtime(RuntimeId::CreateGeneratorObject, args);
        self.release(m);
        self.b.star(object);
        // The initial suspend returns the generator object itself (V8 InitialYield).
        self.suspend_and_dispatch(node.span.start)?;
        if let Some(body) = &node.body {
            self.statements(&body.statements)?;
        }
        self.b
            .set_throw_position(Some(node.span.end.saturating_sub(1)));
        self.b.lda_undefined();
        self.emit_return();
        let state = self.generator.take().expect("set above");
        if state.next != state.resume.len() {
            return Err(super::internal(format!(
                "generator at {}: {} suspend points emitted, {} counted",
                node.span.start,
                state.next,
                state.resume.len()
            )));
        }
        Ok(())
    }

    /// The generator object register.
    fn generator_object(&self, span: Span) -> Gen<Register> {
        match &self.generator {
            Some(g) => Ok(g.object),
            None => Err(super::internal(format!(
                "yield outside a generator body at {}",
                span.start
            ))),
        }
    }

    /// `SuspendGenerator` with the value in the accumulator, then its resume point:
    /// `ResumeGenerator` (acc = the sent value). `live` registers `r0..live` are saved.
    fn suspend(&mut self, live: u32, at: u32) -> Gen {
        let Some(g) = self.generator.as_mut() else {
            return Err(super::internal(format!(
                "suspend outside a generator at {at}"
            )));
        };
        let id = g.next;
        let Some(&label) = g.resume.get(id) else {
            return Err(super::internal(format!(
                "suspend point {id} at {at} was not counted"
            )));
        };
        g.next += 1;
        let object = g.object;
        let regs = RegisterList::new(0, live);
        self.expr_pos(at);
        self.b
            .suspend_generator(object, regs, live, u32::try_from(id).unwrap_or(u32::MAX));
        self.b.bind(label);
        self.b.resume_generator(object, regs, live);
        Ok(())
    }

    /// A suspend point with the value in the accumulator, then the dispatch on the resume
    /// mode (V8 `VisitYield`): throw rethrows the sent value, return returns it through the
    /// finally blocks (G5), next leaves it in the accumulator.
    fn suspend_and_dispatch(&mut self, at: u32) -> Gen {
        let object = self.generator_object(Span::new(at, at))?;
        let m = self.mark();
        // The highest live register + 1: every register below `input` is saved.
        let input = self.temp();
        let live = input.local_index().expect("a local");
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        let (table, cases) = self.b.jump_table(2);
        debug_assert_eq!(RESUME_RETURN, RESUME_NEXT + 1);
        self.b.switch_on_smi(table, RESUME_NEXT);
        // Throw (the fall-through).
        self.b.ldar(input);
        self.expr_pos(at);
        self.b.throw();
        // Return.
        self.b.bind(cases[1]);
        self.b.ldar(input);
        self.perform(Command::Return);
        // Next.
        self.b.bind(cases[0]);
        self.b.ldar(input);
        self.release(m);
        Ok(())
    }

    /// `yield` and `yield*`. Leaves the value of the expression in the accumulator.
    pub(crate) fn yield_expr(&mut self, y: &'a YieldExpression<'a>) -> Gen {
        self.generator_object(y.span)?;
        if y.delegate {
            let Some(arg) = &y.argument else {
                return Err(super::internal("yield* without an operand"));
            };
            return self.yield_star(arg, y.span.start);
        }
        match &y.argument {
            Some(e) => self.expr(e)?,
            None => {
                self.b.lda_undefined();
            }
        }
        // `{value, done: false}` (M2-P1-GEN-NUMBERS: the bytecode wraps a yield).
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(0)).lda_false().star(args.get(1));
        self.expr_pos(y.span.start);
        self.runtime(RuntimeId::CreateIterResultObject, args);
        self.release(m);
        self.suspend_and_dispatch(y.span.start)
    }

    /// `yield* e` (§15.5.5 YieldExpression : `yield* AssignmentExpression`, sync generators),
    /// the V8 delegating loop (`VisitYieldStar`):
    ///
    /// ```text
    /// iter, next = GetIterator(e); input = undefined; mode = next
    /// loop:
    ///   switch mode
    ///     next:   output = next.call(iter, input)
    ///     return: m = iter.return; if m is undefined or null: return input
    ///             output = m.call(iter, input)
    ///     throw:  m = iter.throw; if m is undefined or null:
    ///               IteratorClose(iter, normal); throw TypeError (ThrowThrowMethodMissing)
    ///             output = m.call(iter, input)
    ///   if output is not an object: throw TypeError
    ///   if output.done: break
    ///   input = suspend(output)          (the inner result object, unwrapped)
    ///   mode = GeneratorGetResumeMode(r_gen)
    /// value = output.value
    /// if mode == return: return value (through the finally blocks)
    /// acc = value
    /// ```
    fn yield_star(&mut self, arg: &'a Expression<'a>, at: u32) -> Gen {
        let object = self.generator_object(Span::new(at, at))?;
        let m = self.mark();
        let output = self.temp();
        let mode = self.temp();
        let iter = self.temp();
        let input = self.temp();
        let next = self.temp();
        let method = self.temp();
        self.expr(arg)?;
        self.get_iterator(iter, next, arg.span().start);
        self.b.lda_undefined().star(input);
        self.smi(RESUME_NEXT);
        self.b.star(mode);

        self.loop_depth += 1;
        let header = self.b.bound_label();
        let (after_switch, exit) = (self.b.new_label(), self.b.new_label());
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
        self.b.ldar(input);
        self.perform(Command::Return);
        // Throw.
        self.b.bind(cases[1]);
        let no_throw = self.b.new_label();
        self.call_iterator_method(iter, input, method, "throw", after_switch, no_throw, at);
        self.b.bind(no_throw);
        self.close_iterator_normal(iter, method, at);
        self.expr_pos(at);
        self.runtime(RuntimeId::ThrowThrowMethodMissing, RegisterList::EMPTY);
        // The result must be an object; stop when it is done.
        self.b.bind(after_switch);
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
        // Pass the inner result through, unwrapped; every register up to `method` is live.
        self.b.ldar(output);
        let live = method.local_index().expect("a local") + 1;
        self.suspend(live, at)?;
        self.b.star(input);
        self.runtime_with(RuntimeId::GeneratorGetResumeMode, &[object]);
        self.b.star(mode);
        self.jump_loop(header);
        self.b.bind(exit);
        self.loop_depth -= 1;

        // The value; a return resumption returns it.
        let k_value = self.str_const("value");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(output, k_value, s).star(output);
        let is_value = self.b.new_label();
        self.smi(RESUME_RETURN);
        self.b.test_reference_equal(mode).jump_if_false(is_value);
        self.b.ldar(output);
        self.perform(Command::Return);
        self.b.bind(is_value);
        self.b.ldar(output);
        self.release(m);
        Ok(())
    }

    /// `m = iter[name]`; if `m` is undefined or null jump to `missing`, else
    /// `acc = m.call(iter, input)` and jump to `called` (V8 `BuildCallIteratorMethod`).
    #[allow(clippy::too_many_arguments)]
    fn call_iterator_method(
        &mut self,
        iter: Register,
        input: Register,
        method: Register,
        name: &str,
        called: Label,
        missing: Label,
        at: u32,
    ) {
        let k = self.str_const(name);
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(missing);
        self.b.star(method);
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.call_property1(method, iter, input, call);
        self.b.jump(called);
    }

    /// IteratorClose(iter, normal completion) (§7.4.11): call `iter.return()` if it exists;
    /// its result must be an object.
    fn close_iterator_normal(&mut self, iter: Register, method: Register, at: u32) {
        let done = self.b.new_label();
        let k = self.str_const("return");
        let s = self.slot(FeedbackSlotKind::LoadNamed);
        self.expr_pos(at);
        self.b.get_named_property(iter, k, s);
        self.b.jump_if_undefined_or_null(done);
        self.b.star(method);
        let call = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(at);
        self.b.call_property0(method, iter, call);
        self.b.jump_if_js_receiver(done);
        self.b.star(method);
        self.expr_pos(at);
        self.runtime_with(RuntimeId::ThrowIteratorResultNotAnObject, &[method]);
        self.b.bind(done);
    }
}
