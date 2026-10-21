//! Private names in expressions: `o.#x`, `o.#x = v`, `o.#m()`, `#x in o`. M2 P0 routing stub:
//! every entry point returns today's `unsupported:` error. P2-CLS-CG builds them with the keyed
//! opcodes and the private name symbol from the class context as the key (C7): `GetKeyed`,
//! `SetKeyed`, `TestIn`; a private method loads the brand with `GetKeyed` and the closure from
//! the class context.
//!
//! Callers: `expr.rs` (load, member reference, optional chain, call, `in`, `delete`, keys).
#![allow(dead_code)] // m2-stub

use ep9_bytecode::{Register, RegisterList};
use oxc_ast::ast::{PrivateFieldExpression, PrivateIdentifier, PrivateInExpression};

use super::expr::MemberRef;
use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// `acc = o.#x` (also inside an optional chain).
    pub(crate) fn private_get(&mut self, p: &'a PrivateFieldExpression<'a>) -> Gen {
        self.unsupported("private names", p.span)
    }

    /// The reference parts of `o.#x` as an assignment target.
    pub(crate) fn private_member_ref(
        &mut self,
        p: &'a PrivateFieldExpression<'a>,
    ) -> Gen<MemberRef> {
        self.unsupported("private names", p.span)
    }

    /// The callee of `o.#m(...)`: the function into `callee`, the receiver into `list[0]`
    /// (`list` has `1 + n` registers).
    pub(crate) fn private_callee(
        &mut self,
        p: &'a PrivateFieldExpression<'a>,
        callee: Register,
        list: RegisterList,
    ) -> Gen {
        let _ = (callee, list);
        self.unsupported("private names", p.span)
    }

    /// `#x in o`.
    pub(crate) fn private_in(&mut self, p: &'a PrivateInExpression<'a>) -> Gen {
        self.unsupported("private names", p.span)
    }

    /// `delete o.#x` (an early error, so only reached for an AST that skipped the checks).
    pub(crate) fn private_delete(&mut self, p: &'a PrivateFieldExpression<'a>) -> Gen {
        self.unsupported("private names", p.span)
    }

    /// A private name as a class element key into `r`.
    pub(crate) fn private_key_into(&mut self, p: &'a PrivateIdentifier<'a>, r: Register) -> Gen {
        let _ = r;
        self.unsupported("private names", p.span)
    }
}
