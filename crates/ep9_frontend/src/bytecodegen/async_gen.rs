#![allow(dead_code)] // m3-stub
//! Async generators (`async function*`, ECMA-262 §27.6) and `for await` (§14.7.5).
//!
//! M3-P0 routing stub (P0-D7): every entry gives today's result (`unsupported: generator
//! functions` for an async generator, GEN6 in generator.rs; `unsupported: for await`), so no
//! output changes. The M3-P2 ASYNCGEN role owns this file whole and fills it with ABI 5
//! runtime ids 36-41 (`AsyncGeneratorEnter` ... `CreateAsyncFromSyncIterator`).
//!
//! ```text
//! caller (site at b6109b3)                         entry here
//! generator.rs check_generator (:84-88)            async_generator_closure  any async generator node
//! generator.rs gen_generator (:93-95)              gen_async_generator      the body
//! stmt.rs   for_of (:572)                          for_await_of             `for await (... of ...)`
//! ```

use oxc_ast::ast::{ForOfStatement, Function};
use oxc_span::Span;

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// The check before a closure is made for an async generator node (a declaration, an
    /// expression or a method): `Ok` lets the caller make it.
    pub(crate) fn async_generator_closure(&self, span: Span) -> Gen {
        self.unsupported("generator functions", span)
    }

    /// The body of an async generator function or method.
    pub(crate) fn gen_async_generator(&mut self, node: &'a Function<'a>) -> Gen {
        self.unsupported("generator functions", node.span)
    }

    /// `for await (lhs of rhs) body` (ForIn/OfBodyEvaluation with iteratorKind async), with
    /// the labels of the statement.
    pub(crate) fn for_await_of(&mut self, f: &'a ForOfStatement<'a>, labels: Vec<&'a str>) -> Gen {
        let _ = labels;
        self.unsupported("for await", f.span)
    }
}
