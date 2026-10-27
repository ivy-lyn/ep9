//! `Math.atan`: the correctly rounded arc tangent (H36), a port of CORE-MATH `cr_atan`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/atan/atan.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded arctangent function for binary64 values.
//! Copyright (c) 2023-2026 Alexei Sibidanov <sibid@uvic.ca>.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. ep9 changes: rounding to
//! nearest only (JS); no `errno`; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact
//! software FMA, the same bits); no FMA contraction. The fast path (a table of 129 points and a
//! degree-7 odd polynomial) runs first; its rounding test sends the rest to the double-double
//! path `as_atan_refine2`, a separate entry point.

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
    fasttwosum(ahhh, ahhl)
}

/// The double-double polynomial `c` at `xh + xl`; `l` is added to the top coefficient and the
/// low part of the result is returned with it.
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

/// 129 entries: `A[j][0]` approximates tan(pi/256*j), `A[j][1]` approximates
/// atan(A\[j\]\[0\]) - pi/256*j.
pub(crate) static A: [[f64; 2]; 129] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("0x1.9224e047e368ep-7"), hf("0x1.a3ca6c727c59dp-62")],
    [hf("0x1.92346247a91fp-6"), hf("0x1.138b0ef96a186p-64")],
    [hf("0x1.2dbaae9a05dbp-5"), hf("0x1.36e7f8a3f5e42p-59")],
    [hf("0x1.927278a3b1162p-5"), hf("-0x1.ac986efb92662p-64")],
    [hf("0x1.f7495ea3f3783p-5"), hf("0x1.06ec8011ee816p-59")],
    [hf("0x1.2e239ccff3831p-4"), hf("-0x1.858437d431332p-58")],
    [hf("0x1.60b9f7597fdecp-4"), hf("-0x1.cebd13eb7c513p-60")],
    [hf("0x1.936bb8c5b2da2p-4"), hf("-0x1.840cac0d81db5p-58")],
    [hf("0x1.c63ce377fc802p-4"), hf("0x1.400b0fdaa109ep-58")],
    [hf("0x1.f93183a8db9e9p-4"), hf("0x1.0e04e06c86e72p-59")],
    [hf("0x1.1626d85a91e7p-3"), hf("0x1.f7ad829163ca7p-59")],
    [hf("0x1.2fcac73a6064p-3"), hf("-0x1.2680735ce2cd8p-58")],
    [hf("0x1.4986a74cf4e57p-3"), hf("-0x1.90559690b42e4p-57")],
    [hf("0x1.635c990ce0d36p-3"), hf("0x1.91d29110b41aap-58")],
    [hf("0x1.7d4ec54fb5968p-3"), hf("-0x1.ea90e2718278p-59")],
    [hf("0x1.975f5e0553158p-3"), hf("-0x1.dc82ac14e3e1cp-61")],
    [hf("0x1.b1909efd8b762p-3"), hf("-0x1.73a10fd13daafp-58")],
    [hf("0x1.cbe4ceb4b4cf2p-3"), hf("-0x1.3a7ffbeabda0bp-57")],
    [hf("0x1.e65e3f27c9f2ap-3"), hf("-0x1.db6627a24d523p-57")],
    [hf("0x1.007fa758626aep-2"), hf("-0x1.45f97dd3099f6p-57")],
    [hf("0x1.0de53475f3b3cp-2"), hf("-0x1.6293f68741816p-57")],
    [hf("0x1.1b6103d3597e9p-2"), hf("-0x1.ab240d40633e9p-57")],
    [hf("0x1.28f459ecad74dp-2"), hf("-0x1.de34d14e832ep-61")],
    [hf("0x1.36a08355c63dcp-2"), hf("0x1.af540d9fb4926p-57")],
    [hf("0x1.4466d542bac92p-2"), hf("0x1.da60fdbc82ac4p-57")],
    [hf("0x1.5248ae1701b17p-2"), hf("-0x1.92a601170138ap-56")],
    [hf("0x1.604775fbb27dfp-2"), hf("-0x1.7f1fca1d5d15bp-57")],
    [hf("0x1.6e649f7d78649p-2"), hf("-0x1.4e223ea716c7bp-57")],
    [hf("0x1.7ca1a832d0f84p-2"), hf("0x1.b24c824ac51fcp-56")],
    [hf("0x1.8b00196b3d022p-2"), hf("0x1.4314cd132ba43p-57")],
    [hf("0x1.998188e816bfp-2"), hf("-0x1.11f1e0817879ap-56")],
    [hf("0x1.a827999fcef32p-2"), hf("-0x1.c3dea4dbad538p-57")],
    [hf("0x1.b6f3fc8c61e5bp-2"), hf("0x1.60d1b780ee3ebp-57")],
    [hf("0x1.c5e87185e67b6p-2"), hf("-0x1.ab5edb7dfa545p-59")],
    [hf("0x1.d506c82a2c8p-2"), hf("-0x1.8e1437048b5bdp-57")],
    [hf("0x1.e450e0d273e7ap-2"), hf("-0x1.06951c97b050fp-56")],
    [hf("0x1.f3c8ad985d9eep-2"), hf("-0x1.14af9522ab518p-59")],
    [hf("0x1.01b819b5a7cf7p-1"), hf("-0x1.aba0d7d97d1f2p-56")],
    [hf("0x1.09a4c59bd0d4dp-1"), hf("0x1.095bc4ebc2c42p-59")],
    [hf("0x1.11ab7190834ecp-1"), hf("0x1.798826fa27774p-55")],
    [hf("0x1.19cd3fe8e405dp-1"), hf("0x1.008f6258fc98fp-55")],
    [hf("0x1.220b5ef047825p-1"), hf("-0x1.462af7ceb7de6p-58")],
    [hf("0x1.2a6709a74f289p-1"), hf("-0x1.1184dfd78b472p-56")],
    [hf("0x1.32e1889047ffdp-1"), hf("0x1.9141876dc40c5p-56")],
    [hf("0x1.3b7c3289ed6f3p-1"), hf("0x1.481c20189726cp-55")],
    [hf("0x1.44386db9ce5dbp-1"), hf("0x1.2e851bd025441p-55")],
    [hf("0x1.4d17b087b265dp-1"), hf("0x1.13ada9b8bc419p-56")],
    [hf("0x1.561b82ab7f99p-1"), hf("-0x1.05b4c3c4cbee8p-55")],
    [hf("0x1.5f457e4f4812ep-1"), hf("-0x1.5619249bd96f1p-55")],
    [hf("0x1.6897514751db6p-1"), hf("-0x1.b0a0fbcafc671p-57")],
    [hf("0x1.7212be621be6dp-1"), hf("-0x1.19ff2dc66da45p-55")],
    [hf("0x1.7bb99ed2990cfp-1"), hf("0x1.1320449592d92p-55")],
    [hf("0x1.858de3b716571p-1"), hf("-0x1.1fddcd2f3da8ep-55")],
    [hf("0x1.8f9197bf85eebp-1"), hf("0x1.d44a42e35cc97p-57")],
    [hf("0x1.99c6e0f634394p-1"), hf("-0x1.585a178b4a18dp-56")],
    [hf("0x1.a43002ae4285p-1"), hf("0x1.f95a531b3a97p-57")],
    [hf("0x1.aecf5f9ba35a6p-1"), hf("-0x1.96c2d43ca3392p-60")],
    [hf("0x1.b9a77c18c1af2p-1"), hf("-0x1.a5bed94b05defp-57")],
    [hf("0x1.c4bb009e77983p-1"), hf("0x1.54509d2bff511p-59")],
    [hf("0x1.d00cbc7384d2ep-1"), hf("-0x1.b4c867cef300cp-57")],
    [hf("0x1.db9fa89953fcfp-1"), hf("-0x1.ddfac663d6bc6p-62")],
    [hf("0x1.e776eafc91706p-1"), hf("-0x1.a510683ff7cb6p-56")],
    [hf("0x1.f395d9f0e3c92p-1"), hf("0x1.4fdcd8e4e871p-59")],
    [hf("0x1p+0"), hf("0x0p+0")],
    [hf("0x1.065c900aaf2d8p+0"), hf("-0x1.deec7fc9042adp-55")],
    [hf("0x1.0ce29d0883c99p+0"), hf("-0x1.395ae45e0657dp-55")],
    [hf("0x1.139447e6a86eep+0"), hf("0x1.332cf301a97f3p-55")],
    [hf("0x1.1a73d55278c4bp+0"), hf("-0x1.6cc8c4b78213bp-55")],
    [hf("0x1.2183b0c4573ffp+0"), hf("0x1.70a90841da57ap-55")],
    [hf("0x1.28c66fdaf8f09p+0"), hf("-0x1.ba39bad450eep-57")],
    [hf("0x1.303ed61109e2p+0"), hf("-0x1.8692946d9f93cp-55")],
    [hf("0x1.37efd8d87607ep+0"), hf("0x1.3b711bf765b58p-57")],
    [hf("0x1.3fdca42847507p+0"), hf("0x1.c21387985b081p-56")],
    [hf("0x1.48089f8bf42ccp+0"), hf("-0x1.7ddb19d3d0efcp-55")],
    [hf("0x1.507773c537eadp+0"), hf("-0x1.f5e354cf971f3p-56")],
    [hf("0x1.592d11142fa55p+0"), hf("-0x1.00f0ad675330dp-56")],
    [hf("0x1.622db63c8ecc2p+0"), hf("-0x1.2c93f50ab2c0ep-55")],
    [hf("0x1.6b7df862652p+0"), hf("0x1.bec391adc37d5p-56")],
    [hf("0x1.7522cbdd428a8p+0"), hf("-0x1.9686ddc9ffcf5p-57")],
    [hf("0x1.7f218e25a7461p+0"), hf("-0x1.8d16529514246p-56")],
    [hf("0x1.89801106cc709p+0"), hf("-0x1.092f51e9c2803p-55")],
    [hf("0x1.9444a7462122ap+0"), hf("-0x1.07c06755404c4p-55")],
    [hf("0x1.9f7632fa9e871p+0"), hf("0x1.02e0d43abc92bp-55")],
    [hf("0x1.ab1c35d8a74eap+0"), hf("0x1.d0184e48af6f7p-58")],
    [hf("0x1.b73ee3c3ef16ap+0"), hf("0x1.73be957380bc2p-56")],
    [hf("0x1.c3e738086bc0fp+0"), hf("-0x1.02b6e26c84462p-56")],
    [hf("0x1.d11f0dae40609p+0"), hf("0x1.25c4f3ffa6e1fp-58")],
    [hf("0x1.def13b73c1406p+0"), hf("-0x1.e302db3c6823fp-58")],
    [hf("0x1.ed69b4153a45dp+0"), hf("0x1.3207830326c0ep-56")],
    [hf("0x1.fc95abad6cf4ap+0"), hf("-0x1.6308cee7927bfp-57")],
    [hf("0x1.0641e192ceab3p+1"), hf("-0x1.0147ebf0df4c5p-56")],
    [hf("0x1.0ea21d716fbf7p+1"), hf("-0x1.168533cc41d8bp-56")],
    [hf("0x1.17749711a6679p+1"), hf("-0x1.52a0b0333e9c5p-57")],
    [hf("0x1.20c36c6a7f38ep+1"), hf("0x1.8659eece35395p-57")],
    [hf("0x1.2a99f50fd4f4fp+1"), hf("0x1.20fcad18cb36fp-55")],
    [hf("0x1.3504f333f9de6p+1"), hf("-0x1.52afdbd5a8c74p-56")],
    [hf("0x1.4012ce2586a17p+1"), hf("-0x1.9747a792907d7p-56")],
    [hf("0x1.4bd3d87fe065p+1"), hf("0x1.90c59393b52c8p-56")],
    [hf("0x1.585aa4e1530fap+1"), hf("0x1.af6934f13a3a8p-56")],
    [hf("0x1.65bc6cc825147p+1"), hf("-0x1.8534dcab5ad3ep-59")],
    [hf("0x1.74118e4b6a7c8p+1"), hf("-0x1.555aa8bfca9a1p-56")],
    [hf("0x1.837626d70fdb8p+1"), hf("-0x1.56b3fee9ca72bp-58")],
    [hf("0x1.940ad30abc792p+1"), hf("0x1.4b3fdd4fdc06cp-58")],
    [hf("0x1.a5f59e90600ddp+1"), hf("0x1.285d367c55ddcp-57")],
    [hf("0x1.b9633283b6d14p+1"), hf("-0x1.8712976f17a16p-59")],
    [hf("0x1.ce885653127e7p+1"), hf("-0x1.abe8ab65d49fcp-60")],
    [hf("0x1.e5a3de972a377p+1"), hf("0x1.cd9be81ad764bp-58")],
    [hf("0x1.ff01305ecd8dcp+1"), hf("0x1.742c2922656fap-59")],
    [hf("0x1.0d7dc7cff4c9ep+2"), hf("-0x1.7c842978bee09p-56")],
    [hf("0x1.1d0143e71565fp+2"), hf("0x1.7bc7dea7c3c03p-57")],
    [hf("0x1.2e4ff1626b949p+2"), hf("0x1.aefbe25b404e9p-59")],
    [hf("0x1.41bfee2424771p+2"), hf("-0x1.4bcfaaa95cb2cp-60")],
    [hf("0x1.57be4eaa5e11bp+2"), hf("0x1.0fe741e4ec679p-58")],
    [hf("0x1.70d751908c1b1p+2"), hf("0x1.fe74a5b0ec709p-58")],
    [hf("0x1.8dc25c117782bp+2"), hf("0x1.0ca1c19f710efp-58")],
    [hf("0x1.af73f4ca3310fp+2"), hf("0x1.2867b40ba77d6p-58")],
    [hf("0x1.d7398d15e70dbp+2"), hf("0x1.0fd4e0d4b1547p-57")],
    [hf("0x1.0372fb36b87e2p+3"), hf("0x1.c16c9ecc1621dp-58")],
    [hf("0x1.208dbdae055efp+3"), hf("0x1.6b81a36e75e8cp-58")],
    [hf("0x1.44e6c595afdccp+3"), hf("-0x1.7c22045771848p-58")],
    [hf("0x1.7398c57f3f1adp+3"), hf("0x1.970503be105cp-58")],
    [hf("0x1.b1d03c03d2f7fp+3"), hf("-0x1.f299d010aead2p-60")],
    [hf("0x1.046e9fe60a77ep+4"), hf("0x1.d2b61deff33ecp-58")],
    [hf("0x1.45affed201b55p+4"), hf("0x1.0e84d9567203ap-64")],
    [hf("0x1.b267195b1ffaep+4"), hf("-0x1.ad44b44b92653p-64")],
    [hf("0x1.45e2455e4aaa7p+5"), hf("-0x1.296d577b5e21dp-60")],
    [hf("0x1.45eed6854ce99p+6"), hf("0x1.2db53886013cap-63")],
    [0.0, 0.0],
];

/// The coefficients of a crude piecewise quadratic approximation of atan for a range
/// reduction purpose (to determine the index j).
static C: [[u16; 3]; 31] = [
    [419, 81, 0],
    [500, 81, 0],
    [582, 163, 0],
    [745, 163, 0],
    [908, 326, 0],
    [1234, 326, 0],
    [1559, 651, 0],
    [2210, 650, 1],
    [2860, 1299, 3],
    [4156, 1293, 4],
    [5444, 2569, 24],
    [7989, 2520, 32],
    [10476, 4917, 168],
    [15224, 4576, 200],
    [19601, 8341, 838],
    [27105, 6648, 731],
    [33036, 10210, 1998],
    [41266, 6292, 1117],
    [46469, 7926, 2048],
    [52375, 4038, 849],
    [55587, 4591, 1291],
    [58906, 2172, 479],
    [60612, 2390, 688],
    [62325, 1107, 247],
    [63192, 1207, 349],
    [64056, 556, 124],
    [64491, 605, 175],
    [64923, 278, 62],
    [65141, 303, 88],
    [65358, 139, 31],
    [65467, 151, 44],
];

/// The accurate path, for 0x1p-27 <= |x|; `a` is the approximation of atan(x) from the fast
/// path, so 0x1p-27 <= |a| <= pi/2.
#[inline(always)]
fn as_atan_refine2_with<F: FmaOps>(x: f64, a: f64) -> f64 {
    const CH: [[f64; 2]; 3] = [
        [hf("-0x1.5555555555555p-2"), hf("-0x1.5555555555555p-56")],
        [hf("0x1.999999999999ap-3"), hf("-0x1.999999999bcb8p-57")],
        [hf("-0x1.2492492492492p-3"), hf("-0x1.249242093c016p-57")],
    ];
    const CL: [f64; 4] = [
        hf("0x1.c71c71c71c71cp-4"),
        hf("-0x1.745d1745d1265p-4"),
        hf("0x1.3b13b115bcbc4p-4"),
        hf("-0x1.1107c41ad3253p-4"),
    ];
    let phi = (a.abs() * const { hf("0x1.45f306dc9c883p6") } + 256.5).to_bits();
    // 256.5 < phi < 384.5
    let i = ((phi >> (52 - 8)) & 0xff) as usize; // 0 <= i <= 128
    let (h, hl);
    if i == 128 {
        h = -1.0 / x;
        hl = F::fma(h, x, 1.0) * h;
    } else {
        let ta = A[i][0].copysign(x);
        let zta = x * ta;
        let ztal = F::fma(x, ta, -zta);
        let zmta = x - ta;
        let v = 1.0 + zta;
        let d = 1.0 - v;
        let ev = ((d + zta) - ((d + v) - 1.0)) + ztal;
        let r = 1.0 / v;
        let rl = (F::fma(r, -v, 1.0) - ev * r) * r;
        h = r * zmta;
        hl = F::fma(r, zmta, -h) + rl * zmta;
    }
    let (h2, h2l) = muldd_acc::<F>(h, hl, h, hl);
    let h4 = h2 * h2;
    let (h3, h3l) = muldd_acc::<F>(h, hl, h2, h2l);
    let fl = h2 * ((CL[0] + h2 * CL[1]) + h4 * (CL[2] + h2 * CL[3]));
    let (f, fl) = polydd::<F>(h2, h2l, &CH, fl);
    let (f, fl) = muldd_acc::<F>(h3, h3l, f, fl);
    let (ah, al, at);
    if i == 0 {
        ah = h;
        al = f;
        at = fl;
    } else {
        let mut df = 0.0;
        if i < 128 {
            df = 1.0f64.copysign(x) * A[i][1];
        }
        let id = (i as f64).copysign(x);
        ah = const { hf("0x1.921fb54442dp-7") } * id;
        let al0 = const { hf("0x1.8469898cc518p-55") } * id;
        let at0 = const { hf("-0x1.fc8f8cbb5bf8p-104") } * id;
        let (al1, at1) = adddd(al0, at0, df, 0.0);
        let (al2, at2) = adddd(al1, at1, h, hl);
        let (al3, at3) = adddd(al2, at2, f, fl);
        al = al3;
        at = at3;
    }
    let (v0, v2) = fasttwosum(ah, al);
    let (mut v1, v2) = fasttwosum(v2, at);
    let ax = x.abs();
    let t0 = v0.to_bits();
    let mut t1 = v1.to_bits();
    if ((t1.wrapping_add(1)) & (!0u64 >> 12)) <= 2
        || ((t0 >> 52) & 0x7ff).wrapping_sub((t1 >> 52) & 0x7ff) > 103
    {
        const DB: [[f64; 3]; 12] = [
            [
                hf("0x1.0dc89a3b5501p-7"),
                hf("0x1.0dc70ac228717p-7"),
                hf("0x1p-61"),
            ],
            [
                hf("0x1.e3fb41d2d226p-8"),
                hf("0x1.e3f9013a852f8p-8"),
                hf("0x1p-62"),
            ],
            [
                hf("0x1.7ba49f739829fp-1"),
                hf("0x1.46ac372243536p-1"),
                hf("0x1p-109"),
            ],
            [
                hf("0x1.a933fe176b375p-3"),
                hf("0x1.a33f32ac5ceb5p-3"),
                hf("-0x1p-112"),
            ],
            [
                hf("0x1.bb04a79820063p-8"),
                hf("0x1.bb02ed5c5e956p-8"),
                hf("-0x1p-115"),
            ],
            [
                hf("0x1.cd30a9499618bp-8"),
                hf("0x1.cd2eb65f92a46p-8"),
                hf("-0x1p-112"),
            ],
            [
                hf("0x1.f44aa37b8e66bp-7"),
                hf("0x1.f440b04187c87p-7"),
                hf("-0x1p-112"),
            ],
            [
                hf("0x1.fd2ac95e57ef9p-8"),
                hf("0x1.fd2829febc03ap-8"),
                hf("-0x1p-112"),
            ],
            [
                hf("0x1.6419079bbf601p-6"),
                hf("0x1.640aade8f5427p-6"),
                hf("0x1p-114"),
            ],
            [
                hf("0x1.7ba49f739829fp-1"),
                hf("0x1.46ac372243536p-1"),
                hf("0x1p-110"),
            ],
            [
                hf("0x1.d768804487b07p-3"),
                hf("0x1.cf5676f373ec1p-3"),
                hf("-0x1p-110"),
            ],
            [
                hf("0x1.bb04a79820063p-8"),
                hf("0x1.bb02ed5c5e956p-8"),
                hf("-0x1p-115"),
            ],
        ];
        for row in DB.iter() {
            if ax == row[0] {
                return row[1].copysign(x) + 1.0f64.copysign(x) * row[2];
            }
        }
        if t1 & (!0u64 >> 12) == 0 {
            let w = v2.to_bits();
            if (w ^ t1) >> 63 != 0 {
                t1 = t1.wrapping_sub(1);
            } else {
                t1 = t1.wrapping_add(1);
            }
            v1 = f64::from_bits(t1);
        }
    }
    v1 + v0
}

multiversion! {
    fn as_atan_refine2(x: f64, a: f64) -> f64 = as_atan_refine2_with;
}

#[inline(always)]
fn atan_with<F: FmaOps>(x: f64) -> f64 {
    const CH: [f64; 4] = [
        hf("0x1p+0"),
        hf("-0x1.555555555552bp-2"),
        hf("0x1.9999999069c2p-3"),
        hf("-0x1.248d2c8444ac6p-3"),
    ];
    let t = x.to_bits();
    let at = t & (!0u64 >> 1); // |x|
    let mut i = (at >> 51) as i64 - 2030; // -2030 <= i <= 2065
    if at < 0x3f7b21c475e6362a {
        // |x| < 0x1.b21c475e6362ap-8
        if at == 0 {
            return x; // atan(+/-0) = +/-0
        }
        const CH2: [f64; 4] = [
            hf("-0x1.5555555555555p-2"),
            hf("0x1.99999999998c1p-3"),
            hf("-0x1.249249176aecp-3"),
            hf("0x1.c711fd121ae8p-4"),
        ];
        if at < 0x3e40000000000000 {
            // |x| < 0x1p-27
            return F::fma(const { hf("-0x1p-54") }, x, x);
        }
        let x2 = x * x;
        let x3 = x * x2;
        let x4 = x2 * x2;
        let f = x3 * ((CH2[0] + x2 * CH2[1]) + x4 * (CH2[2] + x2 * CH2[3]));
        let epsp = f * const { hf("0x1.6p-50") };
        let epsm = f * const { hf("0x1.6p-51") };
        let ub = (f + epsp) + x;
        let lb = (f - epsm) + x;
        if ub == lb {
            return ub;
        }
        return as_atan_refine2(x, ub);
    }
    let (h, ah, mut al);
    if at > 0x4062ded8e34a9035 {
        // |x| > 0x1.2ded8e34a9035p+7
        ah = const { hf("0x1.921fb54442d18p+0") }.copysign(x);
        al = const { hf("0x1.1a62633145c07p-54") }.copysign(x);
        if at >= 0x434d02967c31cdb5 {
            // |x| >= 0x1.d02967c31cdb5p+53
            if at > 0x7ff << 52 {
                return x + x; // NaN
            }
            return ah + al;
        }
        h = -1.0 / x;
    } else {
        // now 0x1.b21c475e6362ap-8 <= |x| <= 0x1.2ded8e34a9035p+7, thus 1 <= i <= 30
        let u = t & (!0u64 >> 13);
        let ut = u >> (51 - 16);
        let ut2 = (ut * ut) >> 16;
        let c = &C[i as usize];
        i = ((((c[0] as u64) << 16) + ut * c[1] as u64 - ut2 * c[2] as u64) >> (16 + 9)) as i64;
        let ta = 1.0f64.copysign(x) * A[i as usize][0];
        let id = 1.0f64.copysign(x) * i as f64;
        al = 1.0f64.copysign(x) * A[i as usize][1] + const { hf("0x1.8469898cc517p-55") } * id;
        h = (x - ta) / (1.0 + x * ta);
        ah = const { hf("0x1.921fb54442dp-7") } * id;
    }
    let h2 = h * h;
    let h4 = h2 * h2;
    let f = (CH[0] + h2 * CH[1]) + h4 * (CH[2] + h2 * CH[3]);
    al = F::fma(h, f, al);
    let e = h * const { hf("0x3.fp-52") };
    let ub = (al + e) + ah;
    let lb = (al - e) + ah;
    if ub == lb {
        return ub;
    }
    as_atan_refine2(x, ub)
}

multiversion! {
    /// The arc tangent, correctly rounded (round to nearest, ties to even): CORE-MATH
    /// `cr_atan`. The same bits on every target. `Math.atan` (§21.3.2.6): NaN gives NaN, ±0
    /// gives ±0, ±∞ gives ±π/2 (rounded).
    pub fn atan(x: f64) -> f64 = atan_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atan_special_values() {
        assert!(atan(f64::NAN).is_nan());
        assert_eq!(atan(0.0).to_bits(), 0);
        assert_eq!(atan(-0.0).to_bits(), 1 << 63);
        assert_eq!(atan(f64::INFINITY), std::f64::consts::FRAC_PI_2);
        assert_eq!(atan(f64::NEG_INFINITY), -std::f64::consts::FRAC_PI_2);
        assert_eq!(atan(1.0), std::f64::consts::FRAC_PI_4);
    }
}
