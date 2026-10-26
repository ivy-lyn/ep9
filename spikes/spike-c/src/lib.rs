//! Spike C (M1, throwaway): a small SSA compiler for 2-3 v8-v7 kernels on arm64, one eager
//! deopt from a native frame to a VM-stack frame, and SP-Facts (the same compiler fed by a
//! PROFILED FactSnapshot file). The gate and the method are in
//! `docs/measurements/2026-10-27-spike-c-protocol.md`, written before the compiler exists.
//!
//! ```text
//!  kernel source ──harness (T0 run, live feedback)──> isolate + loaded unit
//!     │                                                   │
//!     │                           facts::live (export_facts, source RUNTIME)
//!     │                           facts::read_profiled (a PROFILED file, SP-Facts)
//!     v                                                   v
//!  bytecode of f ─────────────> build (one graph builder) <── FunctionFacts of f
//!                                   │ ir::Graph (SSA, fact lattice, ref nodes, guards)
//!                                   v
//!                               regalloc (linear scan)
//!                                   v
//!                               emit_a64 ──> spike_b::emit::FunctionCode
//!                                   v
//!            spike_b::jit::InMemory finalizer ──> harness::Session::install_b (Opt slot)
//! ```
//!
//! | Module | Contents | State at M4-P0 | Filled by |
//! |---|---|---|---|
//! | [`harness`] | isolate with the builtins, `print`/`gc`, kernel runs, function lookup, placement (b) install | built | P0-SPIKEC |
//! | [`kernels`] | the protocol kernels: source files, hot functions, drivers, checks | built | P0-SPIKEC |
//! | [`structural`] | `spike-c structural`, the checks of `gate.sh structural` | built | P0-SPIKEC; each later role adds its checks |
//! | [`facts`] | the FactSnapshot input of one function: live (RUNTIME) or a PROFILED file | built | P0-SPIKEC; SP-Facts (month 5) |
//! | [`ir`] | the SSA graph, the fact lattice, the five ref nodes, guards | stub | SPIKEC-1 (M4-P1) |
//! | [`build`](mod@build) | the graph builder: bytecode + FactSnapshot -> [`ir::Graph`] | stub | SPIKEC-1 |
//! | [`regalloc`] | linear-scan register allocation | stub | SPIKEC-1 |
//! | [`emit_a64`] | AArch64 code from the allocated graph (dynasm 5.1.0) | stub | SPIKEC-1 |
//!
//! Rules (protocol §1):
//! - Arm64 only (09-milestones.md:252). On any other architecture the pipeline refuses with
//!   [`spike_b::emit::Refusal::Arch`] and the function stays in T0.
//! - Code made here runs only in this crate's binary and tests: the `spike-native` feature is
//!   never in the ep9 graph (dep-guards check 11), and no JIT tier code runs JS before the M2
//!   gate (09-milestones.md:36).
//! - The installed code uses placement (b) (D21, H14): a VM-stack frame, the T0 handler ABI,
//!   `ep9_vm::install_native_b`.

pub mod build;
pub mod emit_a64;
pub mod facts;
pub mod harness;
pub mod ir;
pub mod kernels;
pub mod regalloc;
pub mod structural;

use std::fmt;

/// A pipeline stage that is not built yet. The function it was asked for stays in T0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotBuilt {
    /// The stage: "graph builder", "register allocation", "a64 emit".
    pub stage: &'static str,
    /// The plan role that builds it.
    pub owner: &'static str,
}

impl fmt::Display for NotBuilt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} not built yet ({})", self.stage, self.owner)
    }
}

impl std::error::Error for NotBuilt {}

/// The contract versions this build links against, as one line (the binary prints it).
pub fn contracts_line() -> String {
    use ep9_vm::versions as vm;
    format!(
        "bytecode ABI {}, FRAME {}, OBJECT_LAYOUT {}, IC_LAYOUT {}, DEOPT_META {}, FACT_SCHEMA {}",
        ep9_bytecode::ABI_VERSION,
        vm::FRAME_VERSION,
        vm::OBJECT_LAYOUT_VERSION,
        vm::IC_LAYOUT_VERSION,
        vm::DEOPT_META_VERSION,
        ep9_facts::FACT_SCHEMA_VERSION,
    )
}

/// Compiles the function `name` of `session` through the whole pipeline (build, register
/// allocation, a64 emit). Every stage is a stub at M4-P0, so this returns the first stage's
/// [`NotBuilt`] as a [`spike_b::emit::Refusal::Limit`].
pub fn compile(
    session: &harness::Session,
    name: &str,
    facts: &facts::FunctionFacts<'_>,
) -> Result<spike_b::emit::FunctionCode, String> {
    let input = session.function_input(name)?;
    if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
        return Err(format!("{name}: {}", spike_b::emit::Refusal::Arch));
    }
    let not_built =
        |e: NotBuilt| format!("{name}: {}", spike_b::emit::Refusal::Limit(e.to_string()));
    let graph = build::build(&input, facts).map_err(not_built)?;
    let alloc = regalloc::allocate(&graph).map_err(not_built)?;
    emit_a64::emit(&input, &graph, &alloc).map_err(|r| format!("{name}: {r}"))
}
