pub fn a(x: f64) -> f64 { x.sin() + x.exp_m1() + x.ln_1p() + x.sin_cos().0 }
pub fn b(x: f32) -> f32 { x.powi(3) + x.algebraic_mul(2.0) }
pub fn c(x: f64) -> f64 { x.sqrt() + x.floor() + x.round_ties_even() + x.abs() + x.to_degrees() }
