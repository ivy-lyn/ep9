//! `Math.atanh`: the correctly rounded inverse hyperbolic tangent (H36), a port of CORE-MATH
//! `cr_atanh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/atanh/atanh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded inverse hyperbolic tangent for binary64 values.
//! Copyright (c) 2023-2025 Alexei Sibidanov <sibid@uvic.ca>.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. The tables `B`, `r1`,
//! `r2`, `t1`..`t4`, `LL` and the refinement `ch` have the same C text as in asinh.c, so this
//! file uses the `crate::asinh` copies; `l1`, `l2` and `c` (log(1/r)/2 and a scaled
//! polynomial) differ and stay here, with the double-double helpers of atanh.c. ep9 changes:
//! rounding to nearest only (JS); no `errno`; every `__builtin_fma` is [`FmaOps::fma`]
//! (hardware or exact software FMA, the same bits); no FMA contraction.

use crate::asinh::{CH_REFINE, CL_REFINE, LL, R1, R2, T1, T2, T3, T4, log_index};
use crate::exact::FmaOps;
use crate::hf;

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
fn adddd(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let s = xh + ch;
    let d = s - xh;
    (s, ((ch - d) + (xh + (d - s))) + (xl + cl))
}

#[inline(always)]
fn muldd_acc<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahlh = ch * xl;
    let alhh = cl * xh;
    let ahhh = ch * xh;
    let mut ahhl = F::fma(ch, xh, -ahhh);
    ahhl += alhh + ahlh;
    fasttwosum(ahhh, ahhl)
}

/// `muldd_acc` without the Fast2Sum normalization.
#[inline(always)]
fn muldd<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahlh = ch * xl;
    let alhh = cl * xh;
    let ahhh = ch * xh;
    let ahhl = F::fma(ch, xh, -ahhh);
    (ahhh, ahhl + (alhh + ahlh))
}

#[inline(always)]
fn mulddd<F: FmaOps>(xh: f64, xl: f64, ch: f64) -> (f64, f64) {
    let hh = xh * ch;
    (hh, F::fma(ch, xh, -hh) + xl * ch)
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
fn as_atanh_zero_with<F: FmaOps>(x: f64) -> f64 {
    const CH: [[f64; 2]; 13] = [
        [hf("0x1.5555555555555p-2"), hf("0x1.5555555555555p-56")],
        [hf("0x1.999999999999ap-3"), hf("-0x1.999999999611cp-57")],
        [hf("0x1.2492492492492p-3"), hf("0x1.2492490f76b25p-57")],
        [hf("0x1.c71c71c71c71cp-4"), hf("0x1.c71cd5c38a112p-58")],
        [hf("0x1.745d1745d1746p-4"), hf("-0x1.7556c4165f4cap-59")],
        [hf("0x1.3b13b13b13b14p-4"), hf("-0x1.b893c3b36052ep-59")],
        [hf("0x1.1111111111105p-4"), hf("0x1.4e1afd723ed1fp-59")],
        [hf("0x1.e1e1e1e1e2678p-5"), hf("-0x1.f86ea96fb1435p-59")],
        [hf("0x1.af286bc9f90ccp-5"), hf("0x1.1e51a6e54fde9p-60")],
        [hf("0x1.8618618c779b6p-5"), hf("-0x1.ab913de95c3bfp-61")],
        [hf("0x1.642c84aa383ebp-5"), hf("0x1.632e747641b12p-59")],
        [hf("0x1.47ae2d205013cp-5"), hf("-0x1.0c9617e7bcff2p-60")],
        [hf("0x1.2f664d60473f9p-5"), hf("0x1.3adb3e2b7f35ep-61")],
    ];
    const CL: [f64; 5] = [
        hf("0x1.1a9a91fd692afp-5"), // degree 29
        hf("0x1.06dfbb35e7f44p-5"), // degree 31
        hf("0x1.037bed4d7588fp-5"), // degree 33
        hf("0x1.5aca6d6d720d6p-6"), // degree 35
        hf("0x1.99ea5700d53a5p-5"), // degree 37
    ];
    let x2 = x * x;
    let x2l = F::fma(x, x, -x2);
    let y2 = x2 * (CL[0] + x2 * (CL[1] + x2 * (CL[2] + x2 * (CL[3] + x2 * CL[4]))));
    let (y1, y2) = polydd::<F>(x2, x2l, &CH, y2);
    let (y1, y2) = mulddd::<F>(y1, y2, x);
    let (y1, y2) = muldd_acc::<F>(x2, x2l, y1, y2);
    let (y0, y1) = fasttwosum(x, y1);
    let (mut y1, y2) = fasttwosum(y1, y2);
    // 22 failures (round to nearest) without this check (C comment)
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
    y0 + y1
}

multiversion! {
    fn as_atanh_zero(x: f64) -> f64 = as_atanh_zero_with;
}

/// log(1/r1\[i\])/2 as (low, high).
static L1: [[f64; 2]; 33] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("-0x1.532c1269e2038p-27"), hf("0x1.62e5p-7")],
    [hf("0x1.ce42d81b54e84p-27"), hf("0x1.62e3cp-6")],
    [hf("-0x1.25826f815ec3dp-26"), hf("0x1.0a2acp-5")],
    [hf("0x1.0db1b1e7cee11p-26"), hf("0x1.62e4ap-5")],
    [hf("-0x1.1f3a8c6c95003p-26"), hf("0x1.bb9dcp-5")],
    [hf("-0x1.774cd4fb8c30dp-26"), hf("0x1.0a2b2p-4")],
    [hf("0x1.452e56c030a0ap-29"), hf("0x1.3687fp-4")],
    [hf("0x1.6b63c4966a79ap-28"), hf("0x1.62e41p-4")],
    [hf("-0x1.b20a21ccb525ep-28"), hf("0x1.8f40ap-4")],
    [hf("0x1.4006cfb3d8f85p-26"), hf("0x1.bb9d1p-4")],
    [hf("-0x1.cdb026b310c41p-26"), hf("0x1.e7f9bp-4")],
    [hf("-0x1.69124fdc0f16dp-26"), hf("0x1.0a2b08p-3")],
    [hf("-0x1.084656cdc2727p-26"), hf("0x1.205958p-3")],
    [hf("-0x1.376fa8b0357fdp-26"), hf("0x1.3687cp-3")],
    [hf("0x1.e56ae55a47b4ap-28"), hf("0x1.4cb5e8p-3")],
    [hf("0x1.070ff8834eeb4p-26"), hf("0x1.62e44p-3")],
    [hf("0x1.623516109f4fep-26"), hf("0x1.79129p-3")],
    [hf("-0x1.ec656b95fbdacp-29"), hf("0x1.8f40bp-3")],
    [hf("0x1.f0ca2e729f51p-28"), hf("0x1.a56ed8p-3")],
    [hf("-0x1.7d260a858354ap-26"), hf("0x1.bb9d68p-3")],
    [hf("0x1.e7279075503d3p-27"), hf("0x1.d1cb9p-3")],
    [hf("0x1.39e1a0a503873p-27"), hf("0x1.e7f9dp-3")],
    [hf("0x1.cd86d7b87c3d6p-26"), hf("0x1.fe27d8p-3")],
    [hf("0x1.060ab88de341ep-26"), hf("0x1.0a2b24p-2")],
    [hf("0x1.20a860d3f939p-28"), hf("0x1.154244p-2")],
    [hf("-0x1.dacee95fc2f1p-27"), hf("0x1.205974p-2")],
    [hf("0x1.45de3a86e0acap-26"), hf("0x1.2b707p-2")],
    [hf("0x1.c164cbfb991afp-27"), hf("0x1.3687bp-2")],
    [hf("0x1.d3f66b24225efp-26"), hf("0x1.419ec4p-2")],
    [hf("0x1.fc023efa144bap-26"), hf("0x1.4cb5f8p-2")],
    [hf("0x1.086a8af6f26cp-28"), hf("0x1.57cd28p-2")],
    [hf("-0x1.05c610ca86c39p-30"), hf("0x1.62e43p-2")],
];

/// log(1/r2\[i\])/2 as (low, high).
static L2: [[f64; 2]; 33] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("-0x1.37e152a129e4ep-28"), hf("0x1.632p-12")],
    [hf("-0x1.3f6c916b8be9cp-26"), hf("0x1.63p-11")],
    [hf("0x1.20505936739d5p-26"), hf("0x1.0a24p-10")],
    [hf("-0x1.23e2e8cb541bap-26"), hf("0x1.62dcp-10")],
    [hf("-0x1.acb7983ac4f5ep-32"), hf("0x1.bbap-10")],
    [hf("0x1.6f7c7689c63aep-28"), hf("0x1.0a2ap-9")],
    [hf("0x1.f5ca695b4c58bp-30"), hf("0x1.368cp-9")],
    [hf("-0x1.c6c18bd953226p-27"), hf("0x1.62e6p-9")],
    [hf("0x1.7a516c34846bdp-26"), hf("0x1.8f46p-9")],
    [hf("-0x1.f3b83dd8b853p-27"), hf("0x1.bbap-9")],
    [hf("-0x1.c3459046e4e57p-31"), hf("0x1.e8p-9")],
    [hf("0x1.b5c7e34cb79f6p-38"), hf("0x1.0a2cp-8")],
    [hf("-0x1.2487e9af9a692p-27"), hf("0x1.205cp-8")],
    [hf("0x1.f21bbc4ad79cep-26"), hf("0x1.3687p-8")],
    [hf("-0x1.550ffc857b731p-29"), hf("0x1.4cb7p-8")],
    [hf("0x1.87458ec1b7b34p-27"), hf("0x1.62e2p-8")],
    [hf("0x1.103d4fe83ee81p-26"), hf("0x1.7911p-8")],
    [hf("0x1.810483d3b398cp-27"), hf("0x1.8f44p-8")],
    [hf("-0x1.2085cb340608ep-27"), hf("0x1.a573p-8")],
    [hf("0x1.12698a119c42fp-26"), hf("0x1.bb9dp-8")],
    [hf("-0x1.edb8c172b4c33p-26"), hf("0x1.d1ccp-8")],
    [hf("-0x1.8b55b87a5e238p-26"), hf("0x1.e7fep-8")],
    [hf("0x1.be5e17763f78ap-26"), hf("0x1.fe2bp-8")],
    [hf("-0x1.c2d496790073ep-30"), hf("0x1.0a2a8p-7")],
    [hf("0x1.6542f523abeecp-26"), hf("0x1.1541p-7")],
    [hf("-0x1.b7fdbe5b193f8p-26"), hf("0x1.205ap-7")],
    [hf("0x1.fa4d42fe30c7cp-26"), hf("0x1.2b7p-7")],
    [hf("0x1.0d46ad04adc86p-26"), hf("0x1.36888p-7")],
    [hf("-0x1.1c22d02d17c4cp-26"), hf("0x1.419fp-7")],
    [hf("0x1.a7d1e330dcccep-30"), hf("0x1.4cb7p-7")],
    [hf("0x1.187025e656ba3p-31"), hf("0x1.57cdp-7")],
    [hf("-0x1.532c1269e2038p-27"), hf("0x1.62e5p-7")],
];

#[inline(always)]
fn atanh_with<F: FmaOps>(x: f64) -> f64 {
    let ax = x.abs();
    let aix = ax.to_bits();
    if aix >= 0x3ff0000000000000 {
        // |x| >= 1
        if aix == 0x3ff0000000000000 {
            return 1.0f64.copysign(x) / 0.0; // |x| = 1: pole
        }
        if aix > 0x7ff0000000000000 {
            return x + x; // nan
        }
        return f64::NAN;
    }
    if aix < 0x3fd0000000000000 {
        // |x| < 0x1p-2. atanh(x) rounds to x to nearest for |x| < 0x1.d12ed0af1a27fp-27.
        if aix < 0x3e4d12ed0af1a27f {
            return F::fma(x, const { hf("0x1p-55") }, x);
        }
        // c(x) = c[0]+c[1]*x^2+...+c[8]*x^16 is a minimax approximation of
        // (atanh(x) - x - x^3/3)/x^5 (relative error <= 2^-64.377 on [0x1.d12ed0af1a27fp-27,1/4])
        const C: [f64; 9] = [
            hf("0x1.999999999999ap-3"),
            hf("0x1.2492492492244p-3"),
            hf("0x1.c71c71c79715fp-4"),
            hf("0x1.745d16f777723p-4"),
            hf("0x1.3b13ca4174634p-4"),
            hf("0x1.110c9724989bdp-4"),
            hf("0x1.e2d17608a5b2ep-5"),
            hf("0x1.a0b56308cba0bp-5"),
            hf("0x1.fb6341208ad2ep-5"),
        ];
        let x2 = x * x;
        let dx2 = F::fma(x, x, -x2);
        let x4 = x2 * x2;
        let x3 = x2 * x;
        let x8 = x4 * x4;
        let dx3 = F::fma(x2, x, -x3) + dx2 * x;
        let p = ((C[0] + x2 * C[1]) + x4 * (C[2] + x2 * C[3]))
            + x8 * (((C[4] + x2 * C[5]) + x4 * (C[6] + x2 * C[7])) + x8 * C[8]);
        let t = F::fma(x2, p, const { hf("0x1.5555555555555p-56") });
        let (ph, pl) = fasttwosum(const { hf("0x1.5555555555555p-2") }, t);
        let (ph, mut pl) = muldd::<F>(ph, pl, x3, dx3);
        let (ph, tl) = fasttwosum(x, ph);
        pl += tl;
        let eps = x * (x4 * const { hf("0x1.dp-53") } + const { hf("0x1p-103") });
        let lb = ph + (pl - eps);
        let ub = ph + (pl + eps);
        if lb == ub {
            return lb;
        }
        return as_atanh_zero(x);
    }
    // |x| >= 0.25 (searched exhaustively with and without FMA, C comment)
    let (ph, pl) = fasttwosum(1.0, ax);
    let (qh, ql) = fasttwosub(1.0, ax);
    let iqh = 1.0 / qh;
    let th = ph * iqh;
    let tl = F::fma(ph, iqh, -th) + (pl + ph * (F::fma(-qh, iqh, 1.0) - ql * iqh)) * iqh;
    const C: [f64; 5] = [
        hf("-0x1p+0"),
        hf("0x1.555555555553p+0"),
        hf("-0x1.fffffffffffap+0"),
        hf("0x1.99999e33a6366p+1"),
        hf("-0x1.555559ef9525fp+2"),
    ];
    let mut t = th.to_bits();
    let ex = (t >> 52) as i32;
    let e = ex - 0x3ff;
    t &= !0u64 >> 12;
    let ed = e as f64;
    let (i1, i2) = log_index(t);
    t |= 0x3ffu64 << 52;
    let tf = f64::from_bits(t);
    let r = (0.5 * R1[i1]) * R2[i2];
    let dx = F::fma(r, tf, -0.5);
    let dx2 = dx * dx;
    let rx = r * tf;
    let dxl = F::fma(r, tf, -rx);
    let f = dx2 * ((C[0] + dx * C[1]) + dx2 * ((C[2] + dx * C[3]) + dx2 * C[4]));
    const L2H: f64 = hf("0x1.62e42fefa3ap-2");
    const L2L: f64 = hf("-0x1.0ca86c3898dp-50");
    let lh = (L1[i1][1] + L2[i2][1]) + L2H * ed;
    let (mut lh, mut ll) = fasttwosum(lh, rx - 0.5);
    ll += (((L2L * ed) + (L1[i1][0] + L2[i2][0])) + dxl) + (0.5 * tl) / th;
    ll += f;
    lh *= 1.0f64.copysign(x);
    ll *= 1.0f64.copysign(x);
    let eps = 38e-24 + dx2 * const { hf("0x1p-49") };
    let lb = lh + (ll - eps);
    let ub = lh + (ll + eps);
    if lb == ub {
        return lb;
    }
    let (th, tl) = fasttwosum(th, tl);
    as_atanh_refine(
        x,
        th,
        tl,
        const { hf("0x1.71547652b82fep+1") } * (lh + ll).abs(),
    )
}

/// The exceptional cases: |x|, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.2dbb7b1c91363p-2"),
        hf("0x1.36f33d51c264dp-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.c493dc899e4a5p-2"),
        hf("0x1.e611aa58ab608p-2"),
        hf("-0x1p-56"),
    ],
];

#[inline(never)]
fn as_atanh_database(x: f64, mut f: f64) -> f64 {
    let ax = x.abs();
    let sgn = 1.0f64.copysign(x);
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    while a <= b {
        let row = &DB[m as usize];
        if row[0] < ax {
            a = m + 1;
        } else if row[0] == ax {
            f = sgn * row[1] + sgn * row[2];
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) / 2;
    }
    f
}

#[inline(always)]
fn as_atanh_refine_with<F: FmaOps>(x: f64, zh: f64, zl: f64, a: f64) -> f64 {
    let mut t = zh.to_bits();
    let ex = (t >> 52) as i32;
    let e = ex - 0x3ff;
    t &= !0u64 >> 12;
    t |= 0x3ffu64 << 52;
    let ed = e as f64;
    let v = ((a - ed) + const { hf("0x1.00008p+0") }).to_bits();
    let i = v.wrapping_sub(0x3ffu64 << 52) >> (52 - 16);
    let i1 = (i >> 12) as usize;
    let i2 = ((i >> 8) & 0xf) as usize;
    let i3 = ((i >> 4) & 0xf) as usize;
    let i4 = (i & 0xf) as usize;
    const L20: f64 = hf("0x1.62e42fefa3ap-2");
    const L21: f64 = hf("-0x1.0ca86c3898dp-50");
    const L22: f64 = hf("0x1.f97b57a079ap-104");
    let el2 = L22 * ed;
    let el1 = L21 * ed;
    let el0 = L20 * ed;
    let l0 = ((LL[0][i1][0] + LL[1][i2][0]) + (LL[2][i3][0] + LL[3][i4][0])) + el0;
    let l1 = (LL[0][i1][1] + LL[1][i2][1]) + (LL[2][i3][1] + LL[3][i4][1]);
    let l2 = (LL[0][i1][2] + LL[1][i2][2]) + (LL[2][i3][2] + LL[3][i4][2]);
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
    let tz = zl.to_bits().wrapping_sub((e as i64 as u64) << 52);
    xl += th * f64::from_bits(tz);
    let (xh, xl) = adddd(xh, xl, sh, sl);
    let sl = xh * (CL_REFINE[0] + xh * (CL_REFINE[1] + xh * CL_REFINE[2]));
    let (sh, sl) = polydd::<F>(xh, xl, &CH_REFINE, sl);
    let (sh, sl) = muldd_acc::<F>(xh, xl, sh, sl);
    let (sh, sl) = adddd(sh, sl, el1, el2);
    let (sh, sl) = adddd(sh, sl, l1, l2);
    let (v0, v2) = fasttwosum(l0, sh);
    let (mut v1, _v2) = fasttwosum(v2, sl);
    let mut t = v1.to_bits();
    if t & (!0u64 >> 12) == 0 {
        // is v1 a power of 2? (5 values up to sign; v1 and v2 have the same sign there, C
        // comment)
        t = t.wrapping_add(1);
        v1 = f64::from_bits(t);
    }
    let t0 = v0.to_bits();
    let er = t.wrapping_add(1) & (!0u64 >> 12);
    let de = ((t0 >> 52) & 0x7ff).wrapping_sub((t >> 52) & 0x7ff);
    let v0 = v0 * 1.0f64.copysign(x);
    let v1 = v1 * 1.0f64.copysign(x);
    let res = v0 + v1;
    if de > 104 || er < 3 {
        return as_atanh_database(x, res);
    }
    res
}

multiversion! {
    fn as_atanh_refine(x: f64, zh: f64, zl: f64, a: f64) -> f64 = as_atanh_refine_with;
}

multiversion! {
    /// The inverse hyperbolic tangent, correctly rounded (round to nearest, ties to even):
    /// CORE-MATH `cr_atanh`. The same bits on every target. `Math.atanh` (§21.3.2.7): NaN and
    /// |x| > 1 give NaN, ±0 gives ±0, ±1 gives ±∞.
    pub fn atanh(x: f64) -> f64 = atanh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atanh_special_values() {
        assert!(atanh(f64::NAN).is_nan());
        assert!(atanh(1.0000000000000002).is_nan());
        assert!(atanh(f64::INFINITY).is_nan());
        assert_eq!(atanh(0.0).to_bits(), 0);
        assert_eq!(atanh(-0.0).to_bits(), 1 << 63);
        assert_eq!(atanh(1.0), f64::INFINITY);
        assert_eq!(atanh(-1.0), f64::NEG_INFINITY);
    }
}
