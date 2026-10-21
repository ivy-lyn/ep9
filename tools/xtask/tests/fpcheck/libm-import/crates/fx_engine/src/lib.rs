//! fp-check fixture: a libm import without the libm crate (the fixture has no crates.io
//! dependency). fp-check must name `fx_engine::fake_libm_cbrt`.
unsafe extern "C" {
    fn cbrt(x: f64) -> f64;
}

#[inline(never)]
pub fn fake_libm_cbrt(x: f64) -> f64 {
    // SAFETY: cbrt is a pure C function of one double.
    unsafe { cbrt(x) }
}

pub fn run(x: f64) -> f64 {
    fake_libm_cbrt(x)
}
