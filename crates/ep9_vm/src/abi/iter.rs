#![allow(dead_code)] // m2-stub
//! The iterator protocol on the builtin ABI (owner P1-ABI-INDEX; H20 ABI-3):
//! `get_iterator`, `iterator_step_value`, `iterator_close`, with a stack-only IteratorRecord.
//!
//! P0 stub: one placeholder that throws "not implemented".

use ep9_value::Throw;

use super::Cx;

impl<'s> Cx<'s> {
    /// Placeholder until P1-ABI-INDEX fills this file.
    pub(crate) fn iterator_protocol_stub(&mut self) -> Throw {
        self.not_implemented("abi::iter")
    }
}
