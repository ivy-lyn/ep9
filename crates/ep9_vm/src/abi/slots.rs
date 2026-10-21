#![allow(dead_code)] // m2-stub
//! Objects with internal slots on the builtin ABI (owner P3-ABI-SLOTS): the append-only
//! ObjectClass list, `new_object_with_slots`, `slot`, `set_slot` (SLOT_OBJECT cells, kind 24,
//! `object::slot_obj`).
//!
//! P0 stub: one placeholder that throws "not implemented".

use ep9_value::Throw;

use super::Cx;

impl<'s> Cx<'s> {
    /// Placeholder until P3-ABI-SLOTS fills this file.
    pub(crate) fn slots_stub(&mut self) -> Throw {
        self.not_implemented("abi::slots")
    }
}
