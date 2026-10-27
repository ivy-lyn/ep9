//! `Math.asinh`: the correctly rounded inverse hyperbolic sine (H36), a port of CORE-MATH
//! `cr_asinh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/asinh/asinh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded inverse hyperbolic sine for binary64 values.
//! Copyright (c) 2023-2025 Alexei Sibidanov <sibid@uvic.ca>.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. The logarithm tables
//! (`B`, `r1`, `r2`, `l1`, `l2`, `c`, and the refinement tables `t1`..`t4`, `LL` and `ch`)
//! have the same C text in acosh.c, and partly in atanh.c, so `crate::acosh` and
//! `crate::atanh` use these copies (`LL[1..3][16]` are not used; acosh.c leaves them
//! zero). Each file keeps its own double-double helpers, which differ in their C text. ep9
//! changes: rounding to nearest only (JS); no `errno`; every `__builtin_fma` is
//! [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA contraction.

use crate::exact::FmaOps;
use crate::hf;

#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
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
    let ch = ahhh + ahhl;
    (ch, (ahhh - ch) + ahhl)
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
    let mut ch = c[i][0] + l;
    let mut cl = ((c[i][0] - ch) + l) + c[i][1];
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd_acc::<F>(xh, xl, ch, cl);
        let th = ch + c[i][0];
        let tl = (c[i][0] - th) + ch;
        ch = th;
        cl += tl + c[i][1];
    }
    (ch, cl)
}

#[inline(always)]
fn as_asinh_zero_with<F: FmaOps>(x: f64, x2h: f64, x2l: f64) -> f64 {
    const CH: [[f64; 2]; 12] = [
        [hf("-0x1.5555555555555p-3"), hf("-0x1.5555555555555p-57")],
        [hf("0x1.3333333333333p-4"), hf("0x1.99999999949dfp-59")],
        [hf("-0x1.6db6db6db6db7p-5"), hf("0x1.2492496091b0cp-60")],
        [hf("0x1.f1c71c71c71c7p-6"), hf("0x1.c71a35cfa0671p-62")],
        [hf("-0x1.6e8ba2e8ba2e9p-6"), hf("0x1.17f937248cf81p-60")],
        [hf("0x1.1c4ec4ec4ec4fp-6"), hf("-0x1.74e3c1dfd4c3dp-60")],
        [hf("-0x1.c999999999977p-7"), hf("-0x1.38e7a467ecc55p-61")],
        [hf("0x1.7a87878786c7ep-7"), hf("0x1.a83c7bace55ebp-61")],
        [hf("-0x1.3fde50d764083p-7"), hf("-0x1.d024df7fa0542p-61")],
        [hf("0x1.12ef3ceae4d12p-7"), hf("-0x1.ba9c13deb261fp-61")],
        [hf("-0x1.df3bd104aa267p-8"), hf("-0x1.546da9bc5b32ap-62")],
        [hf("0x1.a685fc5de7a04p-8"), hf("0x1.40d284a1d67f9p-62")],
    ];
    const CL: [f64; 5] = [
        hf("-0x1.7828d553ec8p-8"),
        hf("0x1.51712f7bee368p-8"),
        hf("-0x1.2e6d98527bcc6p-8"),
        hf("0x1.0095da47b392cp-8"),
        hf("-0x1.3b92d6368192cp-9"),
    ];
    let y2 = x2h * (CL[0] + x2h * (CL[1] + x2h * (CL[2] + x2h * (CL[3] + x2h * CL[4]))));
    let (y1, y2) = polydd::<F>(x2h, x2l, &CH, y2);
    let (y1, y2) = muldd_acc::<F>(y1, y2, x2h, x2l);
    let (y1, y2) = mulddd::<F>(y1, y2, x);
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
    }
    y0 + y1
}

multiversion! {
    fn as_asinh_zero(x: f64, x2h: f64, x2l: f64) -> f64 = as_asinh_zero_with;
}

/// Coefficients of a linear piecewise approximation of log2(x) in \[1,2\] with an error
/// ~< 5.8e-4: (c0, c1) = (ushort, short).
pub(crate) static B: [(u16, i16); 32] = [
    (301, 27565),
    (7189, 24786),
    (13383, 22167),
    (18923, 19696),
    (23845, 17361),
    (28184, 15150),
    (31969, 13054),
    (35231, 11064),
    (37996, 9173),
    (40288, 7372),
    (42129, 5657),
    (43542, 4020),
    (44546, 2457),
    (45160, 962),
    (45399, -468),
    (45281, -1838),
    (44821, -3151),
    (44032, -4412),
    (42929, -5622),
    (41522, -6786),
    (39825, -7905),
    (37848, -8982),
    (35602, -10020),
    (33097, -11020),
    (30341, -11985),
    (27345, -12916),
    (24115, -13816),
    (20661, -14685),
    (16989, -15526),
    (13107, -16339),
    (9022, -17126),
    (4740, -17889),
];

/// 20-bit approximation of 2^(-i/32).
pub(crate) static R1: [f64; 33] = [
    hf("0x1p+0"),
    hf("0x1.f5076p-1"),
    hf("0x1.ea4bp-1"),
    hf("0x1.dfc98p-1"),
    hf("0x1.d5818p-1"),
    hf("0x1.cb72p-1"),
    hf("0x1.c199cp-1"),
    hf("0x1.b7f76p-1"),
    hf("0x1.ae8ap-1"),
    hf("0x1.a5504p-1"),
    hf("0x1.9c492p-1"),
    hf("0x1.93738p-1"),
    hf("0x1.8ace6p-1"),
    hf("0x1.8258ap-1"),
    hf("0x1.7a114p-1"),
    hf("0x1.71f76p-1"),
    hf("0x1.6a09ep-1"),
    hf("0x1.6247ep-1"),
    hf("0x1.5ab08p-1"),
    hf("0x1.5342cp-1"),
    hf("0x1.4bfdap-1"),
    hf("0x1.44e08p-1"),
    hf("0x1.3dea6p-1"),
    hf("0x1.371a8p-1"),
    hf("0x1.306fep-1"),
    hf("0x1.29e9ep-1"),
    hf("0x1.2387ap-1"),
    hf("0x1.1d488p-1"),
    hf("0x1.172b8p-1"),
    hf("0x1.11302p-1"),
    hf("0x1.0b558p-1"),
    hf("0x1.059bp-1"),
    hf("0x1p-1"),
];

/// 20-bit approximation of 2^(-i/1024).
pub(crate) static R2: [f64; 33] = [
    hf("0x1p+0"),
    hf("0x1.ffa74p-1"),
    hf("0x1.ff4eap-1"),
    hf("0x1.fef62p-1"),
    hf("0x1.fe9dap-1"),
    hf("0x1.fe452p-1"),
    hf("0x1.fdeccp-1"),
    hf("0x1.fd946p-1"),
    hf("0x1.fd3c2p-1"),
    hf("0x1.fce3ep-1"),
    hf("0x1.fc8bcp-1"),
    hf("0x1.fc33ap-1"),
    hf("0x1.fbdbap-1"),
    hf("0x1.fb83ap-1"),
    hf("0x1.fb2bcp-1"),
    hf("0x1.fad3ep-1"),
    hf("0x1.fa7c2p-1"),
    hf("0x1.fa246p-1"),
    hf("0x1.f9ccap-1"),
    hf("0x1.f975p-1"),
    hf("0x1.f91d8p-1"),
    hf("0x1.f8c6p-1"),
    hf("0x1.f86e8p-1"),
    hf("0x1.f8172p-1"),
    hf("0x1.f7bfep-1"),
    hf("0x1.f768ap-1"),
    hf("0x1.f7116p-1"),
    hf("0x1.f6ba4p-1"),
    hf("0x1.f6632p-1"),
    hf("0x1.f60c2p-1"),
    hf("0x1.f5b52p-1"),
    hf("0x1.f55e4p-1"),
    hf("0x1.f5076p-1"),
];

/// Natural logarithm of 1/r1\[i\] as (low, high).
pub(crate) static L1: [[f64; 2]; 33] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("-0x1.269e2038315b3p-46"), hf("0x1.62e4eacd4p-6")],
    [hf("-0x1.3f2558bddfc47p-45"), hf("0x1.62e3ce7218p-5")],
    [hf("0x1.07ea13c34efb5p-45"), hf("0x1.0a2ab6d3ecp-4")],
    [hf("0x1.8f3e77084d3bap-44"), hf("0x1.62e4a86d8cp-4")],
    [hf("-0x1.8d92a005f1a7ep-46"), hf("0x1.bb9db7062cp-4")],
    [hf("0x1.58239e799bfe5p-44"), hf("0x1.0a2b1a22ccp-3")],
    [hf("-0x1.a93fcf5f593b7p-44"), hf("0x1.3687f0a298p-3")],
    [hf("-0x1.db4cac32fd2b5p-46"), hf("0x1.62e4116b64p-3")],
    [hf("-0x1.0e65a92ee0f3bp-46"), hf("0x1.8f409e4df6p-3")],
    [hf("-0x1.8261383d475f1p-44"), hf("0x1.bb9d15001cp-3")],
    [hf("-0x1.359886207513bp-44"), hf("0x1.e7f9a8c94p-3")],
    [hf("0x1.811f87496ceb7p-44"), hf("0x1.0a2b052ddbp-2")],
    [hf("0x1.4991ec6cb435cp-44"), hf("0x1.205955ef73p-2")],
    [hf("-0x1.4581abfeb8927p-44"), hf("0x1.3687bd9121p-2")],
    [hf("0x1.cab48f6942703p-44"), hf("0x1.4cb5e8f2b5p-2")],
    [hf("-0x1.df2c452fde132p-47"), hf("0x1.62e4420e2p-2")],
    [hf("0x1.6109f4fdb74bdp-45"), hf("0x1.791292c46ap-2")],
    [hf("-0x1.6b95fbdac7696p-44"), hf("0x1.8f40af84e7p-2")],
    [hf("0x1.7394fa880cbdap-46"), hf("0x1.a56ed8f865p-2")],
    [hf("-0x1.50b06a94eccabp-46"), hf("0x1.bb9d6505b4p-2")],
    [hf("-0x1.be2abf0b38989p-44"), hf("0x1.d1cb91e728p-2")],
    [hf("-0x1.7d6bf1e34da04p-44"), hf("0x1.e7f9d139e2p-2")],
    [hf("-0x1.423c1e14de6edp-44"), hf("0x1.fe27db9b0ep-2")],
    [hf("0x1.c46f1a0efbbc2p-44"), hf("0x1.0a2b25060a8p-1")],
    [hf("0x1.834fe4e3e6018p-45"), hf("0x1.154244482ap-1")],
    [hf("0x1.6a03d0f02b65p-46"), hf("0x1.20597312988p-1")],
    [hf("0x1.d437056526f3p-44"), hf("0x1.2b707145dep-1")],
    [hf("-0x1.a0233728405c5p-45"), hf("0x1.3687b0e0b28p-1")],
    [hf("-0x1.4dbdda10d2bf1p-45"), hf("0x1.419ec5d3f68p-1")],
    [hf("0x1.f7d0a25d154f2p-44"), hf("0x1.4cb5f9fc02p-1")],
    [hf("0x1.15ede4d803b18p-44"), hf("0x1.57cd28421a8p-1")],
    [hf("0x1.ef35793c7673p-45"), hf("0x1.62e42fefa38p-1")],
];

/// Natural logarithm of 1/r2\[i\] as (low, high).
pub(crate) static L2: [[f64; 2]; 33] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("0x1.5abdac3638e99p-44"), hf("0x1.631ec81ep-11")],
    [hf("-0x1.16b8be9bbe239p-45"), hf("0x1.62fd8127p-10")],
    [hf("-0x1.364c6315542ebp-44"), hf("0x1.0a2520508p-9")],
    [hf("0x1.734abe459c9p-45"), hf("0x1.62dadc1dp-9")],
    [hf("0x1.0cf8a761431bfp-44"), hf("0x1.bb9ff94dp-9")],
    [hf("0x1.da2718eb78708p-45"), hf("0x1.0a2a2def8p-8")],
    [hf("0x1.34ada62c59b93p-44"), hf("0x1.368c0fae4p-8")],
    [hf("0x1.d09ab376682d4p-44"), hf("0x1.62e58e4f8p-8")],
    [hf("-0x1.3cb7b94329211p-45"), hf("0x1.8f46bd28cp-8")],
    [hf("-0x1.eec5c297c41dp-45"), hf("0x1.bb9f8312p-8")],
    [hf("-0x1.6411b9395d15p-44"), hf("0x1.e7fff8f3p-8")],
    [hf("-0x1.1c0e59a43053cp-44"), hf("0x1.0a2c0006ep-7")],
    [hf("0x1.6506596e077b6p-46"), hf("0x1.205bdb6fp-7")],
    [hf("0x1.e256bce6faa27p-44"), hf("0x1.36877c86ep-7")],
    [hf("0x1.bd42467b0c8d1p-51"), hf("0x1.4cb6f5578p-7")],
    [hf("-0x1.c4f92132ff0fp-44"), hf("0x1.62e230e8cp-7")],
    [hf("-0x1.80be08bfab39p-44"), hf("0x1.7911440f6p-7")],
    [hf("-0x1.f0b1319ceb1f7p-44"), hf("0x1.8f443020ap-7")],
    [hf("0x1.a65fcfb8de99bp-45"), hf("0x1.a572dbef4p-7")],
    [hf("0x1.4233885d3779cp-46"), hf("0x1.bb9d449a6p-7")],
    [hf("0x1.f46a59e646edbp-44"), hf("0x1.d1cb8491cp-7")],
    [hf("-0x1.c3d2f11c11446p-44"), hf("0x1.e7fd9d2aap-7")],
    [hf("0x1.7763f78a1e0ccp-45"), hf("0x1.fe2b6f978p-7")],
    [hf("0x1.b4c37fc60c043p-44"), hf("0x1.0a2a7c7a5p-6")],
    [hf("-0x1.5b8a822859be3p-46"), hf("0x1.15412ca86p-6")],
    [hf("-0x1.f2d8c9fc064p-44"), hf("0x1.2059c9005p-6")],
    [hf("-0x1.e80e79c20378dp-44"), hf("0x1.2b703f49bp-6")],
    [hf("0x1.68256e4329bdbp-44"), hf("0x1.3688a1a8dp-6")],
    [hf("0x1.7e9741da248c3p-44"), hf("0x1.419edc7bap-6")],
    [hf("0x1.e330dccce602bp-45"), hf("0x1.4cb7034fap-6")],
    [hf("0x1.2f32b5d18eefbp-49"), hf("0x1.57cd01187p-6")],
    [hf("-0x1.269e2038315b3p-46"), hf("0x1.62e4eacd4p-6")],
];

/// x+c\[0\]*x^2+c\[1\]*x^3+...+c\[4\]*x^6 approximates log(1+x) with absolute error bounded by
/// 2^-85.878 for |x| < 2^-11.296.
pub(crate) static C: [f64; 5] = [
    hf("-0x1p-1"),
    hf("0x1.555555555553p-2"),
    hf("-0x1.fffffffffffap-3"),
    hf("0x1.99999e33a6366p-3"),
    hf("-0x1.555559ef9525fp-3"),
];

/// The index pair (i1, i2) of `R1`/`R2` and the reduced argument `t` in [1,2) (as bits) for
/// the significand bits `tu` of the argument (exponent field already cleared), as in asinh.c
/// and acosh.c.
#[inline(always)]
pub(crate) fn log_index(tu: u64) -> (usize, usize) {
    let i = (tu >> (52 - 5)) as usize; // upper 5 bits of the significand
    let d = (tu & (!0u64 >> 17)) as i64; // low 47 bits of the significand
    let (c0, c1) = B[i];
    let j = (tu
        .wrapping_add((c0 as u64) << 33)
        .wrapping_add((c1 as i64 * (d >> 16)) as u64))
        >> (52 - 10);
    ((j >> 5) as usize, (j & 0x1f) as usize)
}

#[inline(always)]
fn asinh_with<F: FmaOps>(x: f64) -> f64 {
    let ax = x.abs();
    let u = ax.to_bits();
    if u < 0x3fbb000000000000 {
        // |x| < 0x1.bp-4
        // for |x| < 0x1.7137449123ef7p-26, asinh(x) rounds to x to nearest
        if u < 0x3e57137449123ef7 {
            // x = 0 apart: fma(-2^-60,-0,-0) gives +0 to nearest
            if u == 0 {
                return x;
            }
            return F::fma(const { hf("-0x1p-60") }, x, x);
        }
        let x2h = x * x;
        let x2l = F::fma(x, x, -x2h);
        let x3h = x2h * x;
        let sl;
        if u < 0x3f93000000000000 {
            // |x| < 0x1.3p-6
            if u < 0x3f30000000000000 {
                // |x| < 0x1p-12
                if u < 0x3e5a000000000000 {
                    // |x| < 0x1.ap-26
                    sl = x3h * const { hf("-0x1.5555555555555p-3") };
                } else {
                    sl = x3h
                        * (const { hf("-0x1.5555555555555p-3") }
                            + x2h * const { hf("0x1.3333327c57c6p-4") });
                }
            } else {
                const CL: [f64; 4] = [
                    hf("-0x1.5555555555555p-3"),
                    hf("0x1.333333332f2ffp-4"),
                    hf("-0x1.6db6d9a665159p-5"),
                    hf("0x1.f186866d775fp-6"),
                ];
                sl = x3h * (CL[0] + x2h * (CL[1] + x2h * (CL[2] + x2h * CL[3])));
            }
        } else {
            // 0x1.3p-6 <= |x| < 0x1.bp-4: a minimax polynomial with relative error < 2^-63.091
            // (tested exhaustively by Vincenzo Innocente, C comment)
            const CL: [f64; 7] = [
                hf("-0x1.5555555555555p-3"),
                hf("0x1.333333333331p-4"),
                hf("-0x1.6db6db6da466cp-5"),
                hf("0x1.f1c71c2ea7be4p-6"),
                hf("-0x1.6e8b651b09d72p-6"),
                hf("0x1.1c309fc0e69c2p-6"),
                hf("-0x1.bab7833c1ep-7"),
            ];
            let c1 = CL[1] + x2h * CL[2];
            let c3 = CL[3] + x2h * CL[4];
            let c5 = CL[5] + x2h * CL[6];
            let x4 = x2h * x2h;
            sl = x3h * (CL[0] + x2h * (c1 + x4 * (c3 + x4 * c5)));
        }
        let eps = const { hf("0x1.79p-53") } * x3h;
        let lb = x + (sl - eps);
        let ub = x + (sl + eps);
        if lb == ub {
            return lb;
        }
        return as_asinh_zero(x, x2h, x2l);
    }
    // |x| >= 0x1.bp-4
    let (mut x2h, mut x2l) = (0.0, 0.0);
    let (ah, al);
    let mut off = 0x3ff;
    if u < 0x4190000000000000 {
        // |x| < 0x1p+26
        x2h = x * x;
        x2l = F::fma(x, x, -x2h);
        let (th, mut tl) = if u < 0x3ff0000000000000 {
            fasttwosum(1.0, x2h)
        } else {
            fasttwosum(x2h, 1.0)
        };
        tl += x2l;
        let ah0 = th.sqrt();
        let rs = 0.5 / th;
        let al0 = (tl - F::fma(ah0, ah0, -th)) * (rs * ah0);
        let (ah1, tl) = fasttwosum(ah0, ax);
        ah = ah1;
        al = al0 + tl;
    } else if u < 0x4330000000000000 {
        // |x| < 0x1p+52 (tested exhaustively from 2^51 to 2^52, C comment)
        ah = 2.0 * ax;
        al = 0.5 / ax;
    } else {
        // |x| >= 0x1p+52
        if u >= 0x7ff0000000000000 {
            return x + x; // +-inf or nan
        }
        off = 0x3fe;
        ah = ax;
        al = 0.0;
    }
    let mut t = ah.to_bits();
    let ex = (t >> 52) as i32;
    let e = ex - off;
    t &= !0u64 >> 12;
    let ed = e as f64;
    let (i1, i2) = log_index(t);
    t |= 0x3ffu64 << 52;
    let r = R1[i1] * R2[i2];
    let dx = F::fma(r, f64::from_bits(t), -1.0);
    let dx2 = dx * dx;
    let f = dx2 * ((C[0] + dx * C[1]) + dx2 * ((C[2] + dx * C[3]) + dx2 * C[4]));
    const L2H: f64 = hf("0x1.62e42fefa38p-1");
    const L2L: f64 = hf("0x1.ef35793c7673p-45");
    let mut lh = L2H * ed + (L1[i1][1] + L2[i2][1]);
    let mut ll = (((L2L * ed + L1[i1][0]) + L2[i2][0]) + al / ah) + f;
    ll += dx;
    lh *= 1.0f64.copysign(x);
    ll *= 1.0f64.copysign(x);
    let eps = 1.63e-19;
    let lb = lh + (ll - eps);
    let ub = lh + (ll + eps);
    if lb == ub {
        return lb;
    }
    if ax < const { hf("0x1p-2") } {
        return as_asinh_zero(x, x2h, x2l);
    }
    as_asinh_refine(x, ah, al, const { hf("0x1.71547652b82fep+0") } * lb.abs())
}

/// The exceptional cases: |x|, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.00f9476450863p-2"),
        hf("0x1.fcb35067f343cp-3"),
        hf("0x1p-57"),
    ],
    [
        hf("0x1.1f0a79315b287p-2"),
        hf("0x1.1b68aae88febap-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.2b9618ff7acb7p-2"),
        hf("0x1.27781d9aa4e25p-2"),
        hf("-0x1p-56"),
    ],
    [
        hf("0x1.389ef683f3aa7p-2"),
        hf("0x1.33f52db6df1afp-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.3b07e0c779ddap-2"),
        hf("0x1.364303e1ad8f6p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.48441df33b6d3p-2"),
        hf("0x1.42e385800f0a4p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.687bd068c1c1ep-2"),
        hf("0x1.616cc75d49226p-2"),
        hf("-0x1p-56"),
    ],
    [
        hf("0x1.8740c4453a056p-2"),
        hf("0x1.7e4f2ad132a1dp-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.891acda11167ep-2"),
        hf("0x1.8009d924a3ffdp-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.bafc3479fc9ccp-2"),
        hf("0x1.ae3773250e7d2p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.c59869f17b483p-2"),
        hf("0x1.b7efa91915c95p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.c8be879787986p-2"),
        hf("0x1.bad0485e0fe0ap-2"),
        hf("-0x1p-56"),
    ],
    [
        hf("0x1.e73b46abb01e1p-2"),
        hf("0x1.d68039861ab53p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.ed6236da268bp-2"),
        hf("0x1.dc0cb8f638126p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.f399ebafc1951p-2"),
        hf("0x1.e1a4f519fab77p-2"),
        hf("-0x1p-56"),
    ],
    [
        hf("0x1.f70975ab0d471p-2"),
        hf("0x1.e4bae8bcd6ea6p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.fbdd4a37760b7p-2"),
        hf("0x1.e90f16eb88c09p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.fee72efb4bfddp-2"),
        hf("0x1.ebc791a88bed8p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.02339d6bdb741p-1"),
        hf("0x1.f0b2264e34555p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.09e7c831b1a23p-1"),
        hf("0x1.fe694c3c89138p-2"),
        hf("0x1p-56"),
    ],
    [
        hf("0x1.16d32c862fc3bp-1"),
        hf("0x1.0a9c9334066dbp-1"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.857954132083dp-1"),
        hf("0x1.67425fe575c88p-1"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.8a5c3b60f7e11p-1"),
        hf("0x1.6b23ad4415a17p-1"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.9740eb419dd04p-1"),
        hf("0x1.754ab7535d47dp-1"),
        hf("0x1p-55"),
    ],
    [
        hf("0x1.a16d9cc06011ap-1"),
        hf("0x1.7d3755d851062p-1"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.bb635be2213d1p-1"),
        hf("0x1.91167cae3cfa9p-1"),
        hf("0x1p-55"),
    ],
    [
        hf("0x1.d4b21ebf542fp-1"),
        hf("0x1.a3fc7e4dd47d1p-1"),
        hf("-0x1p-55"),
    ],
    [
        hf("0x1.7b8516ffd2406p+0"),
        hf("0x1.2f5d3b178914ap+0"),
        hf("0x1p-54"),
    ],
    [
        hf("0x1.9295b9116e2e2p+0"),
        hf("0x1.3bffa8863976p+0"),
        hf("0x1p-54"),
    ],
    [
        hf("0x1.fedc65e32714p+0"),
        hf("0x1.710f91e844f9bp+0"),
        hf("0x1p-54"),
    ],
    [
        hf("0x1.57e377b3f0b4bp+1"),
        hf("0x1.b6e2c73f41415p+0"),
        hf("0x1p-54"),
    ],
    [
        hf("0x1.6056b06a21918p+3"),
        hf("0x1.8c0a26d055288p+1"),
        hf("0x1p-53"),
    ],
    [
        hf("0x1.843e1b5e5979cp+4"),
        hf("0x1.f0f978201eb84p+1"),
        hf("0x1p-53"),
    ],
    [
        hf("0x1.fee8f69c4cd25p+10"),
        hf("0x1.0a19aebb51e9p+3"),
        hf("-0x1p-51"),
    ],
    [
        hf("0x1.0fbc6c02b1c9p+24"),
        hf("0x1.16369cd53bb69p+4"),
        hf("0x1p-50"),
    ],
];

#[inline(never)]
fn as_asinh_database(x: f64, mut f: f64) -> f64 {
    let ax = x.abs();
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    while a <= b {
        let row = &DB[m as usize];
        if row[0] < ax {
            a = m + 1;
        } else if row[0] == ax {
            let sgn = 1.0f64.copysign(x);
            f = sgn * row[1] + sgn * row[2];
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) / 2;
    }
    f
}

/// For 0 <= i <= 16, a 26-bit approximation of 2^(-i/2^4).
pub(crate) static T1: [f64; 17] = [
    hf("0x1p+0"),
    hf("0x1.ea4afap-1"),
    hf("0x1.d5818ep-1"),
    hf("0x1.c199bep-1"),
    hf("0x1.ae89f98p-1"),
    hf("0x1.9c4918p-1"),
    hf("0x1.8ace54p-1"),
    hf("0x1.7a1147p-1"),
    hf("0x1.6a09e68p-1"),
    hf("0x1.5ab07ep-1"),
    hf("0x1.4bfdad8p-1"),
    hf("0x1.3dea65p-1"),
    hf("0x1.306fe08p-1"),
    hf("0x1.2387a7p-1"),
    hf("0x1.172b84p-1"),
    hf("0x1.0b5587p-1"),
    hf("0x1p-1"),
];
/// For 0 <= i < 16, a 26-bit approximation of 2^(-i/2^8).
pub(crate) static T2: [f64; 16] = [
    hf("0x1p+0"),
    hf("0x1.fe9d968p-1"),
    hf("0x1.fd3c228p-1"),
    hf("0x1.fbdba38p-1"),
    hf("0x1.fa7c18p-1"),
    hf("0x1.f91d8p-1"),
    hf("0x1.f7bfdbp-1"),
    hf("0x1.f663278p-1"),
    hf("0x1.f507658p-1"),
    hf("0x1.f3ac948p-1"),
    hf("0x1.f252b38p-1"),
    hf("0x1.f0f9c2p-1"),
    hf("0x1.efa1bfp-1"),
    hf("0x1.ee4aaap-1"),
    hf("0x1.ecf483p-1"),
    hf("0x1.eb9f488p-1"),
];
/// For 0 <= i < 16, a 26-bit approximation of 2^(-i/2^12).
pub(crate) static T3: [f64; 16] = [
    hf("0x1p+0"),
    hf("0x1.ffe9d2p-1"),
    hf("0x1.ffd3a58p-1"),
    hf("0x1.ffbd798p-1"),
    hf("0x1.ffa74e8p-1"),
    hf("0x1.ff91248p-1"),
    hf("0x1.ff7afb8p-1"),
    hf("0x1.ff64d38p-1"),
    hf("0x1.ff4eac8p-1"),
    hf("0x1.ff38868p-1"),
    hf("0x1.ff22618p-1"),
    hf("0x1.ff0c3dp-1"),
    hf("0x1.fef61ap-1"),
    hf("0x1.fedff78p-1"),
    hf("0x1.fec9d68p-1"),
    hf("0x1.feb3b6p-1"),
];
/// For 0 <= i < 16, a 26-bit approximation of 2^(-i/2^16).
pub(crate) static T4: [f64; 16] = [
    hf("0x1p+0"),
    hf("0x1.fffe9dp-1"),
    hf("0x1.fffd3ap-1"),
    hf("0x1.fffbd78p-1"),
    hf("0x1.fffa748p-1"),
    hf("0x1.fff9118p-1"),
    hf("0x1.fff7ae8p-1"),
    hf("0x1.fff64cp-1"),
    hf("0x1.fff4e9p-1"),
    hf("0x1.fff386p-1"),
    hf("0x1.fff2238p-1"),
    hf("0x1.fff0c08p-1"),
    hf("0x1.ffef5d8p-1"),
    hf("0x1.ffedfa8p-1"),
    hf("0x1.ffec98p-1"),
    hf("0x1.ffeb35p-1"),
];
/// Triple-double approximations (h, m, l) of -log(t1\[i\])/2, -log(t2\[i\])/2, -log(t3\[i\])/2 and
/// -log(t4\[i\])/2 (`LL[1..3][16]` are not used).
pub(crate) static LL: [[[f64; 3]; 17]; 4] = [
    [
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("0x1.62e432b24p-6"),
            hf("-0x1.745af34bb54b8p-42"),
            hf("-0x1.17e3ec05cde7p-97"),
        ],
        [
            hf("0x1.62e42e4a8p-5"),
            hf("0x1.111a4eadf312p-44"),
            hf("0x1.cff3027abb119p-93"),
        ],
        [
            hf("0x1.0a2b233f1p-4"),
            hf("-0x1.88ac4ec78af8p-42"),
            hf("0x1.4fa087ca75dfdp-93"),
        ],
        [
            hf("0x1.62e43056cp-4"),
            hf("0x1.6bd65e8b0b7p-46"),
            hf("-0x1.b18e160362c24p-95"),
        ],
        [
            hf("0x1.bb9d3cbd6p-4"),
            hf("0x1.de14aa55ec2bp-42"),
            hf("-0x1.c6ac3f1862a6bp-94"),
        ],
        [
            hf("0x1.0a2b244dap-3"),
            hf("0x1.94def487fea7p-42"),
            hf("-0x1.dead1a4581acfp-94"),
        ],
        [
            hf("0x1.3687aa9b78p-3"),
            hf("0x1.9cec9a50db22p-43"),
            hf("0x1.34a70684f8e0ep-93"),
        ],
        [
            hf("0x1.62e42fabap-3"),
            hf("-0x1.d69047a3aebp-44"),
            hf("-0x1.4e061f79144e2p-95"),
        ],
        [
            hf("0x1.8f40b56d28p-3"),
            hf("0x1.de7d755fd2e2p-42"),
            hf("0x1.bdc7ecf001489p-94"),
        ],
        [
            hf("0x1.bb9d3b61fp-3"),
            hf("0x1.c14f1445b12p-46"),
            hf("0x1.a1d78cbdc5b58p-93"),
        ],
        [
            hf("0x1.e7f9c11f08p-3"),
            hf("-0x1.6e3e0000dae7p-43"),
            hf("0x1.6a4559fadde98p-94"),
        ],
        [
            hf("0x1.0a2b242ec4p-2"),
            hf("0x1.bb7cf852a5fe8p-42"),
            hf("0x1.a6aef11ee43bdp-93"),
        ],
        [
            hf("0x1.205966c764p-2"),
            hf("0x1.ad3a5f214294p-45"),
            hf("0x1.5cc344fa10652p-93"),
        ],
        [
            hf("0x1.3687a98aacp-2"),
            hf("0x1.1623671842fp-45"),
            hf("-0x1.0b428fe1f9e43p-94"),
        ],
        [
            hf("0x1.4cb5ec93f4p-2"),
            hf("0x1.3d50980ea513p-42"),
            hf("0x1.67f0ea083b1c4p-93"),
        ],
        [
            hf("0x1.62e42fefa4p-2"),
            hf("-0x1.8432a1b0e264p-44"),
            hf("0x1.803f2f6af40f3p-93"),
        ],
    ],
    [
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("0x1.62e462b4p-10"),
            hf("0x1.061d003b97318p-42"),
            hf("0x1.d7faee66a2e1ep-93"),
        ],
        [
            hf("0x1.62e44c92p-9"),
            hf("0x1.95a7bff5e239p-42"),
            hf("-0x1.f7e788a87135p-95"),
        ],
        [
            hf("0x1.0a2b1e33p-8"),
            hf("0x1.2a3a1a65aa3ap-43"),
            hf("-0x1.54599c9605442p-93"),
        ],
        [
            hf("0x1.62e4367cp-8"),
            hf("-0x1.4a995b6d9ddcp-45"),
            hf("-0x1.56bb79b254f33p-100"),
        ],
        [
            hf("0x1.bb9d449ap-8"),
            hf("0x1.8a119c42e9bcp-42"),
            hf("-0x1.8ecf7d8d661f1p-93"),
        ],
        [
            hf("0x1.0a2b1f19p-7"),
            hf("0x1.8863771bd10a8p-42"),
            hf("0x1.e9731de7f0155p-94"),
        ],
        [
            hf("0x1.3687ad11p-7"),
            hf("0x1.e026a347ca1c8p-42"),
            hf("0x1.fadc62522444dp-97"),
        ],
        [
            hf("0x1.62e436f28p-7"),
            hf("0x1.25b84f71b70b8p-42"),
            hf("-0x1.fcb3f98612d27p-96"),
        ],
        [
            hf("0x1.8f40b7b38p-7"),
            hf("-0x1.62a0a4fd4758p-43"),
            hf("0x1.3cb3c35d9f6a1p-93"),
        ],
        [
            hf("0x1.bb9d3abbp-7"),
            hf("-0x1.0ec48f94d786p-42"),
            hf("-0x1.6b47d410e4cc7p-93"),
        ],
        [
            hf("0x1.e7f9bb23p-7"),
            hf("0x1.e4415cbc97ap-43"),
            hf("-0x1.3729fdb677231p-93"),
        ],
        [
            hf("0x1.0a2b22478p-6"),
            hf("-0x1.cb73f4505b03p-42"),
            hf("-0x1.1b3b3a3bc370ap-93"),
        ],
        [
            hf("0x1.2059691e8p-6"),
            hf("-0x1.abcc3412f264p-43"),
            hf("-0x1.fe6e998e48673p-95"),
        ],
        [
            hf("0x1.3687a768p-6"),
            hf("-0x1.43901e5c97a9p-42"),
            hf("0x1.b54cdd52a5d88p-96"),
        ],
        [
            hf("0x1.4cb5eb5d8p-6"),
            hf("-0x1.8f106f00f13b8p-42"),
            hf("-0x1.8f793f5fce148p-93"),
        ],
        [
            hf("0x1.62e432b24p-6"),
            hf("-0x1.745af34bb54b8p-42"),
            hf("-0x1.17e3ec05cde7p-97"),
        ],
    ],
    [
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("0x1.62e7bp-14"),
            hf("-0x1.868625640a68p-44"),
            hf("-0x1.34bf0db910f65p-93"),
        ],
        [
            hf("0x1.62e35f6p-13"),
            hf("-0x1.2ee3d96b696ap-43"),
            hf("0x1.a2948cd558655p-94"),
        ],
        [
            hf("0x1.0a2b4b2p-12"),
            hf("0x1.53edbcf1165p-47"),
            hf("-0x1.cfc26ccf6d0e4p-97"),
        ],
        [
            hf("0x1.62e4be1p-12"),
            hf("0x1.783e334614p-52"),
            hf("-0x1.04b96da30e63ap-93"),
        ],
        [
            hf("0x1.bb9e085p-12"),
            hf("-0x1.60785f20acb2p-43"),
            hf("-0x1.f33369bf7dff1p-96"),
        ],
        [
            hf("0x1.0a2b94dp-11"),
            hf("0x1.fd4b3a273353p-42"),
            hf("-0x1.685a35575eff1p-96"),
        ],
        [
            hf("0x1.368810f8p-11"),
            hf("0x1.7ded26dc813p-47"),
            hf("-0x1.4c4d1abca79bfp-96"),
        ],
        [
            hf("0x1.62e47878p-11"),
            hf("0x1.7d2bee9a1f63p-42"),
            hf("0x1.860233b7ad13p-93"),
        ],
        [
            hf("0x1.8f40cb48p-11"),
            hf("-0x1.af034eaf471cp-42"),
            hf("0x1.ae748822d57b7p-94"),
        ],
        [
            hf("0x1.bb9d094p-11"),
            hf("-0x1.7a223013a20fp-42"),
            hf("-0x1.1e499087075b6p-93"),
        ],
        [
            hf("0x1.e7fa32c8p-11"),
            hf("-0x1.b2e67b1b59bdp-43"),
            hf("-0x1.54a41eda30fa6p-93"),
        ],
        [
            hf("0x1.0a2b237p-10"),
            hf("-0x1.7ad97ff4ac7ap-44"),
            hf("0x1.f932da91371ddp-93"),
        ],
        [
            hf("0x1.2059a338p-10"),
            hf("-0x1.96422d90df4p-44"),
            hf("-0x1.90800fbbf2ed3p-94"),
        ],
        [
            hf("0x1.36879824p-10"),
            hf("0x1.0f9054001812p-44"),
            hf("0x1.9567e01e48f9ap-93"),
        ],
        [
            hf("0x1.4cb602cp-10"),
            hf("-0x1.0d709a5ec0b5p-43"),
            hf("0x1.253dfd44635d2p-94"),
        ],
        [
            hf("0x1.62e462b4p-10"),
            hf("0x1.061d003b97318p-42"),
            hf("0x1.d7faee66a2e1ep-93"),
        ],
    ],
    [
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("0x1.63007cp-18"),
            hf("-0x1.db0e38e5aaaap-43"),
            hf("0x1.259a7b94815b9p-93"),
        ],
        [
            hf("0x1.6300f6p-17"),
            hf("0x1.2b1c75580438p-44"),
            hf("0x1.78cabba01e3e4p-93"),
        ],
        [
            hf("0x1.0a2115p-16"),
            hf("-0x1.5ff223730759p-42"),
            hf("0x1.8074feacfe49dp-95"),
        ],
        [
            hf("0x1.62e1ecp-16"),
            hf("-0x1.85d6f6487ce4p-45"),
            hf("0x1.05485074b9276p-93"),
        ],
        [
            hf("0x1.bba301p-16"),
            hf("-0x1.af5d58a7c921p-43"),
            hf("-0x1.30a8c0fd2ff5fp-93"),
        ],
        [
            hf("0x1.0a32298p-15"),
            hf("0x1.590faa0883bdp-43"),
            hf("0x1.95e9bda999947p-93"),
        ],
        [
            hf("0x1.3682f1p-15"),
            hf("0x1.f0224376efaf8p-42"),
            hf("-0x1.5843c0db50d1p-93"),
        ],
        [
            hf("0x1.62e3d8p-15"),
            hf("-0x1.142c13daed4ap-43"),
            hf("0x1.c68a61183ce87p-93"),
        ],
        [
            hf("0x1.8f44dd8p-15"),
            hf("-0x1.aa489f399931p-43"),
            hf("0x1.11c5c376854eap-94"),
        ],
        [
            hf("0x1.bb9601p-15"),
            hf("0x1.9904d8b6a3638p-42"),
            hf("0x1.8c89554493c8fp-93"),
        ],
        [
            hf("0x1.e7f744p-15"),
            hf("0x1.5785ddbe7cba8p-42"),
            hf("0x1.e7ff3cde7d70cp-94"),
        ],
        [
            hf("0x1.0a2c53p-14"),
            hf("-0x1.6d9e8780d0d5p-43"),
            hf("0x1.ad9c178106693p-94"),
        ],
        [
            hf("0x1.205d134p-14"),
            hf("-0x1.214a2e893fccp-43"),
            hf("0x1.548a9500c9822p-93"),
        ],
        [
            hf("0x1.3685e28p-14"),
            hf("0x1.e23588646103p-43"),
            hf("0x1.2a97b26da2d88p-94"),
        ],
        [
            hf("0x1.4cb6c18p-14"),
            hf("0x1.2b7cfcea9e0d8p-42"),
            hf("-0x1.5095048a6b824p-93"),
        ],
        [
            hf("0x1.62e7bp-14"),
            hf("-0x1.868625640a68p-44"),
            hf("-0x1.34bf0db910f65p-93"),
        ],
    ],
];
/// The refinement polynomial (double-double part).
pub(crate) static CH_REFINE: [[f64; 2]; 3] = [
    [hf("0x1p-1"), hf("0x1.24b67ee516e3bp-111")],
    [hf("-0x1p-2"), hf("-0x1.932ce43199a8dp-110")],
    [hf("0x1.5555555555555p-3"), hf("0x1.55540c15cf91fp-57")],
];
/// The refinement polynomial (double part).
pub(crate) const CL_REFINE: [f64; 3] = [
    hf("-0x1p-3"),
    hf("0x1.9999999a0754fp-4"),
    hf("-0x1.55555555c3157p-4"),
];

#[inline(always)]
fn as_asinh_refine_with<F: FmaOps>(x: f64, zh: f64, zl: f64, a: f64) -> f64 {
    let mut t = zh.to_bits();
    let ex = (t >> 52) as i32;
    let e = ex - 0x3ff + (zl == 0.0) as i32;
    t &= !0u64 >> 12;
    t |= 0x3ffu64 << 52;
    let ed = e as f64;
    let v = ((a - ed) + const { hf("0x1.00008p+0") }).to_bits();
    let i = v.wrapping_sub(0x3ffu64 << 52) >> (52 - 16);
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
    let (v0, v1) = fasttwosum(v0, v1);
    let (v1, v2) = fasttwosum(v1, v2);
    let s2 = 2.0f64.copysign(x);
    let v0 = v0 * s2;
    let mut v1 = v1 * s2;
    let v2 = v2 * s2;
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
    let er = t.wrapping_add(41) & (!0u64 >> 12);
    let de = ((t0 >> 52) & 0x7ff).wrapping_sub((t >> 52) & 0x7ff);
    let res = v0 + v1;
    if de > 99 || er < 80 {
        return as_asinh_database(x, res);
    }
    res
}

multiversion! {
    fn as_asinh_refine(x: f64, zh: f64, zl: f64, a: f64) -> f64 = as_asinh_refine_with;
}

multiversion! {
    /// The inverse hyperbolic sine, correctly rounded (round to nearest, ties to even):
    /// CORE-MATH `cr_asinh`. The same bits on every target. `Math.asinh` (§21.3.2.5): NaN, ±0
    /// and ±∞ give themselves.
    pub fn asinh(x: f64) -> f64 = asinh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asinh_special_values() {
        assert!(asinh(f64::NAN).is_nan());
        assert_eq!(asinh(0.0).to_bits(), 0);
        assert_eq!(asinh(-0.0).to_bits(), 1 << 63);
        assert_eq!(asinh(f64::INFINITY), f64::INFINITY);
        assert_eq!(asinh(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(asinh(f64::MAX), hf("0x1.633ce8fb9f87ep+9")); // MPFR
    }
}
