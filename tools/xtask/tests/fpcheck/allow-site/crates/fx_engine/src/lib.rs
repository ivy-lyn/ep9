//! fp-check fixture: an engine crate turns off the root clippy.toml ban. Only
//! crates/ep9_math/src/exact.rs may; fp-check must name `fx_engine::allowed_sqrt`. The object
//! itself is clean (sqrt is exact and inline), so the source scan alone fails it.
#[allow(clippy::disallowed_methods)]
#[inline(never)]
pub fn allowed_sqrt(x: f64) -> f64 {
    x.sqrt()
}

pub fn run(x: f64) -> f64 {
    allowed_sqrt(x)
}
