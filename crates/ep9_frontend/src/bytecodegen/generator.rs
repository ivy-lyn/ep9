//! Generators (`function*`, `yield`, `yield*`). M2 P0 routing stub: every entry point returns
//! today's `unsupported:` error. P1-GEN-CG builds them (V8 model):
//!
//! ```text
//! prologue:  LdaNewTarget (fp-3: the generator object in a resumed frame) -> r_gen
//!            SwitchOnGeneratorState r_gen, [resume points]
//!            first call falls through: CreateGeneratorObject(closure, this); initial suspend
//! yield:     SuspendGenerator r_gen, regs; ResumeGenerator r_gen, regs;
//!            switch on GeneratorGetResumeMode(r_gen): next | return (finally tokens, G5) | throw
//! yield*:    the V8 delegating loop (ThrowThrowMethodMissing)
//! ```
//!
//! Callers: `func.rs` (`hoist_functions`, `closure`, `gen_function`) and `expr.rs`
//! (`Expression::YieldExpression`).
#![allow(dead_code)] // m2-stub

use oxc_ast::ast::{Function, YieldExpression};
use oxc_span::Span;

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// The check before a closure is made for a generator function node (declaration,
    /// expression or method). `is_async` marks an async generator, which keeps the
    /// "generator functions" text until async generators exist (month 3).
    pub(crate) fn check_generator(&self, is_async: bool, span: Span) -> Gen {
        let _ = is_async;
        self.unsupported("generator functions", span)
    }

    /// The body of a generator function (also an async generator).
    pub(crate) fn gen_generator(&mut self, node: &'a Function<'a>) -> Gen {
        self.unsupported("generator functions", node.span)
    }

    /// `yield` and `yield*`. Leaves the value of the expression in the accumulator.
    pub(crate) fn yield_expr(&mut self, y: &'a YieldExpression<'a>) -> Gen {
        self.unsupported("yield", y.span)
    }
}
