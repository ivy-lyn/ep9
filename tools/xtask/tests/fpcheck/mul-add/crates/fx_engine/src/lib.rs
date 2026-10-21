//! fp-check fixture: `mul_add` outside ep9_math. With `-Ctarget-cpu=x86-64-v3` it is an inline
//! `vfmadd`; on the x86-64 baseline it calls the C `fma`. fp-check must name
//! `fx_engine::fused_step` either way.
use std::hint::black_box;

#[inline(never)]
pub fn fused_step(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

pub fn run(x: f64) -> f64 {
    fused_step(x, black_box(0.1), black_box(-1.0))
}
