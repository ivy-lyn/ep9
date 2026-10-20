//! The GC heap: cage, pages, GC stages S0-S3, write barrier, conservative stack scan, roots,
//! verifier, and the visitor hook for unit RW data.
//!
//! M0 week 2 has only the pre-store barrier prototype ([`barrier`]).

pub mod barrier;

pub use barrier::{BARRIER_VERSION, BarrierState, Slot};

#[cfg(test)]
mod tests;
