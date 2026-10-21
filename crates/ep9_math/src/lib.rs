//! One Rust Math (H36, `docs/adr/H36-fp-semantics.md`): every engine crate gets libm-class
//! results, `pow_js`, exact `fmod` and `fma_exact` from here, so each target computes the same
//! bits. This crate has no dependencies (`.github/scripts/dep-guards.sh` check 6).
//!
//! | Item | JS use | v0 status (M0 week 4) |
//! |---|---|---|
//! | [`fmod()`] | `%` (Number::remainder) | exact; musl port |
//! | [`pow_js`] | `**`, `Math.pow` | special cases and exact results; `None` where the correctly rounded core is needed (month 2) |
//! | [`round_js`], [`fround`] | `Math.round`, `Math.fround` | exact |
//! | [`fma_exact`], [`two_prod`], [`two_sum`], [`fma_soft`] | building blocks of the CR functions | exact |
//!
//! Rules (H36): every public function is `#[inline(never)]`, so FMA instructions stay inside
//! `ep9_math::` symbols (`cargo xtask fp-check` allows them only there). No libm call, no
//! `mul_add` outside [`exact::HwFma`]. The only `unsafe` code is the FMA dispatch: its module
//! (`exact.rs`) carries the one `#[allow(unsafe_code)]`.
#![deny(unsafe_code)]

pub mod exact;
mod fmod;
mod pow;
mod round;

pub use exact::{fma_exact, fma_soft, two_prod, two_sum};
pub use fmod::fmod;
pub use pow::pow_js;
pub use round::{fround, round_js};
