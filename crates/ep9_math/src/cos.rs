//! `Math.cos`: the correctly rounded cosine (H36), a port of CORE-MATH `cr_cos`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/cos/cos.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded cosine function for binary64 values.
//! Copyright (c) 2022-2025 Paul Zimmermann and Tom Hubrecht
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The parts that `cos.c` shares with
//! `tan.c` are in `trig.rs`. ep9 changes: rounding to nearest only (JS); no `errno` and no
//! floating-point flags; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software
//! FMA, the same bits); no FMA contraction. |x| <= 0x1.6a09e667f3bccp-27 is
//! `fma(|x|, -2^-28, 1)`; otherwise the double-double path `cos_fast` (reduction modulo 2pi to
//! i/2^11 + r, then the SC table) runs, and its rounding test sends the rest to the 128-bit path
//! `cos_accurate`, a separate entry point.

use crate::exact::FmaOps;
use crate::hf;
use crate::trig::{
    C, MAGIC, S, SC, T, a_mul, add_dint, dint_fromd, dint_tod, eval_pc, eval_pc_fast, eval_ps,
    eval_ps_fast, fast_two_sum, mul_dint, reduce, reduce2, s_mul, set_dd,
};

/// For 0x1.6a09e667f3bccp-27 < x < +Inf, return `(i, h, l, err1)` such that i/2^11 + h + l
/// approximates frac(x/(2pi)), with absolute error < err1.
#[inline(always)]
fn reduce_fast<F: FmaOps>(x: f64) -> (usize, f64, f64, f64) {
    let (mut h, l, err1);
    if x <= const { hf("0x1.921fb54442d17p+2") } {
        // x < 2*pi; | CH+CL - 1/(2pi) | < 2^-110.523
        const CH: f64 = hf("0x1.45f306dc9c883p-3");
        const CL: f64 = hf("-0x1.6b01ec5417056p-57");
        let (hh, ll) = a_mul::<F>(CH, x); // exact
        h = hh;
        l = F::fma(CL, x, ll);
        err1 = const { hf("0x1.d9p-105") } * h; // error < 2^-104.116 * h
    } else {
        // x > 0x1.921fb54442d17p+2
        let tu = x.to_bits();
        let mut e = ((tu >> 52) & 0x7ff) as i64; // 1025 <= e <= 2046
        let m = (1u64 << 52) | (tu & 0xfffffffffffff);
        let mut c = [0u64; 3];
        let mm = u128::from(m);
        // x = m/2^53 * 2^(e-1022)
        if e <= 1074 {
            // 1025 <= e <= 1074: 2^2 <= x < 2^52
            let mut u = mm * u128::from(T[1]);
            c[0] = u as u64;
            c[1] = (u >> 64) as u64;
            u = mm * u128::from(T[0]);
            c[1] = c[1].wrapping_add(u as u64);
            c[2] = ((u >> 64) as u64) + u64::from(c[1] < u as u64);
            e = 1075 - e; // 1 <= e <= 50
        } else {
            // 1075 <= e <= 2046, 2^52 <= x < 2^1024
            let i = ((e - 1138 + 63) / 64) as usize; // ceil((e-1138)/64), 0 <= i <= 15
            let mut u = mm * u128::from(T[i + 2]);
            c[0] = u as u64;
            c[1] = (u >> 64) as u64;
            u = mm * u128::from(T[i + 1]);
            c[1] = c[1].wrapping_add(u as u64);
            c[2] = ((u >> 64) as u64) + u64::from(c[1] < u as u64);
            u = mm * u128::from(T[i]);
            c[2] = c[2].wrapping_add(u as u64);
            e = 1139 + ((i as i64) << 6) - e; // 1 <= e <= 64
        }
        // e is the number of low bits of c[2] contributing to frac(x/(2pi))
        if e == 64 {
            c[0] = c[1];
            c[1] = c[2];
        } else {
            c[0] = (c[1] << (64 - e)) | c[0] >> e;
            c[1] = (c[2] << (64 - e)) | c[1] >> e;
        }
        (h, l) = set_dd(c[1], c[0]);
        err1 = const { hf("0x1.01p-76") };
    }
    let i = (h * const { hf("0x1p11") }).floor();
    h = F::fma(i, const { hf("-0x1p-11") }, h);
    (i as usize, h, l, err1)
}

/// For x > 0x1.6a09e667f3bccp-27: `(h, l, err)` with | h + l - cos(x) | < err.
#[inline(always)]
fn cos_fast<F: FmaOps>(x: f64) -> (f64, f64, f64) {
    let (mut i, mut h, mut l, err1) = reduce_fast::<F>(x);
    // if i >= 2^10: pi <= x <= 2pi, cos(pi+x) = -cos(x)
    let mut neg = i >> 10;
    i &= 0x3ff;
    // if i >= 2^9: pi/2 <= x <= pi, cos(pi/2+x) = -sin(x)
    let mut is_cos = 1 ^ (i >> 9);
    neg ^= i >> 9;
    i &= 0x1ff;
    // if i >= 2^8: cos(pi/2-x) = sin(x)
    if i & 0x100 != 0 {
        // pi/4 <= x_red <= pi/2; 0x1p-11 - h is exact
        is_cos ^= 1;
        i = 0x1ff - i;
        h = const { hf("0x1p-11") } - h;
        l = -l;
    }
    // SC[] evaluates at i/2^11 + SC[i][0]; h - SC[i][0] is exact
    h -= SC[i][0];
    let (uh, ul) = a_mul::<F>(h, h);
    let ul = F::fma(h + h, l, ul);
    // uh+ul approximates (h+l)^2
    let (mut sh, mut sl) = eval_ps_fast::<F>(h, l, uh, ul);
    let (mut ch, mut cl) = eval_pc_fast::<F>(uh, ul);
    let err = if is_cos == 0 {
        (sh, sl) = s_mul::<F>(SC[i][2], sh, sl);
        (ch, cl) = s_mul::<F>(SC[i][1], ch, cl);
        (h, l) = fast_two_sum(ch, sh);
        l += sl + cl;
        // | h + l - cos(x) | < 2^-68.588 + err1
        const { hf("0x1.55p-69") }
    } else {
        (ch, cl) = s_mul::<F>(SC[i][2], ch, cl);
        (sh, sl) = s_mul::<F>(SC[i][1], sh, sl);
        (h, l) = fast_two_sum(ch, -sh);
        l += cl - sl;
        // | h + l - cos(x) | < 2^-68.414 * |h + l| + err1
        const { hf("0x1.81p-69") }
    };
    let sg = if neg != 0 { -1.0 } else { 1.0 };
    (h * sg, l * sg, err + err1)
}

/// For x > 0x1.6a09e667f3bccp-27.
#[inline(never)]
fn cos_accurate(x: f64) -> f64 {
    let mut xd = dint_fromd(x);
    reduce(&mut xd);
    // now |X - x/(2pi) mod 1| < 2^-126.67*X, with 0 <= X < 1.
    let mut neg = false;
    let mut is_cos = true;
    let mut i = reduce2(&mut xd); // exact; X = i/2^11 + r
    if i & 0x400 != 0 {
        // pi <= x < 2*pi: cos(x) = -cos(x-pi)
        neg = true;
        i &= 0x3ff;
    }
    if i & 0x200 != 0 {
        // pi/2 <= x < pi: cos(x) = -sin(x-pi/2)
        neg = !neg;
        is_cos = false;
        i &= 0x1ff;
    }
    if i & 0x100 != 0 {
        // pi/4 <= x < pi/2: cos(x) = sin(pi/2-x), sin(x) = cos(pi/2-x)
        is_cos = !is_cos;
        xd.sgn = 1; // negate X
        xd = add_dint(&MAGIC, &xd); // X -> 2^-11 - X
        i = 0x1ff - i;
    }
    // now 0 <= i < 256 and 0 <= X < 2^-11
    let x2 = mul_dint(&xd, &xd);
    let mut u = eval_pc(&x2); // cos2pi(X)
    let mut v = eval_ps(&xd, &x2); // sin2pi(X)
    if !is_cos {
        // sin2pi(R) ~ sin2pi(i/2^11)*cos2pi(X)+cos2pi(i/2^11)*sin2pi(X)
        u = mul_dint(&S[i], &u);
        v = mul_dint(&C[i], &v);
    } else {
        // cos2pi(R) ~ cos2pi(i/2^11)*cos2pi(X)-sin2pi(i/2^11)*sin2pi(X)
        u = mul_dint(&C[i], &u);
        v = mul_dint(&S[i], &v);
        v.sgn = 1 - v.sgn; // negate V
    }
    u = add_dint(&u, &v);
    // | cos(x) - U | < |U| * 2^-122.650: at most 41 ulps of U.lo
    let err = 41u64;
    let lo0 = u.lo.wrapping_sub(err);
    let hi0 = u.hi.wrapping_sub(u64::from(lo0 > u.lo));
    let lo1 = u.lo.wrapping_add(err);
    let hi1 = u.hi.wrapping_add(u64::from(lo1 < u.lo));
    // check that the upper 54 bits are equal
    if (hi0 >> 10) != (hi1 >> 10) {
        const EXCEPTIONS: [[f64; 3]; 5] = [
            [
                hf("0x1.8000000000009p-23"),
                hf("0x1.fffffffffff7p-1"),
                hf("0x1.b56666666666cp-143"),
            ],
            [
                hf("0x1.8000000000024p-22"),
                hf("0x1.ffffffffffdcp-1"),
                hf("0x1.b56666666667ep-137"),
            ],
            [
                hf("0x1.800000000009p-21"),
                hf("0x1.ffffffffff7p-1"),
                hf("0x1.b5666666666c4p-131"),
            ],
            [
                hf("0x1.20000000000f3p-20"),
                hf("0x1.fffffffffebcp-1"),
                hf("0x1.37642666666fdp-127"),
            ],
            [
                hf("0x1.800000000024p-20"),
                hf("0x1.fffffffffdcp-1"),
                hf("0x1.b5666666667ddp-125"),
            ],
        ];
        for ex in &EXCEPTIONS {
            if x.abs() == ex[0] {
                return ex[1] + ex[2];
            }
        }
        // a hard-to-round case; all of them are known and pass (C comment)
    }
    if neg {
        u.sgn = 1 - u.sgn;
    }
    dint_tod(&u)
}

#[inline(always)]
fn cos_with<F: FmaOps>(x: f64) -> f64 {
    let tu = x.to_bits();
    let e = (tu >> 52) & 0x7ff;
    if e == 0x7ff {
        // NaN, +Inf and -Inf
        if tu << 1 == 0x7ff << 53 {
            return f64::NAN; // the C code returns 0.0 / 0.0
        }
        return x + x;
    }
    // For |x| <= 0x1.6a09e667f3bccp-27, cos(x) rounds to 1 - tiny (to nearest).
    let ax = f64::from_bits(tu & 0x7fffffffffffffff);
    if ax.to_bits() <= 0x3e46a09e667f3bcc {
        return F::fma(ax, const { hf("-0x1p-28") }, 1.0);
    }
    let (h, l, err) = cos_fast::<F>(ax);
    let left = h + (l - err);
    let right = h + (l + err);
    if left == right {
        return left;
    }
    cos_accurate(ax)
}

multiversion! {
    /// The cosine, correctly rounded (round to nearest, ties to even): CORE-MATH `cr_cos`. The
    /// same bits on every target. `Math.cos` (§21.3.2.12): NaN and ±∞ give NaN, ±0 gives 1.
    pub fn cos(x: f64) -> f64 = cos_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_special_values() {
        assert!(cos(f64::NAN).is_nan());
        assert!(cos(f64::INFINITY).is_nan());
        assert!(cos(f64::NEG_INFINITY).is_nan());
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(cos(-0.0), 1.0);
        assert_eq!(cos(std::f64::consts::PI), -1.0);
        assert_eq!(
            cos(std::f64::consts::FRAC_PI_2),
            hf("0x1.1a62633145c07p-54")
        );
    }

    /// The 128-bit path agrees with the fast path where both give an answer.
    #[test]
    fn cos_accurate_agrees() {
        for &x in &[
            0.5,
            1.0,
            3.0,
            100.0,
            1e10,
            1e22,
            1e300,
            hf("0x1.fffffffffffffp+1023"),
        ] {
            assert_eq!(cos_accurate(x).to_bits(), cos(x).to_bits(), "{x}");
        }
    }
}
