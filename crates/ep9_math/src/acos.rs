//! `Math.acos`: the correctly rounded arc cosine (H36), a port of CORE-MATH `cr_acos`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/acos/acos.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded arccosine function for binary64 values.
//! Copyright (c) 2024-2025 Alexei Sibidanov.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The constants and steps are the C ones.
//! The tables `cc`, `s` and `c` are the same as in asin.c (the C comments say so, and their C
//! text is equal), so this file uses the `crate::asin` copies, with its double-double helpers
//! (the C `muldd` of acos.c multiplies its operands in the other order, which gives the same
//! IEEE results). ep9 changes: rounding to nearest only (JS); no `errno`; every
//! `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA
//! contraction; `roundeven_finite` of a value below 2^51 in magnitude is the 1.5 * 2^52 trick.

use crate::asin::{C, CC, S, fastsum, fasttwosum, muldd, polydd, roundeven_small};
use crate::exact::FmaOps;
use crate::hf;

/// Handbook of Floating-Point Arithmetic, Algorithm 4.4 (2Sum).
#[inline(always)]
fn twosum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let a_prime = s - b;
    let b_prime = s - a_prime;
    let delta_a = a - a_prime;
    let delta_b = b - b_prime;
    (s, delta_a + delta_b)
}

#[inline(always)]
fn sum(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let (sh, sl) = twosum(xh, ch);
    (sh, (xl + cl) + sl)
}

#[inline(always)]
fn acos_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    let ax = ix << 1;
    let (mut t, z, zl, jd, f0h, f0l, eps);
    if ax > 0x7fc0000000000000 {
        // |x| > 0.5
        const OFF: [[f64; 2]; 2] = [
            [0.0, 0.0],
            [hf("0x1.921fb54442d18p+1"), hf("0x1.1a62633145c07p-53")],
        ];
        let k = (ix >> 63) as usize;
        f0h = OFF[k][0];
        f0l = OFF[k][1];
        if ax >= 0x7fe0000000000000 {
            // |x| >= 1
            if ax == 0x7fe0000000000000 {
                return f0h + f0l; // |x| = 1
            }
            if ax > 0xffe0000000000000 {
                return x + x; // nan
            }
            return f64::NAN; // |x| > 1
        }
        // For x > 0.5, the double angle formula acos(x) = 2*asin(sqrt((1-x)/2)), and for
        // x < -0.5, acos(x) = pi - 2*asin(sqrt((1+x)/2)).
        t = 2.0 - 2.0 * x.abs();
        jd = roundeven_small(t * const { hf("0x1p5") });
        z = t.sqrt().copysign(x);
        zl = F::fma(z, z, -t) * ((-0.5 / t) * z);
        t = 0.25 * t - jd * const { hf("0x1p-7") };
        eps = (z * t).abs() * const { hf("0x1.8cp-52") } + const { hf("0x1p-105") };
    } else {
        // |x| <= 0.5
        f0h = const { hf("0x1.921fb54442d18p+0") };
        f0l = const { hf("0x1.1a62633145c07p-54") };
        if ax <= 0x7e00000000000000 {
            // |x| < 2^-15
            const C3: f64 = hf("-0x1.5555555555555p-3");
            // No spurious underflow for |x| <= x0 := 0x1.cb3b3869747f4p-55; for those lb = ub,
            // so the accurate path is never called.
            let v = if ax <= 0x791967670d2e8fe8 {
                0.0
            } else {
                (x * x) * (C3 * x)
            };
            let (h, w) = fasttwosum(f0h, -x);
            let l = v + (w + f0l);
            const EPS1: f64 = hf("0x1.34p-79");
            let lb = h + (l - EPS1);
            let ub = h + (l + EPS1);
            if lb != ub {
                return as_acos_refine(x, lb);
            }
            return lb;
        }
        // For 2^-15 <= |x| <= 0.5, acos(x) = pi/2 - asin(x), so the asin argument range is the
        // same for both branches and the tables are shared.
        t = x * x;
        jd = roundeven_small(t * const { hf("0x1p7") });
        t = F::fma(x, x, const { hf("-0x1p-7") } * jd);
        z = -x;
        zl = 0.0;
        // eps < 0 for x > 0, but the rounding test is still correct
        eps = (z * t) * const { hf("0x1.81p-52") };
    }
    // asin(xh+xl) = (xh + xl)*(cc[j][0] + (cc[j][1] + t*Poly(t, cc[j]+2))) where
    // t = xh^2 - j/128 and j = round(128*xh^2)
    let c = &CC[jd as usize];
    let t2 = t * t;
    let d = t * ((c[2] + t * c[3]) + t2 * ((c[4] + t * c[5]) + t2 * (c[6] + t * c[7])));
    let fh = c[0];
    let fl = c[1] + d;
    let (fh, fl) = muldd::<F>(z, zl, fh, fl);
    let (fh, fl) = fastsum(f0h, f0l, fh, fl);
    let lb = fh + (fl - eps);
    let ub = fh + (fl + eps);
    if lb != ub {
        return as_acos_refine(x, lb);
    }
    lb
}

/// `phi` is the fast path approximation of acos(x).
#[inline(always)]
fn as_acos_refine_with<F: FmaOps>(x: f64, phi: f64) -> f64 {
    // Consider x as cos(phi), then sin(phi) is ch + cl = sqrt(1-x^2). The angle rotation
    // formula brings the argument close to zero, where the asin Taylor expansion works well:
    // acos(x) = asin(sqrt(1-x^2)) for x > 0, acos(x) = pi+asin(-sqrt(1-x^2)) for x < 0.
    let s2 = x * x;
    let dx2 = F::fma(x, x, -s2);
    let (c2h, mut c2l) = fasttwosum(1.0, -s2);
    c2l -= dx2;
    let (c2h, c2l) = fasttwosum(c2h, c2l);
    // c2h+c2l approximates 1-x^2
    let ch = c2h.sqrt();
    let cl = (c2l - F::fma(ch, ch, -c2h)) * (0.5 / ch);
    // now ch+cl approximates sqrt(1-x^2)
    let jf = roundeven_small(
        (phi - const { hf("0x1.921fb54442d18p+0") }).abs() * const { hf("0x1.45f306dc9c883p+4") },
    ) as i32;
    // jf = round(|phi-pi/2|*64/pi), 0 <= jf <= 32
    let j = jf as usize;
    let (ch_, cl_, sh, sl) = (S[32 - j][1], S[32 - j][0], S[j][1], S[j][0]);
    // sin(delta) ~ sgn(x) * [(Ch+Cl)*(dsh+dsl) - (Sh+Sl)*(dch+dcl)]
    let ax = x.abs();
    let dsh = ax - sh;
    let dsl = -sl;
    let dch = ch - ch_;
    let dcl = cl - cl_;
    const MAGIC: f64 = hf("0x1.8p-4");
    let sc = F::fma(sh, dch, MAGIC) - MAGIC;
    let dsc = F::fma(sh, dch, -sc);
    let cs = F::fma(ch_, dsh, MAGIC) - MAGIC;
    let dcs = F::fma(ch_, dsh, -cs);
    let v = cs - sc; // exact
    let dv = ((ch_ * dsl + cl_ * dsh) - (sh * dcl + sl * dch)) - (dsc - dcs);
    let (mut v, mut dv) = fasttwosum(v, dv);
    let sgn = 1.0f64.copysign(x);
    let jt = 32.0 - jf as f64 * sgn;
    // pi/2 -/+ jf*pi/64 = jt*pi/64, thus y = jt*pi/64 - delta with 0 <= jt <= 64
    const CT: [f64; 3] = [
        hf("0x1.6e8ba2ec8cb69p-6"),
        hf("0x1.1c4ea7a15c997p-6"),
        hf("0x1.ca8355d39bb67p-7"),
    ];
    let (v2, dv2) = muldd::<F>(v, dv, v, dv);
    v *= -sgn;
    dv *= -sgn;
    let fl = v2 * (CT[0] + v2 * (CT[1] + v2 * CT[2]));
    let (fh, fl) = polydd::<F>(v2, dv2, &C, fl);
    let (fh, fl) = muldd::<F>(v, dv, fh, fl);
    // now fh+fl approximates -delta
    let ph = jt * const { hf("0x1.921fb54442dp-5") };
    let pl = const { hf("0x1.8469898cc518p-53") } * jt;
    let ps = const { hf("-0x1.fc8f8cbb5bf6cp-102") } * jt;
    // since 0 <= jt <= 64, ph and pl are exact
    let (pl, ps) = sum(fh, fl, pl, ps);
    let (ph, pl) = fasttwosum(ph, pl);
    let (pl, ps) = fasttwosum(pl, ps);
    let (ph, pl) = fasttwosum(ph, pl);
    let (mut pl, ps) = fasttwosum(pl, ps);
    let mut t = pl.to_bits();
    let mut e = ((t >> 52) & 0x7ff) as i64 - 1023;
    e = 52 - (107 + e);
    e = e.clamp(0, 52);
    let m = (1u64 << 52) - (1u64 << e);
    let e = if e == 0 { 64 } else { e };
    if t.wrapping_add(1u64 << (e - 1)) & m == 0 {
        if x == const { hf("0x1.ffffffffffdc0p-1") } {
            return const { hf("0x1.8000000000024p-22") } + const { hf("0x1p-76") };
        }
        if x == const { hf("0x1.53ea6c7255e88p-4") } {
            return const { hf("0x1.7cdacb6bbe707p+0") } + const { hf("0x1p-54") };
        }
        if x == const { hf("0x1.fd737be914578p-11") } {
            return const { hf("0x1.91e006d41d8d8p+0") } + const { hf("0x1.8p-53") };
        }
        if x == const { hf("0x1.fffffffffff70p-1") } {
            return const { hf("0x1.8000000000009p-23") } + const { hf("0x1p-77") };
        }
        if x == const { hf("0x1.390e6939cd1a6p-5") } {
            return const { hf("0x1.8856a5d3296a4p+0") } - const { hf("0x1p-109") };
        }
        let w = ps.to_bits();
        if (w ^ t) >> 63 != 0 {
            t = t.wrapping_sub(1);
        } else {
            t = t.wrapping_add(1);
        }
        pl = f64::from_bits(t);
    }
    ph + pl
}

multiversion! {
    fn as_acos_refine(x: f64, phi: f64) -> f64 = as_acos_refine_with;
}

multiversion! {
    /// The arc cosine, correctly rounded (round to nearest, ties to even): CORE-MATH
    /// `cr_acos`. The same bits on every target. `Math.acos` (§21.3.2.2): NaN and |x| > 1 give
    /// NaN, 1 gives +0.
    pub fn acos(x: f64) -> f64 = acos_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acos_special_values() {
        assert!(acos(f64::NAN).is_nan());
        assert!(acos(1.0000000000000002).is_nan());
        assert!(acos(f64::INFINITY).is_nan());
        assert_eq!(acos(1.0).to_bits(), 0);
        assert_eq!(acos(-1.0), std::f64::consts::PI);
        assert_eq!(acos(0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(acos(-0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(acos(0.5), hf("0x1.0c152382d7366p+0"));
    }
}
