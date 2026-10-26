#![allow(dead_code)] // m4-stub
//! Register allocation (stub; SPIKEC-1 builds it in M4-P1): linear scan over the SSA values
//! of an [`ir::Graph`], AArch64 registers only.
//!
//! | Register | Use (placement (b), spikes/spike-b/src/jit_b/a64.rs) | Allocatable |
//! |---|---|---|
//! | x28 | isolate (`Cx`) | no |
//! | x26 | VM frame pointer | no |
//! | x27 | unit-data register (H35-Q9) | no |
//! | x29, x30 | frame record | no |
//! | others | values, spill slots in the VM frame | SPIKEC-1 decides and tests |
//!
//! A value that lives across a guard has a location the deopt exit can read (H29-Q2: reg,
//! stack or const-index).

use crate::NotBuilt;
use crate::ir;

/// Where each SSA value lives (SPIKEC-1).
#[derive(Debug, Default)]
pub struct Allocation {
    _built_by_spikec_1: (),
}

/// Allocates registers for `graph`.
pub fn allocate(graph: &ir::Graph) -> Result<Allocation, NotBuilt> {
    let _ = graph;
    Err(NotBuilt {
        stage: "register allocation",
        owner: "SPIKEC-1, M4-P1",
    })
}
