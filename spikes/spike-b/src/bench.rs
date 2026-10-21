#![allow(dead_code)] // m2-stub
//! `spike-b bench …`: the D21 round-trip bench of the Spike B protocol
//! (`docs/measurements/2026-10-22-spike-b-protocol.md`). Owner: P3-SPIKEB-BENCH (plan-v4 §6),
//! which fills this stub. Configs J and JC time `drive` through `ep9_vm::call_from_rust`; the
//! JIT is [`crate::jit`] (placement (a)).

/// Entry of `spike-b bench <args>` (`main.rs` dispatches here). Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    eprintln!("spike-b bench: not built yet (P3-SPIKEB-BENCH fills src/bench.rs); args: {args:?}");
    2
}
