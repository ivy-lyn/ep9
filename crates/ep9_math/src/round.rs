//! `Math.round` and `Math.fround` (ECMA-262 §21.3.2.28, §21.3.2.17; H36 row "Exact"). Emitters
//! inline the same algorithms (H36 Emitters).

/// 2^52: every double with at least this magnitude is an integer.
const TWO_52: f64 = 4_503_599_627_370_496.0;

/// `Math.round(x)`: the integer closest to `x`, ties toward +∞. Exact; keeps `-0` and returns
/// `-0` for `x` in [-0.5, -0) (spec steps 3-4).
#[inline(never)]
pub fn round_js(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 || x.abs() >= TWO_52 {
        return x;
    }
    if x > 0.0 && x < 0.5 {
        return 0.0;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    // |x| < 2^52, so `x - floor(x)` is exact and `f + 1.0` is exact. No `floor(x + 0.5)`: that
    // rounds 0.49999999999999994 up to 1 and odd values above 2^52 - 1 up by 2.
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `Math.fround(x)`: `x` rounded to the nearest float32 (ties to even), as a double. NaN, the
/// zeros and the infinities stay; too large values become ±∞.
#[inline(never)]
pub fn fround(x: f64) -> f64 {
    f64::from(x as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    /// Expected values from node 24 `Math.round`.
    #[test]
    fn round_js_matches_spec() {
        let cases: &[(f64, f64)] = &[
            (0.5, 1.0),
            (1.5, 2.0),
            (2.5, 3.0),
            (-0.5, -0.0),
            (-1.5, -1.0),
            (-2.5, -2.0),
            (-2.6, -3.0),
            (-2.4, -2.0),
            (0.49999999999999994, 0.0),
            (-0.49999999999999994, -0.0),
            (0.5000000000000001, 1.0),
            (-0.5000000000000001, -1.0),
            (4503599627370495.5, 4503599627370496.0),
            (-4503599627370495.5, -4503599627370495.0),
            (4503599627370497.0, 4503599627370497.0),
            (9007199254740993.0, 9007199254740992.0), // the literal rounds to 2^53
            (1e300, 1e300),
            (5e-324, 0.0),
            (-5e-324, -0.0),
            (0.0, 0.0),
            (-0.0, -0.0),
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        ];
        for &(x, want) in cases {
            assert_eq!(bits(round_js(x)), bits(want), "Math.round({x:e})");
        }
        assert!(round_js(f64::NAN).is_nan());
        // Every half integer in a range rounds up, every other value to the nearest integer.
        for i in -2000i32..2000 {
            let x = f64::from(i) / 4.0;
            let want = (f64::from(i) / 4.0 + 0.5).floor(); // exact for these small values
            let want = if want == 0.0 && x < 0.0 { -0.0 } else { want };
            assert_eq!(bits(round_js(x)), bits(want), "Math.round({x})");
        }
    }

    /// Expected values from node 24 `Math.fround`.
    #[test]
    fn fround_matches_spec() {
        assert_eq!(fround(5.5), 5.5);
        assert_eq!(fround(5.05), 5.050000190734863);
        assert_eq!(fround(0.1), 0.10000000149011612);
        assert_eq!(fround(1.0000000596046448), 1.0); // tie: to even
        assert_eq!(fround(1.0000001788139343), 1.000000238418579); // tie: to even (up)
        assert_eq!(fround(3.4028235677973366e38), f64::INFINITY);
        assert_eq!(fround(3.4028234663852886e38), 3.4028234663852886e38);
        assert_eq!(fround(1e-46), 0.0);
        assert_eq!(bits(fround(-1e-46)), bits(-0.0));
        assert_eq!(fround(1.401298464324817e-45), 1.401298464324817e-45);
        assert_eq!(bits(fround(-0.0)), bits(-0.0));
        assert_eq!(fround(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert!(fround(f64::NAN).is_nan());
    }
}
