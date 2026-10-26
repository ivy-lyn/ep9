#![allow(dead_code)] // m4-stub
//! The graph builder (stub; SPIKEC-1 builds it in M4-P1): one function's verified bytecode
//! plus its [`FunctionFacts`] -> an [`ir::Graph`] in SSA form.
//!
//! One builder serves both fact paths (H30 rule "one graph builder reads bytecode +
//! FactSnapshot"; V9-D32): the live path takes `ep9_vm::abi::export_facts` output
//! ([`crate::facts::live`]), SP-Facts takes a PROFILED file ([`crate::facts::read_profiled`]).
//! The builder never reads a FeedbackVector directly.
//!
//! First target (SPIKEC-1): crypto `am3`, with int32 facts PROVEN from `&` and `>>` and the
//! named-load facts as exported at the phase base. Liveness is computed here (H02-Q1 =
//! computed by the graph builder, V9-D32); the builder reports its cost for H02-Q1.

use spike_b::emit::FunctionInput;

use crate::NotBuilt;
use crate::facts::FunctionFacts;
use crate::ir;

/// Builds the SSA graph of `input` from its bytecode and `facts`.
pub fn build(input: &FunctionInput<'_>, facts: &FunctionFacts<'_>) -> Result<ir::Graph, NotBuilt> {
    let _ = (input, facts);
    Err(NotBuilt {
        stage: "graph builder",
        owner: "SPIKEC-1, M4-P1",
    })
}
