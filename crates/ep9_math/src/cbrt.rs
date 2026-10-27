//! `Math.cbrt`: the correctly rounded cube root (H36), a port of CORE-MATH `cr_cbrt`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/cbrt/cbrt.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly-rounded cubic root of binary64 value.
//! Copyright (c) 2021-2022 Alexei Sibidanov.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The constants and steps are the C ones.
//! ep9 changes: rounding to nearest only (JS), so `rm` is 0, the directed-rounding list
//! `wlist` is gone and no exception flags are saved or restored; every `__builtin_fma` is
//! [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA contraction.
//! There is one path: a cubic Newton step, a linear one with a double-double cube, a second
//! one only near a rounding boundary (fewer than 2^-20 of the inputs), and the exact-case fix.

use crate::exact::FmaOps;
use crate::hf;

#[inline(always)]
fn cbrt_with<F: FmaOps>(x: f64) -> f64 {
    const ESCALE: [f64; 3] = [
        1.0,
        hf("0x1.428a2f98d728bp+0"), // 2^(1/3)
        hf("0x1.965fea53d6e3dp+0"), // 2^(2/3)
    ];
    // The polynomial c0+c1*x+c2*x^2+c3*x^3 approximates x^(1/3) on [1,2] with maximal error
    // < 9.2e-5 (attained at x=2).
    const C: [f64; 4] = [
        hf("0x1.1b0babccfef9cp-1"),
        hf("0x1.2c9a3e94d1da5p-1"),
        hf("-0x1.4dc30b1a1ddbap-3"),
        hf("0x1.7a8d3e4ec9b07p-6"),
    ];
    const U0: f64 = hf("0x1.5555555555555p-2");
    const U1: f64 = hf("0x1.c71c71c71c71cp-3");
    const RSC: [f64; 6] = [1.0, -1.0, 0.5, -0.5, 0.25, -0.25];
    const OFF: f64 = hf("0x1p-53"); // off[rm] for rm = 0 (to nearest)

    let hx = x.to_bits();
    let mut mant = hx & (!0u64 >> 12);
    let sign = hx >> 63;
    let mut e = ((hx >> 52) & 0x7ff) as i64;
    if ((e + 1) & 0x7ff) < 2 {
        let ix = hx & (!0u64 >> 1);
        if e == 0x7ff || ix == 0 {
            return x + x; // 0, inf, nan
        }
        let nz = ix.leading_zeros() as i64 - 11; // subnormal
        mant <<= nz;
        mant &= !0u64 >> 12;
        e -= nz - 1;
    }
    e += 3072;
    let cvt1 = mant | (0x3ffu64 << 52);
    let (et, it) = (e / 3, e % 3);
    // 2^(3k+it) <= x < 2^(3k+it+1), with 0 <= it <= 3
    let mut cvt5 = cvt1 + ((it as u64) << 52);
    cvt5 |= sign << 63;
    let zz = f64::from_bits(cvt5);
    // cbrt(x) = cbrt(zz)*2^(et-1365) where 1 <= |zz| < 8
    let isc = ESCALE[it as usize].to_bits() | (sign << 63);
    let z = f64::from_bits(cvt1);
    // cbrt(zz) = cbrt(z)*isc, where isc encodes 1, 2^(1/3) or 2^(2/3), and 1 <= z < 2
    let r = 1.0 / z;
    let rr = r * RSC[((it as usize) << 1) | sign as usize];
    let z2 = z * z;
    let c0 = C[0] + z * C[1];
    let c2 = C[2] + z * C[3];
    let mut y = c0 + z2 * c2;
    let mut y2 = y * y;
    // y is an approximation of z^(1/3)
    let mut h = y2 * (y * r) - 1.0;
    // h determines the error between y and z^(1/3)
    y -= (h * y) * (U0 - U1 * h);
    // A cubic variant of Newton's method, with the function f(y) = 1-z/y^3.
    y *= f64::from_bits(isc);
    // Now y is an approximation of zz^(1/3), and rr an approximation of 1/zz. Another
    // iteration of Newton-Raphson, this time with a linear approximation only.
    y2 = y * y;
    let mut y2l = F::fma(y, y, -y2);
    // y2 + y2l = y^2 exactly
    let mut y3 = y2 * y;
    let mut y3l = F::fma(y, y2, -y3) + y * y2l;
    // y3 + y3l approximates y^3 with about 106 bits of accuracy
    h = ((y3 - zz) + y3l) * rr;
    let mut dy = h * (y * U0);
    // the approximation of zz^(1/3) is y - dy
    let mut y1 = y - dy;
    dy = (y - y1) - dy;
    // the approximation of zz^(1/3) is now y1 + dy, where |dy| < 1/2 ulp(y)
    let mut ady = dy.abs();
    // For rounding to nearest, ady0 is tiny when dy is near 1/2 ulp(1), or 3/2 ulp(1).
    let mut ady0 = (ady - OFF).abs();
    let mut ady1 = (ady - (const { hf("0x1p-52") } + OFF)).abs();
    if ady0 < const { hf("0x1p-75") } || ady1 < const { hf("0x1p-75") } {
        y2 = y1 * y1;
        y2l = F::fma(y1, y1, -y2);
        y3 = y2 * y1;
        y3l = F::fma(y1, y2, -y3) + y1 * y2l;
        h = ((y3 - zz) + y3l) * rr;
        dy = h * (y1 * U0);
        y = y1 - dy;
        dy = (y1 - y) - dy;
        y1 = y;
        ady = dy.abs();
        ady0 = (ady - OFF).abs();
        ady1 = (ady - (const { hf("0x1p-52") } + OFF)).abs();
        if ady0 < const { hf("0x1p-98") } || ady1 < const { hf("0x1p-98") } {
            let azz = zz.abs();
            if azz == const { hf("0x1.9b78223aa307cp+1") } {
                // ~ 0x1.79d15d0e8d59b80000000000000ffc3dp+0
                y1 = const { hf("0x1.79d15d0e8d59cp+0") }.copysign(zz);
            }
            if azz == const { hf("0x1.a202bfc89ddffp+2") } {
                // ~ 0x1.de87aa837820e80000000000001c0f08p+0
                y1 = const { hf("0x1.de87aa837820fp+0") }.copysign(zz);
            }
        }
    }
    let mut cvt3 = y1.to_bits().wrapping_add(((et - 342 - 1023) as u64) << 52);
    let m0 = (cvt3 << 30) as i64;
    let m1 = m0 >> 63;
    if ((m0 ^ m1) as u64) <= (1u64 << 30) {
        let cvt4 = (y1.to_bits() + (1u64 << 15)) & 0xffffffffffff0000;
        if ((f64::from_bits(cvt4) - y1) - dy).abs() < const { hf("0x1p-60") } || zz.abs() == 1.0 {
            cvt3 = (cvt3 + (1u64 << 15)) & 0xffffffffffff0000;
        }
    }
    f64::from_bits(cvt3)
}

multiversion! {
    /// The cube root, correctly rounded (round to nearest, ties to even): CORE-MATH `cr_cbrt`.
    /// The same bits on every target. `Math.cbrt` (§21.3.2.9): NaN, ±0 and ±∞ give themselves.
    pub fn cbrt(x: f64) -> f64 = cbrt_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbrt_special_values() {
        assert!(cbrt(f64::NAN).is_nan());
        assert_eq!(cbrt(f64::INFINITY), f64::INFINITY);
        assert_eq!(cbrt(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(cbrt(0.0).to_bits(), 0);
        assert_eq!(cbrt(-0.0).to_bits(), 1 << 63);
        assert_eq!(cbrt(8.0), 2.0);
        assert_eq!(cbrt(-27.0), -3.0);
        assert_eq!(cbrt(1.0), 1.0);
        assert_eq!(cbrt(hf("0x1p-1074")), hf("0x1p-358"));
    }
}
