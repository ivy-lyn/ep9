#![allow(dead_code)] // m2-stub
//! Scripts from the builtin ABI (owner P3-EVAL-VM): `Cx::evaluate_script` through the
//! compiler hook (`IsolateConfig::compiler`, C2).
//!
//! P0 stub: one placeholder that throws "not implemented".

use ep9_value::Throw;

use super::Cx;

impl<'s> Cx<'s> {
    /// Placeholder until P3-EVAL-VM fills this file.
    pub(crate) fn evaluate_script_stub(&mut self) -> Throw {
        self.not_implemented("abi::script")
    }
}
