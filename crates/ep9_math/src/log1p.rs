//! `Math.log1p`: the correctly rounded log(1+x) (H36), a port of CORE-MATH `cr_log1p`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/log1p/log1p.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded log(1+x) for binary64 values.
//! Copyright (c) 2024-2026 Alexei Sibidanov <sibid@uvic.ca>.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. ep9 changes: rounding to
//! nearest only (JS); no `errno`; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact
//! software FMA, the same bits); no FMA contraction; `roundeven_finite` of a value below 2^51
//! in magnitude is the 1.5 * 2^52 trick (no libm call).

use crate::asin::roundeven_small;
use crate::exact::FmaOps;
use crate::hf;

#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

#[inline(always)]
fn twosum(xh: f64, ch: f64) -> (f64, f64) {
    let s = xh + ch;
    let d = s - xh;
    (s, (ch - d) + (xh + (d - s)))
}

#[inline(always)]
fn fastsum(xh: f64, xl: f64, yh: f64, yl: f64) -> (f64, f64) {
    let (sh, sl) = fasttwosum(xh, yh);
    (sh, (xl + yl) + sl)
}

#[inline(always)]
fn sum(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let (sh, sl) = twosum(xh, ch);
    (sh, (xl + cl) + sl)
}

#[inline(always)]
fn muldd<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahhh = ch * xh;
    (ahhh, (cl * xh + ch * xl) + F::fma(ch, xh, -ahhh))
}

#[inline(always)]
fn mulddd<F: FmaOps>(x: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahhh = ch * x;
    (ahhh, cl * x + F::fma(ch, x, -ahhh))
}

#[inline(always)]
fn polydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, l) = fasttwosum(c[i][0], l);
    let mut cl = c[i][1] + l;
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd::<F>(xh, xl, ch, cl);
        (ch, cl) = fastsum(c[i][0], c[i][1], ch, cl);
    }
    (ch, cl)
}

#[inline(always)]
fn polyddd<F: FmaOps>(x: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, l) = fasttwosum(c[i][0], l);
    let mut cl = c[i][1] + l;
    while i > 0 {
        i -= 1;
        (ch, cl) = mulddd::<F>(x, ch, cl);
        (ch, cl) = fastsum(c[i][0], c[i][1], ch, cl);
    }
    (ch, cl)
}

/// Lookup tables for the quadripartite scheme centered around zero:
/// -ln(rt\[\]\[\]) = ln\[\]\[\]\[2\] + ln\[\]\[\]\[1\] + ln\[\]\[\]\[0\].
static RT: [[f64; 16]; 4] = [
    [
        hf("0x1.6a09e68p+0"),
        hf("0x1.5ab07ep+0"),
        hf("0x1.4bfdad8p+0"),
        hf("0x1.3dea65p+0"),
        hf("0x1.306fe08p+0"),
        hf("0x1.2387a7p+0"),
        hf("0x1.172b84p+0"),
        hf("0x1.0b5587p+0"),
        hf("0x1p+0"),
        hf("0x1.ea4afap-1"),
        hf("0x1.d5818ep-1"),
        hf("0x1.c199bep-1"),
        hf("0x1.ae89f98p-1"),
        hf("0x1.9c4918p-1"),
        hf("0x1.8ace54p-1"),
        hf("0x1.7a1147p-1"),
    ],
    [
        hf("0x1.059b0dp+0"),
        hf("0x1.04e5f7p+0"),
        hf("0x1.04315e8p+0"),
        hf("0x1.037d43p+0"),
        hf("0x1.02c9a4p+0"),
        hf("0x1.0216818p+0"),
        hf("0x1.0163da8p+0"),
        hf("0x1.00b1af8p+0"),
        hf("0x1p+0"),
        hf("0x1.fe9d968p-1"),
        hf("0x1.fd3c228p-1"),
        hf("0x1.fbdba38p-1"),
        hf("0x1.fa7c18p-1"),
        hf("0x1.f91d8p-1"),
        hf("0x1.f7bfdbp-1"),
        hf("0x1.f663278p-1"),
    ],
    [
        hf("0x1.0058c88p+0"),
        hf("0x1.004dad8p+0"),
        hf("0x1.0042938p+0"),
        hf("0x1.0037798p+0"),
        hf("0x1.002c608p+0"),
        hf("0x1.0021478p+0"),
        hf("0x1.00162fp+0"),
        hf("0x1.000b178p+0"),
        hf("0x1p+0"),
        hf("0x1.ffe9d2p-1"),
        hf("0x1.ffd3a58p-1"),
        hf("0x1.ffbd798p-1"),
        hf("0x1.ffa74e8p-1"),
        hf("0x1.ff91248p-1"),
        hf("0x1.ff7afb8p-1"),
        hf("0x1.ff64d38p-1"),
    ],
    [
        hf("0x1.00058b8p+0"),
        hf("0x1.0004dap+0"),
        hf("0x1.0004288p+0"),
        hf("0x1.0003778p+0"),
        hf("0x1.0002c6p+0"),
        hf("0x1.0002148p+0"),
        hf("0x1.000163p+0"),
        hf("0x1.0000b18p+0"),
        hf("0x1p+0"),
        hf("0x1.fffe9dp-1"),
        hf("0x1.fffd3ap-1"),
        hf("0x1.fffbd78p-1"),
        hf("0x1.fffa748p-1"),
        hf("0x1.fff9118p-1"),
        hf("0x1.fff7ae8p-1"),
        hf("0x1.fff64cp-1"),
    ],
];
static LN: [[[f64; 3]; 16]; 4] = [
    [
        [
            hf("0x1.61fa45b636ea2p-95"),
            hf("-0x1.497697cb3134p-45"),
            hf("-0x1.62e43033a8p-2"),
        ],
        [
            hf("-0x1.42b671e5e6d5ep-93"),
            hf("0x1.fc50ee605bd8p-44"),
            hf("-0x1.3687aa721cp-2"),
        ],
        [
            hf("0x1.0cc2ea968d329p-95"),
            hf("0x1.f48666c24eacp-43"),
            hf("-0x1.0a2b247d58p-2"),
        ],
        [
            hf("-0x1.963904db0a34ep-93"),
            hf("-0x1.58495e50d36ap-43"),
            hf("-0x1.bb9d3d808p-3"),
        ],
        [
            hf("0x1.337e0d9f81652p-95"),
            hf("0x1.c89a0bede978p-45"),
            hf("-0x1.62e42f0378p-3"),
        ],
        [
            hf("-0x1.1bdf53871d50bp-95"),
            hf("0x1.2d67e8a0c1d7p-42"),
            hf("-0x1.0a2b24a1p-3"),
        ],
        [
            hf("-0x1.781dd6fc4053p-98"),
            hf("0x1.07a22a9e81ed8p-42"),
            hf("-0x1.62e43327cp-4"),
        ],
        [
            hf("-0x1.84e4562b8f2f3p-96"),
            hf("-0x1.868afe14895p-43"),
            hf("-0x1.62e435baep-5"),
        ],
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("-0x1.17e3ec05cde7p-96"),
            hf("0x1.174a19689569p-42"),
            hf("0x1.62e432b22p-5"),
        ],
        [
            hf("-0x1.8067ec2a27738p-95"),
            hf("0x1.111a4eadf313p-43"),
            hf("0x1.62e42e4a8p-4"),
        ],
        [
            hf("-0x1.60bef06b14405p-93"),
            hf("0x1.dd4ec4e1d421p-43"),
            hf("0x1.0a2b233f08p-3"),
        ],
        [
            hf("-0x1.b18e160362c24p-94"),
            hf("0x1.6bd65e8b0b7p-45"),
            hf("0x1.62e43056cp-3"),
        ],
        [
            hf("-0x1.c6ac3f1862a6bp-93"),
            hf("-0x1.0f5aad509ea8p-44"),
            hf("0x1.bb9d3cbd68p-3"),
        ],
        [
            hf("-0x1.dead1a4581acfp-93"),
            hf("-0x1.ac842de00564p-43"),
            hf("0x1.0a2b244da4p-2"),
        ],
        [
            hf("-0x1.96b1f2f60e3e4p-93"),
            hf("0x1.9cec9a50db228p-42"),
            hf("0x1.3687aa9b78p-2"),
        ],
    ],
    [
        [
            hf("0x1.dbd93cdc08613p-96"),
            hf("0x1.96f39c49fa8ap-44"),
            hf("-0x1.62e423dd4p-6"),
        ],
        [
            hf("-0x1.3c008f058a015p-94"),
            hf("-0x1.f26429af65b7p-43"),
            hf("-0x1.36879e514p-6"),
        ],
        [
            hf("0x1.f96a86136c1d1p-93"),
            hf("-0x1.bfc5afb2bd378p-42"),
            hf("-0x1.0a2b2240cp-6"),
        ],
        [
            hf("0x1.3917cea885bep-93"),
            hf("-0x1.421e0c297638p-46"),
            hf("-0x1.bb9d4b288p-7"),
        ],
        [
            hf("0x1.f0f1b5d989675p-93"),
            hf("0x1.92a7506c90a18p-42"),
            hf("-0x1.62e43c12p-7"),
        ],
        [
            hf("-0x1.f7f8d6a190051p-93"),
            hf("-0x1.94925ae0e524p-42"),
            hf("-0x1.0a2b41ddp-7"),
        ],
        [
            hf("0x1.ccb0919263869p-94"),
            hf("-0x1.0fb337ff2cf28p-42"),
            hf("-0x1.62e41068p-8"),
        ],
        [
            hf("0x1.c9a5efe4b7c6cp-93"),
            hf("-0x1.966c27788d1cp-44"),
            hf("-0x1.62e3e4ccp-9"),
        ],
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("-0x1.40288ccae8f0fp-95"),
            hf("-0x1.f3c5ff88d19c8p-42"),
            hf("0x1.62e462b6p-9"),
        ],
        [
            hf("-0x1.f7e788a87135p-94"),
            hf("-0x1.a9610028771cp-43"),
            hf("0x1.62e44c93p-8"),
        ],
        [
            hf("0x1.574cc6d3f577dp-93"),
            hf("0x1.2a3a1a65aa398p-42"),
            hf("0x1.0a2b1e33p-7"),
        ],
        [
            hf("-0x1.56bb79b254f33p-99"),
            hf("-0x1.4a995b6d9ddcp-44"),
            hf("0x1.62e4367cp-7"),
        ],
        [
            hf("0x1.c4c209ca6783dp-94"),
            hf("-0x1.d7b98ef45911p-43"),
            hf("0x1.bb9d449a8p-7"),
        ],
        [
            hf("0x1.e9731de7f0155p-93"),
            hf("-0x1.de722390bbd6p-43"),
            hf("0x1.0a2b1f194p-6"),
        ],
        [
            hf("0x1.fadc62522444dp-96"),
            hf("-0x1.fd95cb835e38p-45"),
            hf("0x1.3687ad114p-6"),
        ],
    ],
    [
        [
            hf("-0x1.1f72d2a6a460ep-95"),
            hf("0x1.a25045c37c33p-43"),
            hf("-0x1.62e4795p-10"),
        ],
        [
            hf("0x1.2847e318fd3ffp-93"),
            hf("-0x1.c633239e1cdc8p-42"),
            hf("-0x1.3686e5dcp-10"),
        ],
        [
            hf("0x1.63f5750b9d826p-93"),
            hf("-0x1.cb15590f1cd78p-42"),
            hf("-0x1.0a2b6538p-10"),
        ],
        [
            hf("-0x1.1382d6395c24cp-94"),
            hf("0x1.7636a5400e2bp-43"),
            hf("-0x1.bb9bf138p-11"),
        ],
        [
            hf("0x1.be4491ec20322p-94"),
            hf("0x1.d0c5e2c9b6be8p-42"),
            hf("-0x1.62e53e5p-11"),
        ],
        [
            hf("-0x1.706f04cc2c9f6p-93"),
            hf("-0x1.f3bc0ce9b9a08p-42"),
            hf("-0x1.0a2ab37p-11"),
        ],
        [
            hf("0x1.ab6f41df01d7fp-93"),
            hf("0x1.a7cbc9a97ba4p-44"),
            hf("-0x1.62e0ap-12"),
        ],
        [
            hf("-0x1.67e487663ca3ep-100"),
            hf("-0x1.957976dc5f36p-43"),
            hf("-0x1.62e84fcp-13"),
        ],
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("0x1.9681e48dde135p-93"),
            hf("-0x1.868625640a69p-43"),
            hf("0x1.62e7bp-13"),
        ],
        [
            hf("0x1.a2948cd558655p-93"),
            hf("-0x1.2ee3d96b696ap-42"),
            hf("0x1.62e35f6p-12"),
        ],
        [
            hf("-0x1.cfc26ccf6d0e4p-96"),
            hf("0x1.53edbcf1165p-46"),
            hf("0x1.0a2b4b2p-11"),
        ],
        [
            hf("0x1.f68d24b9e338dp-93"),
            hf("0x1.783e334613p-51"),
            hf("0x1.62e4be1p-11"),
        ],
        [
            hf("-0x1.f33369bf7dff1p-95"),
            hf("-0x1.60785f20acb2p-42"),
            hf("0x1.bb9e085p-11"),
        ],
        [
            hf("-0x1.685a35575eff1p-95"),
            hf("-0x1.5a62ec66568p-48"),
            hf("0x1.0a2b94d4p-10"),
        ],
        [
            hf("-0x1.4c4d1abca79bfp-95"),
            hf("0x1.7ded26dc813p-46"),
            hf("0x1.368810f8p-10"),
        ],
    ],
    [
        [
            hf("0x1.40676dea39b19p-94"),
            hf("0x1.25337681fa9p-42"),
            hf("-0x1.62dc284p-14"),
        ],
        [
            hf("-0x1.ec76c964ac8d6p-94"),
            hf("-0x1.504032e2ed388p-42"),
            hf("-0x1.367d0ecp-14"),
        ],
        [
            hf("0x1.8ecaeb6d9523cp-95"),
            hf("0x1.c28fde83047ep-44"),
            hf("-0x1.0a1dd6cp-14"),
        ],
        [
            hf("-0x1.e4069214576bep-93"),
            hf("0x1.58f2757976ef8p-42"),
            hf("-0x1.bbbcffp-15"),
        ],
        [
            hf("0x1.86146d1f9b91fp-93"),
            hf("-0x1.d4717ca0a323p-42"),
            hf("-0x1.62fe138p-15"),
        ],
        [
            hf("0x1.1eb44b07102c5p-93"),
            hf("-0x1.86ffcda25278p-43"),
            hf("-0x1.0a3eebp-15"),
        ],
        [
            hf("-0x1.95bbb5fd5a466p-93"),
            hf("0x1.1ce399729e7bp-43"),
            hf("-0x1.62ff0ap-16"),
        ],
        [
            hf("-0x1.79059023f8767p-93"),
            hf("-0x1.dbf1c6a400408p-42"),
            hf("-0x1.62ff84p-17"),
        ],
        [hf("0x0p+0"), hf("0x0p+0"), hf("0x0p+0")],
        [
            hf("-0x1.b4cb08d6fd48fp-93"),
            hf("-0x1.db0e38e5aaa98p-42"),
            hf("0x1.63007cp-17"),
        ],
        [
            hf("-0x1.0e6a88bfc3838p-93"),
            hf("0x1.2b1c75580439p-43"),
            hf("0x1.6300f6p-16"),
        ],
        [
            hf("0x1.8074feacfe49dp-94"),
            hf("0x1.401bb919f14ep-42"),
            hf("0x1.0a21148p-15"),
        ],
        [
            hf("-0x1.f56f5f168db15p-93"),
            hf("-0x1.85d6f6487ce2p-44"),
            hf("0x1.62e1ecp-15"),
        ],
        [
            hf("0x1.9eae7e05a0143p-93"),
            hf("-0x1.af5d58a7c9218p-42"),
            hf("0x1.bba301p-15"),
        ],
        [
            hf("-0x1.a859095999ae3p-94"),
            hf("0x1.590faa0883bd8p-42"),
            hf("0x1.0a32298p-14"),
        ],
        [
            hf("0x1.4f787e495e5ep-93"),
            hf("-0x1.fbb791220a18p-46"),
            hf("0x1.3682f14p-14"),
        ],
    ],
];

/// Lookup tables for the fast path: -ln(rf\[\]) = lf\[\]\[1\] + lf\[\]\[0\], with rf from about
/// 1/sqrt(2) to sqrt(2).
static RF: [f64; 64] = [
    hf("0x1.6816818p+0"),
    hf("0x1.642c858p+0"),
    hf("0x1.605816p+0"),
    hf("0x1.5c98828p+0"),
    hf("0x1.58ed23p+0"),
    hf("0x1.5555558p+0"),
    hf("0x1.51d07e8p+0"),
    hf("0x1.4e5e0a8p+0"),
    hf("0x1.4afd6ap+0"),
    hf("0x1.47ae148p+0"),
    hf("0x1.446f868p+0"),
    hf("0x1.4141418p+0"),
    hf("0x1.3e22ccp+0"),
    hf("0x1.3b13b1p+0"),
    hf("0x1.381381p+0"),
    hf("0x1.3521cf8p+0"),
    hf("0x1.323e348p+0"),
    hf("0x1.2f684cp+0"),
    hf("0x1.2c9fb5p+0"),
    hf("0x1.29e4128p+0"),
    hf("0x1.27350b8p+0"),
    hf("0x1.249249p+0"),
    hf("0x1.21fb78p+0"),
    hf("0x1.1f7048p+0"),
    hf("0x1.1cf06bp+0"),
    hf("0x1.1a7b96p+0"),
    hf("0x1.181181p+0"),
    hf("0x1.15b1e6p+0"),
    hf("0x1.135c81p+0"),
    hf("0x1.111111p+0"),
    hf("0x1.0ecf568p+0"),
    hf("0x1.0c9715p+0"),
    hf("0x1.0a68108p+0"),
    hf("0x1.0842108p+0"),
    hf("0x1.0624ddp+0"),
    hf("0x1.041041p+0"),
    hf("0x1.020408p+0"),
    hf("0x1p+0"),
    hf("0x1.f81f82p-1"),
    hf("0x1.f07c1fp-1"),
    hf("0x1.e9131a8p-1"),
    hf("0x1.e1e1e2p-1"),
    hf("0x1.dae6078p-1"),
    hf("0x1.d41d42p-1"),
    hf("0x1.cd85688p-1"),
    hf("0x1.c71c72p-1"),
    hf("0x1.c0e07p-1"),
    hf("0x1.bacf918p-1"),
    hf("0x1.b4e81b8p-1"),
    hf("0x1.af286cp-1"),
    hf("0x1.a98ef6p-1"),
    hf("0x1.a41a418p-1"),
    hf("0x1.9ec8e98p-1"),
    hf("0x1.9999998p-1"),
    hf("0x1.948b1p-1"),
    hf("0x1.8f9c19p-1"),
    hf("0x1.8acb91p-1"),
    hf("0x1.8618618p-1"),
    hf("0x1.8181818p-1"),
    hf("0x1.7d05f4p-1"),
    hf("0x1.78a4c8p-1"),
    hf("0x1.745d178p-1"),
    hf("0x1.702e06p-1"),
    hf("0x1.6c16c18p-1"),
];
static LF: [[f64; 2]; 64] = [
    [hf("-0x1.f2f8281bade6ap-42"), hf("-0x1.5d5bde3994p-2")],
    [hf("0x1.c2843fdd367a4p-42"), hf("-0x1.522ae0438cp-2")],
    [hf("-0x1.06c10c34c14bp-44"), hf("-0x1.4718dc171cp-2")],
    [hf("0x1.cfa4e853f589p-43"), hf("-0x1.3c2526cb34p-2")],
    [hf("-0x1.ce3ac179bd856p-42"), hf("-0x1.314f1e0534p-2")],
    [hf("-0x1.b91f82deb8122p-42"), hf("-0x1.269621934cp-2")],
    [hf("0x1.46bbb83d7163ep-42"), hf("-0x1.1bf995a9a8p-2")],
    [hf("0x1.b842e5a74bdbp-42"), hf("-0x1.1178e84a8p-2")],
    [hf("-0x1.862715e5bb534p-42"), hf("-0x1.071385f4d4p-2")],
    [hf("-0x1.9bcbcbea0cdf8p-42"), hf("-0x1.f991c6eb38p-3")],
    [hf("-0x1.01101cb605958p-43"), hf("-0x1.e530f1067p-3")],
    [hf("0x1.0c38c81ad8f06p-42"), hf("-0x1.d10380b658p-3")],
    [hf("0x1.3aa40992a6d82p-42"), hf("-0x1.bd0874c3cp-3")],
    [hf("0x1.30f68780ae82ep-42"), hf("-0x1.a93ed248bp-3")],
    [hf("-0x1.7d116989d098p-47"), hf("-0x1.95a5ac5f7p-3")],
    [hf("-0x1.1e0012ba619cap-42"), hf("-0x1.823c150518p-3")],
    [hf("0x1.54535d5671858p-43"), hf("-0x1.6f0127cf58p-3")],
    [hf("-0x1.ed87db3498128p-42"), hf("-0x1.5bf407b54p-3")],
    [hf("-0x1.aafde9c9fc39ap-42"), hf("-0x1.4913d94338p-3")],
    [hf("-0x1.015868c234p-43"), hf("-0x1.365fca3158p-3")],
    [hf("0x1.eff33f502c226p-42"), hf("-0x1.23d7126cap-3")],
    [hf("0x1.b8521e874d358p-43"), hf("-0x1.1178e7228p-3")],
    [hf("0x1.54d75afe84568p-43"), hf("-0x1.fe89129dcp-4")],
    [hf("-0x1.1a813f3fa7c1ep-42"), hf("-0x1.da7278384p-4")],
    [hf("-0x1.6c6676f40963ep-42"), hf("-0x1.b6ac8afadp-4")],
    [hf("-0x1.2620b7957a7a6p-42"), hf("-0x1.9335e4d59p-4")],
    [hf("0x1.f8ffee5598f38p-43"), hf("-0x1.700d2f4ebp-4")],
    [hf("-0x1.fab0f5bf42ca2p-42"), hf("-0x1.4d311652p-4")],
    [hf("-0x1.7a3e970b1c3a8p-44"), hf("-0x1.2aa049247p-4")],
    [hf("-0x1.d030435fecb5p-43"), hf("-0x1.08598a59ep-4")],
    [hf("0x1.35084a4fb8ab8p-43"), hf("-0x1.ccb7357dep-5")],
    [hf("0x1.32f36d60b44c4p-43"), hf("-0x1.894aa1cap-5")],
    [hf("0x1.c1bcce5be811p-45"), hf("-0x1.466ae8a2ep-5")],
    [hf("0x1.777740b18714ap-42"), hf("-0x1.0415d81e8p-5")],
    [hf("-0x1.955c057693d94p-43"), hf("-0x1.8492470c8p-6")],
    [hf("0x1.4f71addb8bep-43"), hf("-0x1.020564894p-6")],
    [hf("-0x1.bcda4e198afbp-44"), hf("-0x1.01014f588p-7")],
    [hf("0x1.cp-67"), hf("0x0p+0")],
    [hf("-0x1.fe0df75092c5ep-42"), hf("0x1.fc0a891p-7")],
    [hf("0x1.98036ec7e0a1p-45"), hf("0x1.f829b1e78p-6")],
    [hf("0x1.ba010f49e5ffp-42"), hf("0x1.774593832p-5")],
    [hf("-0x1.3ab13c266d328p-42"), hf("0x1.f0a30a012p-5")],
    [hf("-0x1.71798573e45d4p-43"), hf("0x1.341d78b1cp-4")],
    [hf("0x1.ad32f072669fcp-42"), hf("0x1.6f0d272e5p-4")],
    [hf("-0x1.54e391e16ea38p-43"), hf("0x1.a926d434bp-4")],
    [hf("-0x1.a302bbaf0559p-45"), hf("0x1.e27074e2bp-4")],
    [hf("0x1.cb4cd66e31f3p-44"), hf("0x1.0d77e8cd08p-3")],
    [hf("-0x1.5b7a5bc474128p-44"), hf("0x1.29552e92p-3")],
    [hf("-0x1.7062e8135f74p-46"), hf("0x1.44d2b5e4b8p-3")],
    [hf("0x1.3d4c88fe1f4bp-43"), hf("0x1.5ff3060a78p-3")],
    [hf("-0x1.37b70004a6946p-42"), hf("0x1.7ab890411p-3")],
    [hf("-0x1.4a5885167c1ecp-42"), hf("0x1.9525aa7f48p-3")],
    [hf("0x1.ff9d5953004acp-42"), hf("0x1.af3c940008p-3")],
    [hf("0x1.a21ec41d8219cp-43"), hf("0x1.c8ff7cf9a8p-3")],
    [hf("-0x1.a322bf2f02ae8p-44"), hf("0x1.e27075e2bp-3")],
    [hf("0x1.f1548b8a33616p-42"), hf("0x1.fb9186b5ep-3")],
    [hf("0x1.0e36401f7a006p-42"), hf("0x1.0a324e0f38p-2")],
    [hf("-0x1.9f1fa55382a8ap-42"), hf("0x1.1675cacabcp-2")],
    [hf("-0x1.a69763deb096p-44"), hf("0x1.22941fc0f8p-2")],
    [hf("0x1.d30bc3ac91bdap-42"), hf("0x1.2e8e2bee1p-2")],
    [hf("0x1.7a79cf4d73b28p-44"), hf("0x1.3a64c59694p-2")],
    [hf("0x1.ec345197b22dep-42"), hf("0x1.4618bb81c4p-2")],
    [hf("-0x1.f4810a30aeba8p-44"), hf("0x1.51aad7c2ep-2")],
    [hf("0x1.394d2371c1d1cp-43"), hf("0x1.5d1bdbbd8p-2")],
];

#[inline(always)]
fn log1p_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    let ax = ix << 1;
    let (ln1, ln0, eps);
    // log1p is expected near 0, where it is more accurate than log(1+x)
    if ax < 0x7f60000000000000 {
        // |x| < 0.0625; x tiny first, to avoid a spurious underflow in x*x
        if ax < 0x7940000000000000 {
            // |x| < 0x1p-53
            if ax == 0 {
                return x;
            }
            return F::fma(x.abs(), const { hf("-0x1p-54") }, x);
        }
        let x2 = x * x;
        if ax < 0x7e60000000000000 {
            // |x| < 0x1p-12
            ln1 = x;
            eps = const { hf("0x1.ap-64") } * x;
            if ax < 0x7d43360000000000 {
                // |x| < 0x1.19bp-21
                const C: [f64; 2] = [hf("-0x1.00000000001d1p-1"), hf("0x1.55555555558f7p-2")];
                ln0 = x2 * (C[0] + x * C[1]);
            } else {
                const C: [f64; 4] = [
                    hf("-0x1.ffffffffffffdp-2"),
                    hf("0x1.5555555555551p-2"),
                    hf("-0x1.000000d5555e1p-2"),
                    hf("0x1.99999b442f73fp-3"),
                ];
                ln0 = x2 * ((C[0] + x * C[1]) + x2 * (C[2] + x * C[3]));
            }
        } else {
            const C: [f64; 12] = [
                hf("0x1.5555555555555p-2"),
                hf("-0x1p-2"),
                hf("0x1.9999999999b41p-3"),
                hf("-0x1.555555555583bp-3"),
                hf("0x1.24924923f39ep-3"),
                hf("-0x1.fffffffe42e43p-4"),
                hf("0x1.c71c75511d70bp-4"),
                hf("-0x1.99999de10510fp-4"),
                hf("0x1.7457e81b175f6p-4"),
                hf("-0x1.554fb43e54e0fp-4"),
                hf("0x1.3ed68744f3d18p-4"),
                hf("-0x1.28558ad5a7ac4p-4"),
            ];
            let x3 = x2 * x;
            let x4 = x2 * x2;
            let hx = -0.5 * x;
            ln1 = F::fma(hx, x, x);
            let mut l0 = F::fma(hx, x, x - ln1);
            let f = ((C[0] + x * C[1]) + x2 * (C[2] + x * C[3]))
                + x4 * (((C[4] + x * C[5]) + x2 * (C[6] + x * C[7]))
                    + x4 * ((C[8] + x * C[9]) + x2 * (C[10] + x * C[11])));
            l0 += x3 * f;
            ln0 = l0;
            eps = x3 * const { hf("0x1.b6p-52") };
        }
    } else {
        // |x| >= 0.0625
        const C: [f64; 6] = [
            hf("-0x1.000000000003dp-1"),
            hf("0x1.5555555554cf5p-2"),
            hf("-0x1.ffffffeca2939p-3"),
            hf("0x1.99999a3661724p-3"),
            hf("-0x1.555d345bfe6fdp-3"),
            hf("0x1.247b887a6e5edp-3"),
        ];
        let (mut t, mut dt);
        if (ix as i64) < 0x4340000000000000 && ix < 0xbff0000000000000 {
            // 0.0625 < x < 0x1p+53 or -1 < x < -0.0625. For 1 < x < 2^53 the Fast2Sum
            // pre-condition does not hold, but the result is exact there (C comment).
            (t, dt) = fasttwosum(1.0, x);
        } else if ix < 0x4690000000000000 {
            // x < 0x1p+106
            t = x;
            dt = 1.0;
        } else if ix < 0x7ff0000000000000 {
            // x < 0x1p+1024
            t = x;
            dt = 0.0;
        } else {
            if ax > 0xffe0000000000000 {
                return x + x; // nan
            }
            if ix == 0x7ff0000000000000 {
                return x; // +inf
            }
            if ix == 0xbff0000000000000 {
                return f64::NEG_INFINITY; // -1: pole
            }
            return f64::NAN; // < -1
        }
        let j = t.to_bits().wrapping_sub(0x3fe6a00000000000) as i64;
        let j1 = ((j >> (52 - 6)) & 0x3f) as usize;
        let je = j >> 52;
        let eoff = (je as u64) << 52;
        let mut rs = RF[j1].to_bits();
        if je < 1022 {
            rs = rs.wrapping_sub(eoff);
        } else {
            rs = rs.wrapping_sub(1021u64 << 52);
            const SC: [f64; 3] = [hf("0x1p-1"), hf("0x1p-2"), hf("0x1p-3")];
            // t and dt are scaled instead of rs (no subnormal rs; C comment)
            t *= SC[(je - 1022) as usize];
            dt *= SC[(je - 1022) as usize];
        }
        let rs = f64::from_bits(rs);
        let dh = rs * t;
        let dl = F::fma(rs, t, -dh) + rs * dt;
        let (xh, mut xl) = fasttwosum(dh - 1.0, dl);
        let x2 = xh * xh;
        xl += x2 * ((C[0] + xh * C[1]) + x2 * ((C[2] + xh * C[3]) + x2 * (C[4] + xh * C[5])));
        let l1 = const { hf("0x1.62e42fefa4p-1") } * je as f64;
        let l0 = const { hf("-0x1.8432a1b0e2634p-43") } * je as f64;
        let h = LF[j1][1] + l1;
        let l = LF[j1][0] + l0;
        (ln1, ln0) = fastsum(h, l, xh, xl);
        eps = const { hf("0x1.ap-65") };
    }
    let lb = ln1 + (ln0 - eps);
    let ub = ln1 + (ln0 + eps);
    if lb != ub {
        return as_log1p_refine(x, lb);
    }
    lb
}

#[inline(always)]
fn as_log1p_refine_with<F: FmaOps>(x: f64, a: f64) -> f64 {
    const CZ: [[f64; 2]; 5] = [
        [hf("0x1.5555555555555p-2"), hf("0x1.5555555555556p-56")],
        [hf("-0x1p-2"), hf("0x1.25558eff3c1efp-86")],
        [hf("0x1.999999999999ap-3"), hf("-0x1.999999a91d6cap-57")],
        [hf("-0x1.5555555555555p-3"), hf("-0x1.588aab185593ap-57")],
        [hf("0x1.2492492492492p-3"), hf("0x1.28c490abc528fp-57")],
    ];
    const CZL: [f64; 4] = [
        hf("-0x1.fffffffffc555p-4"),
        hf("0x1.c71c71c7185aap-4"),
        hf("-0x1.9999d44449a31p-4"),
        hf("0x1.745d51f1817c7p-4"),
    ];
    const CY: [[f64; 2]; 5] = [
        [1.0, 0.0],
        [-0.5, 0.0],
        [hf("0x1.5555555555555p-2"), hf("0x1.5555555555555p-56")],
        [hf("-0x1p-2"), hf("-0x1.80007ed9858b7p-107")],
        [hf("0x1.999999999999ap-3"), hf("-0x1.9999999999991p-57")],
    ];
    const CL: [f64; 4] = [
        hf("-0x1.5555555555555p-3"),
        hf("0x1.2492492492492p-3"),
        hf("-0x1.0000000073334p-3"),
        hf("0x1.c71c71c802f68p-4"),
    ];
    let ix = x.to_bits();
    let (mut ln22, mut ln21, mut ln20);
    let ax = ix << 1;
    if ax < 0x7ea0000000000000 {
        if ax < 0x7940000000000000 {
            if ax == 0 {
                return x;
            }
            return F::fma(x.abs(), const { hf("-0x1p-54") }, x);
        }
        let mut x2h = x * x;
        let mut x2l = F::fma(x, x, -x2h);
        let (x3h, x3l) = mulddd::<F>(x, x2h, x2l);
        let sl = x * ((CZL[0] + x * CZL[1]) + x2h * (CZL[2] + x * CZL[3]));
        let (sh, sl) = polyddd::<F>(x, &CZ, sl);
        let (sh, sl) = muldd::<F>(sh, sl, x3h, x3l);
        x2h *= -0.5;
        x2l *= -0.5;
        ln22 = x;
        (ln21, ln20) = fastsum(x2h, x2l, sh, sl);
    } else {
        let j = roundeven_small(a * const { hf("0x1.71547652b82fep+16") }) as i64;
        let i = j + 34952;
        let j1 = ((i >> 12) & 0xf) as usize;
        let j2 = ((i >> 8) & 0xf) as usize;
        let j3 = ((i >> 4) & 0xf) as usize;
        let j4 = (i & 0xf) as usize;
        let je = i >> 16;
        let l0 = (LN[0][j1][0] + LN[1][j2][0]) + (LN[2][j3][0] + LN[3][j4][0]);
        let l1 = (LN[0][j1][1] + LN[1][j2][1]) + (LN[2][j3][1] + LN[3][j4][1]);
        let l2 = (LN[0][j1][2] + LN[1][j2][2]) + (LN[2][j3][2] + LN[3][j4][2]);
        let (t, mut dt);
        if (ix as i64) < 0x4690000000000000 && ix < 0xbfe0000000000000 {
            let (s, e) = twosum(1.0, x);
            t = s;
            dt = e.to_bits();
            if dt << 1 == 0 {
                dt = 0;
            }
        } else {
            t = if (ix as i64) >= 0x4690000000000000 {
                x
            } else {
                1.0 + x
            };
            dt = 0;
        }
        let tf = f64::from_bits(t.to_bits().wrapping_sub((je as u64) << 52));
        let t12 = RT[0][j1] * RT[1][j2];
        let t34 = RT[2][j3] * RT[3][j4];
        let th = t12 * t34;
        let tl = F::fma(t12, t34, -th);
        let dh = th * tf;
        let dl = F::fma(th, tf, -dh);
        let sh = tl * tf;
        let sl = F::fma(tl, tf, -sh);
        let (xh, xl) = fasttwosum(dh - 1.0, dl);
        let (mut xh, mut xl) = fastsum(xh, xl, sh, sl);
        if dt != 0 {
            dt = dt.wrapping_sub((je as u64) << 52);
            let dtf = f64::from_bits(dt);
            let ddh = th * dtf;
            let ddl = F::fma(th, dtf, -ddh) + tl * dtf;
            (xh, xl) = fastsum(xh, xl, ddh, ddl);
        }
        let sl = xh * ((CL[0] + xh * CL[1]) + (xh * xh) * (CL[2] + xh * CL[3]));
        let (sh, sl) = polydd::<F>(xh, xl, &CY, sl);
        let (sh, sl) = muldd::<F>(xh, xl, sh, sl);
        if je != 0 {
            let jf = je as f64;
            ln22 = const { hf("0x1.62e42fefa4p-1") } * jf;
            ln21 = const { hf("-0x1.8432a1b0e28p-43") } * jf;
            ln20 = const { hf("0x1.cc01f97b57a08p-87") } * jf;
            ln22 += l2;
            (ln21, ln20) = fastsum(ln21, ln20, l1, l0);
        } else {
            ln22 = l2;
            ln21 = l1;
            ln20 = l0;
        }
        (ln21, ln20) = sum(ln21, ln20, sh, sl);
    }
    (ln22, ln21) = fasttwosum(ln22, ln21);
    (ln21, ln20) = fasttwosum(ln21, ln20);
    let mut t = ln21.to_bits();
    if t & (!0u64 >> 12) == 0 {
        let w = ln20.to_bits();
        if (w ^ t) >> 63 != 0 {
            t = t.wrapping_sub(1);
        } else {
            t = t.wrapping_add(1);
        }
        ln21 = f64::from_bits(t);
    }
    ln22 + ln21
}

multiversion! {
    fn as_log1p_refine(x: f64, a: f64) -> f64 = as_log1p_refine_with;
}

multiversion! {
    /// log(1+x), correctly rounded (round to nearest, ties to even): CORE-MATH `cr_log1p`.
    /// The same bits on every target. `Math.log1p` (§21.3.2.21): NaN and x < -1 give NaN, ±0
    /// gives ±0, -1 gives -∞, +∞ gives +∞.
    pub fn log1p(x: f64) -> f64 = log1p_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log1p_special_values() {
        assert!(log1p(f64::NAN).is_nan());
        assert!(log1p(-1.5).is_nan());
        assert!(log1p(f64::NEG_INFINITY).is_nan());
        assert_eq!(log1p(0.0).to_bits(), 0);
        assert_eq!(log1p(-0.0).to_bits(), 1 << 63);
        assert_eq!(log1p(-1.0), f64::NEG_INFINITY);
        assert_eq!(log1p(f64::INFINITY), f64::INFINITY);
    }
}
