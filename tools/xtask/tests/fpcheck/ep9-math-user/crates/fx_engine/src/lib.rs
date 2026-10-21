//! fp-check fixture: FMA through ep9_math only. fp-check must pass, and see the FMA
//! instructions inside `ep9_math::` functions: on the x86-64 baseline in the
//! `#[target_feature(enable = "fma")]` copy of the dispatch, with no call to the C `fma`.
use std::hint::black_box;

pub fn run(x: f64) -> f64 {
    let (p, e) = ep9_math::two_prod(x, black_box(0.3));
    ep9_math::fma_exact(x, black_box(0.1), -1.0) + p + e
}
