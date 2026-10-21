//! Calls whose callee is a dynamic lookup (`with`, sloppy direct eval): `f()` where `f`
//! resolves to [`Resolution::Dynamic`](crate::scope::Resolution::Dynamic). M2 P0 routing stub:
//! the same code as any other callee (`LdaLookupSlot`, then a call with an undefined receiver).
//! P2-LOOKUP makes it `CallRuntimeForPair(LoadLookupSlotForCall, name)`, so that `with (o) f()`
//! gets `this === o`:
//!
//! ```text
//! CallRuntimeForPair LoadLookupSlotForCall, r_name, (r_callee, list[0] = receiver)
//! CallProperty r_callee, list           (the property call path of `call`)
//! ```
//!
//! Caller: `expr.rs` (`callee_and_receiver`).
#![allow(dead_code)] // m2-stub

use ep9_bytecode::{Register, RegisterList};
use oxc_ast::ast::{Expression, IdentifierReference};

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// Evaluates the callee `callee_expr` (the identifier `id`, maybe in parentheses) into
    /// `callee`. Returns `Some(list)` of `1 + n` registers with the receiver in `list[0]` for a
    /// property-style call, or `None` for a call with an undefined receiver.
    pub(crate) fn dynamic_callee(
        &mut self,
        callee_expr: &'a Expression<'a>,
        id: &'a IdentifierReference<'a>,
        callee: Register,
        n: u32,
    ) -> Gen<Option<RegisterList>> {
        let _ = (id, n);
        self.expr(callee_expr)?;
        self.b.star(callee);
        Ok(None)
    }
}
