//! fp-check fixture, the control: IEEE `+ - * /` and `sqrt` only. fp-check must pass.
#[inline(never)]
pub fn affine(x: f64) -> f64 {
    (x * 3.0 + 1.0) / 2.0 - x.sqrt()
}

pub fn run(x: f64) -> f64 {
    affine(x)
}
