#![allow(dead_code)] // m2-stub
//! Jobs on the builtin ABI (owner P4-PROMISE; RT-9): `enqueue_job`,
//! `perform_microtask_checkpoint` and the rejection tracker, over the heap job queue of
//! `jobs.rs`.
//!
//! P0 stub: one placeholder that throws "not implemented".

use ep9_value::Throw;

use super::Cx;

impl<'s> Cx<'s> {
    /// Placeholder until P4-PROMISE fills this file.
    pub(crate) fn jobs_stub(&mut self) -> Throw {
        self.not_implemented("abi::jobs")
    }
}
