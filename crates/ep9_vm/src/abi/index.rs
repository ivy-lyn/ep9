#![allow(dead_code)] // m2-stub
//! Index access on the builtin ABI (owner P1-ABI-INDEX; H20 ABI-1): `get_index`,
//! `has_index`, `set_index` and `delete_index`, with no atom for a dense element.
//!
//! P0 stub: one placeholder that throws "not implemented".

use ep9_value::Throw;

use super::Cx;

impl<'s> Cx<'s> {
    /// Placeholder until P1-ABI-INDEX fills this file.
    pub(crate) fn index_access_stub(&mut self) -> Throw {
        self.not_implemented("abi::index")
    }
}
