#![allow(dead_code)] // m2-stub
//! Placement (b) of D21 (JIT frames on the VM stack, entered with the 4 handler arguments).
//! Built only if the gate run of the Spike B protocol gives "fail" (M2-D21-FAIL); owner
//! P3-SPIKEB-BENCH (plan-v4 §6 task 4), then the P4 slot. It uses the frozen
//! [`crate::emit`] traits and does not change [`crate::jit`] (placement (a)).
