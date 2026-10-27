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
//! | Module | Contents | Built by |
//! |---|---|---|
//! | [`harness`] | isolate with the builtins, `print`/`gc`, kernel runs, function lookup, placement (b) install | P0-SPIKEC |
//! | [`kernels`] | the protocol kernels: source files, hot functions, drivers, checks | P0-SPIKEC |
//! | [`structural`] | `spike-c structural`, the checks of `gate.sh structural` | P0-SPIKEC; SPIKEC-1 adds layout, compile and lattice checks |
//! | [`facts`] | the FactSnapshot input of one function: live (RUNTIME) or a PROFILED file | P0-SPIKEC; SP-Facts (month 5) |
//! | [`cfg`](mod@cfg) | reachable bytecode as blocks, register liveness (H02-Q1) | SPIKEC-1 (M4-P1) |
//! | [`ir`] | the SSA graph, the fact lattice, the five ref nodes, guards with frame states | SPIKEC-1 |
//! | [`build`](mod@build) | the graph builder: bytecode + FactSnapshot -> [`ir::Graph`] | SPIKEC-1; int facts SPIKEC-2; keyed facts SPIKEC-3 |
//! | [`regalloc`] | linear-scan register allocation | SPIKEC-1 |
//! | [`emit_a64`] | AArch64 code from the allocated graph (dynasm 5.1.0); guard exits = the "deopt not built" trap | SPIKEC-1 |
//! | [`vm_layout`] | VM layout facts the code reads that `ep9_vm::layout` does not export, checked on live objects | SPIKEC-1 |
//! | [`tier`] | config C of the protocol: a kernel run with its hot function compiled, against T0 | SPIKEC-1 |
//! | [`timing`] | the class S timing of protocol §3 (T0, C, H0, H1; paired ratios, the median interval), a Mac report | SPIKEC-2 |
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
pub mod cfg;
pub mod emit_a64;
pub mod facts;
pub mod harness;
pub mod ir;
pub mod kernels;
pub mod regalloc;
pub mod structural;
pub mod tier;
pub mod timing;
pub mod vm_layout;

use spike_b::emit::{FunctionCode, FunctionInput, Refusal};

/// What one compile made and measured.
#[derive(Clone, Debug)]
pub struct CompileReport {
    pub name: String,
    pub build: build::BuildStats,
    /// Live graph nodes and blocks.
    pub nodes: usize,
    pub blocks: usize,
    /// Guards backed by a fact, and unbacked guards (see [`ir`]).
    pub fact_guards: usize,
    pub unbacked_guards: usize,
    pub intervals: usize,
    pub spilled: usize,
    pub max_live: usize,
    pub emit: emit_a64::EmitReport,
    /// The graph as text (`spike-c show`).
    pub graph: String,
}

impl CompileReport {
    /// One line for gate.sh and the role report.
    pub fn line(&self) -> String {
        format!(
            "{}: {} bytes, {} bytecode instructions in {} blocks -> {} nodes; guards {} fact + {} unbacked; \
             {} intervals, {} spilled, {} live at most; liveness {} us, {} bytes (H02-Q1)",
            self.name,
            self.emit.code_bytes,
            self.build.instructions,
            self.blocks,
            self.nodes,
            self.fact_guards,
            self.unbacked_guards,
            self.intervals,
            self.spilled,
            self.max_live,
            self.build.liveness_us,
            self.build.liveness_bytes,
        )
    }

    /// The keyed sites and what their records gave (SPIKEC-3), one line.
    pub fn keyed_line(&self) -> String {
        let sites: Vec<String> = self
            .build
            .keyed
            .iter()
            .map(|k| {
                format!(
                    "@{} {} s{} keys {} -> key check {:?}",
                    k.offset,
                    if k.store { "store" } else { "load" },
                    k.slot,
                    k.keys.map_or("(no record)".to_string(), |s| {
                        let names: Vec<&str> = ep9_facts::KeySet::BITS
                            .iter()
                            .filter(|(b, _)| s.contains(*b))
                            .map(|(_, n)| *n)
                            .collect();
                        format!("{{{}}}", names.join(" "))
                    }),
                    k.key_basis,
                )
            })
            .collect();
        format!(
            "{}: keyed sites {}{}{}",
            self.name,
            self.build.keyed.len(),
            if sites.is_empty() { "" } else { ": " },
            sites.join("; ")
        )
    }
}

/// The whole pipeline on one function: graph builder, lattice check, register allocation,
/// a64 emit. Refuses with [`Refusal::Arch`] on any architecture but AArch64 (protocol §1),
/// after the arch-independent stages ran.
pub fn compile_input(
    input: &FunctionInput<'_>,
    facts: &facts::FunctionFacts<'_>,
) -> Result<(FunctionCode, CompileReport), Refusal> {
    let (graph, stats) = build::build(input, facts)?;
    let alloc = regalloc::allocate(&graph).map_err(Refusal::Limit)?;
    regalloc::verify(&graph, &alloc).map_err(Refusal::Limit)?;
    if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
        return Err(Refusal::Arch);
    }
    let (code, emit) = emit_a64::emit(input, &graph, &alloc)?;
    let (fact_guards, unbacked_guards) = graph.guard_counts();
    let report = CompileReport {
        name: input.name.to_string(),
        build: stats,
        nodes: graph.order().count(),
        blocks: graph.blocks.len(),
        fact_guards,
        unbacked_guards,
        intervals: alloc.intervals,
        spilled: alloc.spilled,
        max_live: alloc.max_live,
        emit,
        graph: graph.to_string(),
    };
    Ok((code, report))
}

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

/// Compiles the global function `name` of `session` with `facts` (see [`compile_input`]).
/// The VM layout copies are checked first, on a session of their own ([`vm_layout::check`]).
pub fn compile(
    session: &harness::Session,
    name: &str,
    facts: &facts::FunctionFacts<'_>,
) -> Result<(FunctionCode, CompileReport), String> {
    let mut probe = harness::Session::new(&spike_b::harness::Config::default())?;
    vm_layout::check(&mut probe).map_err(|e| format!("vm layout: {e}"))?;
    let input = session.function_input(name)?;
    compile_input(&input, facts).map_err(|r| format!("{name}: {r}"))
}
