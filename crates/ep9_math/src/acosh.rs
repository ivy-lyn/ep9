//! `Math.acosh`: the correctly rounded inverse hyperbolic cosine (H36), a port of CORE-MATH
//! `cr_acosh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/acosh/acosh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded inverse hyperbolic cosine for binary64 values.
//! Copyright (c) 2023 Alexei Sibidanov.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The constants and steps are the C ones.
//! The logarithm tables have the same C text as in asinh.c, so this file uses the
//! `crate::asinh` copies (`LL[1..3][16]`, zero here, are never read). The double-double
//! helpers of acosh.c differ from asinh.c (a 2Sum in `adddd`, Fast2Sum in `muldd_acc`) and stay
//! here. ep9 changes: rounding to nearest only (JS); no `errno`; every `__builtin_fma` is
//! [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA contraction.

use crate::asinh::{C, CH_REFINE, CL_REFINE, L1, L2, LL, R1, R2, T1, T2, T3, T4, log_index};
use crate::exact::FmaOps;
use crate::hf;

#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

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

/// twosum(xh,ch) + (xl + cl).
#[inline(always)]
fn adddd(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let (s, l) = twosum(xh, ch);
    (s, l + (xl + cl))
}

/// Algorithm 10 (DWTimesDW1) of Joldes, Muller, Popescu, "Tight and rigorous error bounds for
/// basic building blocks of double-word arithmetic" (relative error <= 7u^2).
#[inline(always)]
fn muldd_acc<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahlh = ch * xl;
    let alhh = cl * xh;
    let ahhh = ch * xh;
    let mut ahhl = F::fma(ch, xh, -ahhh);
    ahhl += alhh + ahlh;
    fasttwosum(ahhh, ahhl)
}

#[inline(always)]
fn mulddd<F: FmaOps>(xh: f64, xl: f64, ch: f64) -> (f64, f64) {
    let ahlh = ch * xl;
    let ahhh = ch * xh;
    let mut ahhl = F::fma(ch, xh, -ahhh);
    ahhl += ahlh;
    let ch = ahhh + ahhl;
    (ch, (ahhh - ch) + ahhl)
}

#[inline(always)]
fn polydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, mut cl) = fasttwosum(c[i][0], l);
    cl += c[i][1];
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd_acc::<F>(xh, xl, ch, cl);
        let (th, tl) = fasttwosum(c[i][0], ch);
        ch = th;
        cl += tl + c[i][1];
    }
    (ch, cl)
}

#[inline(always)]
fn as_acosh_one_with<F: FmaOps>(x: f64, sh: f64, sl: f64) -> f64 {
    const CH: [[f64; 2]; 10] = [
        [hf("-0x1.5555555555555p-4"), hf("-0x1.5555555554af1p-58")],
        [hf("0x1.3333333333333p-6"), hf("0x1.9999998933f0ep-61")],
        [hf("-0x1.6db6db6db6db7p-8"), hf("0x1.24929b16ec6b7p-63")],
        [hf("0x1.f1c71c71c71c7p-10"), hf("0x1.c56d45e265e2cp-66")],
        [hf("-0x1.6e8ba2e8ba2e9p-11"), hf("0x1.6d50ce7188d3dp-65")],
        [hf("0x1.1c4ec4ec4ec43p-12"), hf("0x1.c6791d1cf399ap-66")],
        [hf("-0x1.c99999999914fp-14"), hf("0x1.ee0d9408a2e2ap-68")],
        [hf("0x1.7a878787648e2p-15"), hf("-0x1.1cea281e08012p-69")],
        [hf("-0x1.3fde50d0cb4b9p-16"), hf("0x1.0335101403d9dp-72")],
        [hf("0x1.12ef3bf8a0a74p-17"), hf("0x1.f9c6b51787043p-80")],
    ];
    const CL: [f64; 6] = [
        hf("-0x1.df3b9d1296ea9p-19"),
        hf("0x1.a681d7d2298ebp-20"),
        hf("-0x1.77ead7b1ca449p-21"),
        hf("0x1.4edd2ddb3721fp-22"),
        hf("-0x1.1bf173531ee23p-23"),
        hf("0x1.613229230e255p-25"),
    ];
    let y2 = x * (CL[0] + x * (CL[1] + x * (CL[2] + x * (CL[3] + x * (CL[4] + x * CL[5])))));
    let (y1, y2) = polydd::<F>(x, 0.0, &CH, y2);
    let (y1, y2) = mulddd::<F>(y1, y2, x);
    let (y0, y1) = fasttwosum(1.0, y1);
    let y1 = y1 + y2;
    let (y0, y1) = muldd_acc::<F>(y0, y1, sh, sl);
    y0 + y1
}

multiversion! {
    fn as_acosh_one(x: f64, sh: f64, sl: f64) -> f64 = as_acosh_one_with;
}

#[inline(always)]
fn acosh_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    if ix >= 0x7ff0000000000000 {
        // x < 0 or NaN/Inf
        let aix = ix << 1;
        if ix == 0x7ff0000000000000 || aix > (0x7ffu64 << 53) {
            return x + x; // +inf or nan
        }
        return f64::NAN;
    }
    if (ix as i64) <= 0x3ff0000000000000 {
        // 0 <= x <= 1
        if ix == 0x3ff0000000000000 {
            return 0.0; // x=1
        }
        return f64::NAN;
    }
    // now x > 1
    let (g, eps);
    let mut off = 0x3fe;
    let mut t = ix;
    if ix < 0x3ff1e83e425aee63 {
        // 1 < x < 0x1.1e83e425aee63p+0 (checked exhaustively, with and without FMA)
        let z = x - 1.0;
        let iz = (-0.25) / z;
        let zt = 2.0 * z;
        let sh = zt.sqrt();
        let sl = F::fma(sh, sh, -zt) * (sh * iz);
        const CL: [f64; 9] = [
            hf("-0x1.5555555555555p-4"),
            hf("0x1.3333333332f95p-6"),
            hf("-0x1.6db6db6d5534cp-8"),
            hf("0x1.f1c71c1e04356p-10"),
            hf("-0x1.6e8b8e3e40d58p-11"),
            hf("0x1.1c4ba825ac4fep-12"),
            hf("-0x1.c9045534e6d9ep-14"),
            hf("0x1.71fedae26a76bp-15"),
            hf("-0x1.f1f4f8cc65342p-17"),
        ];
        let z2 = z * z;
        let z4 = z2 * z2;
        let ds = F::fma(
            sh * z,
            CL[0]
                + z * (((CL[1] + z * CL[2]) + z2 * (CL[3] + z * CL[4]))
                    + z4 * ((CL[5] + z * CL[6]) + z2 * (CL[7] + z * CL[8]))),
            sl,
        );
        let eps = ds * const { hf("0x1.00p-50") } - const { hf("0x1p-104") } * sh;
        let lb = sh + (ds - eps);
        let ub = sh + (ds + eps);
        if lb == ub {
            return lb;
        }
        return as_acosh_one(z, sh, sl);
    } else if ix < 0x405bf00000000000 {
        // 0x1.1e83e425aee63p+0 <= x < 111.75 (checked exhaustively with/without FMA)
        off = 0x3ff;
        let x2h = x * x;
        let wh = x2h - 1.0;
        let wl = F::fma(x, x, -x2h);
        let sh = wh.sqrt();
        let ish = 0.5 / wh;
        let sl = (wl - F::fma(sh, sh, -wh)) * (sh * ish);
        let (th, mut tl) = fasttwosum(x, sh);
        tl += sl;
        t = th.to_bits();
        g = tl / th;
        eps = const { hf("0x1.81p-63") };
    } else if ix < 0x4087100000000000 {
        // 111.75 <= x < 738 (tested exhaustively with/without FMA)
        const CL: [f64; 4] = [
            hf("0x1.5c4b6148816e2p-66"),
            hf("-0x1.000000000005cp-2"),
            hf("-0x1.7fffffebf3e6cp-4"),
            hf("-0x1.aab6691f2bae7p-5"),
        ];
        let z = 1.0 / (x * x);
        g = CL[0] + z * (CL[1] + z * (CL[2] + z * CL[3]));
        eps = const { hf("0x1.c3p-63") };
    } else if ix < 0x40e0100000000000 {
        // 738 <= x < 32896 (tested exhaustively with/without FMA)
        const CL: [f64; 3] = [
            hf("-0x1.7f77c8429c6c6p-67"),
            hf("-0x1.ffffffffff214p-3"),
            hf("-0x1.8000268641bfep-4"),
        ];
        let z = 1.0 / (x * x);
        g = CL[0] + z * (CL[1] + z * CL[2]);
        eps = const { hf("0x1.9ap-63") };
    } else if ix < 0x41ea000000000000 {
        // 32896 <= x < 0x1.ap+31
        const CL: [f64; 2] = [hf("0x1.7a0ed2effdd1p-67"), hf("-0x1.000000017d048p-2")];
        let z = 1.0 / (x * x);
        g = CL[0] + z * CL[1];
        eps = const { hf("0x1.99p-63") };
    } else {
        // 0x1.ap+31 <= x
        g = 0.0;
        eps = const { hf("0x1.b2p-63") };
    }
    let ex = (t >> 52) as i32;
    let e = ex - off;
    t &= !0u64 >> 12; // zero out the exponent field
    let ed = e as f64; // 2*x = (1+t/2^52)*2^ed
    let (i1, i2) = log_index(t);
    t |= 0x3ffu64 << 52;
    let r = R1[i1] * R2[i2];
    let dx = F::fma(r, f64::from_bits(t), -1.0);
    let dx2 = dx * dx;
    let f = dx2 * ((C[0] + dx * C[1]) + dx2 * ((C[2] + dx * C[3]) + dx2 * C[4]));
    // l2h+l2l approximates log(2) with error less than 2^-102.018; l2h*ed is exact
    const L2H: f64 = hf("0x1.62e42fefa38p-1");
    const L2L: f64 = hf("0x1.ef35793c7673p-45");
    let lh = (L1[i1][1] + L2[i2][1]) + L2H * ed;
    let t1 = (L2L * ed) + (L1[i1][0] + L2[i2][0]);
    let t2 = f + t1;
    let t3 = g + t2;
    let ll = dx + t3;
    let lb = lh + (ll - eps);
    let ub = lh + (ll + eps);
    if lb == ub {
        return lb;
    }
    // 0x1.71547652b82fep+0 approximates 1/log(2)
    as_acosh_refine(x, const { hf("0x1.71547652b82fep+0") } * lb)
}

/// The exceptional values, sorted by increasing x: x, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.5bff041b260fep+0"),
        hf("0x1.a6031cd5f93bap-1"),
        hf("0x1p-55"),
    ],
    [
        hf("0x1.9efdca62b700ap+0"),
        hf("0x1.104b648f113a1p+0"),
        hf("0x1p-54"),
    ],
    [
        hf("0x1.a5bf3acfde4b2p+0"),
        hf("0x1.1585720f35cd9p+0"),
        hf("-0x1p-54"),
    ],
    [
        hf("0x1.45ea160ddc71fp+7"),
        hf("0x1.725811dcf6782p+2"),
        hf("0x1p-52"),
    ],
    [
        hf("0x1.2a686e4b567cep+10"),
        hf("0x1.f1c928e7f1e65p+2"),
        hf("0x1p-52"),
    ],
    [
        hf("0x1.cb62eec26bd78p+15"),
        hf("0x1.759a2ad4c4d56p+3"),
        hf("0x1p-51"),
    ],
    [
        hf("0x1.3bf8009648dcp+16"),
        hf("0x1.7fce95ea5c653p+3"),
        hf("-0x1p-53"),
    ],
];

#[inline(never)]
fn as_acosh_database(x: f64, mut f: f64) -> f64 {
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    while a <= b {
        let row = &DB[m as usize];
        if row[0] < x {
            a = m + 1;
        } else if row[0] == x {
            f = row[1] + row[2];
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) / 2;
    }
    f
}

/// `a` is an approximation of acosh(x)/log(2).
#[inline(always)]
fn as_acosh_refine_with<F: FmaOps>(x: f64, a: f64) -> f64 {
    let ix = x.to_bits();
    let (zh, zl);
    let mut huge = 0; // exponent adjustment
    if ix < 0x4190000000000000 {
        // x < 2^26
        let x2h = x * x;
        let x2l = F::fma(x, x, -x2h);
        let wh = x2h - 1.0;
        let (wh, wl) = fasttwosum(wh, x2l);
        let sh = wh.sqrt();
        let sl = (wl - F::fma(sh, sh, -wh)) / (2.0 * sh);
        let (zh0, zl0) = fasttwosum(x, sh);
        let zl0 = zl0 + sl;
        (zh, zl) = fasttwosum(zh0, zl0);
    } else if ix < 0x4330000000000000 {
        // x < 2^52
        zh = 2.0 * x;
        zl = -0.5 / x;
    } else {
        // x >= 2^52: zh = 2*x would overflow for x > 2^1023, so add 1 to e below with zl == 0
        zh = x;
        zl = 0.0;
        huge = 1;
    }
    // zh+zl approximates x+sqrt(x^2-1), or (x+sqrt(x^2-1))/2 for x >= 2^52
    let mut t = zh.to_bits();
    let ex = (t >> 52) as i32;
    let e = ex - 0x3ff + huge; // 0 <= e <= 1024
    t &= !0u64 >> 12;
    t |= 0x3ffu64 << 52;
    // now 1 <= t < 2 and x+sqrt(x^2-1) ~ t*2^e
    let ed = e as f64;
    let v = ((a - ed) + const { hf("0x1.00008p+0") }).to_bits();
    let i = v.wrapping_sub(0x3ffu64 << 52) >> (52 - 16); // i = floor(2^16*(v-1))
    let i1 = ((i >> 12) & 0x1f) as usize;
    let i2 = ((i >> 8) & 0xf) as usize;
    let i3 = ((i >> 4) & 0xf) as usize;
    let i4 = (i & 0xf) as usize;
    const L20: f64 = hf("0x1.62e42fefa38p-2");
    const L21: f64 = hf("0x1.ef35793c768p-46");
    const L22: f64 = hf("-0x1.9ff0342542fc3p-91");
    let el2 = L22 * ed;
    let el1 = L21 * ed;
    let el0 = L20 * ed;
    // the sums L[0] and L[1] are exact (C comments)
    let mut l0 = (LL[0][i1][0] + LL[1][i2][0]) + (LL[2][i3][0] + LL[3][i4][0]);
    let l1 = (LL[0][i1][1] + LL[1][i2][1]) + (LL[2][i3][1] + LL[3][i4][1]);
    let l2 = (LL[0][i1][2] + LL[1][i2][2]) + (LL[2][i3][2] + LL[3][i4][2]);
    l0 += el0;
    let t12 = T1[i1] * T2[i2];
    let t34 = T3[i3] * T4[i4];
    let th = t12 * t34;
    let tl = F::fma(t12, t34, -th);
    let tf = f64::from_bits(t);
    let dh = th * tf;
    let dl = F::fma(th, tf, -dh);
    let sh = tl * tf;
    let sl = F::fma(tl, tf, -sh);
    let (xh, mut xl) = fasttwosum(dh - 1.0, dl);
    if zl != 0.0 {
        // divide by 2^e like t was zh/2^e
        let tz = zl.to_bits().wrapping_sub((e as i64 as u64) << 52);
        xl += th * f64::from_bits(tz);
    }
    let (xh, xl) = adddd(xh, xl, sh, sl);
    let sl = xh * (CL_REFINE[0] + xh * (CL_REFINE[1] + xh * CL_REFINE[2]));
    let (sh, sl) = polydd::<F>(xh, xl, &CH_REFINE, sl);
    let (sh, sl) = muldd_acc::<F>(xh, xl, sh, sl);
    let (sh, sl) = adddd(sh, sl, el1, el2);
    let (sh, sl) = adddd(sh, sl, l1, l2);
    let (v0, v2) = fasttwosum(l0, sh);
    let (v1, v2) = fasttwosum(v2, sl);
    let v0 = v0 * 2.0;
    let mut v1 = v1 * 2.0;
    let v2 = v2 * 2.0;
    let mut t = v1.to_bits();
    if t & (!0u64 >> 12) == 0 {
        let w = v2.to_bits();
        if (w ^ t) >> 63 != 0 {
            t = t.wrapping_sub(1);
        } else {
            t = t.wrapping_add(1);
        }
        v1 = f64::from_bits(t);
    }
    let t0 = v0.to_bits();
    let er = t.wrapping_add(7) & (!0u64 >> 12);
    let de = ((t0 >> 52) & 0x7ff).wrapping_sub((t >> 52) & 0x7ff);
    let res = v0 + v1;
    if de > 102 || er < 15 {
        return as_acosh_database(x, res);
    }
    res
}

multiversion! {
    fn as_acosh_refine(x: f64, a: f64) -> f64 = as_acosh_refine_with;
}

multiversion! {
    /// The inverse hyperbolic cosine, correctly rounded (round to nearest, ties to even):
    /// CORE-MATH `cr_acosh`. The same bits on every target. `Math.acosh` (§21.3.2.3): NaN and
    /// x < 1 give NaN, 1 gives +0, +∞ gives +∞.
    pub fn acosh(x: f64) -> f64 = acosh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acosh_special_values() {
        assert!(acosh(f64::NAN).is_nan());
        assert!(acosh(0.5).is_nan());
        assert!(acosh(0.0).is_nan());
        assert!(acosh(-0.0).is_nan());
        assert!(acosh(-1.0).is_nan());
        assert!(acosh(f64::NEG_INFINITY).is_nan());
        assert_eq!(acosh(1.0).to_bits(), 0);
        assert_eq!(acosh(f64::INFINITY), f64::INFINITY);
    }
}
