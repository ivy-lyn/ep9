//! Exact remainder for JS `%` (Number::remainder, ECMA-262 §6.1.6.1.6; H36 row `%`).
//!
//! Rust `%` on floats lowers to the C `fmod` on every target, so engine code calls this function
//! instead (`cargo xtask fp-check` bans the `fmod` symbol).
//!
//! Ported for ep9 from musl src/math/fmod.c at v1.2.6 (9fa28ece75d8a2191de7c5bb53bed224c5947417;
//! the file last changed in ee2ee92d62c4, 2013-09-05). MIT, Copyright © 2005-2020 Rich Felker,
//! et al. (THIRD-PARTY-LICENSE, "musl"). ep9 changes: the NaN case returns `f64::NAN` instead of
//! `(x*y)/(x*y)` (JS has one NaN), and `for` loops became `while` loops.

/// Exact IEEE-754 remainder with the sign of `x` (JS `%`). A port of musl `src/math/fmod.c`
/// (MIT, Copyright 2005-2020 Rich Felker et al.).
#[inline(never)]
pub fn fmod(x: f64, y: f64) -> f64 {
    let mut uxi = x.to_bits();
    let mut uyi = y.to_bits();
    let mut ex = ((uxi >> 52) & 0x7ff) as i64;
    let mut ey = ((uyi >> 52) & 0x7ff) as i64;
    let sx = uxi >> 63;

    if uyi << 1 == 0 || y.is_nan() || ex == 0x7ff {
        return f64::NAN;
    }
    if uxi << 1 <= uyi << 1 {
        if uxi << 1 == uyi << 1 {
            return 0.0 * x;
        }
        return x;
    }

    // Normalize x and y.
    if ex == 0 {
        let mut i = uxi << 12;
        while i >> 63 == 0 {
            ex -= 1;
            i <<= 1;
        }
        uxi <<= (-ex + 1) as u32;
    } else {
        uxi &= u64::MAX >> 12;
        uxi |= 1 << 52;
    }
    if ey == 0 {
        let mut i = uyi << 12;
        while i >> 63 == 0 {
            ey -= 1;
            i <<= 1;
        }
        uyi <<= (-ey + 1) as u32;
    } else {
        uyi &= u64::MAX >> 12;
        uyi |= 1 << 52;
    }

    // x mod y.
    while ex > ey {
        let i = uxi.wrapping_sub(uyi);
        if i >> 63 == 0 {
            if i == 0 {
                return 0.0 * x;
            }
            uxi = i;
        }
        uxi <<= 1;
        ex -= 1;
    }
    let i = uxi.wrapping_sub(uyi);
    if i >> 63 == 0 {
        if i == 0 {
            return 0.0 * x;
        }
        uxi = i;
    }
    while uxi >> 52 == 0 {
        uxi <<= 1;
        ex -= 1;
    }

    // Scale the result.
    if ex > 0 {
        uxi -= 1 << 52;
        uxi |= (ex as u64) << 52;
    } else {
        uxi >>= (-ex + 1) as u32;
    }
    uxi |= sx << 63;
    f64::from_bits(uxi)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ep9_vm numconv.rs test, moved unchanged (M0 week 4).
    #[test]
    fn fmod_is_exact_and_follows_js() {
        assert_eq!(fmod(5.5, 2.0), 1.5);
        assert_eq!(fmod(-5.5, 2.0), -1.5);
        assert_eq!(fmod(5.0, -3.0), 2.0);
        assert!(fmod(-4.0, 2.0).is_sign_negative());
        assert!(fmod(1.0, 0.0).is_nan());
        assert!(fmod(f64::INFINITY, 2.0).is_nan());
        assert!(fmod(f64::NAN, 2.0).is_nan());
        assert_eq!(fmod(3.0, f64::INFINITY), 3.0);
        assert!(fmod(-0.0, 5.0).is_sign_negative());
        // Reference values: node 0.1 % 0.01, 1e300 % 3 (exact remainders).
        assert_eq!(fmod(0.1, 0.01), 3.469446951953614e-18);
        assert_eq!(fmod(1e300, 3.0), 0.0);
        assert_eq!(fmod(5e-324, 2e-323), 5e-324);
        // A plain reference: exact for these integer pairs.
        for a in -50i32..50 {
            for b in [-7i32, -3, 1, 2, 5, 11] {
                let r = fmod(f64::from(a), f64::from(b));
                assert_eq!(r, f64::from(a % b), "{a} % {b}");
            }
        }
    }

    /// More bit-exact vectors: zero signs, subnormals, huge exponent gaps, and NaN for every
    /// JS NaN case. Expected values are exact remainders (checked with node `%`).
    #[test]
    fn fmod_edges() {
        let bits = |x: f64| x.to_bits();
        // The result keeps the sign of x, also when it is zero.
        assert_eq!(bits(fmod(-6.0, 3.0)), bits(-0.0));
        assert_eq!(bits(fmod(6.0, -3.0)), bits(0.0));
        assert_eq!(bits(fmod(-0.0, f64::INFINITY)), bits(-0.0));
        // |x| < |y|: x unchanged.
        assert_eq!(bits(fmod(-5e-324, 1.0)), bits(-5e-324));
        assert_eq!(fmod(f64::MAX, f64::INFINITY), f64::MAX);
        // Huge quotient: 2^1023 mod 3 = 2 (2^odd mod 3), and the max subnormal divisor.
        assert_eq!(fmod(8.98846567431158e307, 3.0), 2.0);
        assert_eq!(fmod(f64::MAX, 1.0), 0.0);
        let max_sub = f64::from_bits((1 << 52) - 1);
        assert_eq!(fmod(f64::MIN_POSITIVE, max_sub), 5e-324);
        // NaN cases (JS Number::remainder steps 1-3).
        for (x, y) in [
            (f64::NAN, 1.0),
            (1.0, f64::NAN),
            (f64::INFINITY, 1.0),
            (f64::NEG_INFINITY, f64::INFINITY),
            (1.0, 0.0),
            (1.0, -0.0),
            (0.0, 0.0),
        ] {
            assert!(fmod(x, y).is_nan(), "{x} % {y}");
        }
    }
}
