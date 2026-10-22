#![allow(dead_code)] // m2-stub
//! `spike-b unit …`: SP-Unit (AOT unit text with 0 relocations, LinkSlot and IcCell through
//! the unit-data register x27 / r14, per-isolate RW copies). Owner: P4-SP-UNIT (plan-v4 §7),
//! behind the frozen [`crate::emit`] `Emitter`/`Finalize` traits; it fills this stub.

/// Entry of `spike-b unit <args>` (`main.rs` dispatches here). Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    eprintln!("spike-b unit: not built yet (P4-SP-UNIT fills src/unit/mod.rs); args: {args:?}");
    2
}
