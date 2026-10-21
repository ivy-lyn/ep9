//! `**` and `Math.pow` (ECMA-262 Number::exponentiate, §6.1.6.1.3; H36 Decision "pow").
//!
//! v0 (M0 week 4) answers every input whose result the spec fixes, and every input with an exact
//! result. The rest needs the correctly rounded core `pow_cr` (CORE-MATH algorithm, month 2,
//! H36 Fix-by M1); for those inputs [`pow_js`] returns `None` and the caller throws
//! "not implemented".
//!
//! | Order | Inputs | Result | Source |
//! |---|---|---|---|
//! | 1-4 | `y` NaN; `y` ±0; `x` NaN; `y` ±∞ with \|x\| = 1 | NaN; 1; NaN; NaN | H36 steps 1-4 |
//! | 5 | `y` = 2, 0.5, 1, -1 | `x*x`; `sqrt(x + 0)` (-∞ gives +∞); `x`; `1/x` | H36 step 5: each is the CR result |
//! | 6 | `x` ±0 or ±∞; `y` ±∞; `x` < 0 with `y` not an integer; `x` = 1; `x` = -1 with `y` an integer | the spec values (C99 Annex F agrees after steps 1-4) | H36 step 6, special cases |
//! | 7 | integer `x`, integer `y` in 0..=64, \|result\| <= 2^53 | the exact integer | the M0 `pow_exact` (ep9_vm numconv.rs) |
//! | - | everything else: `x` finite, nonzero, not ±1; `y` finite, nonzero | `None` (v0) | `pow_cr`, month 2 |
//!
//! No `powi`, no repeated multiplication of non-integers, no host `pow` (H36).

/// `x ** y` (Number::exponentiate). `None`: the result needs the correctly rounded core, which
/// lands in month 2 (module doc); every `Some` is the exact or correctly rounded result.
#[inline(never)]
pub fn pow_js(x: f64, y: f64) -> Option<f64> {
    // H36 steps 1-4 (JS, where JS differs from C99 Annex F).
    if y.is_nan() {
        return Some(f64::NAN);
    }
    if y == 0.0 {
        return Some(1.0);
    }
    if x.is_nan() {
        return Some(f64::NAN);
    }
    if y.is_infinite() && x.abs() == 1.0 {
        return Some(f64::NAN);
    }
    // H36 step 5. Each result is correctly rounded: one IEEE operation on exact inputs.
    if y == 2.0 {
        return Some(x * x);
    }
    if y == 0.5 {
        // `x + 0.0` turns -0 into +0 (pow(-0, 0.5) = +0); sqrt(-∞) would be NaN.
        return Some(if x == f64::NEG_INFINITY {
            f64::INFINITY
        } else {
            (x + 0.0).sqrt()
        });
    }
    if y == 1.0 {
        return Some(x);
    }
    if y == -1.0 {
        return Some(1.0 / x);
    }
    // H36 step 6: the special values, then the exact integer path.
    special(x, y).or_else(|| pow_exact(x, y))
}

/// `y` is an odd integer. Every double of magnitude 2^53 or more is even.
fn is_odd_integer(y: f64) -> bool {
    let a = y.abs();
    a < 9_007_199_254_740_992.0 && a.fract() == 0.0 && (a as u64) & 1 == 1
}

/// ECMA-262 Number::exponentiate steps 4-12 (C99 Annex F F.10.4.4 gives the same values once
/// H36 steps 1-4 have run), plus `x` = ±1 with a finite `y`. Called with `y` not NaN, not ±0,
/// `x` not NaN, and not (`y` ±∞ with |x| = 1).
fn special(x: f64, y: f64) -> Option<f64> {
    let odd = is_odd_integer(y);
    if x.is_infinite() {
        return Some(match (x > 0.0, y > 0.0) {
            (true, true) => f64::INFINITY,
            (true, false) => 0.0,
            (false, true) if odd => f64::NEG_INFINITY,
            (false, true) => f64::INFINITY,
            (false, false) if odd => -0.0,
            (false, false) => 0.0,
        });
    }
    if x == 0.0 {
        let neg = x.is_sign_negative() && odd;
        return Some(match (y > 0.0, neg) {
            (true, false) => 0.0,
            (true, true) => -0.0,
            (false, false) => f64::INFINITY,
            (false, true) => f64::NEG_INFINITY,
        });
    }
    if y.is_infinite() {
        // |x| != 1 here.
        return Some(if (x.abs() > 1.0) == (y > 0.0) {
            f64::INFINITY
        } else {
            0.0
        });
    }
    // x and y are finite and nonzero.
    if x < 0.0 && y.fract() != 0.0 {
        return Some(f64::NAN);
    }
    if x == 1.0 {
        return Some(1.0);
    }
    if x == -1.0 {
        // y is an integer here (the line above returned otherwise).
        return Some(if odd { -1.0 } else { 1.0 });
    }
    None
}

/// Number::exponentiate for the cases M0 could answer exactly: the spec's special cases for NaN
/// and zero exponents, exponent 1, and integer bases with small non-negative integer exponents
/// whose result is an integer of at most 2^53. `None` for everything else (H36: no libm `pow`).
///
/// Moved unchanged from ep9_vm `numconv::pow_exact` (M0 week 4). [`pow_js`] runs it last.
fn pow_exact(base: f64, exp: f64) -> Option<f64> {
    if exp.is_nan() {
        return Some(f64::NAN);
    }
    if exp == 0.0 {
        return Some(1.0);
    }
    if base.is_nan() {
        return Some(f64::NAN);
    }
    if exp == 1.0 {
        return Some(base);
    }
    if base == 0.0 && exp > 0.0 && exp.fract() == 0.0 {
        let odd = crate::fmod(exp, 2.0) == 1.0;
        return Some(if base.is_sign_negative() && odd {
            -0.0
        } else {
            0.0
        });
    }
    if base.fract() != 0.0 || !(0.0..=64.0).contains(&exp) || exp.fract() != 0.0 {
        return None;
    }
    if base.abs() > 9_007_199_254_740_992.0 {
        return None;
    }
    let b = base as i128;
    let mut r: i128 = 1;
    for _ in 0..exp as u32 {
        r = r.checked_mul(b)?;
        if r.unsigned_abs() > 1u128 << 53 {
            return None;
        }
    }
    Some(r as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f64 = f64::INFINITY;
    const NEG_INF: f64 = f64::NEG_INFINITY;

    /// Same bits, or both NaN.
    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    fn check(x: f64, y: f64, want: Option<f64>) {
        let got = pow_js(x, y);
        let ok = match (got, want) {
            (Some(g), Some(w)) => same(g, w),
            (None, None) => true,
            _ => false,
        };
        assert!(ok, "pow_js({x:e}, {y:e}) = {got:?}, want {want:?}");
    }

    /// The ep9_vm numconv.rs test of `pow_exact`, moved unchanged (M0 week 4).
    #[test]
    fn pow_exact_cases() {
        assert_eq!(pow_exact(2.0, 10.0), Some(1024.0));
        assert_eq!(pow_exact(-3.0, 3.0), Some(-27.0));
        assert_eq!(pow_exact(7.5, 0.0), Some(1.0));
        assert!(pow_exact(f64::NAN, 0.0) == Some(1.0));
        assert!(pow_exact(2.0, f64::NAN).unwrap().is_nan());
        assert_eq!(pow_exact(1.25, 1.0), Some(1.25));
        assert!(pow_exact(-0.0, 3.0).unwrap().is_sign_negative());
        assert_eq!(pow_exact(2.0, 54.0), None);
        assert_eq!(pow_exact(2.0, 0.5), None);
        assert_eq!(pow_exact(1.5, 2.0), None);
    }

    /// The M0 `numconv::exponentiate` (pow_exact plus the infinite-exponent rules), kept as the
    /// reference: wherever it answered, `pow_js` gives the same bits.
    fn m0_exponentiate(base: f64, exp: f64) -> Option<f64> {
        if let Some(r) = pow_exact(base, exp) {
            return Some(r);
        }
        if exp.is_infinite() && !base.is_nan() {
            let a = base.abs();
            return Some(if a == 1.0 {
                f64::NAN
            } else if (a > 1.0) == (exp > 0.0) {
                f64::INFINITY
            } else {
                0.0
            });
        }
        None
    }

    fn sample_values() -> Vec<f64> {
        let mut v = vec![
            0.0,
            1.0,
            2.0,
            3.0,
            0.5,
            0.25,
            1.5,
            7.0,
            10.0,
            64.0,
            65.0,
            53.0,
            0.1,
            1e-300,
            1e300,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            9_007_199_254_740_992.0,
            9_007_199_254_740_994.0,
            0.999_999_999_999_999,
            1.000_000_000_000_001,
            std::f64::consts::PI,
            INF,
            f64::NAN,
        ];
        let neg: Vec<f64> = v.iter().map(|x| -x).collect();
        v.extend(neg);
        v
    }

    #[test]
    fn pow_js_keeps_every_m0_answer() {
        let vals = sample_values();
        let mut answered = 0;
        for &x in &vals {
            for &y in &vals {
                if let Some(old) = m0_exponentiate(x, y) {
                    answered += 1;
                    check(x, y, Some(old));
                }
            }
        }
        assert!(answered > 300, "{answered}");
        for x in -40i32..=40 {
            for y in 0..=64 {
                let (x, y) = (f64::from(x), f64::from(y));
                if let Some(old) = m0_exponentiate(x, y) {
                    check(x, y, Some(old));
                }
            }
        }
    }

    /// test262 language/expressions/exponentiation applying-the-exp-operator_A1-A23 and the
    /// ECMA-262 Number::exponentiate steps, one row each.
    #[test]
    fn pow_js_spec_special_values() {
        let max = f64::MAX;
        let pi = std::f64::consts::PI;
        // A1-A3: NaN exponent, zero exponent, NaN base.
        check(1.0, f64::NAN, Some(f64::NAN));
        check(f64::NAN, -0.0, Some(1.0));
        check(f64::NAN, 1.5, Some(f64::NAN));
        // A4-A9: y = ±∞.
        for x in [1.0, -1.0] {
            check(x, INF, Some(f64::NAN));
            check(x, NEG_INF, Some(f64::NAN));
        }
        for x in [1.000_000_000_000_001, 2.0, max, -2.0, NEG_INF, INF] {
            check(x, INF, Some(INF));
            check(x, NEG_INF, Some(0.0));
        }
        for x in [
            0.999_999_999_999_999,
            0.5,
            0.0,
            -0.0,
            -0.5,
            -0.999_999_999_999_999,
        ] {
            check(x, INF, Some(0.0));
            check(x, NEG_INF, Some(INF));
        }
        // A11-A12: x = +∞.
        for y in [1e-15, 1.0, max, INF, 2.0, 0.5] {
            check(INF, y, Some(INF));
            check(INF, -y, Some(0.0));
        }
        // A13-A16: x = -∞.
        for y in [1.0, 111.0, 111_111.0] {
            check(NEG_INF, y, Some(NEG_INF));
            check(NEG_INF, -y, Some(-0.0));
        }
        for y in [1e-15, 2.0, pi, max, INF, 0.5] {
            check(NEG_INF, y, Some(INF));
            check(NEG_INF, -y, Some(0.0));
        }
        // A17-A22: x = ±0.
        for y in [1e-15, 1.0, max, INF, 2.0, 0.5] {
            check(0.0, y, Some(0.0));
            check(0.0, -y, Some(INF));
        }
        for y in [1.0, 111.0, 111_111.0] {
            check(-0.0, y, Some(-0.0));
            check(-0.0, -y, Some(NEG_INF));
        }
        for y in [1e-15, 2.0, pi, max, INF, 0.5] {
            check(-0.0, y, Some(0.0));
            check(-0.0, -y, Some(INF));
        }
        // A23: negative finite base, finite non-integer exponent.
        for x in [-max, -pi, -1.0, -0.5, -5e-324] {
            for y in [-pi, -0.1, 0.1, pi, 1e-15] {
                check(x, y, Some(f64::NAN));
            }
        }
        // ±1 bases with finite exponents.
        check(1.0, 1e300, Some(1.0));
        check(1.0, -0.1, Some(1.0));
        check(-1.0, 1e300, Some(1.0));
        check(-1.0, 9_007_199_254_740_991.0, Some(-1.0));
        check(-1.0, -3.0, Some(-1.0));
        // Odd-integer test at the 2^53 edge: 2^53 + 2 is even, 2^53 - 1 is odd.
        check(-0.0, 9_007_199_254_740_994.0, Some(0.0));
        check(-0.0, 9_007_199_254_740_991.0, Some(-0.0));
        check(NEG_INF, -9_007_199_254_740_991.0, Some(-0.0));
    }

    /// H36 step 5, and inputs that v0 leaves to `pow_cr`.
    #[test]
    fn pow_js_step5_and_v0_gaps() {
        check(1.5, 2.0, Some(2.25));
        check(-3.0, 2.0, Some(9.0));
        check(1e200, 2.0, Some(INF));
        check(0.1, 2.0, Some(0.1 * 0.1));
        check(2.0, 0.5, Some(std::f64::consts::SQRT_2));
        check(-0.0, 0.5, Some(0.0));
        check(-4.0, 0.5, Some(f64::NAN));
        check(NEG_INF, 0.5, Some(INF));
        check(2.0, -1.0, Some(0.5));
        check(-0.0, -1.0, Some(NEG_INF));
        check(3.0, -1.0, Some(1.0 / 3.0));
        check(0.1, 1.0, Some(0.1));
        // Exact integer path.
        check(2.0, 32.0, Some(4_294_967_296.0));
        check(-2.0, 53.0, Some(-9_007_199_254_740_992.0));
        check(10.0, 15.0, Some(1e15));
        // Left for the correctly rounded core (month 2).
        check(2.0, 0.3, None);
        check(2.0, 54.0, None);
        check(2.0, -2.0, None);
        check(10.0, -1.5, None);
        check(1.5, 3.0, None);
        check(-8.0, 1.0 / 3.0, Some(f64::NAN)); // negative base, non-integer exponent
        check(-1.5, 3.0, None);
        check(0.5, 1e300, None);
    }
}
