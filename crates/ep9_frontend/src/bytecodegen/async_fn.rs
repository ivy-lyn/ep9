#![allow(dead_code)] // m3-stub
//! Async functions, async arrows, async methods and `await` (ECMA-262 §15.8, §27.7).
//!
//! M3-P0 routing stub (P0-D7): every entry gives today's result (`unsupported: async
//! functions`, `unsupported: await`), so no output changes. The M3-P1 ASYNC role owns this
//! file whole and fills it: the generator opcodes 0x99-0x9b in a function with the `ASYNC`
//! flag, and ABI 5 runtime ids 32-35 (`AsyncFunctionEnter`, `AsyncFunctionAwait`,
//! `AsyncFunctionResolve`, `AsyncFunctionReject`).
//!
//! ```text
//! caller (site at b6109b3)                         entry here
//! func.rs   hoist_functions (:342)                 async_closure        an async declaration
//! func.rs   closure (:1198)                        async_closure        expression, method
//! expr.rs   ArrowFunctionExpression (:307)         async_closure        an async arrow
//! mod.rs    declare_eval_bindings (:812)           async_closure        sloppy eval function
//! func.rs   gen_function (:1093)                   gen_async_function   the body
//! func.rs   gen_arrow (:1113)                      gen_async_arrow      the body
//! expr.rs   AwaitExpression (:315)                 await_expr           `await e`
//! ```
//!
//! An async generator (`async function*`) goes to `async_gen.rs`, not here.

use oxc_ast::ast::{ArrowFunctionExpression, AwaitExpression, Function};
use oxc_span::Span;

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// The check before a closure is made for an async function node that is not a generator
    /// (a declaration, an expression, a method, an arrow): `Ok` lets the caller make it.
    pub(crate) fn async_closure(&self, span: Span) -> Gen {
        self.unsupported("async functions", span)
    }

    /// The body of an async function or async method (not a generator).
    pub(crate) fn gen_async_function(&mut self, node: &'a Function<'a>) -> Gen {
        self.unsupported("async functions", node.span)
    }

    /// The body of an async arrow function.
    pub(crate) fn gen_async_arrow(&mut self, node: &'a ArrowFunctionExpression<'a>) -> Gen {
        self.unsupported("async functions", node.span)
    }

    /// `await e` (§27.7.5.3 Await), in an async function body or a module top level.
    pub(crate) fn await_expr(&mut self, a: &'a AwaitExpression<'a>) -> Gen {
        self.unsupported("await", a.span)
    }
}
