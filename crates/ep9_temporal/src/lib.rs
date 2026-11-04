//! Temporal (plan Q2; design SPEC-T-D11): the glue from the builtin ABI (H20, `ep9_vm::Cx`) to
//! `temporal_rs`. The crate has no `unsafe` and reaches the heap only through `Cx`.
//!
//! ```text
//! ep9_builtins::MODULES = [.., bigint, ep9_temporal::MODULES..]      one table per isolate
//! ep9_temporal
//!   tzdb      ICU zoneinfo64.res (data/) -> one provider per process (no ep9_vm import)
//!   T0: data and provider only; the Temporal classes come in later work packages
//! ```
//!
//! Rules of the glue (design §3.1): only the `*_with_provider` forms of temporal_rs; a
//! temporal_rs value lives only on the Rust stack during one builtin call; no `Local` in a
//! Rust container; `Err` only from a `cx.throw_*` call.
#![forbid(unsafe_code)]

pub mod tzdb;
