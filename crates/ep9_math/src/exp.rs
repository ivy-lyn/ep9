//! `Math.exp`: the correctly rounded e^x (H36), a port of CORE-MATH `cr_exp`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/exp/exp.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded exponential function for binary64 values.
//! Copyright (c) 2022-2026 Alexei Sibidanov.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! (The C notice also gives the author's e-mail address; ep9's public tree carries no e-mail
//! addresses, so it is left out here and in `THIRD-PARTY-LICENSE`.)
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the tables keep their C text through [`crate::hf`]. ep9 changes:
//! - rounding to nearest only (JS); no `errno`, no floating-point exception flags, so the
//!   `volatile` overflow and underflow products become their round-to-nearest values (+∞, +0)
//!   and `as_todenormal` only clears the exponent bits;
//! - every `__builtin_fma` is [`FmaOps::fma`] (hardware FMA or the exact software FMA, the same
//!   bits), and there is no FMA contraction anywhere (CORE-MATH README, "FMA contraction":
//!   the code works with contraction everywhere or nowhere);
//! - `roundeven_finite` of a value below 2^51 in magnitude is the 1.5 * 2^52 trick, which
//!   rounds ties to even in the default rounding mode with no libm call (`roundeven` is a libm
//!   symbol on x86-64 baseline; `cargo xtask fp-check` bans it);
//! - `as_ldexp` adds to the exponent bits (the C non-x86 branch).

use crate::exact::FmaOps;
use crate::hf;

/// x + y = s + e exactly, for |x| >= |y| (or x = 0).
#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

#[inline(always)]
fn fastsum(xh: f64, xl: f64, yh: f64, yl: f64) -> (f64, f64) {
    let (sh, sl) = fasttwosum(xh, yh);
    (sh, (xl + yl) + sl)
}

#[inline(always)]
fn muldd<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahhh = ch * xh;
    let l = (ch * xl + cl * xh) + F::fma(ch, xh, -ahhh);
    (ahhh, l)
}

/// Algorithm 4 of Jeannerod, Joldes, Louvet, Muller, "Extended-Precision FMA under
/// Parameterized Double-Word Overlap" (Arith 2026), with CORE-MATH's two changes: `g` and the
/// low part are a multiplication and an addition, not FMAs.
#[inline(always)]
fn fast_fma_dw<F: FmaOps>(ah: f64, al: f64, bh: f64, bl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let dh = F::fma(ah, bh, ch);
    let t = ch - dh;
    let e = F::fma(ah, bh, t);
    let f = e + cl;
    let g = ah * bl + f;
    (dh, al * bh + g)
}

#[inline(always)]
fn opolydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]]) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, mut cl) = (c[i][0], c[i][1]);
    while i > 0 {
        i -= 1;
        (ch, cl) = fast_fma_dw::<F>(xh, xl, ch, cl, c[i][0], c[i][1]);
    }
    (ch, cl)
}

/// x rounded to an integer, ties to even, for |x| < 2^51 (C `roundeven_finite` on the inputs
/// that exp gives it: |x| < 745 * 2^12.53).
#[inline(always)]
fn roundeven_small(x: f64) -> f64 {
    const MAGIC: f64 = 6_755_399_441_055_744.0; // 1.5 * 2^52
    (x + MAGIC) - MAGIC
}

/// x * 2^i, by adding i to the exponent bits (no overflow or underflow on the exp inputs).
#[inline(always)]
fn as_ldexp(x: f64, i: i64) -> f64 {
    f64::from_bits(x.to_bits().wrapping_add((i as u64) << 52))
}

/// Sets the exponent bits of x to 0 (the subnormal range).
#[inline(always)]
fn as_todenormal(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & (!0u64 >> 12))
}

/// The exceptional cases for which the accurate path does not give the correctly rounded value
/// for at least one rounding mode, sorted by the first value (x, then the result as a sum).
/// Warning (C): the values are multiplied by 2^ie afterwards.
#[rustfmt::skip]
static DB: [[f64; 3]; 21] = [
  [hf("-0x1.8f80e06f3a04cp+4"), hf("0x1.f80aafa92b498p+0"), hf("-0x1.191595b7e88edp-106")],
  [hf("-0x1.daf3afee51991p+2"), hf("0x1.39c6240f78d7p+0"), hf("-0x1.2b74a3833bf1p-105")],
  [hf("-0x1.07e05e6e03b83p-2"), hf("0x1.8bb0d03b36f1cp+0"), hf("0x1.ffffffffffffep-54")],
  [hf("-0x1.daf693d64fadap-4"), hf("0x1.c7f14af0a08ebp+0"), hf("-0x1.2aa7e4ef70195p-108")],
  [hf("-0x1.a8f783d749a8fp-4"), hf("0x1.cd8abd4de5c33p+0"), hf("0x1.fffffffffffffp-54")],
  [hf("-0x1.40a990279c2cp-6"), hf("0x1.f613a44b93cdcp+0"), hf("0x1.ffffffffffffdp-54")],
  [hf("-0x1.a2fefefd580dfp-13"), hf("0x1.ffe5d0bb7eabfp+0"), hf("0x1.81905a57d6a44p-111")],
  [hf("-0x1.664716b68a409p-14"), hf("0x1.fff4cde6a0bfbp+0"), hf("-0x1.fffffffffffffp-54")],
  [hf("-0x1.0401ae48409b5p-28"), hf("0x1.ffffffdf7fca3p-1"), hf("0x1.fffffffffffffp-55")],
  [hf("-0x1.ff171507f8ba5p-30"), hf("0x1.fffffff007475p-1"), hf("0x1.fffffffffffffp-55")],
  [hf("-0x1.85e60704a3a9cp-30"), hf("0x1.fffffff3d0cfdp-1"), hf("-0x1.fffffffffffffp-55")],
  [hf("-0x1.4bd46601ae1efp-31"), hf("0x1.fffffffad0ae7p-1"), hf("-0x1.fffffffffffffp-55")],
  [hf("0x1p-53"), hf("0x1.0000000000001p+0"), hf("-0x1.fffffffffffffp-54")],
  [hf("0x1.ffff7fffe0001p-36"), hf("0x1.000000001ffffp+0"), hf("0x1.fffffffffffffp-54")],
  [hf("0x1.8cd99fffb319cp-34"), hf("0x1.0000000063367p+0"), hf("-0x1.fffffffffffffp-54")],
  [hf("0x1.413f33fe6ce07p-31"), hf("0x1.00000002827e7p+0"), hf("-0x1.fffffffffffffp-54")],
  [hf("0x1.f8e165f8388f7p-30"), hf("0x1.00000007e3859p+0"), hf("0x1.fffffffffffffp-54")],
  [hf("0x1.282aae3169ee7p-20"), hf("0x1.00001282ab8e7p+0"), hf("-0x1.fffffffffffffp-54")],
  [hf("0x1.e5ba92aa9b1efp-20"), hf("0x1.00001e5baaf77p+0"), hf("0x1.ffffffffffffep-54")],
  [hf("0x1.6a4d1af9cc989p-8"), hf("0x1.016b4df3299d7p+0"), hf("0x1.ffffffffffffep-54")],
  [hf("0x1.aca7ae8da5a7bp+0"), hf("0x1.557d4acd7e557p+0"), hf("-0x1.fffffffffffffp-54")],
];

#[inline(never)]
fn as_exp_database(x: f64, f: f64) -> f64 {
    let (mut a, mut b) = (0usize, DB.len());
    // Binary search with the invariant DB[a][0] <= x < DB[b][0].
    while a + 1 < b {
        let m = (a + b) / 2;
        if x < DB[m][0] {
            b = m;
        } else {
            a = m;
        }
    }
    if x == DB[a][0] {
        return DB[a][1] + DB[a][2];
    }
    f
}

/// For 0 <= i < 2^6, `T0[i]` is a double-double approximation of 2^(i/2^6), as (low, high).
/// Some values differ slightly from the double-double rounding to nearest, to minimize the
/// number of exceptional cases (`optimize()` in CORE-MATH exp.sage).
#[rustfmt::skip]
static T0: [[f64; 2]; 64] = [
  [hf("0x0p+0"), hf("0x1p+0")], [hf("-0x1.19083535b085cp-56"), hf("0x1.02c9a3e778061p+0")],
  [hf("0x1.d73e2a475b466p-55"), hf("0x1.059b0d3158574p+0")], [hf("0x1.186be4bb284fdp-57"), hf("0x1.0874518759bc8p+0")],
  [hf("0x1.8a62e4adc610bp-54"), hf("0x1.0b5586cf9890fp+0")], [hf("0x1.03a1727c57b55p-59"), hf("0x1.0e3ec32d3d1a2p+0")],
  [hf("-0x1.6c51039449b3ap-54"), hf("0x1.11301d0125b51p+0")], [hf("-0x1.32fbf9af1369ep-54"), hf("0x1.1429aaea92dep+0")],
  [hf("-0x1.19041b9d78a76p-55"), hf("0x1.172b83c7d517bp+0")], [hf("0x1.e5b4c7b4968e4p-55"), hf("0x1.1a35beb6fcb75p+0")],
  [hf("0x1.e016e00a2643cp-54"), hf("0x1.1d4873168b9aap+0")], [hf("0x1.dc775814a8496p-55"), hf("0x1.2063b88628cd6p+0")],
  [hf("0x1.9b07eb6c70572p-54"), hf("0x1.2387a6e756238p+0")], [hf("0x1.2bd339940e9dap-55"), hf("0x1.26b4565e27cddp+0")],
  [hf("0x1.612e8afad1255p-55"), hf("0x1.29e9df51fdee1p+0")], [hf("0x1.0024754db41d4p-54"), hf("0x1.2d285a6e4030bp+0")],
  [hf("0x1.6f46ad23182e4p-55"), hf("0x1.306fe0a31b715p+0")], [hf("0x1.32721843659a6p-54"), hf("0x1.33c08b26416ffp+0")],
  [hf("-0x1.63aeabf42eaep-54"), hf("0x1.371a7373aa9cbp+0")], [hf("-0x1.5e436d661f5dbp-56"), hf("0x1.3a7db34e59ff7p+0")],
  [hf("0x1.ada0911f09ebap-55"), hf("0x1.3dea64c123422p+0")], [hf("-0x1.ef3691c309278p-58"), hf("0x1.4160a21f72e2ap+0")],
  [hf("0x1.89b7a04ef80dp-59"), hf("0x1.44e086061892dp+0")], [hf("0x1.3c1a3b69062fp-56"), hf("0x1.486a2b5c13cdp+0")],
  [hf("0x1.d4397afec42e2p-56"), hf("0x1.4bfdad5362a27p+0")], [hf("-0x1.4b309d25957e3p-54"), hf("0x1.4f9b2769d2ca7p+0")],
  [hf("-0x1.07abe1db13cabp-55"), hf("0x1.5342b569d4f82p+0")], [hf("0x1.9bb2c011d93acp-54"), hf("0x1.56f4736b527dap+0")],
  [hf("0x1.6324c054647acp-54"), hf("0x1.5ab07dd485429p+0")], [hf("0x1.ba6f93080e65ep-54"), hf("0x1.5e76f15ad2148p+0")],
  [hf("-0x1.383c17e40b497p-54"), hf("0x1.6247eb03a5585p+0")], [hf("-0x1.bb60987591c34p-54"), hf("0x1.6623882552225p+0")],
  [hf("-0x1.bdd3413b26456p-54"), hf("0x1.6a09e667f3bcdp+0")], [hf("-0x1.bbe3a683c88aap-57"), hf("0x1.6dfb23c651a2fp+0")],
  [hf("-0x1.16e4786887a9ap-55"), hf("0x1.71f75e8ec5f74p+0")], [hf("-0x1.0245957316dd4p-54"), hf("0x1.75feb564267c9p+0")],
  [hf("-0x1.41577ee04993p-55"), hf("0x1.7a11473eb0187p+0")], [hf("0x1.05d02ba15797ep-56"), hf("0x1.7e2f336cf4e62p+0")],
  [hf("-0x1.d4c1dd41532d8p-54"), hf("0x1.82589994cce13p+0")], [hf("-0x1.fc6f89bd4f6bap-54"), hf("0x1.868d99b4492edp+0")],
  [hf("0x1.6e9f156864b29p-54"), hf("0x1.8ace5422aa0dbp+0")], [hf("0x1.5cc13a2e3976fp-55"), hf("0x1.8f1ae99157736p+0")],
  [hf("-0x1.75fc781b57ebcp-57"), hf("0x1.93737b0cdc5e5p+0")], [hf("-0x1.d185b7c1b85dp-54"), hf("0x1.97d829fde4e5p+0")],
  [hf("0x1.c7c46b071f2bep-56"), hf("0x1.9c49182a3f09p+0")], [hf("-0x1.359495d1cd532p-54"), hf("0x1.a0c667b5de565p+0")],
  [hf("-0x1.d2f6edb8d41e3p-54"), hf("0x1.a5503b23e255dp+0")], [hf("0x1.0fac90ef7fd32p-54"), hf("0x1.a9e6b5579fdbfp+0")],
  [hf("0x1.7a1cd345dcc82p-54"), hf("0x1.ae89f995ad3adp+0")], [hf("-0x1.2805e3084d708p-57"), hf("0x1.b33a2b84f15fbp+0")],
  [hf("-0x1.5584f7e54ac3bp-56"), hf("0x1.b7f76f2fb5e47p+0")], [hf("0x1.23dd07a2d9e84p-55"), hf("0x1.bcc1e904bc1d2p+0")],
  [hf("0x1.11065895048dep-55"), hf("0x1.c199bdd85529cp+0")], [hf("0x1.2884dff483cadp-54"), hf("0x1.c67f12e57d14bp+0")],
  [hf("0x1.503cbd1e949d9p-56"), hf("0x1.cb720dcef9069p+0")], [hf("-0x1.cbc3743797a9cp-54"), hf("0x1.d072d4a07897cp+0")],
  [hf("0x1.2ed02d75b3706p-55"), hf("0x1.d5818dcfba487p+0")], [hf("0x1.c2300696db533p-54"), hf("0x1.da9e603db3285p+0")],
  [hf("-0x1.1a5cd4f184b5cp-54"), hf("0x1.dfc97337b9b5fp+0")], [hf("0x1.39e8980a9cc9p-55"), hf("0x1.e502ee78b3ff6p+0")],
  [hf("-0x1.e9c23179c2894p-54"), hf("0x1.ea4afa2a490dap+0")], [hf("0x1.dc7f486a4b6b1p-54"), hf("0x1.efa1bee615a27p+0")],
  [hf("0x1.9d3e12dd8a18dp-54"), hf("0x1.f50765b6e454p+0")], [hf("0x1.74853f3a5931fp-55"), hf("0x1.fa7c1819e90d8p+0")],
];

/// For 0 <= i < 2^6, `T1[i]` is a double-double approximation of 2^(i/2^12), as (low, high),
/// with the same kind of adjustments as [`T0`].
#[rustfmt::skip]
static T1: [[f64; 2]; 64] = [
  [hf("0x0p+0"), hf("0x1p+0")], [hf("0x1.ae8e38c59c72cp-54"), hf("0x1.000b175effdc7p+0")],
  [hf("-0x1.7b5d0d58ea8f4p-58"), hf("0x1.00162f3904052p+0")], [hf("0x1.4115cb6b16a8dp-54"), hf("0x1.0021478e11ce6p+0")],
  [hf("-0x1.d7c96f201bb2ep-55"), hf("0x1.002c605e2e8cfp+0")], [hf("0x1.84711d4c35eap-54"), hf("0x1.003779a95f959p+0")],
  [hf("-0x1.0484245243778p-55"), hf("0x1.0042936faa3d8p+0")], [hf("-0x1.4b237da2025fap-54"), hf("0x1.004dadb113dap+0")],
  [hf("-0x1.5e00e62d6b30ep-56"), hf("0x1.0058c86da1c0ap+0")], [hf("0x1.a1d6cedbb948p-54"), hf("0x1.0063e3a559473p+0")],
  [hf("-0x1.4acf197a00141p-54"), hf("0x1.006eff583fc3dp+0")], [hf("-0x1.eaf2ea42391a6p-57"), hf("0x1.007a1b865a8cap+0")],
  [hf("0x1.da93f90835f7ap-56"), hf("0x1.0085382faef83p+0")], [hf("-0x1.6a79084ab0937p-55"), hf("0x1.00905554425d4p+0")],
  [hf("0x1.86364f8fbe8f8p-54"), hf("0x1.009b72f41a12bp+0")], [hf("-0x1.82e8e14e3110bp-55"), hf("0x1.00a6910f3b6fdp+0")],
  [hf("-0x1.4f6b2a7609f72p-55"), hf("0x1.00b1afa5abcbfp+0")], [hf("-0x1.e1a258ea8f71ap-56"), hf("0x1.00bcceb7707ecp+0")],
  [hf("0x1.4362ca5bc26f2p-56"), hf("0x1.00c7ee448ee02p+0")], [hf("0x1.095a56c919d02p-54"), hf("0x1.00d30e4d0c483p+0")],
  [hf("-0x1.406ac4e81a646p-57"), hf("0x1.00de2ed0ee0f5p+0")], [hf("0x1.b5a6902767e08p-54"), hf("0x1.00e94fd0398ep+0")],
  [hf("-0x1.91b206085932p-54"), hf("0x1.00f4714af41d3p+0")], [hf("0x1.427068ab22306p-55"), hf("0x1.00ff93412315cp+0")],
  [hf("0x1.c1d0660524e08p-54"), hf("0x1.010ab5b2cbd11p+0")], [hf("-0x1.e7bdfb3204be8p-54"), hf("0x1.0115d89ff3a8bp+0")],
  [hf("0x1.843aa8b9cbbc6p-55"), hf("0x1.0120fc089ff63p+0")], [hf("-0x1.34104ee7edae8p-56"), hf("0x1.012c1fecd613bp+0")],
  [hf("-0x1.2b6aeb6176892p-56"), hf("0x1.0137444c9b5b5p+0")], [hf("0x1.a8cd33b8a1bb3p-56"), hf("0x1.01426927f5278p+0")],
  [hf("0x1.2edc08e5da99ap-56"), hf("0x1.014d8e7ee8d2fp+0")], [hf("0x1.57ba2dc7e0c72p-55"), hf("0x1.0158b4517bb88p+0")],
  [hf("0x1.b61299ab8cdb8p-54"), hf("0x1.0163da9fb3335p+0")], [hf("-0x1.90565902c5f43p-54"), hf("0x1.016f0169949edp+0")],
  [hf("0x1.70fc41c5c2d54p-55"), hf("0x1.017a28af25567p+0")], [hf("0x1.4b9a6e145d76cp-54"), hf("0x1.018550706ab62p+0")],
  [hf("-0x1.008eff5142bfap-56"), hf("0x1.019078ad6a19fp+0")], [hf("-0x1.77669f033c7dep-54"), hf("0x1.019ba16628de2p+0")],
  [hf("-0x1.09bb78eeead0ap-54"), hf("0x1.01a6ca9aac5f3p+0")], [hf("0x1.371231477ece4p-54"), hf("0x1.01b1f44af9f9ep+0")],
  [hf("0x1.5e7626621eb5ap-56"), hf("0x1.01bd1e77170b4p+0")], [hf("-0x1.bc72b100828a4p-54"), hf("0x1.01c8491f08f08p+0")],
  [hf("-0x1.ce39cbbab8bbep-57"), hf("0x1.01d37442d507p+0")], [hf("0x1.16996709da2e5p-55"), hf("0x1.01de9fe280ac8p+0")],
  [hf("-0x1.c11f5239bf536p-55"), hf("0x1.01e9cbfe113efp+0")], [hf("0x1.e1d4eb5edc6b4p-55"), hf("0x1.01f4f8958c1c6p+0")],
  [hf("-0x1.afb99946ee3fp-54"), hf("0x1.020025a8f6a35p+0")], [hf("-0x1.8f06d8a148a32p-54"), hf("0x1.020b533856324p+0")],
  [hf("-0x1.2bf310fc54eb8p-55"), hf("0x1.02168143b0281p+0")], [hf("-0x1.c95a035eb4177p-54"), hf("0x1.0221afcb09e3ep+0")],
  [hf("-0x1.491793e46834cp-54"), hf("0x1.022cdece68c4fp+0")], [hf("-0x1.3e8d0d9c4908fp-56"), hf("0x1.02380e4dd22adp+0")],
  [hf("-0x1.314aa16278aa4p-54"), hf("0x1.02433e494b755p+0")], [hf("0x1.48daf888e965p-55"), hf("0x1.024e6ec0da046p+0")],
  [hf("0x1.56dc8046821f4p-55"), hf("0x1.02599fb483385p+0")], [hf("0x1.45b42356b9d46p-54"), hf("0x1.0264d1244c719p+0")],
  [hf("-0x1.082ef51b61d7ep-56"), hf("0x1.027003103b10ep+0")], [hf("0x1.2106ed0920a32p-56"), hf("0x1.027b357854772p+0")],
  [hf("-0x1.fd4cf26ea5d11p-54"), hf("0x1.0286685c9e059p+0")], [hf("-0x1.09f8775e78085p-54"), hf("0x1.02919bbd1d1d8p+0")],
  [hf("0x1.64cbba902ca25p-58"), hf("0x1.029ccf99d720ap+0")], [hf("0x1.4383ef231d208p-54"), hf("0x1.02a803f2d170dp+0")],
  [hf("0x1.4a47a505b3a46p-54"), hf("0x1.02b338c811703p+0")], [hf("0x1.e47120223467fp-54"), hf("0x1.02be6e199c811p+0")],
];

/// The degree-1 to 7 coefficients of Q(x) ~ exp(x) - 1 for |x| < 2^-13.528 (double-double).
#[rustfmt::skip]
static CH_ACCURATE: [[f64; 2]; 7] = [
    [hf("0x1p+0"), 0.0],                                   // degree 1
    [hf("0x1p-1"), hf("0x1.712f72ecec2cfp-99")],           // degree 2
    [hf("0x1.5555555555555p-3"), hf("0x1.5555555554d07p-57")], // degree 3
    [hf("0x1.5555555555555p-5"), hf("0x1.55194d28275dap-59")], // degree 4
    [hf("0x1.1111111111111p-7"), hf("0x1.12faa0e1c0f7bp-63")], // degree 5
    [hf("0x1.6c16c16da6973p-10"), hf("-0x1.4ba45ab25d2a3p-64")], // degree 6
    [hf("0x1.a01a019eb7f31p-13"), hf("-0x1.9091d845ecd36p-67")], // degree 7
];

/// The accurate path, for |x| > 2^-54 (smaller |x| is handled at the start of [`exp`]).
#[inline(always)]
fn exp_accurate_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    const S: f64 = hf("0x1.71547652b82fep+12");
    let t = roundeven_small(x * S);
    let jt = t as i64;
    let (i0, i1, ie) = ((jt >> 6) & 0x3f, jt & 0x3f, jt >> 12);
    let (t0h, t0l) = (T0[i0 as usize][1], T0[i0 as usize][0]);
    let (t1h, t1l) = (T1[i1 as usize][1], T1[i1 as usize][0]);
    let (th, tl) = muldd::<F>(t0h, t0l, t1h, t1l);

    // Cody-Waite reduction: |x| < 745 gives |t| < 2^23, and l2h fits in 29 bits, so l2h*t is
    // exact.
    const L2H: f64 = hf("0x1.62e42ffp-13");
    const L2L: f64 = hf("0x1.718432a1b0e26p-47");
    const L2LL: f64 = hf("0x1.9ff0342542fc3p-102");
    let dx = x - L2H * t;
    let dxl = L2L * t;
    let dxll = L2LL * t + F::fma(L2L, t, -dxl);
    let (dxh, dxl) = fasttwosum(dx, dxl);
    let dxl = dxl + dxll;
    let (fh, fl) = opolydd::<F>(dxh, dxl, &CH_ACCURATE);
    let (fh, fl) = muldd::<F>(dxh, dxl, fh, fl);
    let (fh, fl) = muldd::<F>(fh, fl, th, tl);
    let (fh, fl) = fastsum(th, tl, fh, fl);
    if ix > 0xc086232bdd7abcd2 {
        // x < -0x1.6232bdd7abcd2p+9: 2^-1022-ie with -1075 <= ie <= -1022.
        let sub = f64::from_bits(((1 - ie) as u64) << 52);
        let (fh, e) = fasttwosum(sub, fh);
        as_todenormal(fh + (fl + e))
    } else {
        let (fh, fl) = fasttwosum(fh, fl);
        // d <= 4 only for the one argument x = -0x1.07e05e6e03b83p-2 (C comment).
        let d = fl.to_bits().wrapping_add(4) & (!0u64 >> 12);
        let fh = if d <= 4 { as_exp_database(x, fh) } else { fh };
        as_ldexp(fh, ie)
    }
}

multiversion! {
    /// The accurate path of [`exp`] (C `as_exp_accurate`, cold).
    #[cold]
    fn exp_accurate(x: f64) -> f64 = exp_accurate_with;
}

/// The degree-4 approximation 1 + x*(c0 + c1*x + c2*x^2 + c3*x^3) of exp(x) on
/// [-log(2)/2^13, log(2)/2^13], absolute error below 2^-76.173 (Sollya).
#[rustfmt::skip]
const CH_FAST: [f64; 4] = [
    hf("0x1p+0"),
    hf("0x1p-1"),
    hf("0x1.55555557e54ffp-3"),
    hf("0x1.55555553a12f4p-5"),
];

#[inline(always)]
fn exp_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    let aix = ix & (!0u64 >> 1);
    // exp(x) rounds to 1 for |x| <= 2^-54.
    if aix <= 0x3c90000000000000 {
        return 1.0 + x;
    }
    if aix >= 0x40862e42fefa39f0 {
        // |x| >= 0x1.62e42fefa39fp+9
        if aix > 0x7ff0000000000000 {
            return x + x; // NaN
        }
        if aix == 0x7ff0000000000000 {
            return if ix >> 63 != 0 { 0.0 } else { x }; // -∞ gives +0, +∞ gives +∞
        }
        if ix >> 63 == 0 {
            return f64::INFINITY; // x >= 0x1.62e42fefa39fp+9: overflow
        }
        if aix >= 0x40874910d52d3052 {
            return 0.0; // x <= -0x1.74910d52d3052p+9: underflow to +0
        }
    }
    const S: f64 = hf("0x1.71547652b82fep+12");
    let t = roundeven_small(x * S);
    let jt = t as i64;
    let (i0, i1, ie) = ((jt >> 6) & 0x3f, jt & 0x3f, jt >> 12);
    let (t0h, t0l) = (T0[i0 as usize][1], T0[i0 as usize][0]);
    let (t1h, t1l) = (T1[i1 as usize][1], T1[i1 as usize][0]);
    let (th, tl) = muldd::<F>(t0h, t0l, t1h, t1l);
    const L2H: f64 = hf("0x1.62e42ffp-13");
    const L2L: f64 = hf("0x1.718432a1b0e26p-47");
    // Cody-Waite reduction (see the accurate path); |dx| < log(2)/2^13 (C comment).
    let dx = (x - L2H * t) + L2L * t;
    let dx2 = dx * dx;
    let p = (CH_FAST[0] + dx * CH_FAST[1]) + dx2 * (CH_FAST[2] + dx * CH_FAST[3]);
    let fh = th;
    let tx = th * dx;
    let fl = tl + tx * p;
    const EPS: f64 = hf("0x1.27p-62");
    if ix > 0xc086232bdd7abcd2 {
        // Subnormal result: x < -0x1.6232bdd7abcd2p+9.
        let sub = f64::from_bits(((1 - ie) as u64) << 52);
        let (fh, e) = fasttwosum(sub, fh);
        let fl = fl + e;
        let ub = fh + (fl + EPS);
        let lb = fh + (fl - EPS);
        if ub != lb {
            return exp_accurate(x);
        }
        as_todenormal(lb)
    } else {
        let ub = fh + (fl + EPS);
        let lb = fh + (fl - EPS);
        if ub != lb {
            return exp_accurate(x);
        }
        as_ldexp(lb, ie)
    }
}

multiversion! {
    /// e^x, correctly rounded (round to nearest, ties to even): CORE-MATH `cr_exp`. The same bits
    /// on every target. `Math.exp` (§21.3.2.14): NaN gives NaN, +∞ gives +∞, -∞ gives +0, ±0
    /// gives 1.
    pub fn exp(x: f64) -> f64 = exp_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn exp_special_values() {
        assert!(exp(f64::NAN).is_nan());
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(bits(exp(f64::NEG_INFINITY)), 0);
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(-0.0), 1.0);
        assert_eq!(exp(710.0), f64::INFINITY);
        assert_eq!(bits(exp(-746.0)), 0);
        assert_eq!(exp(1.0), std::f64::consts::E);
    }

    /// CORE-MATH worst cases (`src/binary64/exp/exp.wc`: the first 4 and 8 drawn at random),
    /// with the MPFR 4.2 results (gmpy2, binary64 context, round to nearest).
    #[test]
    fn exp_core_math_worst_cases() {
        for (x, want) in [
            ("0x1.62e42fefa39efp+9", "0x1.fffffffffff2ap+1023"),
            ("-0x1.74910d52d3051p+9", "0x0.0000000000001p-1022"),
            ("-0x1.da8f75c935b64p-3", "0x1.961a3fec71e83p-1"),
            ("-0x1.b9fdb69d50b1ep-12", "0x1.ffc8c3442cfe7p-1"),
            ("-0x1.5d00000000078p-45", "0x1.ffffffffffea3p-1"),
            ("-0x1.6bae3b7ca1989p+9", "0x0.00000018ed39ap-1022"),
            ("-0x1.104ae8bfd5495p-12", "0x1.ffddf7c480df0p-1"),
            ("-0x1.61f6283e6d60dp+9", "0x1.9af5704bae887p-1022"),
            ("-0x1.33337e3f22456p-12", "0x1.ffd99b00d36fbp-1"),
            ("-0x1.7a6008d179fdfp-28", "0x1.ffffffd0b3fefp-1"),
        ] {
            assert_eq!(bits(exp(hf(x))), bits(hf(want)), "exp({x})");
        }
        assert_eq!(exp(hf("0x1.62e42fefa39f0p+9")), f64::INFINITY);
        assert_eq!(bits(exp(hf("-0x1.74910d52d3052p+9"))), 0);
    }

    /// Each exceptional case of the C table gives its listed result, times 2^ie.
    #[test]
    fn exp_database_cases() {
        for row in DB.iter() {
            let ie = (roundeven_small(row[0] * hf("0x1.71547652b82fep+12")) as i64) >> 12;
            let want = as_ldexp(row[1] + row[2], ie);
            assert_eq!(bits(exp(row[0])), bits(want), "exp({:e})", row[0]);
        }
    }

    /// The hardware and the software FMA give the same bits (H36: one result on every target).
    #[test]
    fn exp_soft_equals_hw() {
        let mut s = 0x9e3779b97f4a7c15u64;
        for _ in 0..200_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = f64::from_bits(s & 0xc08fffffffffffff); // |x| < 1024, both signs
            let hw = exp_with::<crate::exact::HwFma>(x);
            let sw = exp_with::<crate::exact::SoftFma>(x);
            assert!(
                bits(hw) == bits(sw) || (hw.is_nan() && sw.is_nan()),
                "{x:e}"
            );
            let hw = exp_accurate_with::<crate::exact::HwFma>(x);
            let sw = exp_accurate_with::<crate::exact::SoftFma>(x);
            if x.abs() > hf("0x1p-54") && x.abs() < 745.0 {
                assert_eq!(bits(hw), bits(sw), "accurate {x:e}");
            }
        }
    }
}
