//! `Math.round`, `Math.fround` and `Math.f16round` (ECMA-262 §21.3.2.28, §21.3.2.17,
//! sec-math.f16round; H36 row "Exact"). Emitters inline the same algorithms for `round` and
//! `fround` (H36 Emitters); `f16round` is a call.

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

/// `Math.f16round(x)` steps 2-6: `x` rounded to the nearest binary16 value (ties to even), as a
/// double. NaN, the zeros and the infinities stay; a value of magnitude 65520 (the midpoint
/// above the largest binary16, 65504) or more becomes ±∞; a value below 2^-25 in magnitude
/// becomes ±0. One rounding from the double, by integer steps: no `f32` step (that would round
/// twice), no libm, no FMA (H36).
#[inline(never)]
pub fn f16round(x: f64) -> f64 {
    // 2. If n is NaN, return NaN.
    // 3. If n is one of +0𝔽, -0𝔽, +∞𝔽, or -∞𝔽, return n.
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    // 4. Let n16 be the result of converting n to IEEE 754-2019 binary16 format using
    //    roundTiesToEven mode.
    // 5. Let n64 be the result of converting n16 to IEEE 754-2019 binary64 format.
    let b = x.to_bits();
    let sign = b & (1 << 63);
    let field = ((b >> 52) & 0x7ff) as i32;
    if field == 0 {
        // A subnormal double is below 2^-1022: it rounds to ±0.
        return f64::from_bits(sign);
    }
    // |x| = m * 2^(e - 52), with 2^52 <= m < 2^53.
    let e = field - 1023;
    if e > 15 {
        // |x| >= 2^16 > 65520.
        return f64::from_bits(sign | (0x7ff << 52));
    }
    let m = (b & ((1 << 52) - 1)) | (1 << 52);
    // The binary16 step at |x|: 2^(e - 10) for a normal binary16 (11 significant bits), and
    // 2^-24 (the subnormal step) below 2^-14.
    let q = if e >= -14 { e - 10 } else { -24 };
    // Drop s >= 42 low bits of m and round to nearest, ties to even.
    let s = (q - (e - 52)) as u32;
    let r = if s >= 64 {
        // |x| < 2^(q - 11): far below half a step.
        0
    } else {
        let r = m >> s;
        let rest = m & ((1 << s) - 1);
        let half = 1 << (s - 1);
        if rest > half || (rest == half && r & 1 == 1) {
            r + 1
        } else {
            r
        }
    };
    // 6. Return r * 2^q with the sign of x. r <= 2^11 and 2^q is a normal double, so the product
    //    is exact. r = 2^11 at e = 15 gives 2^16: past the binary16 range, so ±∞.
    let mag = r as f64 * f64::from_bits(((q + 1023) as u64) << 52);
    if mag >= 65536.0 {
        return f64::from_bits(sign | (0x7ff << 52));
    }
    f64::from_bits(sign | mag.to_bits())
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

    /// The binary16 value of the bit pattern `h` (sign bit clear), by an independent route:
    /// `f * 2^-24` for a subnormal, `(1024 + f) * 2^(field - 25)` for a normal.
    fn half_value(h: u16) -> f64 {
        let field = i32::from(h >> 10);
        let f = f64::from(h & 0x3ff);
        let pow2 = |k: i32| f64::from_bits(((k + 1023) as u64) << 52);
        if field == 0 {
            f * pow2(-24)
        } else {
            (1024.0 + f) * pow2(field - 25)
        }
    }

    fn next_up(x: f64) -> f64 {
        f64::from_bits(x.to_bits() + 1)
    }

    fn next_down(x: f64) -> f64 {
        f64::from_bits(x.to_bits() - 1)
    }

    /// Every finite binary16 value stays; every midpoint between two neighbours goes to the
    /// even one; one double ulp off the midpoint goes to the nearer one. Both signs.
    #[test]
    fn f16round_every_binary16_value_and_midpoint() {
        for h in 0u16..0x7c00 {
            let lo = half_value(h);
            let even_lo = h & 1 == 0;
            for sign in [1.0, -1.0] {
                let v = sign * lo;
                assert_eq!(bits(f16round(v)), bits(v), "f16round({v:e}) h={h:#x}");
            }
            if h == 0x7bff {
                break;
            }
            let hi = half_value(h + 1);
            let mid = (lo + hi) / 2.0; // exact: binary16 values have 11 significant bits
            let tie = if even_lo { lo } else { hi };
            for sign in [1.0, -1.0] {
                let s = |x: f64| sign * x;
                assert_eq!(bits(f16round(s(mid))), bits(s(tie)), "tie {mid:e}");
                assert_eq!(
                    bits(f16round(s(next_up(mid)))),
                    bits(s(hi)),
                    "above {mid:e}"
                );
                assert_eq!(
                    bits(f16round(s(next_down(mid)))),
                    bits(s(lo)),
                    "below {mid:e}"
                );
            }
        }
    }

    /// Expected values from node 26 `Math.f16round` and from the binary16 limits.
    #[test]
    fn f16round_matches_spec() {
        assert!(f16round(f64::NAN).is_nan());
        assert_eq!(bits(f16round(0.0)), bits(0.0));
        assert_eq!(bits(f16round(-0.0)), bits(-0.0));
        assert_eq!(f16round(f64::INFINITY), f64::INFINITY);
        assert_eq!(f16round(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(f16round(1.337), 1.3369140625);
        assert_eq!(f16round(0.1), 0.0999755859375);
        assert_eq!(f16round(65504.0), 65504.0);
        assert_eq!(f16round(65519.99999999999), 65504.0);
        assert_eq!(f16round(65520.0), f64::INFINITY); // the tie goes to the even 2^16: ∞
        assert_eq!(f16round(-65520.0), f64::NEG_INFINITY);
        assert_eq!(f16round(1e300), f64::INFINITY);
        assert_eq!(f16round(5.960464477539063e-8), 5.960464477539063e-8); // 2^-24
        assert_eq!(bits(f16round(2.9802322387695312e-8)), bits(0.0)); // 2^-25: tie to 0
        assert_eq!(f16round(2.980232238769532e-8), 5.960464477539063e-8); // just above 2^-25
        assert_eq!(bits(f16round(-1e-10)), bits(-0.0));
        assert_eq!(bits(f16round(5e-324)), bits(0.0));
        assert_eq!(bits(f16round(-5e-324)), bits(-0.0));
        assert_eq!(f16round(6.103515625e-5), 6.103515625e-5); // 2^-14, the smallest normal
        assert_eq!(f16round(6.097555160522461e-5), 6.097555160522461e-5); // largest subnormal
        // 1 + 2^-11 + 2^-40 is above the midpoint 1 + 2^-11, so it rounds up to 1 + 2^-10.
        // Double rounding through f32 would give 1: the f32 step drops 2^-40 and leaves the
        // tie 1 + 2^-11, which then goes to the even 1.
        let x = 1.0 + f64::from_bits((1023 - 11) << 52) + f64::from_bits((1023 - 40) << 52);
        assert_eq!(f16round(x), 1.0009765625);
        assert_eq!(f16round(1.00048828125), 1.0); // 1 + 2^-11: tie to even
        assert_eq!(f16round(1.0014648437500002), 1.001953125);
    }
}
