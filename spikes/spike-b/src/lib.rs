//! Spike B (M0 month 2, throwaway): native code for T0 frames and the D21 measurement.
//!
//! P2 (SPIKE-B) built the JIT of placement (a) and the structural gate; P3 (SPIKEB-BENCH) and
//! P4 (SP-UNIT) add their modules in their own files (the stubs below), so neither needs this
//! file or `main.rs`.
//!
//! | Module | Contents | Owner |
//! |---|---|---|
//! | [`emit`] | the `Emitter` / `Finalize` traits (frozen at the P2 merge) | P2-SPIKE-B |
//! | [`jit`] | placement (a): the 11-opcode JIT for AArch64 and x86-64, the in-memory finalizer | P2-SPIKE-B |
//! | [`harness`] | compile JS, make isolates, JIT and install functions, capture `print` | P2-SPIKE-B |
//! | [`structural`] | the gate checks (a)-(h) of `gate.sh structural` | P2-SPIKE-B |
//! | [`bench`](mod@bench) | `spike-b bench` (stub) | P3-SPIKEB-BENCH |
//! | [`jit_b`] | placement (b) (stub) | P3-SPIKEB-BENCH, only after a "fail" verdict |
//! | [`unit`](mod@unit) | `spike-b unit` (stub) | P4-SP-UNIT |
//!
//! The protocol: `docs/measurements/2026-10-22-spike-b-protocol.md`.

pub mod bench;
pub mod emit;
pub mod harness;
pub mod jit;
pub mod jit_b;
pub mod structural;
pub mod unit;

/// The contract versions this build of the spike links against, as one line. The binary
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
