//! `Math.sinh`: the correctly rounded hyperbolic sine (H36), a port of CORE-MATH `cr_sinh`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/sinh/sinh.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded hyperbolic sine for binary64 values.
//! Copyright (c) 2023-2026 Alexei Sibidanov and Paul Zimmermann.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! The correctness of this code is proven in "The CORE-MATH sinh is correctly rounded",
//! Guillaume Melquiond and Paul Zimmermann, <https://core-math.gitlabpages.inria.fr/sinh.pdf>
//! (March 2026).
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. ep9 changes: rounding to
//! nearest only (JS); no `errno`; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact
//! software FMA, the same bits); no FMA contraction; the x86-64 SSE mask of `v0` is the
//! integer mask of the C non-x86 branch (the same bits). The tables `t0` and `t1`, the
//! double-double helpers and `as_exp_accurate` are the same in cosh.c, so `crate::cosh` uses
//! these copies.

use crate::exact::FmaOps;
use crate::hf;

#[inline(always)]
pub(crate) fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

#[inline(always)]
pub(crate) fn muldd<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let h = xh * ch;
    (h, (F::fma(xh, ch, -h) + xh * cl) + xl * ch)
}

#[inline(always)]
fn mulddd<F: FmaOps>(xh: f64, xl: f64, ch: f64) -> (f64, f64) {
    let h = xh * ch;
    (h, F::fma(xh, ch, -h) + xl * ch)
}

#[inline(always)]
pub(crate) fn polydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, mut cl) = fasttwosum(c[i][0], l);
    cl += c[i][1];
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd::<F>(xh, xl, ch, cl);
        let e;
        (ch, e) = fasttwosum(c[i][0], ch);
        cl = (cl + c[i][1]) + e;
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
    let (fh, fl) = muldd::<F>(dxh, dxl, fh, fl);
    let (fh, fl) = muldd::<F>(th, tl, fh, fl);
    let zh = th + fh;
    let zl = (th - zh) + fh;
    let uh = zh + tl;
    let ul = ((zh - uh) + tl) + zl;
    let vh = uh + fl;
    let vl = ((uh - vh) + fl) + ul;
    (vh, vl)
}

multiversion! {
    /// The accurate exp(x) = (th + tl) * exp(x - t*log(2)/2^12) of sinh.c and cosh.c.
    pub(crate) fn as_exp_accurate(x: f64, t: f64, th: f64, tl: f64) -> (f64, f64) = as_exp_accurate_with;
}

#[inline(always)]
fn as_sinh_zero_with<F: FmaOps>(x: f64) -> f64 {
    const CH: [[f64; 2]; 5] = [
        [hf("0x1.5555555555555p-3"), hf("0x1.555555555552fp-57")],
        [hf("0x1.1111111111111p-7"), hf("0x1.11111115cf00dp-63")],
        [hf("0x1.a01a01a01a01ap-13"), hf("0x1.a0011c925b85cp-73")],
        [hf("0x1.71de3a556c734p-19"), hf("-0x1.b4e2835532bcdp-73")],
        [hf("0x1.ae64567f54482p-26"), hf("-0x1.defcf17a6ab79p-81")],
    ];
    let x2 = x * x;
    let x2l = F::fma(x, x, -x2);
    let y2 = x2
        * (const { hf("0x1.6124613aef206p-33") }
            + x2 * (const { hf("0x1.ae7f36beea815p-41") }
                + x2 * const { hf("0x1.95785063cd974p-49") }));
    let (y1, y2) = polydd::<F>(x2, x2l, &CH, y2);
    let (y1, y2) = mulddd::<F>(y1, y2, x);
    let (y1, y2) = muldd::<F>(y1, y2, x2, x2l);
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
    fn as_sinh_zero(x: f64) -> f64 = as_sinh_zero_with;
}

/// The exceptional cases: |x|, then the result as a sum of two doubles.
static DB: &[[f64; 3]] = &[
    [
        hf("0x1.364303e1ad8f6p-2"),
        hf("0x1.3b07e0c779ddap-2"),
        hf("-0x1.bcp-106"),
    ],
    [
        hf("0x1.4169f234f23b9p-2"),
        hf("0x1.46b7b3b358f99p-2"),
        hf("-0x1p-56"),
    ],
    [
        hf("0x1.616cc75d49226p-2"),
        hf("0x1.687bd068c1c1ep-2"),
        hf("0x1.ap-111"),
    ],
    [
        hf("0x1.ae3773250e7d2p-2"),
        hf("0x1.bafc3479fc9ccp-2"),
        hf("-0x1p-105"),
    ],
    [
        hf("0x1.b7efa91915c95p-2"),
        hf("0x1.c59869f17b483p-2"),
        hf("-0x1p-104"),
    ],
    [
        hf("0x1.d68039861ab53p-2"),
        hf("0x1.e73b46abb01e1p-2"),
        hf("-0x1.2p-109"),
    ],
    [
        hf("0x1.e90f16eb88c09p-2"),
        hf("0x1.fbdd4a37760b7p-2"),
        hf("-0x1.f8p-108"),
    ],
    [
        hf("0x1.a3fc7e4dd47d1p-1"),
        hf("0x1.d4b21ebf542fp-1"),
        hf("0x1.ep-107"),
    ],
    [
        hf("0x1.aa3b649a96091p-1"),
        hf("0x1.dd32c5ed1e93p-1"),
        hf("0x1.8ap-106"),
    ],
    [
        hf("0x1.c13876341b62ep-1"),
        hf("0x1.fd1d7f1c8170cp-1"),
        hf("0x1.72p-105"),
    ],
    [
        hf("0x1.2f5d3b178914ap+0"),
        hf("0x1.7b8516ffd2406p+0"),
        hf("-0x1.28p-104"),
    ],
    [
        hf("0x1.3ffc12b81cbc2p+0"),
        hf("0x1.9a0ff413a1af3p+0"),
        hf("0x1.cp-107"),
    ],
    [
        hf("0x1.44f65dff00782p+0"),
        hf("0x1.a38a3c3227609p+0"),
        hf("-0x1p-103"),
    ],
    [
        hf("0x1.7346e3c591a14p+0"),
        hf("0x1.01e9cfa77b855p+1"),
        hf("0x1.p-102"),
    ],
    [
        hf("0x1.b6e2c73f41415p+0"),
        hf("0x1.57e377b3f0b4bp+1"),
        hf("-0x1p-102"),
    ],
    [
        hf("0x1.dc5059d4e507dp+0"),
        hf("0x1.9168c60ed5256p+1"),
        hf("0x1.c6p-104"),
    ],
    [
        hf("0x1.f737f1e8378c7p+0"),
        hf("0x1.bffd3f94f40fbp+1"),
        hf("0x1.a4p-104"),
    ],
    [
        hf("0x1.3359640329982p+1"),
        hf("0x1.5e40df3f985bep+2"),
        hf("0x1.97p-102"),
    ],
    [
        hf("0x1.58a4ff5adac35p+1"),
        hf("0x1.d671928665bddp+2"),
        hf("0x1p-102"),
    ],
    [
        hf("0x1.8c0a26d055288p+1"),
        hf("0x1.6056b06a21918p+3"),
        hf("-0x1.bep-102"),
    ],
    [
        hf("0x1.bc3c2d0c95f52p+1"),
        hf("0x1.00fef7383a978p+4"),
        hf("0x1.61p-100"),
    ],
    [
        hf("0x1.0a19aebb51e9p+3"),
        hf("0x1.fee8f69c4cd25p+10"),
        hf("0x1.48p-95"),
    ],
    [
        hf("0x1.3eb8f61734227p+3"),
        hf("0x1.4ab1cf45e4e26p+13"),
        hf("0x1p-90"),
    ],
    [
        hf("0x1.43a81752eabe7p+3"),
        hf("0x1.81d364845ecfap+13"),
        hf("-0x1p-90"),
    ],
    [
        hf("0x1.16369cd53bb69p+4"),
        hf("0x1.0fbc6c02b1c9p+24"),
        hf("-0x1.9p-81"),
    ],
    [
        hf("0x1.20e29ea8b51e2p+4"),
        hf("0x1.08b8abba28abcp+25"),
        hf("0x1.9bp-79"),
    ],
    [
        hf("0x1.92a5c27afbe82p+4"),
        hf("0x1.3c81f9a247253p+35"),
        hf("0x1p-67"),
    ],
    [
        hf("0x1.a1e4f11b513d7p+4"),
        hf("0x1.9a65b6c2e2185p+36"),
        hf("-0x1.bcp-70"),
    ],
    [
        hf("0x1.c089fcf166171p+4"),
        hf("0x1.5c452e0e37569p+39"),
        hf("0x1.4p-69"),
    ],
    [
        hf("0x1.e42a98b3a0be5p+4"),
        hf("0x1.938768ca4f8aap+42"),
        hf("0x1.6dp-62"),
    ],
    [
        hf("0x1.04db52248cbb8p+5"),
        hf("0x1.0794072349523p+46"),
        hf("0x1.0e8p-57"),
    ],
    [
        hf("0x1.21bc021eeb97ep+5"),
        hf("0x1.3065064a170fbp+51"),
        hf("0x1.088p-52"),
    ],
    [
        hf("0x1.39fc4d3bb711p+5"),
        hf("0x1.8a4e90733b95ep+55"),
        hf("0x1.6ep-50"),
    ],
    [
        hf("0x1.3c895d86e96c9p+5"),
        hf("0x1.0f33837882a6p+56"),
        hf("-0x1.28p-49"),
    ],
    [
        hf("0x1.e07e71bfcf06fp+5"),
        hf("0x1.91ec4412c344fp+85"),
        hf("0x1p-24"),
    ],
    [
        hf("0x1.f7216c4b435c9p+5"),
        hf("0x1.a97e7be23e65ap+89"),
        hf("-0x1p-15"),
    ],
    [
        hf("0x1.6474c604cc0d7p+6"),
        hf("0x1.7a8f65ad009bdp+127"),
        hf("-0x1.08p+20"),
    ],
    [
        hf("0x1.7a60ee15e3e9dp+6"),
        hf("0x1.62e4dc3bbf53fp+135"),
        hf("0x1.bp+29"),
    ],
    [
        hf("0x1.1f0da93354198p+7"),
        hf("0x1.0bd73b73fc74cp+206"),
        hf("0x1.59p+102"),
    ],
    [
        hf("0x1.54cd1fea7663ap+7"),
        hf("0x1.c90810d354618p+244"),
        hf("0x1.2p+135"),
    ],
    [
        hf("0x1.556c678d5e976p+7"),
        hf("0x1.37e7ac4e7f9b3p+245"),
        hf("0x1.02p+141"),
    ],
    [
        hf("0x1.7945e34b18a9ap+7"),
        hf("0x1.1b0e4936a8c9bp+271"),
        hf("-0x1.fap+166"),
    ],
    [
        hf("0x1.2da9e5e6af0bp+8"),
        hf("0x1.27d6fe867d6f6p+434"),
        hf("0x1.0ap+329"),
    ],
    [
        hf("0x1.54ceba01331d5p+8"),
        hf("0x1.9a86785b5ef3ep+490"),
        hf("-0x1.22p+386"),
    ],
    [
        hf("0x1.9e7b643238a14p+8"),
        hf("0x1.f5da7fe652978p+596"),
        hf("0x1p+493"),
    ],
    [
        hf("0x1.c7206c1b753e4p+8"),
        hf("0x1.8670de0b68cadp+655"),
        hf("-0x1.78p+548"),
    ],
    [
        hf("0x1.d6479eba7c971p+8"),
        hf("0x1.62a88613629b6p+677"),
        hf("-0x1.4p+568"),
    ],
    [
        hf("0x1.eb9914d4ac1c8p+8"),
        hf("0x1.2b67eff65dce8p+708"),
        hf("-0x1.02p+603"),
    ],
    [
        hf("0x1.0bc04af1b09f5p+9"),
        hf("0x1.7b1d97c902985p+771"),
        hf("0x1.56p+666"),
    ],
    [
        hf("0x1.26ee1a46d8c8bp+9"),
        hf("0x1.fbe20477df4a7p+849"),
        hf("-0x1.55p+745"),
    ],
    [
        hf("0x1.4a869881f72acp+9"),
        hf("0x1.9ea7540a3d1f9p+952"),
        hf("-0x1.2dp+848"),
    ],
];

#[inline(never)]
fn as_sinh_database(x: f64, mut f: f64) -> f64 {
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

/// 2^(i/64) for 0 <= i < 64 as (low, high) double pairs.
pub(crate) static T0: [[f64; 2]; 64] = [
    [hf("0x0p+0"), hf("0x1p+0")],
    [hf("-0x1.19083535b085ep-56"), hf("0x1.02c9a3e778061p+0")],
    [hf("0x1.d73e2a475b466p-55"), hf("0x1.059b0d3158574p+0")],
    [hf("0x1.186be4bb285p-57"), hf("0x1.0874518759bc8p+0")],
    [hf("0x1.8a62e4adc610ap-54"), hf("0x1.0b5586cf9890fp+0")],
    [hf("0x1.03a1727c57b52p-59"), hf("0x1.0e3ec32d3d1a2p+0")],
    [hf("-0x1.6c51039449b3ap-54"), hf("0x1.11301d0125b51p+0")],
    [hf("-0x1.32fbf9af1369ep-54"), hf("0x1.1429aaea92dep+0")],
    [hf("-0x1.19041b9d78a76p-55"), hf("0x1.172b83c7d517bp+0")],
    [hf("0x1.e5b4c7b4968e4p-55"), hf("0x1.1a35beb6fcb75p+0")],
    [hf("0x1.e016e00a2643cp-54"), hf("0x1.1d4873168b9aap+0")],
    [hf("0x1.dc775814a8494p-55"), hf("0x1.2063b88628cd6p+0")],
    [hf("0x1.9b07eb6c70572p-54"), hf("0x1.2387a6e756238p+0")],
    [hf("0x1.2bd339940e9dap-55"), hf("0x1.26b4565e27cddp+0")],
    [hf("0x1.612e8afad1256p-55"), hf("0x1.29e9df51fdee1p+0")],
    [hf("0x1.0024754db41d4p-54"), hf("0x1.2d285a6e4030bp+0")],
    [hf("0x1.6f46ad23182e4p-55"), hf("0x1.306fe0a31b715p+0")],
    [hf("0x1.32721843659a6p-54"), hf("0x1.33c08b26416ffp+0")],
    [hf("-0x1.63aeabf42eae2p-54"), hf("0x1.371a7373aa9cbp+0")],
    [hf("-0x1.5e436d661f5e2p-56"), hf("0x1.3a7db34e59ff7p+0")],
    [hf("0x1.ada0911f09ebcp-55"), hf("0x1.3dea64c123422p+0")],
    [hf("-0x1.ef3691c309278p-58"), hf("0x1.4160a21f72e2ap+0")],
    [hf("0x1.89b7a04ef80dp-59"), hf("0x1.44e086061892dp+0")],
    [hf("0x1.3c1a3b69062fp-56"), hf("0x1.486a2b5c13cdp+0")],
    [hf("0x1.d4397afec42e2p-56"), hf("0x1.4bfdad5362a27p+0")],
    [hf("-0x1.4b309d25957e4p-54"), hf("0x1.4f9b2769d2ca7p+0")],
    [hf("-0x1.07abe1db13cacp-55"), hf("0x1.5342b569d4f82p+0")],
    [hf("0x1.9bb2c011d93acp-54"), hf("0x1.56f4736b527dap+0")],
    [hf("0x1.6324c054647acp-54"), hf("0x1.5ab07dd485429p+0")],
    [hf("0x1.ba6f93080e65ep-54"), hf("0x1.5e76f15ad2148p+0")],
    [hf("-0x1.383c17e40b496p-54"), hf("0x1.6247eb03a5585p+0")],
    [hf("-0x1.bb60987591c34p-54"), hf("0x1.6623882552225p+0")],
    [hf("-0x1.bdd3413b26456p-54"), hf("0x1.6a09e667f3bcdp+0")],
    [hf("-0x1.bbe3a683c88aap-57"), hf("0x1.6dfb23c651a2fp+0")],
    [hf("-0x1.16e4786887a9ap-55"), hf("0x1.71f75e8ec5f74p+0")],
    [hf("-0x1.0245957316dd4p-54"), hf("0x1.75feb564267c9p+0")],
    [hf("-0x1.41577ee04993p-55"), hf("0x1.7a11473eb0187p+0")],
    [hf("0x1.05d02ba15797ep-56"), hf("0x1.7e2f336cf4e62p+0")],
    [hf("-0x1.d4c1dd41532d8p-54"), hf("0x1.82589994cce13p+0")],
    [hf("-0x1.fc6f89bd4f6bap-54"), hf("0x1.868d99b4492edp+0")],
    [hf("0x1.6e9f156864b26p-54"), hf("0x1.8ace5422aa0dbp+0")],
    [hf("0x1.5cc13a2e3976cp-55"), hf("0x1.8f1ae99157736p+0")],
    [hf("-0x1.75fc781b57ebcp-57"), hf("0x1.93737b0cdc5e5p+0")],
    [hf("-0x1.d185b7c1b85dp-54"), hf("0x1.97d829fde4e5p+0")],
    [hf("0x1.c7c46b071f2bep-56"), hf("0x1.9c49182a3f09p+0")],
    [hf("-0x1.359495d1cd532p-54"), hf("0x1.a0c667b5de565p+0")],
    [hf("-0x1.d2f6edb8d41e2p-54"), hf("0x1.a5503b23e255dp+0")],
    [hf("0x1.0fac90ef7fd32p-54"), hf("0x1.a9e6b5579fdbfp+0")],
    [hf("0x1.7a1cd345dcc82p-54"), hf("0x1.ae89f995ad3adp+0")],
    [hf("-0x1.2805e3084d708p-57"), hf("0x1.b33a2b84f15fbp+0")],
    [hf("-0x1.5584f7e54ac3ap-56"), hf("0x1.b7f76f2fb5e47p+0")],
    [hf("0x1.23dd07a2d9e84p-55"), hf("0x1.bcc1e904bc1d2p+0")],
    [hf("0x1.11065895048dep-55"), hf("0x1.c199bdd85529cp+0")],
    [hf("0x1.2884dff483cacp-54"), hf("0x1.c67f12e57d14bp+0")],
    [hf("0x1.503cbd1e949dcp-56"), hf("0x1.cb720dcef9069p+0")],
    [hf("-0x1.cbc3743797a9cp-54"), hf("0x1.d072d4a07897cp+0")],
    [hf("0x1.2ed02d75b3706p-55"), hf("0x1.d5818dcfba487p+0")],
    [hf("0x1.c2300696db532p-54"), hf("0x1.da9e603db3285p+0")],
    [hf("-0x1.1a5cd4f184b5cp-54"), hf("0x1.dfc97337b9b5fp+0")],
    [hf("0x1.39e8980a9cc9p-55"), hf("0x1.e502ee78b3ff6p+0")],
    [hf("-0x1.e9c23179c2894p-54"), hf("0x1.ea4afa2a490dap+0")],
    [hf("0x1.dc7f486a4b6bp-54"), hf("0x1.efa1bee615a27p+0")],
    [hf("0x1.9d3e12dd8a18ap-54"), hf("0x1.f50765b6e454p+0")],
    [hf("0x1.74853f3a5931ep-55"), hf("0x1.fa7c1819e90d8p+0")],
];

/// 2^(i/2^12) for 0 <= i < 64 as (low, high) double pairs.
pub(crate) static T1: [[f64; 2]; 64] = [
    [hf("0x0p+0"), hf("0x1p+0")],
    [hf("0x1.ae8e38c59c72ap-54"), hf("0x1.000b175effdc7p+0")],
    [hf("-0x1.7b5d0d58ea8f4p-58"), hf("0x1.00162f3904052p+0")],
    [hf("0x1.4115cb6b16a8ep-54"), hf("0x1.0021478e11ce6p+0")],
    [hf("-0x1.d7c96f201bb2ep-55"), hf("0x1.002c605e2e8cfp+0")],
    [hf("0x1.84711d4c35eap-54"), hf("0x1.003779a95f959p+0")],
    [hf("-0x1.0484245243778p-55"), hf("0x1.0042936faa3d8p+0")],
    [hf("-0x1.4b237da2025fap-54"), hf("0x1.004dadb113dap+0")],
    [hf("-0x1.5e00e62d6b30ep-56"), hf("0x1.0058c86da1c0ap+0")],
    [hf("0x1.a1d6cedbb948p-54"), hf("0x1.0063e3a559473p+0")],
    [hf("-0x1.4acf197a00142p-54"), hf("0x1.006eff583fc3dp+0")],
    [hf("-0x1.eaf2ea42391a6p-57"), hf("0x1.007a1b865a8cap+0")],
    [hf("0x1.da93f90835f76p-56"), hf("0x1.0085382faef83p+0")],
    [hf("-0x1.6a79084ab093cp-55"), hf("0x1.00905554425d4p+0")],
    [hf("0x1.86364f8fbe8f8p-54"), hf("0x1.009b72f41a12bp+0")],
    [hf("-0x1.82e8e14e3110ep-55"), hf("0x1.00a6910f3b6fdp+0")],
    [hf("-0x1.4f6b2a7609f72p-55"), hf("0x1.00b1afa5abcbfp+0")],
    [hf("-0x1.e1a258ea8f71ap-56"), hf("0x1.00bcceb7707ecp+0")],
    [hf("0x1.4362ca5bc26f2p-56"), hf("0x1.00c7ee448ee02p+0")],
    [hf("0x1.095a56c919d02p-54"), hf("0x1.00d30e4d0c483p+0")],
    [hf("-0x1.406ac4e81a646p-57"), hf("0x1.00de2ed0ee0f5p+0")],
    [hf("0x1.b5a6902767e08p-54"), hf("0x1.00e94fd0398ep+0")],
    [hf("-0x1.91b206085932p-54"), hf("0x1.00f4714af41d3p+0")],
    [hf("0x1.427068ab22306p-55"), hf("0x1.00ff93412315cp+0")],
    [hf("0x1.c1d0660524e08p-54"), hf("0x1.010ab5b2cbd11p+0")],
    [hf("-0x1.e7bdfb3204be8p-54"), hf("0x1.0115d89ff3a8bp+0")],
    [hf("0x1.843aa8b9cbbc6p-55"), hf("0x1.0120fc089ff63p+0")],
    [hf("-0x1.34104ee7edae8p-56"), hf("0x1.012c1fecd613bp+0")],
    [hf("-0x1.2b6aeb6176892p-56"), hf("0x1.0137444c9b5b5p+0")],
    [hf("0x1.a8cd33b8a1bb2p-56"), hf("0x1.01426927f5278p+0")],
    [hf("0x1.2edc08e5da99ap-56"), hf("0x1.014d8e7ee8d2fp+0")],
    [hf("0x1.57ba2dc7e0c72p-55"), hf("0x1.0158b4517bb88p+0")],
    [hf("0x1.b61299ab8cdb8p-54"), hf("0x1.0163da9fb3335p+0")],
    [hf("-0x1.90565902c5f44p-54"), hf("0x1.016f0169949edp+0")],
    [hf("0x1.70fc41c5c2d54p-55"), hf("0x1.017a28af25567p+0")],
    [hf("0x1.4b9a6e145d76cp-54"), hf("0x1.018550706ab62p+0")],
    [hf("-0x1.008eff5142bfap-56"), hf("0x1.019078ad6a19fp+0")],
    [hf("-0x1.77669f033c7dep-54"), hf("0x1.019ba16628de2p+0")],
    [hf("-0x1.09bb78eeead0ap-54"), hf("0x1.01a6ca9aac5f3p+0")],
    [hf("0x1.371231477ece6p-54"), hf("0x1.01b1f44af9f9ep+0")],
    [hf("0x1.5e7626621eb5ap-56"), hf("0x1.01bd1e77170b4p+0")],
    [hf("-0x1.bc72b100828a4p-54"), hf("0x1.01c8491f08f08p+0")],
    [hf("-0x1.ce39cbbab8bbep-57"), hf("0x1.01d37442d507p+0")],
    [hf("0x1.16996709da2e2p-55"), hf("0x1.01de9fe280ac8p+0")],
    [hf("-0x1.c11f5239bf536p-55"), hf("0x1.01e9cbfe113efp+0")],
    [hf("0x1.e1d4eb5edc6b4p-55"), hf("0x1.01f4f8958c1c6p+0")],
    [hf("-0x1.afb99946ee3fp-54"), hf("0x1.020025a8f6a35p+0")],
    [hf("-0x1.8f06d8a148a32p-54"), hf("0x1.020b533856324p+0")],
    [hf("-0x1.2bf310fc54eb6p-55"), hf("0x1.02168143b0281p+0")],
    [hf("-0x1.c95a035eb4176p-54"), hf("0x1.0221afcb09e3ep+0")],
    [hf("-0x1.491793e46834cp-54"), hf("0x1.022cdece68c4fp+0")],
    [hf("-0x1.3e8d0d9c4909p-56"), hf("0x1.02380e4dd22adp+0")],
    [hf("-0x1.314aa16278aa4p-54"), hf("0x1.02433e494b755p+0")],
    [hf("0x1.48daf888e965p-55"), hf("0x1.024e6ec0da046p+0")],
    [hf("0x1.56dc8046821f4p-55"), hf("0x1.02599fb483385p+0")],
    [hf("0x1.45b42356b9d46p-54"), hf("0x1.0264d1244c719p+0")],
    [hf("-0x1.082ef51b61d7ep-56"), hf("0x1.027003103b10ep+0")],
    [hf("0x1.2106ed0920a34p-56"), hf("0x1.027b357854772p+0")],
    [hf("-0x1.fd4cf26ea5d0ep-54"), hf("0x1.0286685c9e059p+0")],
    [hf("-0x1.09f8775e78084p-54"), hf("0x1.02919bbd1d1d8p+0")],
    [hf("0x1.64cbba902ca28p-58"), hf("0x1.029ccf99d720ap+0")],
    [hf("0x1.4383ef231d206p-54"), hf("0x1.02a803f2d170dp+0")],
    [hf("0x1.4a47a505b3a46p-54"), hf("0x1.02b338c811703p+0")],
    [hf("0x1.e47120223468p-54"), hf("0x1.02be6e199c811p+0")],
];

/// The integer part of round(|x|*2^12/log(2)) through the 0x1.8000002p+26 shift (`v0`), and
/// that integer as a double (`t`), as in sinh.c and cosh.c.
#[inline(always)]
pub(crate) fn reduce<F: FmaOps>(ax: f64) -> (u64, f64) {
    const S: f64 = hf("0x1.71547652b82fep+12");
    // 0x1.8000002p+26 = 0x1.8p26 + 0.5: round the integer part of ax*s to nearest
    let v0 = F::fma(ax, S, const { hf("0x1.8000002p+26") });
    let jt = v0.to_bits();
    let v = jt & !((1u64 << 26) - 1);
    (jt, f64::from_bits(v) - const { hf("0x1.8p26") })
}

/// The C `ch[]` of the main path (sinh.c and cosh.c).
pub(crate) const CH4: [f64; 4] = [
    hf("0x1p+0"),
    hf("0x1p-1"),
    hf("0x1.5555555aaaaaep-3"),
    hf("0x1.55555551c98cp-5"),
];

#[inline(always)]
fn sinh_with<F: FmaOps>(x: f64) -> f64 {
    // sinh(x) is approximated by a minimax polynomial for |x| < 0.25. For other arguments,
    // sinh(x) = (exp(|x|)-exp(-|x|))/2*copysign(1,x): for |x| < 5 both exponentials are
    // computed with slightly more than double precision; for 5 < |x| < 36.736801 exp(-|x|)
    // is small and computed in double precision; for 36.736801 < |x| < 710.47586 only
    // exp(|x|) is computed, with an accuracy of 106 bits.
    let ax = x.abs();
    let (jt, t) = reduce::<F>(ax);
    let aix = ax.to_bits();
    if aix < 0x3fd0000000000000 {
        // |x| < 0x1p-2
        if aix < 0x3e57137449123ef7 {
            // |x| < x0 = 0x1.7137449123ef7p-26
            return F::fma(x, const { hf("0x1p-55") }, x);
        }
        // With p = c[0]*x^3 + c[1]*x^5 + c[2]*x^7 + c[3]*x^9 + c[4]*x^11, q = x + p is a
        // minimax approximation of sinh(x) on [x0,1/4] with |q - sinh(x)|/x^3 < 2^-56.584.
        const C: [f64; 5] = [
            hf("0x1.5555555555555p-3"),
            hf("0x1.111111111151ep-7"),
            hf("0x1.a01a019d0c767p-13"),
            hf("0x1.71de444a96e11p-19"),
            hf("0x1.ae8465375242p-26"),
        ];
        let x2 = x * x;
        let x3 = x2 * x;
        let x4 = x2 * x2;
        let p = x3 * ((C[0] + x2 * C[1]) + x4 * ((C[2] + x2 * C[3]) + x4 * C[4]));
        let e = x3 * const { hf("0x1.cp-53") };
        let lb = x + (p - e);
        let ub = x + (p + e);
        if lb == ub {
            return lb;
        }
        return as_sinh_zero(x);
    }
    if aix > 0x408633ce8fb9f87d {
        // |x| >~ 710.47586
        if aix >= 0x7ff0000000000000 {
            return x + x; // nan Inf
        }
        return const { hf("0x1p1023") }.copysign(x) * 2.0;
    }
    // now 0.25 <= |x| < 710.47586; this branch was checked exhaustively with/without FMA
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
    let sgn = 1.0f64.copysign(x);
    if aix > 0x4014000000000000 {
        // |x| > 5
        if aix > 0x40425e4f7b2737fa {
            // |x| >~ 36.736801
            sp = ((1021 + ie) as u64) << 52;
            rh = th;
            rl = tl + th * pp;
            rh *= sgn;
            rl *= sgn;
            let e = const { hf("0x1.1b6p-63") } * th;
            let lb = rh + (rl - e);
            let ub = rh + (rl + e);
            if lb == ub {
                return (lb * f64::from_bits(sp)) * 2.0;
            }
            (th, tl) = as_exp_accurate(ax, t, th, tl);
            (th, tl) = fasttwosum(th, tl);
            th *= sgn;
            tl *= sgn;
            let ml = (tl.to_bits().wrapping_add(8) & (!0u64 >> 12)) as i64;
            th += tl;
            th *= 2.0;
            th *= f64::from_bits(sp);
            // when ml <= 16 is false, we never have eh-el > 103
            if ml <= 16 {
                return as_sinh_database(x, th);
            }
            return th;
        }
        // now 5 < |x| < 36.736801
        let q0h = T0[j0 as usize][1];
        let q1h = T1[j1 as usize][1];
        let mut qh = q0h * q1h;
        th *= f64::from_bits(sp);
        tl *= f64::from_bits(sp);
        qh *= f64::from_bits(sm);
        let pm = mx * ((ch[0] + mx * ch[1]) + dx2 * (ch[2] + mx * ch[3]));
        let em = qh + qh * pm;
        rh = th;
        rl = (tl - em) + th * pp;
        rh *= sgn;
        rl *= sgn;
        let e = const { hf("0x1.202p-63") } * rh;
        let lb = rh + (rl - e);
        let ub = rh + (rl + e);
        if lb == ub {
            return lb;
        }
        (th, tl) = as_exp_accurate(ax, t, th, tl);
        if aix > 0x403f666666666666 {
            // |x| > 31.4
            rh = th - qh;
            rl = ((th - rh) - qh) + tl;
        } else {
            // 5 < |x| <= 31.4
            qh = q0h * q1h;
            let q0l = T0[j0 as usize][0];
            let q1l = T1[j1 as usize][0];
            let mut ql = (q0h * q1l + q1h * q0l) + F::fma(q0h, q1h, -qh);
            qh *= f64::from_bits(sm);
            ql *= f64::from_bits(sm);
            (qh, ql) = as_exp_accurate(-ax, -t, qh, ql);
            rh = th - qh;
            rl = (((th - rh) - qh) - ql) + tl;
        }
    } else {
        // 0.25 <= |x| <= 5
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
        rh = fph - fmh;
        rl = (((fph - rh) - fmh) - fml) + fpl;
        rh *= sgn;
        rl *= sgn;
        let e = const { hf("0x1.c0ap-62") } * rh;
        let lb = rh + (rl - e);
        let ub = rh + (rl + e);
        if lb == ub {
            return lb;
        }
        (th, tl) = as_exp_accurate(ax, t, th, tl);
        (qh, ql) = as_exp_accurate(-ax, -t, qh, ql);
        rh = th - qh;
        rl = (((th - rh) - qh) - ql) + tl;
    }
    (rh, rl) = fasttwosum(rh, rl);
    let ml = (rl.to_bits().wrapping_add(8) & (!0u64 >> 12)) as i64;
    rh *= sgn;
    rl *= sgn;
    rh += rl;
    // when ml <= 16 is false, we never have eh-el > 103
    if ml <= 16 {
        return as_sinh_database(x, rh);
    }
    rh
}

multiversion! {
    /// The hyperbolic sine, correctly rounded (round to nearest, ties to even): CORE-MATH
    /// `cr_sinh`. The same bits on every target. `Math.sinh` (§21.3.2.31): NaN, ±0 and ±∞ give
    /// themselves.
    pub fn sinh(x: f64) -> f64 = sinh_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinh_special_values() {
        assert!(sinh(f64::NAN).is_nan());
        assert_eq!(sinh(0.0).to_bits(), 0);
        assert_eq!(sinh(-0.0).to_bits(), 1 << 63);
        assert_eq!(sinh(f64::INFINITY), f64::INFINITY);
        assert_eq!(sinh(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(sinh(711.0), f64::INFINITY);
        assert_eq!(sinh(-711.0), f64::NEG_INFINITY);
    }
}
