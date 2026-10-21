//! fp-check fixture: float `%` in an engine crate. Rust lowers it to the C `fmod` on every
//! target (H36); fp-check must name `fx_engine::js_remainder`.
use std::hint::black_box;

#[inline(never)]
pub fn js_remainder(a: f64, b: f64) -> f64 {
    a % b
}

pub fn run(x: f64) -> f64 {
    js_remainder(x, black_box(3.0))
}
