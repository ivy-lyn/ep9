//! One Rust Math (H36, `docs/adr/H36-fp-semantics.md`): every engine crate gets libm-class
//! results, `pow_js`, exact `fmod` and `fma_exact` from here, so each target computes the same
//! bits. This crate has no dependencies (`.github/scripts/dep-guards.sh` check 6).
//!
//! The only `unsafe` code is the FMA dispatch: its module carries the one
//! `#[allow(unsafe_code)]`.
#![deny(unsafe_code)]
