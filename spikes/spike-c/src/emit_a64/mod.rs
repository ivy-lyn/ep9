#![allow(dead_code)] // m4-stub
//! AArch64 code from an allocated [`ir::Graph`] (stub; SPIKEC-1 builds it in M4-P1), with
//! dynasm 5.1.0 (the spikes/spike-b pin).
//!
//! The output is a [`spike_b::emit::FunctionCode`]: position-independent code, entry at offset
//! 0, with the placement (b) ABI (`ep9_vm::install_native_b`: the T0 handler ABI `(cx, pc, fp,
//! acc)`, leave by a tail jump to `cx.table[*pc]`), its CFI, and `native_offsets` per bytecode
//! offset. So the `spike_b::jit::InMemory` finalizer places it and
//! [`crate::harness::Session::install_b`] installs it.
//!
//! Guards (H32, JIT mode): a failed guard jumps to its exit stub. Until month 5 builds the
//! eager deopt entry in `ep9_vm`, each exit stub calls a "deopt not built" trap; SPIKEC-4 makes
//! the per-exit position-independent metadata (pc offset from the code start, bytecode offset,
//! value locations, RW exit-counter index; H29-Q1, Q2, Q5).

use spike_b::emit::{FunctionCode, FunctionInput, Refusal};

use crate::NotBuilt;
use crate::ir;
use crate::regalloc::Allocation;

/// Emits the code of `graph` (built from `input`) with the registers of `alloc`.
pub fn emit(
    input: &FunctionInput<'_>,
    graph: &ir::Graph,
    alloc: &Allocation,
) -> Result<FunctionCode, Refusal> {
    let _ = (input, graph, alloc);
    let e = NotBuilt {
        stage: "a64 emit",
        owner: "SPIKEC-1, M4-P1",
    };
    Err(Refusal::Limit(e.to_string()))
}
