//! `Math.cosh`: the correctly rounded hyperbolic cosine (H36), a port of CORE-MATH `cr_cosh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/cosh/cosh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded hyperbolic cosine for binary64 values.
//! Copyright (c) 2023-2025 Alexei Sibidanov <sibid@uvic.ca>
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones. The tables `t0` and `t1`, the double-double helpers and `as_exp_accurate` have the
//! same C text as in sinh.c (the C `muldd` of cosh.c multiplies its operands in the other
//! order, which gives the same IEEE results), so this file uses the `crate::sinh` copies.
//! ep9 changes: rounding to nearest only (JS); no `errno`; every `__builtin_fma` is
//! [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA contraction.

use crate::exact::FmaOps;
use crate::hf;
use crate::sinh::{CH4, T0, T1, as_exp_accurate, fasttwosum, muldd, polydd, reduce};

#[inline(always)]
fn as_cosh_zero_with<F: FmaOps>(x: f64) -> f64 {
    const CH: [[f64; 2]; 4] = [
        [hf("0x1p-1"), hf("-0x1.c7e8db669f624p-111")],
        [hf("0x1.5555555555555p-5"), hf("0x1.5555555556135p-59")],
        [hf("0x1.6c16c16c16c17p-10"), hf("-0x1.f49f4a6e838f2p-65")],
        [hf("0x1.a01a01a01a01ap-16"), hf("0x1.a4ffbe15316aap-76")],
    ];
    const CL: [f64; 4] = [
        hf("0x1.27e4fb7789f5cp-22"), // degree 10
        hf("0x1.1eed8eff9089cp-29"), // degree 12
        hf("0x1.939749ce13dadp-37"), // degree 14
        hf("0x1.ae9891efb6691p-45"), // degree 16
    ];
    let x2 = x * x;
    let x2l = F::fma(x, x, -x2);
    let y2 = x2 * (CL[0] + x2 * (CL[1] + x2 * (CL[2] + x2 * CL[3])));
    let (y1, y2) = polydd::<F>(x2, x2l, &CH, y2);
    let (y1, y2) = muldd::<F>(y1, y2, x2, x2l);
    let (y0, y1) = fasttwosum(1.0, y1);
    let (mut y1, y2) = fasttwosum(y1, y2);
    let mut t = y1.to_bits();
    if t & (!0u64 >> 12) == 0 {
        let w = y2.to_bits();
        if (w ^ t) >> 63 != 0 {
            t = t.wrapping_sub(1);
        } else {
            t = t.wrapping_add(1);
        }
        y1 = f64::from_bits(t);
    }
    if t & (!0u64 >> 12) == !0u64 >> 12 {
        return as_cosh_database(x, y0 + y1);
    }
    y0 + y1
}

multiversion! {
    fn as_cosh_zero(x: f64) -> f64 = as_cosh_zero_with;
}

/// The exceptional cases: |x|, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.9a5e3cbe1985ep-4"),
        hf("0x1.01492f72f984bp+0"),
        hf("-0x1p-107"),
    ],
    [
        hf("0x1.52a11832e847dp-3"),
        hf("0x1.0381e68cac923p+0"),
        hf("0x1p-104"),
    ],
    [
        hf("0x1.bf0305e2c6c37p-3"),
        hf("0x1.061f4c39e16f2p+0"),
        hf("0x1p-107"),
    ],
    [
        hf("0x1.17326ffc09f68p-2"),
        hf("0x1.099318a43ac8p+0"),
        hf("0x1p-104"),
    ],
    [
        hf("0x1.3d27bf16d8bdbp-2"),
        hf("0x1.0c6091056e06ap+0"),
        hf("-0x1p-107"),
    ],
    [
        hf("0x1.03923f2b47c07p-1"),
        hf("0x1.219c1989e3373p+0"),
        hf("-0x1p-54"),
    ],
    [
        hf("0x1.a6031cd5f93bap-1"),
        hf("0x1.5bff041b260fep+0"),
        hf("-0x1p-107"),
    ],
    [
        hf("0x1.104b648f113a1p+0"),
        hf("0x1.9efdca62b700ap+0"),
        hf("-0x1p-109"),
    ],
    [
        hf("0x1.1585720f35cd9p+0"),
        hf("0x1.a5bf3acfde4b2p+0"),
        hf("0x1p-105"),
    ],
    [
        hf("0x1.e9cc7ed2e1a7ep+0"),
        hf("0x1.bb0ff220d8eb5p+1"),
        hf("-0x1p-53"),
    ],
    [
        hf("0x1.43180ea854696p+1"),
        hf("0x1.91f1122b6b63ap+2"),
        hf("0x1p-102"),
    ],
    [
        hf("0x1.725811dcf6782p+2"),
        hf("0x1.45ea160ddc71fp+7"),
        hf("-0x1p-100"),
    ],
    [
        hf("0x1.5afd56f7d565bp+3"),
        hf("0x1.8ff8e0ccea7cp+14"),
        hf("0x1p-90"),
    ],
    [
        hf("0x1.759a2ad4c4d56p+3"),
        hf("0x1.cb62eec26bd78p+15"),
        hf("-0x1p-92"),
    ],
    [
        hf("0x1.7fce95ea5c653p+3"),
        hf("0x1.3bf8009648dcp+16"),
        hf("0x1p-88"),
    ],
    [
        hf("0x1.743d5609348acp+4"),
        hf("0x1.7a87a8bb7fa28p+32"),
        hf("-0x1p-22"),
    ],
    [
        hf("0x1.e07e71bfcf06fp+5"),
        hf("0x1.91ec4412c344fp+85"),
        hf("0x1p-24"),
    ],
    [
        hf("0x1.6474c604cc0d7p+6"),
        hf("0x1.7a8f65ad009bdp+127"),
        hf("-0x1p+20"),
    ],
    [
        hf("0x1.54cd1fea7663ap+7"),
        hf("0x1.c90810d354618p+244"),
        hf("0x1p+135"),
    ],
    [
        hf("0x1.2da9e5e6af0bp+8"),
        hf("0x1.27d6fe867d6f6p+434"),
        hf("0x1p+329"),
    ],
    [
        hf("0x1.d6479eba7c971p+8"),
        hf("0x1.62a88613629b6p+677"),
        hf("-0x1p+568"),
    ],
];

#[inline(never)]
fn as_cosh_database(x: f64, mut f: f64) -> f64 {
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    let ax = x.abs();
    while a <= b {
        let row = &DB[m as usize];
        if row[0] < ax {
            a = m + 1;
        } else if row[0] == ax {
            f = row[1] + row[2];
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) / 2;
    }
    f
}

#[inline(always)]
fn cosh_with<F: FmaOps>(x: f64) -> f64 {
    // cosh(x) ~ 1+x^2*P(x^2) for |x| < 0.125. For other arguments,
    // cosh(x) = (exp(|x|)+exp(-|x|))/2: for |x| < 5 both exponentials are computed with
    // slightly more than double precision; for 5 < |x| < 36.736801 exp(-|x|) is small and
    // computed in double precision; for 36.736801 < |x| < 710.47586 only exp(|x|) is computed,
    // with an accuracy of 106 bits.
    let ax = x.abs();
    let (jt, t) = reduce::<F>(ax);
    let aix = ax.to_bits();
    if aix < 0x3fc0000000000000 {
        // |x| < 0.125
        if aix < 0x3e50000000000000 {
            // |x| < 0x1p-26
            return F::fma(ax, const { hf("0x1p-55") }, 1.0);
        }
        // q(x) = 1 + c0*x^2 + c1*x^4 + c2*x^6 + c3*x^8 + c4*x^10 approximates cosh(x) on
        // [2^-26, 0.125] with |q(x) - cosh(x)| < 2^-67.518 * x^2.
        const C: [f64; 5] = [
            hf("0x1p-1"),
            hf("0x1.5555555555554p-5"),
            hf("0x1.6c16c16c1d0cp-10"),
            hf("0x1.a01a0075066b4p-16"),
            hf("0x1.27faff8dcc1c8p-22"),
        ];
        let x2 = x * x;
        let x4 = x2 * x2;
        let p = x2 * ((C[0] + x2 * C[1]) + x4 * ((C[2] + x2 * C[3]) + x4 * C[4]));
        let e = x2 * const { hf("0x1.84p-51") };
        let lb = 1.0 + (p - e);
        let ub = 1.0 + (p + e);
        if lb == ub {
            return lb;
        }
        return as_cosh_zero(x);
    }
    if aix > 0x408633ce8fb9f87d {
        // |x| > 0x1.633ce8fb9f87dp+9
        if aix > 0x7ff0000000000000 {
            return x + x; // nan
        }
        if aix == 0x7ff0000000000000 {
            return x.abs(); // inf
        }
        return const { hf("0x1p1023") } * 2.0;
    }
    // now 0.125 <= |x| <= 0x1.633ce8fb9f87dp+9 (tested exhaustively with/without FMA by
    // Vincenzo Innocente and Paul Zimmermann)
    let il = ((jt << 14) >> 40) as i64;
    let jl = -il;
    let (i1, i0, ie) = (il & 0x3f, (il >> 6) & 0x3f, il >> 12);
    let (j1, j0, je) = (jl & 0x3f, (jl >> 6) & 0x3f, jl >> 12);
    let mut sp = ((1022 + ie) as u64) << 52;
    let sm = ((1022 + je) as u64) << 52;
    let (t0h, t0l) = (T0[i0 as usize][1], T0[i0 as usize][0]);
    let (t1h, t1l) = (T1[i1 as usize][1], T1[i1 as usize][0]);
    let mut th = t0h * t1h;
    let mut tl = (t0h * t1l + t1h * t0l) + F::fma(t0h, t1h, -th);
    const L2H: f64 = hf("0x1.62e42ffp-13");
    const L2L: f64 = hf("0x1.718432a1b0e26p-47");
    let dx = (ax - L2H * t) + L2L * t;
    let dx2 = dx * dx;
    let mx = -dx;
    let ch = &CH4;
    let pp = dx * ((ch[0] + dx * ch[1]) + dx2 * (ch[2] + dx * ch[3]));
    let (mut rh, mut rl);
    if aix > 0x4014000000000000 {
        // |x| > 5
        if aix > 0x40425e4f7b2737fa {
            // |x| >~ 36.736801
            sp = ((1021 + ie) as u64) << 52;
            rh = th;
            rl = tl + th * pp;
            let e = 0.12e-18 * th;
            let lb = rh + (rl - e);
            let ub = rh + (rl + e);
            if lb == ub {
                return (lb * f64::from_bits(sp)) * 2.0;
            }
            (th, tl) = as_exp_accurate(ax, t, th, tl);
            (th, tl) = fasttwosum(th, tl);
            let ml = (tl.to_bits().wrapping_add(8) & (!0u64 >> 12)) as i64;
            th += tl;
            th *= 2.0;
            th *= f64::from_bits(sp);
            // when ml <= 16 is false, we never have eh-el > 103
            if ml <= 16 {
                return as_cosh_database(x, th);
            }
            return th;
        }
        let q0h = T0[j0 as usize][1];
        let q1h = T1[j1 as usize][1];
        let mut qh = q0h * q1h;
        th *= f64::from_bits(sp);
        tl *= f64::from_bits(sp);
        qh *= f64::from_bits(sm);
        let pm = mx * ((ch[0] + mx * ch[1]) + dx2 * (ch[2] + mx * ch[3]));
        let em = qh + qh * pm;
        rh = th;
        rl = (tl + em) + th * pp;
        let e = const { hf("0x1.202p-63") } * rh;
        let lb = rh + (rl - e);
        let ub = rh + (rl + e);
        if lb == ub {
            return lb;
        }
        (th, tl) = as_exp_accurate(ax, t, th, tl);
        if aix > 0x403f666666666666 {
            // |x| > 0x1.f666666666666p+4
            rh = th + qh;
            rl = ((th - rh) + qh) + tl;
        } else {
            qh = q0h * q1h;
            let q0l = T0[j0 as usize][0];
            let q1l = T1[j1 as usize][0];
            let mut ql = (q0h * q1l + q1h * q0l) + F::fma(q0h, q1h, -qh);
            qh *= f64::from_bits(sm);
            ql *= f64::from_bits(sm);
            (qh, ql) = as_exp_accurate(-ax, -t, qh, ql);
            rh = th + qh;
            rl = (((th - rh) + qh) + ql) + tl;
        }
    } else {
        // |x| <= 5
        let (q0h, q0l) = (T0[j0 as usize][1], T0[j0 as usize][0]);
        let (q1h, q1l) = (T1[j1 as usize][1], T1[j1 as usize][0]);
        let mut qh = q0h * q1h;
        let mut ql = (q0h * q1l + q1h * q0l) + F::fma(q0h, q1h, -qh);
        th *= f64::from_bits(sp);
        tl *= f64::from_bits(sp);
        qh *= f64::from_bits(sm);
        ql *= f64::from_bits(sm);
        let pm = mx * ((ch[0] + mx * ch[1]) + dx2 * (ch[2] + mx * ch[3]));
        let (fph, fpl) = (th, tl + th * pp);
        let (fmh, fml) = (qh, ql + qh * pm);
        rh = fph + fmh;
        rl = (((fph - rh) + fmh) + fml) + fpl;
        let e = const { hf("0x1.c0ap-62") } * rh;
        let lb = rh + (rl - e);
        let ub = rh + (rl + e);
        if lb == ub {
            return lb;
        }
        (th, tl) = as_exp_accurate(ax, t, th, tl);
        (qh, ql) = as_exp_accurate(-ax, -t, qh, ql);
        rh = th + qh;
        rl = (((th - rh) + qh) + ql) + tl;
    }
    (rh, rl) = fasttwosum(rh, rl);
    let ml = (rl.to_bits().wrapping_add(8) & (!0u64 >> 12)) as i64;
    rh += rl;
    // when ml <= 16 is false, we never have eh-el > 103
    if ml <= 16 {
        return as_cosh_database(x, rh);
    }
    rh
}

multiversion! {
    /// The hyperbolic cosine, correctly rounded (round to nearest, ties to even): CORE-MATH
    /// `cr_cosh`. The same bits on every target. `Math.cosh` (§21.3.2.13): NaN gives NaN, ±0
    /// gives 1, ±∞ gives +∞.
    pub fn cosh(x: f64) -> f64 = cosh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosh_special_values() {
        assert!(cosh(f64::NAN).is_nan());
        assert_eq!(cosh(0.0), 1.0);
        assert_eq!(cosh(-0.0), 1.0);
        assert_eq!(cosh(f64::INFINITY), f64::INFINITY);
        assert_eq!(cosh(f64::NEG_INFINITY), f64::INFINITY);
        assert_eq!(cosh(-711.0), f64::INFINITY);
    }
}
