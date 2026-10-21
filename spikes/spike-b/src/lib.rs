#![allow(dead_code)] // m2-stub
//! Spike B (M0 month 2, throwaway): native code for T0 frames and the D21 measurement.
//!
//! P0 skeleton: the crate, its own workspace and lock, and the engine path dependencies
//! (`ep9_vm` with `spike-native`, `ep9_platform` with `jit`). The protocol comes first
//! (`docs/measurements/2026-10-22-spike-b-protocol.md`); the spike code comes in P2
//! (SPIKE-B), the bench in P3 (SPIKEB-BENCH) and SP-Unit in P4 (SP-UNIT). Those owners add
//! their modules here.

/// The contract versions this build of the spike links against, as one line. The P0 binary
/// prints it, so a build shows which engine contracts the spike was built with.
pub fn contracts_line() -> String {
    use ep9_vm::versions as vm;
    format!(
        "bytecode ABI {}, FRAME {}, OBJECT_LAYOUT {}, unit container {}",
        ep9_bytecode::ABI_VERSION,
        vm::FRAME_VERSION,
        vm::OBJECT_LAYOUT_VERSION,
        ep9_unit::CONTAINER_VERSION,
    )
}
