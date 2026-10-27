//! `Math.tanh`: the correctly rounded hyperbolic tangent (H36), a port of CORE-MATH `cr_tanh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/tanh/tanh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded hyperbolic tangent for binary64 values.
//! Copyright (c) 2023-2026 Alexei Sibidanov, Cyprien Peignier, Paul Zimmermann
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones. The tables `t0` and `t1` and the `as_exp_accurate` coefficient table have the same C
//! text as in sinh.c, so this file uses the `crate::sinh` tables; its double-double helpers
//! (`muldd_acc`, `polydd`) differ from sinh.c and stay here. ep9 changes: rounding to nearest
//! only (JS); no `errno`; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software
//! FMA, the same bits); no FMA contraction; the x86-64 SSE mask of `v0` is the integer mask of
//! the C non-x86 branch (the same bits).

use crate::exact::FmaOps;
use crate::hf;
use crate::sinh::{T0, T1};

#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

#[inline(always)]
fn fasttwosub(x: f64, y: f64) -> (f64, f64) {
    let s = x - y;
    let z = x - s;
    (s, z - y)
}

#[inline(always)]
fn muldd_acc<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let plh = xl * ch;
    let phl = xh * cl;
    let phh = xh * ch;
    let mut phh_rest = F::fma(xh, ch, -phh);
    phh_rest += phl + plh;
    fasttwosum(phh, phh_rest)
}

#[inline(always)]
fn mulddd_acc<F: FmaOps>(xh: f64, xl: f64, c: f64) -> (f64, f64) {
    let pl = xl * c;
    let ph = xh * c;
    let mut ph_rest = F::fma(xh, c, -ph);
    ph_rest += pl;
    fasttwosum(ph, ph_rest)
}

/// At input, `l` is the approximation of the upper part of the polynomial (evaluated with
/// double arithmetic only).
#[inline(always)]
fn polydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, t) = fasttwosum(c[i][0], l);
    let mut cl = t + c[i][1];
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd_acc::<F>(xh, xl, ch, cl);
        let (th, tl) = fasttwosum(c[i][0], ch);
        ch = th;
        cl = (cl + c[i][1]) + tl;
    }
    (ch, cl)
}

#[inline(always)]
fn as_exp_accurate_with<F: FmaOps>(x: f64, t: f64, th: f64, tl: f64) -> (f64, f64) {
    const CH: [[f64; 2]; 3] = [
        [hf("0x1p+0"), hf("0x1.6c16bd194535dp-94")],
        [hf("0x1p-1"), hf("-0x1.8259d904fd34fp-93")],
        [hf("0x1.5555555555555p-3"), hf("0x1.53e93e9f26e62p-57")],
    ];
    const L2H: f64 = hf("0x1.62e42ffp-13");
    const L2L: f64 = hf("0x1.718432a1b0e26p-47");
    const L2LL: f64 = hf("0x1.9ff0342542fc3p-102");
    let dx = x - L2H * t;
    let dxl = L2L * t;
    let dxll = L2LL * t + F::fma(L2L, t, -dxl);
    let dxh = dx + dxl;
    let dxl = ((dx - dxh) + dxl) + dxll;
    let fl = dxh
        * (const { hf("0x1.5555555555555p-5") }
            + dxh
                * (const { hf("0x1.11111113e93e9p-7") }
                    + dxh * const { hf("0x1.6c16c169400a7p-10") }));
    let (fh, fl) = polydd::<F>(dxh, dxl, &CH, fl);
    let (fh, fl) = muldd_acc::<F>(dxh, dxl, fh, fl);
    let (fh, fl) = muldd_acc::<F>(th, tl, fh, fl);
    let zh = th + fh;
    let zl = (th - zh) + fh;
    let uh = zh + tl;
    let ul = ((zh - uh) + tl) + zl;
    let vh = uh + fl;
    let vl = ((uh - vh) + fl) + ul;
    (vh, vl)
}

/// The accurate path for |x| >= 0.25: tanh(|x|) = 1 - 2*exp(-2|x|)/(1 + exp(-2|x|)) with
/// exp(-2|x|) = (th + tl) * exp(-2|x| - t*log(2)/2^13).
#[inline(always)]
fn tanh_accurate_with<F: FmaOps>(x: f64, ax: f64, t: f64, th: f64, tl: f64) -> f64 {
    let (rh, rl) = as_exp_accurate_with::<F>(-2.0 * ax, t, th, tl);
    let (qh, qd) = fasttwosum(1.0, rh);
    let ql = rl + qd;
    let (qh, ql) = fasttwosum(qh, ql);
    let rqh = 1.0 / qh;
    let rql = (ql * rqh + F::fma(rqh, qh, -1.0)) * -rqh;
    let (ph, pl) = muldd_acc::<F>(rh, rl, rqh, rql);
    let (rh, rl) = fasttwosub(0.5, ph);
    let rl = rl - pl;
    let (rh, rl) = fasttwosum(rh, rl);
    let res = 2.0f64.copysign(x) * rh + 2.0f64.copysign(x) * rl;
    let lu = rl.to_bits();
    if (lu.wrapping_add(32) & (!0u64 >> 12)) < 65 {
        return as_tanh_database(x, res);
    }
    res
}

multiversion! {
    fn tanh_accurate(x: f64, ax: f64, t: f64, th: f64, tl: f64) -> f64 = tanh_accurate_with;
}

/// |x| < 0.25.
#[inline(always)]
fn as_tanh_zero_with<F: FmaOps>(x: f64) -> f64 {
    const CH: [[f64; 2]; 9] = [
        [hf("-0x1.5555555555555p-2"), hf("-0x1.5555555554cc4p-56")],
        [hf("0x1.1111111111111p-3"), hf("0x1.111110f8c0178p-59")],
        [hf("-0x1.ba1ba1ba1ba1cp-5"), hf("0x1.7917c1d676ff5p-59")],
        [hf("0x1.664f4882c10fap-6"), hf("-0x1.9d5cb27c0af28p-63")],
        [hf("-0x1.226e355e6c23cp-7"), hf("-0x1.c9674586913f3p-61")],
        [hf("0x1.d6d3d0e157db3p-9"), hf("-0x1.71376fa06ce94p-65")],
        [hf("-0x1.7da36452b5e46p-10"), hf("-0x1.aba8d51bd9cp-65")],
        [hf("0x1.3558247faa32dp-11"), hf("-0x1.e0cfb423aedfdp-65")],
        [hf("-0x1.f57d76ea30928p-13"), hf("-0x1.c30601213cae9p-67")],
    ];
    const CL: [f64; 5] = [
        hf("0x1.967e0a63ca836p-14"),
        hf("-0x1.497b99d2a77d1p-15"),
        hf("0x1.0ae346258cbdep-16"),
        hf("-0x1.aade68fb2f076p-18"),
        hf("0x1.22e609bf8671fp-19"),
    ];
    let x2 = x * x;
    let x2l = F::fma(x, x, -x2);
    let y2 = x2 * (CL[0] + x2 * (CL[1] + x2 * (CL[2] + x2 * (CL[3] + x2 * CL[4]))));
    let (y1, y2) = polydd::<F>(x2, x2l, &CH, y2);
    let (y1, y2) = mulddd_acc::<F>(y1, y2, x);
    let (y1, y2) = muldd_acc::<F>(y1, y2, x2, x2l);
    let (y0, y1) = fasttwosum(x, y1);
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
        if x.abs() == const { hf("0x1.ac343b179fec4p-3") } {
            return as_tanh_database(x, y0 + y1);
        }
    }
    y0 + y1
}

multiversion! {
    fn as_tanh_zero(x: f64) -> f64 = as_tanh_zero_with;
}

/// The exceptional cases: |x|, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.ac343b179fec4p-3"),
        hf("0x1.a612499c53078p-3"),
        hf("0x1p-57"),
    ],
    [
        hf("0x1.00764a988bf73p-2"),
        hf("0x1.f676484c0703bp-3"),
        hf("-0x1p-104"),
    ],
    [
        hf("0x1.17d1e8a63711fp-2"),
        hf("0x1.110e96a6c2d96p-2"),
        hf("-0x1p-105"),
    ],
    [
        hf("0x1.291c601a05276p-2"),
        hf("0x1.210b7d0c03743p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.36f33d51c264dp-2"),
        hf("0x1.2dbb7b1c91363p-2"),
        hf("-0x1p-106"),
    ],
    [
        hf("0x1.43eaea23649c3p-2"),
        hf("0x1.39877ed028641p-2"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.d88d7550b2826p-2"),
        hf("0x1.b9a3637366afdp-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.e611aa58ab608p-2"),
        hf("0x1.c493dc899e4a6p-2"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.01efe7ac8c15dp-1"),
        hf("0x1.dc3fe1b524821p-2"),
        hf("-0x1p-104"),
    ],
    [
        hf("0x1.1005ec0bccabbp-1"),
        hf("0x1.f20b1c8557dedp-2"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.33dfeb0fa4bfep-1"),
        hf("0x1.1372f9ee76e99p-1"),
        hf("0x1p-55"),
    ],
    [
        hf("0x1.49f24ac5cac35p-1"),
        hf("0x1.22c495ff06104p-1"),
        hf("-0x1p-104"),
    ],
];

#[inline(never)]
fn as_tanh_database(x: f64, mut f: f64) -> f64 {
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    let ax = x.abs();
    while a <= b {
        let row = &DB[m as usize];
        if row[0] < ax {
            a = m + 1;
        } else if row[0] == ax {
            f = 1.0f64.copysign(x) * row[1] + 1.0f64.copysign(x) * row[2];
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) / 2;
    }
    f
}

#[inline(always)]
fn tanh_with<F: FmaOps>(x: f64) -> f64 {
    // tanh(x) is a minimax polynomial for |x| < 0.25. Elsewhere,
    // tanh(|x|) = 1 - 2*exp(-2*|x|)/(1 + exp(-2*|x|)); for |x| > 3.683 the second term is
    // small and a less precise exponential is enough.
    let ax = x.abs();
    let aix = ax.to_bits();
    // For |x| >= 0x1.30fc1931f09cap+4, tanh(x) rounds to +1 or -1 to nearest; this also avoids
    // a spurious overflow in the computation of v0 below.
    if aix >= 0x40330fc1931f09ca {
        if aix > 0x7ff0000000000000 {
            return x + x; // nan
        }
        let f = 1.0f64.copysign(x);
        if aix == 0x7ff0000000000000 {
            return f;
        }
        let df = const { hf("0x1p-55") }.copysign(x);
        return f - df;
    }
    const S: f64 = hf("-0x1.71547652b82fep+13");
    let v0 = F::fma(ax, S, const { hf("0x1.8000004p+25") });
    let jt = v0.to_bits();
    let v = jt & !((1u64 << 27) - 1);
    let t = f64::from_bits(v) - const { hf("0x1.8p25") };
    let i1 = ((jt >> 27) & 0x3f) as usize;
    let i0 = ((jt >> 33) & 0x3f) as usize;
    let ie = ((jt << 13) as i64) >> 52;
    let sp = f64::from_bits(((1023 + ie) as u64) << 52);
    const CH: [f64; 4] = [
        hf("0x1p+1"),
        hf("0x1p+1"),
        hf("0x1.55555557e54ffp+0"),
        hf("0x1.55555553a12f4p-1"),
    ];
    let t0h = T0[i0][1];
    let t1h = T1[i1][1];
    let mut th = t0h * t1h;
    let mut tl;
    if aix < 0x400d76c8b4395810 {
        // |x| ~< 3.683
        if aix < 0x3fd0000000000000 {
            // |x| < 0x1p-2
            if aix <= 0x3e4d12ed0af1a27f {
                // |x| <= 0x1.d12ed0af1a27fp-27
                if aix == 0 {
                    return x;
                }
                return F::fma(x, const { hf("-0x1p-55") }, x);
            }
            const C: [f64; 8] = [
                hf("-0x1.5555555555555p-2"),
                hf("0x1.1111111110f33p-3"),
                hf("-0x1.ba1ba1b9b8ea6p-5"),
                hf("0x1.664f4838e0a43p-6"),
                hf("-0x1.226e17d1bc09bp-7"),
                hf("0x1.d6c64dfba2565p-9"),
                hf("-0x1.7bdd094d327afp-10"),
                hf("0x1.1535ad0c31d0ep-11"),
            ];
            let x2 = x * x;
            let x3 = x2 * x;
            let x4 = x2 * x2;
            let x8 = x4 * x4;
            let p1 = (C[4] + x2 * C[5]) + x4 * (C[6] + x2 * C[7]);
            let mut p0 = (C[0] + x2 * C[1]) + x4 * (C[2] + x2 * C[3]);
            p0 += x8 * p1;
            p0 *= x3;
            let (rh, rl) = fasttwosum(x, p0);
            let e = x3 * const { hf("0x1.c0p-52") };
            let lb = rh + (rl - e);
            let ub = rh + (rl + e);
            if lb == ub {
                return lb;
            }
            return as_tanh_zero(x);
        }
        let t0l = T0[i0][0];
        let t1l = T1[i1][0];
        tl = (t0h * t1l + t1h * t0l) + F::fma(t0h, t1h, -th);
        th *= sp;
        tl *= sp;
        const L2H: f64 = hf("-0x1.62e42ffp-14");
        const L2L: f64 = hf("-0x1.718432a1b0e26p-48");
        let dx = (L2H * t - ax) - L2L * t;
        let dx2 = dx * dx;
        let p = dx * ((CH[0] + dx * CH[1]) + dx2 * (CH[2] + dx * CH[3]));
        let rh = th;
        let rl = tl + rh * p;
        let (rh, rl) = fasttwosum(rh, rl);
        let (ph, pl) = (rh, rl);
        let (qh, qd) = fasttwosum(1.0, rh);
        let ql = rl + qd;
        let rqh = 1.0 / qh;
        let rql = (ql * rqh + F::fma(rqh, qh, -1.0)) * -rqh;
        let (ph, pl) = muldd_acc::<F>(ph, pl, rqh, rql);
        // Tested exhaustively with/without FMA contraction (C comment).
        let e = rh * const { hf("0x1.0dp-62") };
        let (mut rh, mut rl) = fasttwosub(0.5, ph);
        rl -= pl;
        rh *= 2.0f64.copysign(x);
        rl *= 2.0f64.copysign(x);
        let lb = rh + (rl - e);
        let ub = rh + (rl + e);
        if lb == ub {
            return lb;
        }
    } else {
        // 3.683 ~< x < 0x1.30fc1931f09cap+4
        const L2: f64 = hf("-0x1.62e42fefa39efp-14");
        let dx = F::fma(L2, t, -ax);
        let dx2 = dx * dx;
        let p = dx * ((CH[0] + dx * CH[1]) + dx2 * (CH[2] + dx * CH[3]));
        let mut rh = th * sp;
        rh += (p + ((2.0 * const { hf("0x1.3p-55") }) * ax)) * rh;
        // Tested exhaustively with/without FMA contraction (C comment).
        let e = rh * const { hf("0x1.1p-49") };
        rh = (2.0 * rh) / (1.0 + rh);
        let one = 1.0f64.copysign(x);
        rh = rh.copysign(x);
        let lb = one - (rh + e);
        let ub = one - (rh - e);
        if lb == ub {
            return lb;
        }
        let t0l = T0[i0][0];
        let t1l = T1[i1][0];
        tl = (t0h * t1l + t1h * t0l) + F::fma(t0h, t1h, -th);
        th *= sp;
        tl *= sp;
    }
    tanh_accurate(x, ax, t, th, tl)
}

multiversion! {
    /// The hyperbolic tangent, correctly rounded (round to nearest, ties to even): CORE-MATH
    /// `cr_tanh`. The same bits on every target. `Math.tanh` (§21.3.2.34): NaN and ±0 give
    /// themselves, ±∞ gives ±1.
    pub fn tanh(x: f64) -> f64 = tanh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tanh_special_values() {
        assert!(tanh(f64::NAN).is_nan());
        assert_eq!(tanh(0.0).to_bits(), 0);
        assert_eq!(tanh(-0.0).to_bits(), 1 << 63);
        assert_eq!(tanh(f64::INFINITY), 1.0);
        assert_eq!(tanh(f64::NEG_INFINITY), -1.0);
        assert_eq!(tanh(20.0), 1.0);
        assert_eq!(tanh(-20.0), -1.0);
    }
}
