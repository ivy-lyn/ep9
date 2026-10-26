//! `**` and `Math.pow` (ECMA-262 Number::exponentiate, §6.1.6.1.3; H36 Decision "pow").
//!
//! [`pow_js`] answers the inputs whose result the spec fixes and the exact integer results
//! first (M0 week 4), then calls the correctly rounded core `pow_cr`: a port of CORE-MATH
//! `cr_pow` (M4-P1, H36 Fix-by M1).
//!
//! | Order | Inputs | Result | Source |
//! |---|---|---|---|
//! | 1-4 | `y` NaN; `y` ±0; `x` NaN; `y` ±∞ with \|x\| = 1 | NaN; 1; NaN; NaN | H36 steps 1-4 |
//! | 5 | `y` = 2, 0.5, 1, -1 | `x*x`; `sqrt(x + 0)` (-∞ gives +∞); `x`; `1/x` | H36 step 5: each is the CR result |
//! | 6 | `x` ±0 or ±∞; `y` ±∞; `x` < 0 with `y` not an integer; `x` = 1; `x` = -1 with `y` an integer | the spec values (C99 Annex F agrees after steps 1-4) | H36 step 6, special cases |
//! | 7 | integer `x`, integer `y` in 0..=64, \|result\| <= 2^53 | the exact integer | the M0 `pow_exact` (ep9_vm numconv.rs) |
//! | 8 | everything else: `x` finite, nonzero, not ±1; `y` finite, nonzero | the correctly rounded x^y (round to nearest, ties to even) | `pow_cr`, CORE-MATH `cr_pow` |
//!
//! No `powi`, no repeated multiplication of non-integers, no host `pow` (H36).
//!
//! # The CORE-MATH port
//!
//! Ported for ep9 from CORE-MATH `src/binary64/pow/{pow.c, pow.h, dint.h, qint.h}` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly-rounded power function for two binary64 values.
//! pow.c, pow.h: Copyright (c) 2022-2025 CERN and Inria
//! dint.h, qint.h: Copyright (c) 2022, 2023 CERN and Inria
//! Authors: Tom Hubrecht and Paul Zimmermann
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). Three phases, as in C (reference \[5\] of
//! pow.c: Hubrecht, Jeannerod, Zimmermann, "Towards a correctly-rounded and fast power
//! function in binary64 arithmetic", ARITH 2023):
//!
//! 1. double-double log and exp (`log_1`, `exp_1`), with a rounding test; it answers almost
//!    every input;
//! 2. 128-bit fixed point ([`Dint`], `log_2`, `exp_2`), a rounding test, then the exact and
//!    midpoint cases (`exact_pow`);
//! 3. 256-bit fixed point ([`Qint`], `log_3`, `exp_3`) for the hardest cases.
//!
//! The tables, constants and steps are the C ones; the double tables keep their C text through
//! [`crate::hf`], the 128- and 256-bit tables their C fields. ep9 changes:
//! - rounding to nearest only (JS): no `fegetround` branches, no `errno`, no exception flags
//!   (`get_flag`, `set_flag`, `feraiseexcept` go away), so `is_exact` runs only when phase 1
//!   fails (in C it runs first, for the flags);
//! - the special values are [`pow_js`]'s (JS differs from C99 for `y` NaN, ±∞ with |x| = 1);
//!   the core sees finite nonzero x and y only;
//! - every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software FMA, the same bits),
//!   with no FMA contraction (CORE-MATH README, "FMA contraction");
//! - `roundeven_finite` is the 1.5 * 2^52 trick on the small inputs of phase 1, and `is_int`
//!   reads the bits; the `round` of `is_exact` is a +0.5 and a conversion (no libm call; `cargo
//!   xtask fp-check`);
//! - C `unsigned __int128` is `u128`; a shift by a count the C leaves undefined (only on
//!   inputs the error analysis excludes) gives 0;
//! - the final "Unexpected worst-case found" exit of C (never reached: every hard case is
//!   known) returns the phase-3 rounding.

use crate::exact::FmaOps;
use crate::hf;

/// `x ** y` (Number::exponentiate): the exact or correctly rounded result (module doc).
#[inline(never)]
pub fn pow_js(x: f64, y: f64) -> f64 {
    // H36 steps 1-4 (JS, where JS differs from C99 Annex F).
    if y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && x.abs() == 1.0 {
        return f64::NAN;
    }
    // H36 step 5. Each result is correctly rounded: one IEEE operation on exact inputs.
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        // `x + 0.0` turns -0 into +0 (pow(-0, 0.5) = +0); sqrt(-∞) would be NaN.
        return if x == f64::NEG_INFINITY {
            f64::INFINITY
        } else {
            (x + 0.0).sqrt()
        };
    }
    if y == 1.0 {
        return x;
    }
    if y == -1.0 {
        return 1.0 / x;
    }
    // H36 step 6: the special values, the exact integer path, then the correctly rounded core.
    if let Some(r) = special(x, y) {
        return r;
    }
    if let Some(r) = pow_exact(x, y) {
        return r;
    }
    pow_cr(x, y)
}

/// `y` is an odd integer. Every double of magnitude 2^53 or more is even.
fn is_odd_integer(y: f64) -> bool {
    let a = y.abs();
    a < 9_007_199_254_740_992.0 && a.fract() == 0.0 && (a as u64) & 1 == 1
}

/// ECMA-262 Number::exponentiate steps 4-12 (C99 Annex F F.10.4.4 gives the same values once
/// H36 steps 1-4 have run), plus `x` = ±1 with a finite `y`. Called with `y` not NaN, not ±0,
/// `x` not NaN, and not (`y` ±∞ with |x| = 1).
fn special(x: f64, y: f64) -> Option<f64> {
    let odd = is_odd_integer(y);
    if x.is_infinite() {
        return Some(match (x > 0.0, y > 0.0) {
            (true, true) => f64::INFINITY,
            (true, false) => 0.0,
            (false, true) if odd => f64::NEG_INFINITY,
            (false, true) => f64::INFINITY,
            (false, false) if odd => -0.0,
            (false, false) => 0.0,
        });
    }
    if x == 0.0 {
        let neg = x.is_sign_negative() && odd;
        return Some(match (y > 0.0, neg) {
            (true, false) => 0.0,
            (true, true) => -0.0,
            (false, false) => f64::INFINITY,
            (false, true) => f64::NEG_INFINITY,
        });
    }
    if y.is_infinite() {
        // |x| != 1 here.
        return Some(if (x.abs() > 1.0) == (y > 0.0) {
            f64::INFINITY
        } else {
            0.0
        });
    }
    // x and y are finite and nonzero.
    if x < 0.0 && y.fract() != 0.0 {
        return Some(f64::NAN);
    }
    if x == 1.0 {
        return Some(1.0);
    }
    if x == -1.0 {
        // y is an integer here (the line above returned otherwise).
        return Some(if odd { -1.0 } else { 1.0 });
    }
    None
}

/// Number::exponentiate for the cases M0 could answer exactly: the spec's special cases for NaN
/// and zero exponents, exponent 1, and integer bases with small non-negative integer exponents
/// whose result is an integer of at most 2^53. `None` for everything else (H36: no libm `pow`).
///
/// Moved unchanged from ep9_vm `numconv::pow_exact` (M0 week 4). [`pow_js`] runs it before the
/// correctly rounded core.
fn pow_exact(base: f64, exp: f64) -> Option<f64> {
    if exp.is_nan() {
        return Some(f64::NAN);
    }
    if exp == 0.0 {
        return Some(1.0);
    }
    if base.is_nan() {
        return Some(f64::NAN);
    }
    if exp == 1.0 {
        return Some(base);
    }
    if base == 0.0 && exp > 0.0 && exp.fract() == 0.0 {
        let odd = crate::fmod(exp, 2.0) == 1.0;
        return Some(if base.is_sign_negative() && odd {
            -0.0
        } else {
            0.0
        });
    }
    if base.fract() != 0.0 || !(0.0..=64.0).contains(&exp) || exp.fract() != 0.0 {
        return None;
    }
    if base.abs() > 9_007_199_254_740_992.0 {
        return None;
    }
    let b = base as i128;
    let mut r: i128 = 1;
    for _ in 0..exp as u32 {
        r = r.checked_mul(b)?;
        if r.unsigned_abs() > 1u128 << 53 {
            return None;
        }
    }
    Some(r as f64)
}

// ---------------------------------------------------------------------------------------------
// The correctly rounded core, phase 1 (C pow.c `log_1`, `exp_1` and the start of `cr_pow`;
// pow.h helpers and double tables).
// ---------------------------------------------------------------------------------------------

/// x rounded to an integer, ties to even, for |x| < 2^51 (C `roundeven_finite` on the inputs
/// it gets in phase 1: |x| < 745 * 2^12.53).
#[inline(always)]
fn roundeven_small(x: f64) -> f64 {
    const MAGIC: f64 = 6_755_399_441_055_744.0; // 1.5 * 2^52
    (x + MAGIC) - MAGIC
}

/// C `is_int` (x == roundeven(x)) for any x, from the bits: every double of magnitude 2^52 or
/// more is an integer (also ±∞, as `roundeven` gives); NaN is not; else the fraction bits
/// below the binary point must be 0.
#[inline(always)]
fn is_int(x: f64) -> bool {
    let u = x.to_bits();
    let e = ((u >> 52) & 0x7ff) as i64 - 0x3ff;
    if e >= 52 {
        return !x.is_nan();
    }
    if e < 0 {
        return x == 0.0;
    }
    u & ((1u64 << (52 - e)) - 1) == 0
}

/// hi + lo = a + b exactly, assuming |a| >= |b| (C `fast_two_sum`, round to nearest).
#[inline(always)]
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let hi = a + b;
    let e = hi - a;
    (hi, b - e)
}

/// a + (bh + bl), assuming |a| >= |bh| (C `fast_sum`).
#[inline(always)]
fn fast_sum(a: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, lo) = fast_two_sum(a, bh);
    (hi, lo + bl)
}

/// hi + lo = a * b exactly (C `a_mul`).
#[inline(always)]
fn a_mul<F: FmaOps>(a: f64, b: f64) -> (f64, f64) {
    let hi = a * b;
    (hi, F::fma(a, b, -hi))
}

/// a * (bh + bl) (C `s_mul`).
#[inline(always)]
fn s_mul<F: FmaOps>(a: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, s) = a_mul::<F>(a, bh);
    (hi, F::fma(a, bl, s))
}

/// (ah + al) * (bh + bl) - al * bl (C `d_mul`).
#[inline(always)]
fn d_mul<F: FmaOps>(ah: f64, al: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, s) = a_mul::<F>(ah, bh);
    let t = F::fma(al, bh, s);
    (hi, F::fma(ah, bl, t))
}

/// For 181 <= i <= 362, `INVERSE[i - 181]` is a 9-bit approximation of `1/x[i]`, where
/// `i*2^-8 <= x[i] < (i+1)*2^-8`, such that r*y - 1 is exact for every y in the interval;
/// |r*y - 1| < 0.0040283203125 (C `_INVERSE`, pow.h).
#[rustfmt::skip]
static INVERSE: [f64; 182] = [
    hf("0x1.69p+0"), hf("0x1.67p+0"), hf("0x1.65p+0"), hf("0x1.63p+0"), hf("0x1.61p+0"), hf("0x1.5fp+0"), hf("0x1.5ep+0"),
    hf("0x1.5cp+0"), hf("0x1.5ap+0"), hf("0x1.58p+0"), hf("0x1.56p+0"), hf("0x1.54p+0"), hf("0x1.53p+0"), hf("0x1.51p+0"),
    hf("0x1.4fp+0"), hf("0x1.4ep+0"), hf("0x1.4cp+0"), hf("0x1.4ap+0"), hf("0x1.48p+0"), hf("0x1.47p+0"), hf("0x1.45p+0"),
    hf("0x1.44p+0"), hf("0x1.42p+0"), hf("0x1.4p+0"), hf("0x1.3fp+0"), hf("0x1.3dp+0"), hf("0x1.3cp+0"), hf("0x1.3ap+0"),
    hf("0x1.39p+0"), hf("0x1.37p+0"), hf("0x1.36p+0"), hf("0x1.34p+0"), hf("0x1.33p+0"), hf("0x1.32p+0"), hf("0x1.3p+0"),
    hf("0x1.2fp+0"), hf("0x1.2dp+0"), hf("0x1.2cp+0"), hf("0x1.2bp+0"), hf("0x1.29p+0"), hf("0x1.28p+0"), hf("0x1.27p+0"),
    hf("0x1.25p+0"), hf("0x1.24p+0"), hf("0x1.23p+0"), hf("0x1.21p+0"), hf("0x1.2p+0"), hf("0x1.1fp+0"), hf("0x1.1ep+0"),
    hf("0x1.1cp+0"), hf("0x1.1bp+0"), hf("0x1.1ap+0"), hf("0x1.19p+0"), hf("0x1.17p+0"), hf("0x1.16p+0"), hf("0x1.15p+0"),
    hf("0x1.14p+0"), hf("0x1.13p+0"), hf("0x1.12p+0"), hf("0x1.1p+0"), hf("0x1.0fp+0"), hf("0x1.0ep+0"), hf("0x1.0dp+0"),
    hf("0x1.0cp+0"), hf("0x1.0bp+0"), hf("0x1.0ap+0"), hf("0x1.09p+0"), hf("0x1.08p+0"), hf("0x1.07p+0"), hf("0x1.06p+0"),
    hf("0x1.05p+0"), hf("0x1.04p+0"), hf("0x1.03p+0"), hf("0x1.02p+0"), hf("0x1.00p+0"), hf("0x1.00p+0"), hf("0x1.fdp-1"),
    hf("0x1.fbp-1"), hf("0x1.f9p-1"), hf("0x1.f7p-1"), hf("0x1.f5p-1"), hf("0x1.f3p-1"), hf("0x1.f1p-1"), hf("0x1.fp-1"),
    hf("0x1.eep-1"), hf("0x1.ecp-1"), hf("0x1.eap-1"), hf("0x1.e8p-1"), hf("0x1.e6p-1"), hf("0x1.e5p-1"), hf("0x1.e3p-1"),
    hf("0x1.e1p-1"), hf("0x1.dfp-1"), hf("0x1.ddp-1"), hf("0x1.dcp-1"), hf("0x1.dap-1"), hf("0x1.d8p-1"), hf("0x1.d7p-1"),
    hf("0x1.d5p-1"), hf("0x1.d3p-1"), hf("0x1.d2p-1"), hf("0x1.dp-1"), hf("0x1.cep-1"), hf("0x1.cdp-1"), hf("0x1.cbp-1"),
    hf("0x1.c9p-1"), hf("0x1.c8p-1"), hf("0x1.c6p-1"), hf("0x1.c5p-1"), hf("0x1.c3p-1"), hf("0x1.c2p-1"), hf("0x1.cp-1"),
    hf("0x1.bfp-1"), hf("0x1.bdp-1"), hf("0x1.bcp-1"), hf("0x1.bap-1"), hf("0x1.b9p-1"), hf("0x1.b7p-1"), hf("0x1.b6p-1"),
    hf("0x1.b4p-1"), hf("0x1.b3p-1"), hf("0x1.b1p-1"), hf("0x1.bp-1"), hf("0x1.aep-1"), hf("0x1.adp-1"), hf("0x1.acp-1"),
    hf("0x1.aap-1"), hf("0x1.a9p-1"), hf("0x1.a7p-1"), hf("0x1.a6p-1"), hf("0x1.a5p-1"), hf("0x1.a3p-1"), hf("0x1.a2p-1"),
    hf("0x1.a1p-1"), hf("0x1.9fp-1"), hf("0x1.9ep-1"), hf("0x1.9dp-1"), hf("0x1.9cp-1"), hf("0x1.9ap-1"), hf("0x1.99p-1"),
    hf("0x1.98p-1"), hf("0x1.96p-1"), hf("0x1.95p-1"), hf("0x1.94p-1"), hf("0x1.93p-1"), hf("0x1.91p-1"), hf("0x1.9p-1"),
    hf("0x1.8fp-1"), hf("0x1.8ep-1"), hf("0x1.8dp-1"), hf("0x1.8bp-1"), hf("0x1.8ap-1"), hf("0x1.89p-1"), hf("0x1.88p-1"),
    hf("0x1.87p-1"), hf("0x1.86p-1"), hf("0x1.84p-1"), hf("0x1.83p-1"), hf("0x1.82p-1"), hf("0x1.81p-1"), hf("0x1.8p-1"),
    hf("0x1.7fp-1"), hf("0x1.7ep-1"), hf("0x1.7cp-1"), hf("0x1.7bp-1"), hf("0x1.7ap-1"), hf("0x1.79p-1"), hf("0x1.78p-1"),
    hf("0x1.77p-1"), hf("0x1.76p-1"), hf("0x1.75p-1"), hf("0x1.74p-1"), hf("0x1.73p-1"), hf("0x1.72p-1"), hf("0x1.71p-1"),
    hf("0x1.7p-1"), hf("0x1.6fp-1"), hf("0x1.6ep-1"), hf("0x1.6dp-1"), hf("0x1.6cp-1"), hf("0x1.6bp-1"), hf("0x1.6ap-1"),
];

/// `LOG_INV[i - 181] = (h, l)` approximates `-log(INVERSE[i - 181])` (C `_LOG_INV`, pow.h).
#[rustfmt::skip]
static LOG_INV: [[f64; 2]; 182] = [
    [hf("-0x1.5ff3070a79p-2"), hf("-0x1.e9e439f105039p-45")],
    [hf("-0x1.5a42ab0f4dp-2"), hf("0x1.e63af2df7ba69p-50")],
    [hf("-0x1.548a2c3addp-2"), hf("-0x1.3167e63081cf7p-45")],
    [hf("-0x1.4ec97326p-2"), hf("-0x1.34d7aaf04d104p-45")],
    [hf("-0x1.4900680401p-2"), hf("0x1.8bccffe1a0f8cp-44")],
    [hf("-0x1.432ef2a04fp-2"), hf("0x1.fb129931715adp-44")],
    [hf("-0x1.404308686ap-2"), hf("-0x1.f8ef43049f7d3p-44")],
    [hf("-0x1.3a64c55694p-2"), hf("-0x1.7a71cbcd735dp-44")],
    [hf("-0x1.347dd9a988p-2"), hf("0x1.5594dd4c58092p-45")],
    [hf("-0x1.2e8e2bae12p-2"), hf("0x1.67b1e99b72bd8p-45")],
    [hf("-0x1.2895a13de8p-2"), hf("-0x1.a8d7ad24c13fp-44")],
    [hf("-0x1.22941fbcf8p-2"), hf("0x1.a6976f5eb0963p-44")],
    [hf("-0x1.1f8ff9e48ap-2"), hf("-0x1.7946c040cbe77p-45")],
    [hf("-0x1.1980d2dd42p-2"), hf("-0x1.b7b3a7a361c9ap-45")],
    [hf("-0x1.136870293bp-2"), hf("0x1.d3e8499d67123p-44")],
    [hf("-0x1.1058bf9ae5p-2"), hf("0x1.4ab9d817d52cdp-44")],
    [hf("-0x1.0a324e2739p-2"), hf("-0x1.c6bee7ef4030ep-47")],
    [hf("-0x1.0402594b4dp-2"), hf("-0x1.036b89ef42d7fp-48")],
    [hf("-0x1.fb9186d5e4p-3"), hf("0x1.d572aab993c87p-47")],
    [hf("-0x1.f550a564b8p-3"), hf("0x1.323e3a09202fep-45")],
    [hf("-0x1.e8c0252aa6p-3"), hf("0x1.6805b80e8e6ffp-45")],
    [hf("-0x1.e27076e2bp-3"), hf("0x1.a342c2af0003cp-44")],
    [hf("-0x1.d5c216b4fcp-3"), hf("0x1.1ba91bbca681bp-45")],
    [hf("-0x1.c8ff7c79aap-3"), hf("0x1.7794f689f8434p-45")],
    [hf("-0x1.c2968558c2p-3"), hf("0x1.cfd73dee38a4p-45")],
    [hf("-0x1.b5b519e8fcp-3"), hf("0x1.4b722ec011f31p-44")],
    [hf("-0x1.af3c94e80cp-3"), hf("0x1.a4e633fcd9066p-52")],
    [hf("-0x1.a23bc1fe2cp-3"), hf("0x1.539cd91dc9f0bp-44")],
    [hf("-0x1.9bb362e7ep-3"), hf("0x1.1f2a8a1ce0ffcp-45")],
    [hf("-0x1.8e928de886p-3"), hf("-0x1.a8154b13d72d5p-44")],
    [hf("-0x1.87fa06520cp-3"), hf("-0x1.22120401202fcp-44")],
    [hf("-0x1.7ab890210ep-3"), hf("0x1.bdb9072534a58p-45")],
    [hf("-0x1.740f8f5404p-3"), hf("0x1.0b66c99018aa1p-44")],
    [hf("-0x1.6d60fe719ep-3"), hf("0x1.bc6e557134767p-44")],
    [hf("-0x1.5ff3070a7ap-3"), hf("0x1.8586f183bebf2p-44")],
    [hf("-0x1.59338d9982p-3"), hf("-0x1.0ba68b7555d4ap-48")],
    [hf("-0x1.4ba36f39a6p-3"), hf("0x1.4354bb3f219e5p-44")],
    [hf("-0x1.44d2b6ccb8p-3"), hf("0x1.70cc16135783cp-46")],
    [hf("-0x1.3dfc2b0eccp-3"), hf("-0x1.8a72a62b8c13fp-45")],
    [hf("-0x1.303d718e48p-3"), hf("0x1.680b5ce3ecb05p-50")],
    [hf("-0x1.29552f82p-3"), hf("0x1.5b967f4471dfcp-44")],
    [hf("-0x1.2266f190a6p-3"), hf("0x1.4d20ab840e7f6p-45")],
    [hf("-0x1.1478584674p-3"), hf("-0x1.563451027c75p-46")],
    [hf("-0x1.0d77e7cd08p-3"), hf("-0x1.cb2cd2ee2f482p-44")],
    [hf("-0x1.0671512ca6p-3"), hf("0x1.a47579cdc0a3dp-45")],
    [hf("-0x1.f0a30c0118p-4"), hf("0x1.d599e83368e91p-44")],
    [hf("-0x1.e27076e2bp-4"), hf("0x1.a342c2af0003cp-45")],
    [hf("-0x1.d4313d66ccp-4"), hf("0x1.9454379135713p-45")],
    [hf("-0x1.c5e548f5bcp-4"), hf("-0x1.d0c57585fbe06p-46")],
    [hf("-0x1.a926d3a4acp-4"), hf("-0x1.563650bd22a9cp-44")],
    [hf("-0x1.9ab4246204p-4"), hf("0x1.8a64826787061p-45")],
    [hf("-0x1.8c345d6318p-4"), hf("-0x1.b20f5acb42a66p-44")],
    [hf("-0x1.7da766d7bp-4"), hf("-0x1.2cc844480c89bp-44")],
    [hf("-0x1.60658a9374p-4"), hf("-0x1.0c3b1dee9c4f8p-44")],
    [hf("-0x1.51b073f06p-4"), hf("-0x1.83f69278e686ap-44")],
    [hf("-0x1.42edcbea64p-4"), hf("-0x1.bc0eeea7c9acdp-46")],
    [hf("-0x1.341d7961bcp-4"), hf("-0x1.1d0929983761p-44")],
    [hf("-0x1.253f62f0ap-4"), hf("-0x1.416f8fb69a701p-44")],
    [hf("-0x1.16536eea38p-4"), hf("0x1.47c5e768fa309p-46")],
    [hf("-0x1.f0a30c0118p-5"), hf("0x1.d599e83368e91p-45")],
    [hf("-0x1.d276b8adbp-5"), hf("-0x1.6a423c78a64bp-46")],
    [hf("-0x1.b42dd71198p-5"), hf("0x1.c827ae5d6704cp-46")],
    [hf("-0x1.95c830ec9p-5"), hf("0x1.c148297c5feb8p-45")],
    [hf("-0x1.77458f633p-5"), hf("0x1.181dce586af09p-44")],
    [hf("-0x1.58a5bafc9p-5"), hf("0x1.b2b739570ad39p-45")],
    [hf("-0x1.39e87b9fe8p-5"), hf("-0x1.eafd480ad9015p-44")],
    [hf("-0x1.1b0d98924p-5"), hf("0x1.3401e9ae889bbp-44")],
    [hf("-0x1.f829b0e78p-6"), hf("-0x1.980267c7e09e4p-45")],
    [hf("-0x1.b9fc027bp-6"), hf("0x1.b9a010ae6922ap-44")],
    [hf("-0x1.7b91b07d6p-6"), hf("0x1.3b955b602ace4p-44")],
    [hf("-0x1.3cea44347p-6"), hf("0x1.6a2c432d6a40bp-44")],
    [hf("-0x1.fc0a8b0fcp-7"), hf("-0x1.f1e7cf6d3a69cp-50")],
    [hf("-0x1.7dc475f82p-7"), hf("0x1.eb1245b5da1f5p-44")],
    [hf("-0x1.fe02a6b1p-8"), hf("-0x1.9e23f0dda40e4p-46")],
    [0.0, 0.0],
    [0.0, 0.0],
    [hf("0x1.812121458p-8"), hf("0x1.ad50382973f27p-46")],
    [hf("0x1.41929f968p-7"), hf("0x1.977c755d01368p-46")],
    [hf("0x1.c317384c8p-7"), hf("-0x1.41f33fcefb9fep-44")],
    [hf("0x1.228fb1feap-6"), hf("0x1.713e3284991fep-45")],
    [hf("0x1.63d617869p-6"), hf("0x1.7abf389596542p-47")],
    [hf("0x1.a55f548c6p-6"), hf("-0x1.de0709f2d03c9p-45")],
    [hf("0x1.e72bf2814p-6"), hf("-0x1.8d75149774d47p-45")],
    [hf("0x1.0415d89e78p-5"), hf("-0x1.dddc7f461c516p-44")],
    [hf("0x1.252f32f8dp-5"), hf("0x1.83e9ae021b67bp-45")],
    [hf("0x1.466aed42ep-5"), hf("-0x1.c167375bdfd28p-45")],
    [hf("0x1.67c94f2d48p-5"), hf("0x1.dac20827cca0cp-44")],
    [hf("0x1.894aa149f8p-5"), hf("0x1.9a19a8be97661p-44")],
    [hf("0x1.aaef2d0fbp-5"), hf("0x1.0fc1a353bb42ep-45")],
    [hf("0x1.bbcebfc69p-5"), hf("-0x1.7bf868c317c2ap-46")],
    [hf("0x1.dda8adc68p-5"), hf("-0x1.1b1ac64d9e42fp-45")],
    [hf("0x1.ffa6911ab8p-5"), hf("0x1.3008c98381a8fp-45")],
    [hf("0x1.10e45b3cbp-4"), hf("-0x1.7cf69284a3465p-44")],
    [hf("0x1.2207b5c784p-4"), hf("0x1.49d8cfc10c7bfp-44")],
    [hf("0x1.2aa04a447p-4"), hf("0x1.7a48ba8b1cb41p-44")],
    [hf("0x1.3bdf5a7d2p-4"), hf("-0x1.19bd0ad125895p-44")],
    [hf("0x1.4d3115d208p-4"), hf("-0x1.53a2582f4e1efp-48")],
    [hf("0x1.55e10050ep-4"), hf("0x1.c1d740c53c72ep-47")],
    [hf("0x1.674f089364p-4"), hf("0x1.a79994c9d3302p-44")],
    [hf("0x1.78d02263d8p-4"), hf("0x1.69b5794b69fb7p-47")],
    [hf("0x1.8197e2f41p-4"), hf("-0x1.c0fe460d20041p-44")],
    [hf("0x1.9335e5d594p-4"), hf("0x1.3115c3abd47dap-45")],
    [hf("0x1.a4e7640b1cp-4"), hf("-0x1.e42b6b94407c8p-47")],
    [hf("0x1.adc77ee5bp-4"), hf("-0x1.573b209c31904p-44")],
    [hf("0x1.bf968769fcp-4"), hf("0x1.4218c8d824283p-45")],
    [hf("0x1.d179788218p-4"), hf("0x1.36433b5efbeedp-44")],
    [hf("0x1.da72763844p-4"), hf("0x1.a89401fa71733p-46")],
    [hf("0x1.ec739830ap-4"), hf("0x1.11fcba80cdd1p-44")],
    [hf("0x1.f57bc7d9p-4"), hf("0x1.76a6c9ea8b04ep-46")],
    [hf("0x1.03cdc0a51ep-3"), hf("0x1.81a9cf169fc5cp-44")],
    [hf("0x1.08598b59e4p-3"), hf("-0x1.7e5dd7009902cp-45")],
    [hf("0x1.1178e8227ep-3"), hf("0x1.1ef78ce2d07f2p-45")],
    [hf("0x1.160c8024b2p-3"), hf("0x1.ec2d2a9009e3dp-45")],
    [hf("0x1.1f3b925f26p-3"), hf("-0x1.5f74e9b083633p-46")],
    [hf("0x1.23d712a49cp-3"), hf("0x1.00d238fd3df5cp-46")],
    [hf("0x1.2d1610c868p-3"), hf("0x1.39d6ccb81b4a1p-47")],
    [hf("0x1.31b994d3a4p-3"), hf("0x1.f098ee3a5081p-44")],
    [hf("0x1.3b08b6758p-3"), hf("-0x1.aade8f29320fbp-44")],
    [hf("0x1.3fb45a5992p-3"), hf("0x1.19713c0cae559p-44")],
    [hf("0x1.4913d8333cp-3"), hf("-0x1.53e43558124c4p-44")],
    [hf("0x1.4dc7b897bcp-3"), hf("0x1.c79b60ae1ff0fp-47")],
    [hf("0x1.5737cc9018p-3"), hf("0x1.9baa7a6b887f6p-44")],
    [hf("0x1.5bf406b544p-3"), hf("-0x1.27023eb68981cp-46")],
    [hf("0x1.6574ebe8c2p-3"), hf("-0x1.98c1d34f0f462p-44")],
    [hf("0x1.6a399dabbep-3"), hf("-0x1.8f934e66a15a6p-44")],
    [hf("0x1.6f0128b756p-3"), hf("0x1.577390d31ef0fp-44")],
    [hf("0x1.7898d85444p-3"), hf("0x1.8e67be3dbaf3fp-44")],
    [hf("0x1.7d6903caf6p-3"), hf("-0x1.4c06b17c301d7p-45")],
    [hf("0x1.871213750ep-3"), hf("0x1.328eb42f9af75p-44")],
    [hf("0x1.8beafeb39p-3"), hf("-0x1.73d54aae92cd1p-47")],
    [hf("0x1.90c6db9fccp-3"), hf("-0x1.935f57718d7cap-46")],
    [hf("0x1.9a8778debap-3"), hf("0x1.470fa3efec39p-44")],
    [hf("0x1.9f6c40708ap-3"), hf("-0x1.337d94bcd3f43p-44")],
    [hf("0x1.a454082e6ap-3"), hf("0x1.60a77c81f7171p-44")],
    [hf("0x1.ae2ca6f672p-3"), hf("0x1.7a8d5ae54f55p-44")],
    [hf("0x1.b31d8575bcp-3"), hf("0x1.c794e562a63cbp-44")],
    [hf("0x1.b811730b82p-3"), hf("0x1.e90683b9cd768p-46")],
    [hf("0x1.bd087383bep-3"), hf("-0x1.d4bc4595412b6p-45")],
    [hf("0x1.c6ffbc6fp-3"), hf("0x1.ee138d3a69d43p-44")],
    [hf("0x1.cc000c9db4p-3"), hf("-0x1.d6d585d57aff9p-46")],
    [hf("0x1.d1037f2656p-3"), hf("-0x1.84a7e75b6f6e4p-47")],
    [hf("0x1.db13db0d48p-3"), hf("0x1.2806a847527e6p-44")],
    [hf("0x1.e020cc6236p-3"), hf("-0x1.52b00adb91424p-45")],
    [hf("0x1.e530effe72p-3"), hf("-0x1.fdbdbb13f7c18p-44")],
    [hf("0x1.ea4449f04ap-3"), hf("0x1.5e91663732a36p-44")],
    [hf("0x1.f474b134ep-3"), hf("-0x1.bae49f1df7b5ep-44")],
    [hf("0x1.f991c6cb3cp-3"), hf("-0x1.90d04cd7cc834p-44")],
    [hf("0x1.feb2233eap-3"), hf("0x1.f3418de00938bp-45")],
    [hf("0x1.01eae5626cp-2"), hf("0x1.a43dcfade85aep-44")],
    [hf("0x1.047e60cde8p-2"), hf("0x1.dbdf10d397f3cp-45")],
    [hf("0x1.09aa572e6cp-2"), hf("0x1.b50a1e1734342p-44")],
    [hf("0x1.0c42d67616p-2"), hf("0x1.7188b163ceae9p-45")],
    [hf("0x1.0edd060b78p-2"), hf("0x1.019b52d8435f5p-47")],
    [hf("0x1.1178e8227ep-2"), hf("0x1.1ef78ce2d07f2p-44")],
    [hf("0x1.14167ef367p-2"), hf("0x1.e0c07824daaf5p-44")],
    [hf("0x1.16b5ccbadp-2"), hf("-0x1.23299042d74bfp-44")],
    [hf("0x1.1bf99635a7p-2"), hf("-0x1.1ac89575c2125p-44")],
    [hf("0x1.1e9e16788ap-2"), hf("-0x1.82eaed3c8b65ep-44")],
    [hf("0x1.214456d0ecp-2"), hf("-0x1.caf0428b728a3p-44")],
    [hf("0x1.23ec5991ecp-2"), hf("-0x1.6dbe448a2e522p-44")],
    [hf("0x1.269621134ep-2"), hf("-0x1.1b61f10522625p-44")],
    [hf("0x1.2941afb187p-2"), hf("-0x1.210c2b730e28bp-44")],
    [hf("0x1.2bef07cdc9p-2"), hf("0x1.a9cfa4a5004f4p-45")],
    [hf("0x1.314f1e1d36p-2"), hf("-0x1.8e27ad3213cb8p-45")],
    [hf("0x1.3401e12aedp-2"), hf("-0x1.17c73556e291dp-44")],
    [hf("0x1.36b6776be1p-2"), hf("0x1.16ecdb0f177c8p-46")],
    [hf("0x1.396ce359bcp-2"), hf("-0x1.5839c5663663dp-47")],
    [hf("0x1.3c25277333p-2"), hf("0x1.83b54b606bd5cp-46")],
    [hf("0x1.3edf463c17p-2"), hf("-0x1.f067c297f2c3fp-44")],
    [hf("0x1.419b423d5fp-2"), hf("-0x1.ce379226de3ecp-44")],
    [hf("0x1.44591e053ap-2"), hf("-0x1.6e95892923d88p-47")],
    [hf("0x1.4718dc271cp-2"), hf("0x1.06c18fb4c14c5p-44")],
    [hf("0x1.49da7f3bccp-2"), hf("0x1.07b334daf4b9ap-44")],
    [hf("0x1.4c9e09e173p-2"), hf("-0x1.e20891b0ad8a4p-45")],
    [hf("0x1.4f637ebbaap-2"), hf("-0x1.fc158cb3124b9p-44")],
    [hf("0x1.522ae0738ap-2"), hf("0x1.ebe708164c759p-45")],
    [hf("0x1.54f431b7bep-2"), hf("0x1.a8954c0910952p-46")],
    [hf("0x1.57bf753c8dp-2"), hf("0x1.fadedee5d40efp-46")],
    [hf("0x1.5a8cadbbeep-2"), hf("-0x1.7c79b0af7ecf8p-48")],
    [hf("0x1.5d5bddf596p-2"), hf("-0x1.a0b2a08a465dcp-47")],
    [hf("0x1.602d08af09p-2"), hf("0x1.ebe9176df3f65p-46")],
    [hf("0x1.630030b3abp-2"), hf("-0x1.db623e731aep-45")],
];

/// `T1[i] = (h, l)` approximates 2^(i/2^6) (C `T1`, pow.h).
#[rustfmt::skip]
static T1: [[f64; 2]; 64] = [
    [              hf("0x1p+0"),                 hf("0x0p+0")],
    [hf("0x1.02c9a3e778061p+0"), hf("-0x1.19083535b085dp-56")],
    [hf("0x1.059b0d3158574p+0"),  hf("0x1.d73e2a475b465p-55")],
    [hf("0x1.0874518759bc8p+0"),  hf("0x1.186be4bb284ffp-57")],
    [hf("0x1.0b5586cf9890fp+0"),  hf("0x1.8a62e4adc610bp-54")],
    [hf("0x1.0e3ec32d3d1a2p+0"),  hf("0x1.03a1727c57b53p-59")],
    [hf("0x1.11301d0125b51p+0"), hf("-0x1.6c51039449b3ap-54")],
    [ hf("0x1.1429aaea92dep+0"), hf("-0x1.32fbf9af1369ep-54")],
    [hf("0x1.172b83c7d517bp+0"), hf("-0x1.19041b9d78a76p-55")],
    [hf("0x1.1a35beb6fcb75p+0"),  hf("0x1.e5b4c7b4968e4p-55")],
    [hf("0x1.1d4873168b9aap+0"),  hf("0x1.e016e00a2643cp-54")],
    [hf("0x1.2063b88628cd6p+0"),  hf("0x1.dc775814a8495p-55")],
    [hf("0x1.2387a6e756238p+0"),  hf("0x1.9b07eb6c70573p-54")],
    [hf("0x1.26b4565e27cddp+0"),  hf("0x1.2bd339940e9d9p-55")],
    [hf("0x1.29e9df51fdee1p+0"),  hf("0x1.612e8afad1255p-55")],
    [hf("0x1.2d285a6e4030bp+0"),  hf("0x1.0024754db41d5p-54")],
    [hf("0x1.306fe0a31b715p+0"),  hf("0x1.6f46ad23182e4p-55")],
    [hf("0x1.33c08b26416ffp+0"),  hf("0x1.32721843659a6p-54")],
    [hf("0x1.371a7373aa9cbp+0"), hf("-0x1.63aeabf42eae2p-54")],
    [hf("0x1.3a7db34e59ff7p+0"), hf("-0x1.5e436d661f5e3p-56")],
    [hf("0x1.3dea64c123422p+0"),  hf("0x1.ada0911f09ebcp-55")],
    [hf("0x1.4160a21f72e2ap+0"), hf("-0x1.ef3691c309278p-58")],
    [hf("0x1.44e086061892dp+0"),   hf("0x1.89b7a04ef80dp-59")],
    [ hf("0x1.486a2b5c13cdp+0"),   hf("0x1.3c1a3b69062fp-56")],
    [hf("0x1.4bfdad5362a27p+0"),  hf("0x1.d4397afec42e2p-56")],
    [hf("0x1.4f9b2769d2ca7p+0"), hf("-0x1.4b309d25957e3p-54")],
    [hf("0x1.5342b569d4f82p+0"), hf("-0x1.07abe1db13cadp-55")],
    [hf("0x1.56f4736b527dap+0"),  hf("0x1.9bb2c011d93adp-54")],
    [hf("0x1.5ab07dd485429p+0"),  hf("0x1.6324c054647adp-54")],
    [hf("0x1.5e76f15ad2148p+0"),  hf("0x1.ba6f93080e65ep-54")],
    [hf("0x1.6247eb03a5585p+0"), hf("-0x1.383c17e40b497p-54")],
    [hf("0x1.6623882552225p+0"), hf("-0x1.bb60987591c34p-54")],
    [hf("0x1.6a09e667f3bcdp+0"), hf("-0x1.bdd3413b26456p-54")],
    [hf("0x1.6dfb23c651a2fp+0"), hf("-0x1.bbe3a683c88abp-57")],
    [hf("0x1.71f75e8ec5f74p+0"), hf("-0x1.16e4786887a99p-55")],
    [hf("0x1.75feb564267c9p+0"), hf("-0x1.0245957316dd3p-54")],
    [hf("0x1.7a11473eb0187p+0"), hf("-0x1.41577ee04992fp-55")],
    [hf("0x1.7e2f336cf4e62p+0"),  hf("0x1.05d02ba15797ep-56")],
    [hf("0x1.82589994cce13p+0"), hf("-0x1.d4c1dd41532d8p-54")],
    [hf("0x1.868d99b4492edp+0"), hf("-0x1.fc6f89bd4f6bap-54")],
    [hf("0x1.8ace5422aa0dbp+0"),  hf("0x1.6e9f156864b27p-54")],
    [hf("0x1.8f1ae99157736p+0"),  hf("0x1.5cc13a2e3976cp-55")],
    [hf("0x1.93737b0cdc5e5p+0"), hf("-0x1.75fc781b57ebcp-57")],
    [ hf("0x1.97d829fde4e5p+0"), hf("-0x1.d185b7c1b85d1p-54")],
    [ hf("0x1.9c49182a3f09p+0"),  hf("0x1.c7c46b071f2bep-56")],
    [hf("0x1.a0c667b5de565p+0"), hf("-0x1.359495d1cd533p-54")],
    [hf("0x1.a5503b23e255dp+0"), hf("-0x1.d2f6edb8d41e1p-54")],
    [hf("0x1.a9e6b5579fdbfp+0"),  hf("0x1.0fac90ef7fd31p-54")],
    [hf("0x1.ae89f995ad3adp+0"),  hf("0x1.7a1cd345dcc81p-54")],
    [hf("0x1.b33a2b84f15fbp+0"), hf("-0x1.2805e3084d708p-57")],
    [hf("0x1.b7f76f2fb5e47p+0"), hf("-0x1.5584f7e54ac3bp-56")],
    [hf("0x1.bcc1e904bc1d2p+0"),  hf("0x1.23dd07a2d9e84p-55")],
    [hf("0x1.c199bdd85529cp+0"),  hf("0x1.11065895048ddp-55")],
    [hf("0x1.c67f12e57d14bp+0"),  hf("0x1.2884dff483cadp-54")],
    [hf("0x1.cb720dcef9069p+0"),  hf("0x1.503cbd1e949dbp-56")],
    [hf("0x1.d072d4a07897cp+0"), hf("-0x1.cbc3743797a9cp-54")],
    [hf("0x1.d5818dcfba487p+0"),  hf("0x1.2ed02d75b3707p-55")],
    [hf("0x1.da9e603db3285p+0"),  hf("0x1.c2300696db532p-54")],
    [hf("0x1.dfc97337b9b5fp+0"), hf("-0x1.1a5cd4f184b5cp-54")],
    [hf("0x1.e502ee78b3ff6p+0"),  hf("0x1.39e8980a9cc8fp-55")],
    [hf("0x1.ea4afa2a490dap+0"), hf("-0x1.e9c23179c2893p-54")],
    [hf("0x1.efa1bee615a27p+0"),   hf("0x1.dc7f486a4b6bp-54")],
    [ hf("0x1.f50765b6e454p+0"),  hf("0x1.9d3e12dd8a18bp-54")],
    [hf("0x1.fa7c1819e90d8p+0"),  hf("0x1.74853f3a5931ep-55")],
];

/// `T2[i] = (h, l)` approximates 2^(i/2^12) (C `T2`, pow.h).
#[rustfmt::skip]
static T2: [[f64; 2]; 64] = [
    [              hf("0x1p+0"),                 hf("0x0p+0")],
    [hf("0x1.000b175effdc7p+0"),  hf("0x1.ae8e38c59c72ap-54")],
    [hf("0x1.00162f3904052p+0"), hf("-0x1.7b5d0d58ea8f4p-58")],
    [hf("0x1.0021478e11ce6p+0"),  hf("0x1.4115cb6b16a8ep-54")],
    [hf("0x1.002c605e2e8cfp+0"), hf("-0x1.d7c96f201bb2fp-55")],
    [hf("0x1.003779a95f959p+0"),  hf("0x1.84711d4c35e9fp-54")],
    [hf("0x1.0042936faa3d8p+0"), hf("-0x1.0484245243777p-55")],
    [ hf("0x1.004dadb113dap+0"), hf("-0x1.4b237da2025f9p-54")],
    [hf("0x1.0058c86da1c0ap+0"), hf("-0x1.5e00e62d6b30dp-56")],
    [hf("0x1.0063e3a559473p+0"),  hf("0x1.a1d6cedbb9481p-54")],
    [hf("0x1.006eff583fc3dp+0"), hf("-0x1.4acf197a00142p-54")],
    [hf("0x1.007a1b865a8cap+0"), hf("-0x1.eaf2ea42391a5p-57")],
    [hf("0x1.0085382faef83p+0"),  hf("0x1.da93f90835f75p-56")],
    [hf("0x1.00905554425d4p+0"), hf("-0x1.6a79084ab093cp-55")],
    [hf("0x1.009b72f41a12bp+0"),  hf("0x1.86364f8fbe8f8p-54")],
    [hf("0x1.00a6910f3b6fdp+0"), hf("-0x1.82e8e14e3110ep-55")],
    [hf("0x1.00b1afa5abcbfp+0"), hf("-0x1.4f6b2a7609f71p-55")],
    [hf("0x1.00bcceb7707ecp+0"), hf("-0x1.e1a258ea8f71bp-56")],
    [hf("0x1.00c7ee448ee02p+0"),  hf("0x1.4362ca5bc26f1p-56")],
    [hf("0x1.00d30e4d0c483p+0"),  hf("0x1.095a56c919d02p-54")],
    [hf("0x1.00de2ed0ee0f5p+0"), hf("-0x1.406ac4e81a645p-57")],
    [ hf("0x1.00e94fd0398ep+0"),  hf("0x1.b5a6902767e09p-54")],
    [hf("0x1.00f4714af41d3p+0"), hf("-0x1.91b2060859321p-54")],
    [hf("0x1.00ff93412315cp+0"),  hf("0x1.427068ab22306p-55")],
    [hf("0x1.010ab5b2cbd11p+0"),  hf("0x1.c1d0660524e08p-54")],
    [hf("0x1.0115d89ff3a8bp+0"), hf("-0x1.e7bdfb3204be8p-54")],
    [hf("0x1.0120fc089ff63p+0"),  hf("0x1.843aa8b9cbbc6p-55")],
    [hf("0x1.012c1fecd613bp+0"), hf("-0x1.34104ee7edae9p-56")],
    [hf("0x1.0137444c9b5b5p+0"), hf("-0x1.2b6aeb6176892p-56")],
    [hf("0x1.01426927f5278p+0"),  hf("0x1.a8cd33b8a1bb3p-56")],
    [hf("0x1.014d8e7ee8d2fp+0"),  hf("0x1.2edc08e5da99ap-56")],
    [hf("0x1.0158b4517bb88p+0"),  hf("0x1.57ba2dc7e0c73p-55")],
    [hf("0x1.0163da9fb3335p+0"),  hf("0x1.b61299ab8cdb7p-54")],
    [hf("0x1.016f0169949edp+0"), hf("-0x1.90565902c5f44p-54")],
    [hf("0x1.017a28af25567p+0"),  hf("0x1.70fc41c5c2d53p-55")],
    [hf("0x1.018550706ab62p+0"),  hf("0x1.4b9a6e145d76cp-54")],
    [hf("0x1.019078ad6a19fp+0"), hf("-0x1.008eff5142bf9p-56")],
    [hf("0x1.019ba16628de2p+0"), hf("-0x1.77669f033c7dep-54")],
    [hf("0x1.01a6ca9aac5f3p+0"), hf("-0x1.09bb78eeead0ap-54")],
    [hf("0x1.01b1f44af9f9ep+0"),  hf("0x1.371231477ece5p-54")],
    [hf("0x1.01bd1e77170b4p+0"),  hf("0x1.5e7626621eb5bp-56")],
    [hf("0x1.01c8491f08f08p+0"), hf("-0x1.bc72b100828a5p-54")],
    [ hf("0x1.01d37442d507p+0"), hf("-0x1.ce39cbbab8bbep-57")],
    [hf("0x1.01de9fe280ac8p+0"),  hf("0x1.16996709da2e2p-55")],
    [hf("0x1.01e9cbfe113efp+0"), hf("-0x1.c11f5239bf535p-55")],
    [hf("0x1.01f4f8958c1c6p+0"),  hf("0x1.e1d4eb5edc6b3p-55")],
    [hf("0x1.020025a8f6a35p+0"), hf("-0x1.afb99946ee3fp-54")],
    [hf("0x1.020b533856324p+0"), hf("-0x1.8f06d8a148a32p-54")],
    [hf("0x1.02168143b0281p+0"), hf("-0x1.2bf310fc54eb6p-55")],
    [hf("0x1.0221afcb09e3ep+0"), hf("-0x1.c95a035eb4175p-54")],
    [hf("0x1.022cdece68c4fp+0"), hf("-0x1.491793e46834dp-54")],
    [hf("0x1.02380e4dd22adp+0"), hf("-0x1.3e8d0d9c49091p-56")],
    [hf("0x1.02433e494b755p+0"), hf("-0x1.314aa16278aa3p-54")],
    [hf("0x1.024e6ec0da046p+0"),  hf("0x1.48daf888e9651p-55")],
    [hf("0x1.02599fb483385p+0"),  hf("0x1.56dc8046821f4p-55")],
    [hf("0x1.0264d1244c719p+0"),  hf("0x1.45b42356b9d47p-54")],
    [hf("0x1.027003103b10ep+0"), hf("-0x1.082ef51b61d7ep-56")],
    [hf("0x1.027b357854772p+0"),  hf("0x1.2106ed0920a34p-56")],
    [hf("0x1.0286685c9e059p+0"), hf("-0x1.fd4cf26ea5d0fp-54")],
    [hf("0x1.02919bbd1d1d8p+0"), hf("-0x1.09f8775e78084p-54")],
    [hf("0x1.029ccf99d720ap+0"),  hf("0x1.64cbba902ca27p-58")],
    [hf("0x1.02a803f2d170dp+0"),  hf("0x1.4383ef231d207p-54")],
    [hf("0x1.02b338c811703p+0"),  hf("0x1.4a47a505b3a47p-54")],
    [hf("0x1.02be6e199c811p+0"),  hf("0x1.e47120223467fp-54")],
];

/// Degree 3 to 8 of a polynomial for log(1+x) - x + x^2/2 on [-0.0040283203125,
/// 0.0040283203125], absolute error below 2^-81.63 (C `P_1`, Sollya).
#[rustfmt::skip]
const P_1: [f64; 6] = [
    hf("0x1.5555555555558p-2"),  // degree 3
    hf("-0x1.0000000000003p-2"), // degree 4
    hf("0x1.999999981f535p-3"),  // degree 5
    hf("-0x1.55555553d1eb4p-3"), // degree 6
    hf("0x1.2494526fd4a06p-3"),  // degree 7
    hf("-0x1.0001f0c80e8cep-3"), // degree 8
];

/// Degree 0 to 4 of a polynomial for exp(x) on [-2^-12.905, 2^-12.905], absolute error below
/// 2^-74.34 (C `Q_1`, Sollya).
#[rustfmt::skip]
const Q_1: [f64; 5] = [
    hf("0x1p0"),                // degree 0
    hf("0x1p0"),                // degree 1
    hf("0x1p-1"),               // degree 2
    hf("0x1.5555555997996p-3"), // degree 3
    hf("0x1.5555555849d8dp-5"), // degree 4
];

/// exp(z) for |z| < 2^-12.905, relative error below 2^-64.902632, as qh + ql (C `q_1`).
#[inline(always)]
fn q_1<F: FmaOps>(z: f64) -> (f64, f64) {
    let mut q = F::fma(Q_1[4], z, Q_1[3]);
    q = F::fma(q, z, Q_1[2]);
    let h0 = F::fma(q, z, Q_1[1]);
    let (h1, l1) = a_mul::<F>(z, h0);
    fast_sum(Q_1[0], h1, l1)
}

/// log(1+z) - z for |z| <= 33*2^-13, as ph + pl (C `p_1`).
#[inline(always)]
fn p_1<F: FmaOps>(z: f64) -> (f64, f64) {
    let (wh, wl) = a_mul::<F>(z, z);
    let t = F::fma(P_1[5], z, P_1[4]);
    let mut u = F::fma(P_1[3], z, P_1[2]);
    let mut v = F::fma(P_1[1], z, P_1[0]);
    u = F::fma(t, wh, u);
    v = F::fma(u, wh, v);
    u = v * wh;
    (-0.5 * wh, F::fma(u, z, -0.5 * wl))
}

/// log(x) for x > 0 (not 1, finite) as h + l, and 1 when 1/sqrt(2) < x < sqrt(2) with a
/// cancellation (the larger error bound applies) (C `log_1`).
#[inline(always)]
fn log_1<F: FmaOps>(x: f64) -> (f64, f64, usize) {
    let xu = x.to_bits();
    let mut m = xu & (!0u64 >> 12);
    let mut e = ((xu >> 52) & 0x7ff) as i64;
    let tu;
    if e != 0 {
        tu = m | (0x3ff << 52);
        m += 1 << 52;
        e -= 0x3ff;
    } else {
        // x is subnormal.
        let k = m.leading_zeros() - 11;
        e = -0x3fe - i64::from(k);
        m <<= k;
        tu = m | (0x3ff << 52);
    }
    // Now x = 2^e * t = 2^(e-52) * m with 1 <= t < 2 and 2^52 <= m < 2^53.
    let mut t = f64::from_bits(tu);
    // If m > sqrt(2), divide it by 2 so that 1/sqrt(2) < t < sqrt(2).
    let c = usize::from(m >= 0x16a09e667f3bcd);
    const CY: [f64; 2] = [1.0, 0.5];
    const CM: [u32; 2] = [44, 45];
    e += c as i64;
    let big_e = e as f64;
    let i = (m >> CM[c]) as usize; // i/2^8 <= t < (i+1)/2^8
    t *= CY[c];
    let r = INVERSE[i - 181];
    let l1 = LOG_INV[i - 181][0];
    let l2 = LOG_INV[i - 181][1];
    let z = F::fma(r, t, -1.0);
    const LOG2_H: f64 = hf("0x1.62e42fefa38p-1");
    const LOG2_L: f64 = hf("0x1.ef35793c7673p-45");
    let th = F::fma(big_e, LOG2_H, l1);
    let tl = F::fma(big_e, LOG2_L, l2);
    let (h, l) = fast_sum(th, z, tl);
    let (ph, pl) = p_1::<F>(z);
    let (h, l) = fast_sum(h, ph, l + pl);
    if e == 0 && l.abs() > h.abs() * hf("0x1p-24") {
        let (h, l) = fast_two_sum(h, l);
        return (h, l, 1);
    }
    (h, l, 0)
}

/// exp(rh + rl) times s (±1) as eh + el, relative error below 2^-63.78597, for
/// RHO1 <= rh <= RHO2; outside, ±DBL_MAX twice (sure overflow), ±0 and a tiny el (sure
/// underflow), or NaN (phase 2 decides) (C `exp_1`).
#[inline(always)]
fn exp_1<F: FmaOps>(rh: f64, rl: f64, s: f64) -> (f64, f64) {
    const RHO0: f64 = hf("-0x1.74910ee4e8a27p+9");
    const RHO1: f64 = hf("-0x1.483b8cca421afp+9");
    const RHO2: f64 = hf("0x1.62e42e709a95bp+9");
    const RHO3: f64 = hf("0x1.62e4316ea5df9p+9");
    if rh.is_nan() || rh > RHO2 {
        if !rh.is_nan() && rh > RHO3 {
            let v = hf("0x1.fffffffffffffp+1023") * s;
            return (v, v);
        }
        return (f64::NAN, f64::NAN);
    }
    if rh < RHO1 {
        if rh < RHO0 {
            return (0.0 * s, hf("0x1p-1074") * (0.5 * s));
        }
        return (f64::NAN, f64::NAN);
    }
    const INVLOG2: f64 = hf("0x1.71547652b82fep+12");
    let k = roundeven_small(rh * INVLOG2);
    const LOG2H: f64 = hf("0x1.62e42fefa39efp-13");
    const LOG2L: f64 = hf("0x1.abc9e3b39803fp-68");
    let zh = F::fma(LOG2H, -k, rh);
    let zl = F::fma(LOG2L, -k, rl);
    let big_k = k as i64;
    let m = (big_k >> 12) + 0x3ff;
    let i2 = ((big_k >> 6) & 0x3f) as usize;
    let i1 = (big_k & 0x3f) as usize;
    let (t1h, t1l, t2h, t2l) = (T1[i2][0], T1[i2][1], T2[i1][0], T2[i1][1]);
    let (eh, el) = d_mul::<F>(t2h, t2l, t1h, t1l);
    let (qh, ql) = q_1::<F>(zh + zl);
    let (eh, el) = d_mul::<F>(eh, el, qh, ql);
    // 1 < m < 2047 here (the range tests above).
    let d = f64::from_bits((m as u64) << 52) * s;
    (eh * d, el * d)
}

/// The correctly rounded x^y for finite nonzero x and y (C `cr_pow` after its special
/// values): phase 1 here, phases 2 and 3 in [`pow_accurate`].
#[inline(always)]
fn pow_cr_with<F: FmaOps>(x: f64, y: f64) -> f64 {
    let x0 = x; // the original x
    let mut x = x;
    let mut s = 1.0; // the sign of the result
    if x <= 0.0 {
        // x < 0 (x = ±0 is a special value).
        if !is_int(y) {
            return f64::NAN;
        }
        // s = -1 for an odd y.
        let y_parity = if y.abs() >= hf("0x1p53") {
            0
        } else {
            (y as i64) & 1
        };
        s = if y_parity == 0 { 1.0 } else { -1.0 };
        x = -x;
    }
    // Phase 1 (Algorithm phase_1 of reference [5]).
    let (mut lh, mut ll, cancel) = log_1::<F>(x);
    // Avoid a spurious underflow or overflow in y*log(x): 2^-969 <= |y| < 2^1014.
    let ey = (y.to_bits() >> 52) & 0x7ff;
    if !(0x36..0x7f5).contains(&ey) {
        lh = f64::NAN;
        ll = f64::NAN;
    }
    let (rh, rl) = s_mul::<F>(y, lh, ll);
    let (res_h, res_l) = exp_1::<F>(rh, rl, s);
    // The error bounds 2^-63.797 and 2^-57.579 of Algorithm phase_1.
    const ERR: [f64; 2] = [hf("0x1.27p-64"), hf("0x1.57p-58")];
    let res_min = res_h + F::fma(ERR[cancel], -res_h, res_l);
    let res_max = res_h + F::fma(ERR[cancel], res_h, res_l);
    if res_min == res_max {
        // res_max, not res_min: better on underflow (Laurent Théry; C comment).
        return res_max;
    }
    // Easy cases (pow_js answers them before; kept as in C).
    if y == 1.0 {
        return s * x;
    }
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        return x.sqrt();
    }
    pow_accurate(x0, x, y, s)
}

multiversion! {
    /// The correctly rounded x^y for finite nonzero x and y (module doc, order 8).
    fn pow_cr(x: f64, y: f64) -> f64 = pow_cr_with;
}

// ---------------------------------------------------------------------------------------------
// Phase 2: 128-bit floating point (C dint.h of pow, and pow.h `dint_tod`). Integers only.
// ---------------------------------------------------------------------------------------------

/// (-1)^sgn * (hi + lo/2^64) * 2^(ex-63): the binary point is after the top bit of hi
/// (C `dint64_t` of pow's dint.h; `r` there is hi:lo as one 128-bit integer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dint {
    hi: u64,
    lo: u64,
    ex: i64,
    sgn: u64,
}

impl Dint {
    #[inline(always)]
    fn r(&self) -> u128 {
        (u128::from(self.hi) << 64) | u128::from(self.lo)
    }

    #[inline(always)]
    fn with_r(r: u128, ex: i64, sgn: u64) -> Dint {
        Dint {
            hi: (r >> 64) as u64,
            lo: r as u64,
            ex,
            sgn,
        }
    }
}

/// -1.
const M_ONE: Dint = Dint {
    hi: 0x8000000000000000,
    lo: 0x0,
    ex: 0,
    sgn: 0x1,
};
/// log(2) to nearest, absolute error below 2^-129.97.
const LOG2: Dint = Dint {
    hi: 0xb17217f7d1cf79ab,
    lo: 0xc9e3b39803f2f6af,
    ex: -1,
    sgn: 0x0,
};
/// 2^12/log(2), absolute error below 2^-52.96.
const LOG2_INV: Dint = Dint {
    hi: 0xb8aa3b295c17f0bc,
    lo: 0x0,
    ex: 12,
    sgn: 0x0,
};
/// 0, with dint_tod(ZERO) = 0.
const ZERO: Dint = Dint {
    hi: 0x0,
    lo: 0x0,
    ex: -1076,
    sgn: 0x0,
};

/// C `cmp`, `cmpu128` as an ordering of (ex, r) for nonzero values; 0 is the smallest
/// (C `cmp_dint_abs`).
#[inline(always)]
fn cmp_dint_abs(a: &Dint, b: &Dint) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (a.hi == 0, b.hi == 0) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => (a.ex, a.r()).cmp(&(b.ex, b.r())),
    }
}

/// a + b, error at most 2 ulps (1 when the signs agree); exact in the Sterbenz case (C
/// `add_dint`).
fn add_dint(a: &Dint, b: &Dint) -> Dint {
    use core::cmp::Ordering;
    if a.hi | a.lo == 0 {
        return *b;
    }
    let (a, b) = match cmp_dint_abs(a, b) {
        Ordering::Equal => {
            if a.sgn ^ b.sgn != 0 {
                return ZERO;
            }
            let mut r = *a;
            r.ex += 1;
            return r;
        }
        Ordering::Less => (b, a),
        Ordering::Greater => (a, b),
    };
    // From now on |a| > |b|, so a.ex >= b.ex.
    let big_a = a.r();
    let mut big_b = b.r();
    let k = a.ex.wrapping_sub(b.ex) as u64;
    if k > 0 {
        big_b = if k < 128 { big_b >> k } else { 0 };
    }
    let sgn = a.sgn;
    let mut ex_r = a.ex; // tentative exponent of the result
    let c = if a.sgn ^ b.sgn != 0 {
        // Different signs: C = A - B (C is not 0: |A| = |B| was excluded).
        let mut c = big_a.wrapping_sub(big_b);
        let ch = (c >> 64) as u64;
        let mut ex = if ch != 0 {
            ch.leading_zeros()
        } else {
            64 + (c as u64).leading_zeros()
        };
        if ex > 0 {
            c = if k == 1 {
                // Sterbenz case.
                (big_a << ex).wrapping_sub(b.r() << (ex - 1))
            } else {
                (big_a << ex).wrapping_sub(big_b << ex)
            };
            ex_r -= i64::from(ex);
            ex = ((c >> 64) as u64).leading_zeros();
        }
        ex_r -= i64::from(ex);
        c << ex
    } else {
        let c = big_a.wrapping_add(big_b);
        if c < big_a {
            ex_r += 1;
            (1u128 << 127) | (c >> 1)
        } else {
            c
        }
    };
    Dint::with_r(c, ex_r, sgn)
}

/// a + b from the upper limbs only, error at most 2 ulps (ulp_64) (C `add_dint_11`). As in C,
/// where the result goes to b (every call here: `add_dint_11(r, &c, r)`), the low limb of b
/// stays in the result on the computed paths.
fn add_dint_11(a: &Dint, b: &Dint) -> Dint {
    use core::cmp::Ordering;
    let dest_lo = b.lo;
    if a.hi == 0 {
        return *b;
    }
    if b.hi == 0 {
        return *a;
    }
    let (a, b) = match (a.ex, a.hi).cmp(&(b.ex, b.hi)) {
        Ordering::Equal => {
            if a.sgn ^ b.sgn != 0 {
                return ZERO;
            }
            let mut r = *a;
            r.ex += 1;
            return r;
        }
        Ordering::Less => (b, a),
        Ordering::Greater => (a, b),
    };
    let big_a = a.hi;
    let mut big_b = b.hi;
    if a.ex > b.ex {
        let k = (a.ex - b.ex) as u64;
        big_b = if k < 64 { big_b >> k } else { 0 };
    }
    let sgn = a.sgn;
    let mut ex_r = a.ex;
    let c: u64 = if a.sgn ^ b.sgn != 0 {
        let mut c = big_a - big_b;
        let mut ex = c.leading_zeros();
        if ex > 0 {
            c = (big_a << ex).wrapping_sub(big_b << ex);
            ex_r -= i64::from(ex);
            ex = c.leading_zeros();
        }
        ex_r -= i64::from(ex);
        c << ex
    } else {
        let c = big_a.wrapping_add(big_b);
        if c < big_a {
            ex_r += 1;
            (1u64 << 63) | (c >> 1)
        } else {
            c
        }
    };
    Dint {
        hi: c,
        lo: dest_lo,
        ex: ex_r,
        sgn,
    }
}

/// a * b, error at most 6 ulps (C `mul_dint`).
fn mul_dint(a: &Dint, b: &Dint) -> Dint {
    let bh = u128::from(b.hi);
    let bl = u128::from(b.lo);
    let m1 = u128::from(a.hi) * bl;
    let m2 = u128::from(a.lo) * bh;
    let mut r = u128::from(a.hi) * bh;
    // No overflow: r <= (B-1)^2 with B = 2^64 (C comment).
    r += (m1 >> 64) + (m2 >> 64);
    let ex = (r >> 127) as u64;
    r <<= 1 - ex;
    Dint::with_r(r, a.ex + b.ex + ex as i64, a.sgn ^ b.sgn)
}

/// a * b for a b with low limb 0, error at most 2 ulps (C `mul_dint_21`).
fn mul_dint_21(a: &Dint, b: &Dint) -> Dint {
    let bh = u128::from(b.hi);
    let hi = u128::from(a.hi) * bh;
    let lo = u128::from(a.lo) * bh;
    let mut r = hi + (lo >> 64);
    let ex = (r >> 127) as u64;
    r <<= 1 - ex;
    Dint::with_r(r, a.ex + b.ex + ex as i64, a.sgn ^ b.sgn)
}

/// a * b from the upper limbs, exact (C `mul_dint_11`).
fn mul_dint_11(a: &Dint, b: &Dint) -> Dint {
    let mut r = u128::from(a.hi) * u128::from(b.hi);
    let ex = (r >> 127) as u64;
    r <<= 1 - ex;
    Dint::with_r(r, a.ex + b.ex + ex as i64, a.sgn ^ b.sgn)
}

/// a * b for an integer b, error below 1 ulp (C `mul_dint_int64`; |b| is small here).
fn mul_dint_int64(a: &Dint, b: i64) -> Dint {
    if b == 0 {
        return ZERO;
    }
    let c = b.unsigned_abs();
    let sgn = if b < 0 { u64::from(a.sgn == 0) } else { a.sgn };
    let mut ex = a.ex + 64;
    let mut r = u128::from(a.hi) * u128::from(c);
    // If c = 1, the upper limb can be 0.
    let hi = (r >> 64) as u64;
    let m = if hi != 0 { hi.leading_zeros() } else { 64 };
    r <<= m;
    ex -= i64::from(m);
    let mut l = u128::from(a.lo) * u128::from(c);
    l = l.checked_shl(m.wrapping_sub(1)).unwrap_or(0) >> 63;
    r = r.wrapping_add(l);
    if r < l {
        r = (1u128 << 127) | (r >> 1);
        ex += 1;
    }
    Dint::with_r(r, ex, sgn)
}

/// The C `fast_extract` of pow.h: (exponent - 0x3ff, significand with the hidden bit).
#[inline(always)]
fn fast_extract(x: f64) -> (i64, u64) {
    let u = x.to_bits();
    let e = ((u >> 52) & 0x7ff) as i64;
    let m = (u & (!0u64 >> 12)) + if e != 0 { 1u64 << 52 } else { 0 };
    (e - 0x3ff, m)
}

/// x as a [`Dint`], exact (C `dint_fromd`; x nonzero).
fn dint_fromd(b: f64) -> Dint {
    let (ex, hi) = fast_extract(b);
    let t = hi.leading_zeros();
    Dint {
        hi: hi << t,
        lo: 0,
        ex: ex - if t > 11 { i64::from(t) - 12 } else { 0 },
        sgn: u64::from(b < 0.0),
    }
}

/// a rounded toward zero to an integer (C `dint_toi`).
fn dint_toi(a: &Dint) -> i64 {
    if a.ex < 0 {
        return 0;
    }
    let r = (a.hi >> (63 - a.ex)) as i64;
    if a.sgn != 0 { -r } else { r }
}

/// (e, m) with m odd and x = 2^e * m (C `extract`).
fn extract(x: f64) -> (i64, u64) {
    let u = x.to_bits();
    let e = ((u >> 52) & 0x7ff) as i64;
    let mut m = (u & (!0u64 >> 12)) + if e != 0 { 1u64 << 52 } else { 0 };
    let t = m.trailing_zeros();
    m >>= t;
    (e + i64::from(t) - (0x433 - i64::from(e == 0)), m)
}

/// a rounded to 54 bits: (G, k) (C `round_54`; as in `exact_pow`, only for numbers that end
/// with only ones or only zeros).
fn round_54(x: &Dint) -> (i64, i64) {
    (x.ex - 53, ((x.hi >> 10) + ((x.hi >> 9) & 0x1)) as i64)
}

/// x * 2^e (C `pow2`).
fn pow2(x: f64, e: i64) -> f64 {
    let mut x = x;
    if e & 0x1 != 0 {
        x *= 2.0;
    }
    let e2 = f64::from_bits(((((e >> 1) + 0x3ff) as u64) & 0x7ff) << 52);
    (x * e2) * e2
}

/// a rounded to nearest, for a in the subnormal range (C `dint_tod_subnormal`, the
/// FE_TONEAREST branch).
fn dint_tod_subnormal(a: &Dint) -> f64 {
    // Shift hi:lo right by ex bits so that the last bit of hi is 2^-1074.
    let ex = (-(1011 + a.ex)) as u64; // ex >= 12
    if ex >= 64 {
        // Every bit goes: |a| < 2^-1074.
        let rb = a.hi >> 63;
        let sb = (a.hi << 1) | a.lo;
        let ret = if ex > 64 || rb == 0 || sb == 0 {
            0.0
        } else {
            hf("0x1p-1074")
        };
        return if a.sgn != 0 { -ret } else { ret };
    }
    let mut hi = a.hi >> ex;
    let rb = (a.hi >> (ex - 1)) & 0x1; // round bit
    let sb = (a.hi << (65 - ex)) != 0 || a.lo != 0; // sticky bit
    hi += if sb { rb } else { hi & rb };
    // hi <= 2^52 holds the low bits of the result (2^52 is the smallest normal, as wanted).
    f64::from_bits(hi | (a.sgn << 63))
}

/// a rounded to nearest (C `dint_tod` of pow.h, round to nearest).
fn dint_tod(a: &Dint) -> f64 {
    if a.ex < -1022 {
        return dint_tod_subnormal(a);
    }
    // r is the significand in [1, 2).
    let mut r = (a.hi >> 11) | (0x3ff << 52);
    let mut rd = 0.0;
    if (a.hi >> 10) & 0x1 != 0 {
        rd += hf("0x1p-53");
    }
    if a.hi & 0x3ff != 0 || a.lo != 0 {
        rd += hf("0x1p-54");
    }
    if a.sgn != 0 {
        rd = -rd;
    }
    r |= a.sgn << 63;
    let mut rf = f64::from_bits(r) + rd;
    let e;
    if a.ex > -1023 {
        // A normal double.
        if a.ex > 1023 {
            if a.ex == 1024 {
                // 2^1024 <= |a| < 2^1025.
                rf *= 2.0;
                e = hf("0x1p+1023");
            } else {
                rf = hf("0x1.fffffffffffffp+1023");
                e = hf("0x1.fffffffffffffp+1023");
            }
        } else {
            e = f64::from_bits((((a.ex + 1023) & 0x7ff) as u64) << 52);
        }
    } else {
        // Subnormal (not reached: a.ex < -1022 went to dint_tod_subnormal; kept as in C).
        if a.ex < -1074 {
            if a.ex == -1075 {
                rf *= 0.5;
                e = hf("0x1p-1074");
            } else {
                rf = hf("0x0.0000000000001p-1022");
                e = hf("0x0.0000000000001p-1022");
            }
        } else {
            e = f64::from_bits(1u64 << (a.ex + 1074));
        }
    }
    rf * e
}

/// For 90 <= i <= 181, INVERSE_2_1[i - 90] approximates 1/x for i/2^7 <= x < (i+1)/2^7
/// (C `_INVERSE_2_1`).
#[rustfmt::skip]
static INVERSE_2_1: [Dint; 92] = [
    Dint { hi: 0xb500000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=90 */
    Dint { hi: 0xb300000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=91 */     
    Dint { hi: 0xb100000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=92 */     
    Dint { hi: 0xaf00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=93 */     
    Dint { hi: 0xad80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=94 */     
    Dint { hi: 0xab80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=95 */     
    Dint { hi: 0xaa00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=96 */     
    Dint { hi: 0xa800000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=97 */     
    Dint { hi: 0xa680000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=98 */     
    Dint { hi: 0xa480000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=99 */     
    Dint { hi: 0xa300000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=100 */    
    Dint { hi: 0xa180000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=101 */    
    Dint { hi: 0xa000000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=102 */    
    Dint { hi: 0x9e80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=103 */    
    Dint { hi: 0x9d00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=104 */    
    Dint { hi: 0x9b80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=105 */    
    Dint { hi: 0x9a00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=106 */    
    Dint { hi: 0x9880000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=107 */    
    Dint { hi: 0x9700000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=108 */    
    Dint { hi: 0x9580000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=109 */    
    Dint { hi: 0x9480000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=110 */    
    Dint { hi: 0x9300000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=111 */    
    Dint { hi: 0x9180000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=112 */    
    Dint { hi: 0x9080000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=113 */    
    Dint { hi: 0x8f00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=114 */    
    Dint { hi: 0x8e00000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=115 */
    Dint { hi: 0x8c80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=116 */
    Dint { hi: 0x8b80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=117 */
    Dint { hi: 0x8a80000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=118 */
    Dint { hi: 0x8900000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=119 */
    Dint { hi: 0x8800000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=120 */
    Dint { hi: 0x8700000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=121 */
    Dint { hi: 0x8580000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=122 */
    Dint { hi: 0x8480000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=123 */
    Dint { hi: 0x8380000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=124 */
    Dint { hi: 0x8280000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=125 */
    Dint { hi: 0x8180000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=126 */
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=127 */
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* i=128 */
    Dint { hi: 0xfd00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=129 */
    Dint { hi: 0xfb00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=130 */
    Dint { hi: 0xf900000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=131 */
    Dint { hi: 0xf780000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=132 */
    Dint { hi: 0xf580000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=133 */
    Dint { hi: 0xf380000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=134 */
    Dint { hi: 0xf200000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=135 */
    Dint { hi: 0xf000000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=136 */
    Dint { hi: 0xee80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=137 */
    Dint { hi: 0xec80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=138 */
    Dint { hi: 0xeb00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=139 */
    Dint { hi: 0xe900000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=140 */
    Dint { hi: 0xe780000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=141 */
    Dint { hi: 0xe600000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=142 */
    Dint { hi: 0xe480000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=143 */
    Dint { hi: 0xe300000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=144 */
    Dint { hi: 0xe100000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=145 */
    Dint { hi: 0xdf80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=146 */
    Dint { hi: 0xde00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=147 */
    Dint { hi: 0xdc80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=148 */
    Dint { hi: 0xdb00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=149 */
    Dint { hi: 0xd980000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=150 */
    Dint { hi: 0xd880000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=151 */
    Dint { hi: 0xd700000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=152 */
    Dint { hi: 0xd580000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=153 */
    Dint { hi: 0xd400000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=154 */
    Dint { hi: 0xd280000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=155 */
    Dint { hi: 0xd180000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=156 */
    Dint { hi: 0xd000000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=157 */
    Dint { hi: 0xce80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=158 */
    Dint { hi: 0xcd80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=159 */
    Dint { hi: 0xcc00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=160 */
    Dint { hi: 0xcb00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=161 */
    Dint { hi: 0xc980000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=162 */
    Dint { hi: 0xc880000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=163 */
    Dint { hi: 0xc700000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=164 */
    Dint { hi: 0xc600000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=165 */
    Dint { hi: 0xc500000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=166 */
    Dint { hi: 0xc380000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=167 */
    Dint { hi: 0xc280000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=168 */
    Dint { hi: 0xc180000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=169 */
    Dint { hi: 0xc000000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=170 */
    Dint { hi: 0xbf00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=171 */
    Dint { hi: 0xbe00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=172 */
    Dint { hi: 0xbd00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=173 */
    Dint { hi: 0xbc00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=174 */
    Dint { hi: 0xba80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=175 */
    Dint { hi: 0xb980000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=176 */
    Dint { hi: 0xb880000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=177 */
    Dint { hi: 0xb780000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=178 */
    Dint { hi: 0xb680000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=179 */
    Dint { hi: 0xb580000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=180 */
    Dint { hi: 0xb480000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* i=181 */
];

/// For 8128 <= j <= 8256, INVERSE_2_2[j - 8128] (C `_INVERSE_2_2`).
#[rustfmt::skip]
static INVERSE_2_2: [Dint; 129] = [
    Dint { hi: 0x8100000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8128 */
    Dint { hi: 0x80fc000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8129 */
    Dint { hi: 0x80f8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8130 */
    Dint { hi: 0x80f4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8131 */
    Dint { hi: 0x80f0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8132 */
    Dint { hi: 0x80ec000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8133 */
    Dint { hi: 0x80e8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8134 */
    Dint { hi: 0x80e4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8135 */
    Dint { hi: 0x80e0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8136 */
    Dint { hi: 0x80dc000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8137 */
    Dint { hi: 0x80d8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8138 */
    Dint { hi: 0x80d4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8139 */
    Dint { hi: 0x80d0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8140 */
    Dint { hi: 0x80cc000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8141 */
    Dint { hi: 0x80c8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8142 */
    Dint { hi: 0x80c4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8143 */
    Dint { hi: 0x80c0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8144 */
    Dint { hi: 0x80bc000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8145 */
    Dint { hi: 0x80b8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8146 */
    Dint { hi: 0x80b4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8147 */
    Dint { hi: 0x80b0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8148 */
    Dint { hi: 0x80ac000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8149 */
    Dint { hi: 0x80a8000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8150 */
    Dint { hi: 0x80a4000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8151 */
    Dint { hi: 0x80a0000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8152 */
    Dint { hi: 0x809c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8153 */
    Dint { hi: 0x8098000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8154 */
    Dint { hi: 0x8094000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8155 */
    Dint { hi: 0x8090000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8156 */
    Dint { hi: 0x808c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8157 */
    Dint { hi: 0x8088000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8158 */
    Dint { hi: 0x8084000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8159 */
    Dint { hi: 0x8080000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8160 */
    Dint { hi: 0x807c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8161 */
    Dint { hi: 0x8078000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8162 */
    Dint { hi: 0x8074000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8163 */
    Dint { hi: 0x8070000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8164 */
    Dint { hi: 0x806c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8165 */
    Dint { hi: 0x8068000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8166 */
    Dint { hi: 0x8064000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8167 */
    Dint { hi: 0x8060000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8168 */
    Dint { hi: 0x805c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8169 */
    Dint { hi: 0x8058000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8170 */
    Dint { hi: 0x8054000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8171 */
    Dint { hi: 0x8050000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8172 */
    Dint { hi: 0x804c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8173 */
    Dint { hi: 0x8048000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8174 */
    Dint { hi: 0x8044000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8175 */
    Dint { hi: 0x8040000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8176 */
    Dint { hi: 0x803c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8177 */
    Dint { hi: 0x8038000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8178 */
    Dint { hi: 0x8034000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8179 */
    Dint { hi: 0x8030000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8180 */
    Dint { hi: 0x802c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8181 */
    Dint { hi: 0x8028000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8182 */
    Dint { hi: 0x8024000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8183 */
    Dint { hi: 0x8020000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8184 */
    Dint { hi: 0x801c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8185 */
    Dint { hi: 0x8018000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8186 */
    Dint { hi: 0x8014000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8187 */
    Dint { hi: 0x8010000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8188 */
    Dint { hi: 0x800c000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8189 */
    Dint { hi: 0x8008000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8190 */
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8191 */
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 }, /* j=8192 */
    Dint { hi: 0xfff4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8193 */
    Dint { hi: 0xffec000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8194 */
    Dint { hi: 0xffe4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8195 */
    Dint { hi: 0xffdc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8196 */
    Dint { hi: 0xffd4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8197 */
    Dint { hi: 0xffcc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8198 */
    Dint { hi: 0xffc4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8199 */
    Dint { hi: 0xffbc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8200 */
    Dint { hi: 0xffb4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8201 */
    Dint { hi: 0xffac000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8202 */
    Dint { hi: 0xffa4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8203 */
    Dint { hi: 0xff9c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8204 */
    Dint { hi: 0xff94000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8205 */
    Dint { hi: 0xff8c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8206 */
    Dint { hi: 0xff84000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8207 */
    Dint { hi: 0xff7c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8208 */
    Dint { hi: 0xff74000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8209 */
    Dint { hi: 0xff6c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8210 */
    Dint { hi: 0xff64000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8211 */
    Dint { hi: 0xff5c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8212 */
    Dint { hi: 0xff54000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8213 */
    Dint { hi: 0xff4c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8214 */
    Dint { hi: 0xff44000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8215 */
    Dint { hi: 0xff3c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8216 */
    Dint { hi: 0xff34000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8217 */
    Dint { hi: 0xff2c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8218 */
    Dint { hi: 0xff24000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8219 */
    Dint { hi: 0xff1c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8220 */
    Dint { hi: 0xff14000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8221 */
    Dint { hi: 0xff0c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8222 */
    Dint { hi: 0xff04000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8223 */
    Dint { hi: 0xfefc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8224 */
    Dint { hi: 0xfef4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8225 */
    Dint { hi: 0xfeec000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8226 */
    Dint { hi: 0xfee4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8227 */
    Dint { hi: 0xfedc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8228 */
    Dint { hi: 0xfed4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8229 */
    Dint { hi: 0xfecc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8230 */
    Dint { hi: 0xfec4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8231 */
    Dint { hi: 0xfebc000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8232 */
    Dint { hi: 0xfeb4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8233 */
    Dint { hi: 0xfeac000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8234 */
    Dint { hi: 0xfea4000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8235 */
    Dint { hi: 0xfe9c000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8236 */
    Dint { hi: 0xfe98000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8237 */
    Dint { hi: 0xfe90000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8238 */
    Dint { hi: 0xfe88000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8239 */
    Dint { hi: 0xfe80000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8240 */
    Dint { hi: 0xfe78000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8241 */
    Dint { hi: 0xfe70000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8242 */
    Dint { hi: 0xfe68000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8243 */
    Dint { hi: 0xfe60000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8244 */
    Dint { hi: 0xfe58000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8245 */
    Dint { hi: 0xfe50000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8246 */
    Dint { hi: 0xfe48000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8247 */
    Dint { hi: 0xfe40000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8248 */
    Dint { hi: 0xfe38000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8249 */
    Dint { hi: 0xfe30000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8250 */
    Dint { hi: 0xfe28000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8251 */
    Dint { hi: 0xfe20000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8252 */
    Dint { hi: 0xfe18000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8253 */
    Dint { hi: 0xfe10000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8254 */
    Dint { hi: 0xfe08000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8255 */
    Dint { hi: 0xfe00000000000000, lo: 0x0, ex: -1, sgn: 0x0 }, /* j=8256 */
];

/// LOG_INV_2_1[i - 90] approximates -log(INVERSE_2_1[i - 90]) (C `_LOG_INV_2_1`).
#[rustfmt::skip]
static LOG_INV_2_1: [Dint; 92] = [
    Dint { hi: 0xb1641795ce3ca97b, lo: 0x7af915300e517391, ex: -2, sgn: 0x1 }, /* i=90 */
    Dint { hi: 0xabb3b8ba2ad362a4, lo: 0xd5b6506cc17a01f1, ex: -2, sgn: 0x1 }, /* i=91 */
    Dint { hi: 0xa5f2fcabbbc506da, lo: 0x64ca4fb7ec323d73, ex: -2, sgn: 0x1 }, /* i=92 */
    Dint { hi: 0xa0218434353f1de8, lo: 0x6093efa632530ac8, ex: -2, sgn: 0x1 }, /* i=93 */
    Dint { hi: 0x9bb93315fec2d792, lo: 0xa7589fba0865790e, ex: -2, sgn: 0x1 }, /* i=94 */
    Dint { hi: 0x95c981d5c4e924ed, lo: 0x29404f5aa577d6b2, ex: -2, sgn: 0x1 }, /* i=95 */
    Dint { hi: 0x914a0fde7bcb2d12, lo: 0x1429ed3aea197a5d, ex: -2, sgn: 0x1 }, /* i=96 */
    Dint { hi: 0x8b3ae55d5d30701c, lo: 0xe63eab883717047e, ex: -2, sgn: 0x1 }, /* i=97 */
    Dint { hi: 0x86a35abcd5ba5903, lo: 0xec81c3cbd925cccf, ex: -2, sgn: 0x1 }, /* i=98 */
    Dint { hi: 0x8073622d6a80e634, lo: 0x6a97009015316071, ex: -2, sgn: 0x1 }, /* i=99 */
    Dint { hi: 0xf7856e5ee2c9b290, lo: 0xc6f2a1b84190a7d7, ex: -3, sgn: 0x1 }, /* i=100 */
    Dint { hi: 0xee0de5055f63eb06, lo: 0x98a33316df83ba57, ex: -3, sgn: 0x1 }, /* i=101 */
    Dint { hi: 0xe47fbe3cd4d10d61, lo: 0x2ec0f797fdcd1257, ex: -3, sgn: 0x1 }, /* i=102 */
    Dint { hi: 0xdada8cf47dad2374, lo: 0x4ffb833c3409ee78, ex: -3, sgn: 0x1 }, /* i=103 */
    Dint { hi: 0xd11de0ff15ab18c9, lo: 0xb88d83d4cc613f20, ex: -3, sgn: 0x1 }, /* i=104 */
    Dint { hi: 0xc74946f4436a0552, lo: 0xc4f5cb531201c0d1, ex: -3, sgn: 0x1 }, /* i=105 */
    Dint { hi: 0xbd5c481086c848df, lo: 0x1b596b5030403240, ex: -3, sgn: 0x1 }, /* i=106 */
    Dint { hi: 0xb3566a13956a86f6, lo: 0xff1b1e1574d9fd54, ex: -3, sgn: 0x1 }, /* i=107 */
    Dint { hi: 0xa9372f1d0da1bd17, lo: 0x200eb71e58cd36de, ex: -3, sgn: 0x1 }, /* i=108 */
    Dint { hi: 0x9efe158766314e54, lo: 0xc571827efe892fc4, ex: -3, sgn: 0x1 }, /* i=109 */
    Dint { hi: 0x981eb8c723fe97f4, lo: 0xa31c134fb702d432, ex: -3, sgn: 0x1 }, /* i=110 */
    Dint { hi: 0x8db956a97b3d0148, lo: 0x3023472cd739f9de, ex: -3, sgn: 0x1 }, /* i=111 */
    Dint { hi: 0x8338a89652cb7150, lo: 0xc647eb86498c2ce1, ex: -3, sgn: 0x1 }, /* i=112 */
    Dint { hi: 0xf85186008b15330b, lo: 0xe64b8b775997898d, ex: -4, sgn: 0x1 }, /* i=113 */
    Dint { hi: 0xe2f2a47ade3a18ae, lo: 0xb0bf7c0b0d8bb4ed, ex: -4, sgn: 0x1 }, /* i=114 */
    Dint { hi: 0xd49369d256ab1b28, lo: 0x5e9154e1d5263cd5, ex: -4, sgn: 0x1 }, /* i=115 */
    Dint { hi: 0xbed3b36bd8966422, lo: 0x240644d7d9ed08af, ex: -4, sgn: 0x1 }, /* i=116 */
    Dint { hi: 0xb032c549ba861d8e, lo: 0xf74e27bc92ce336a, ex: -4, sgn: 0x1 }, /* i=117 */
    Dint { hi: 0xa176e5f5323781dd, lo: 0xd4f935996c92e8cc, ex: -4, sgn: 0x1 }, /* i=118 */
    Dint { hi: 0x8b29b7751bd70743, lo: 0x12e0b9ee992f236d, ex: -4, sgn: 0x1 }, /* i=119 */
    Dint { hi: 0xf85186008b15330b, lo: 0xe64b8b775997898d, ex: -5, sgn: 0x1 }, /* i=120 */
    Dint { hi: 0xda16eb88cb8df614, lo: 0x68a63ecfb66e94ac, ex: -5, sgn: 0x1 }, /* i=121 */
    Dint { hi: 0xac52dd7e4726a463, lo: 0x547a963a91bb3012, ex: -5, sgn: 0x1 }, /* i=122 */
    Dint { hi: 0x8d86cc491ecbfe16, lo: 0x51776453b7e8254d, ex: -5, sgn: 0x1 }, /* i=123 */
    Dint { hi: 0xdcfe013d7c8cbfde, lo: 0xa32dbac46f30cfff, ex: -6, sgn: 0x1 }, /* i=124 */
    Dint { hi: 0x9e75221a352ba779, lo: 0xa52b7ea62f2198d0, ex: -6, sgn: 0x1 }, /* i=125 */
    Dint { hi: 0xbee23afc0853b6e9, lo: 0x289782c20df350a1, ex: -7, sgn: 0x1 }, /* i=126 */
    Dint { hi: 0x0, lo: 0x0, ex: 127, sgn: 0x1 }, /* i=127 */
    Dint { hi: 0x0, lo: 0x0, ex: 127, sgn: 0x1 }, /* i=128 */
    Dint { hi: 0xc122451c45155104, lo: 0xb16137f09a002b3c, ex: -7, sgn: 0x0 }, /* i=129 */
    Dint { hi: 0xa195492cc06604e6, lo: 0x4a18dff7cdb4ae5c, ex: -6, sgn: 0x0 }, /* i=130 */
    Dint { hi: 0xe31e9760a5578c63, lo: 0xf9eb2f284f31c35c, ex: -6, sgn: 0x0 }, /* i=131 */
    Dint { hi: 0x8a4f1f2002d46756, lo: 0x5be970314148c645, ex: -5, sgn: 0x0 }, /* i=132 */
    Dint { hi: 0xab8ae2601e777722, lo: 0x3b89d7f254f8d4d, ex: -5, sgn: 0x0 }, /* i=133 */
    Dint { hi: 0xcd0c3dab9ef3dd1b, lo: 0x13b26f298aa357c8, ex: -5, sgn: 0x0 }, /* i=134 */
    Dint { hi: 0xe65b9e6eed965c36, lo: 0xe09f5fe2058d6006, ex: -5, sgn: 0x0 }, /* i=135 */
    Dint { hi: 0x842cc5acf1d03445, lo: 0x1fecdfa819b96098, ex: -4, sgn: 0x0 }, /* i=136 */
    Dint { hi: 0x9103dae3c2a4ec67, lo: 0xe0863df62ab5671a, ex: -4, sgn: 0x0 }, /* i=137 */
    Dint { hi: 0xa242f01edefd6a37, lo: 0x469355b78dc796e3, ex: -4, sgn: 0x0 }, /* i=138 */
    Dint { hi: 0xaf4ad26cbc8e5be7, lo: 0xe8b8b88a14ff0ce, ex: -4, sgn: 0x0 }, /* i=139 */
    Dint { hi: 0xc0cbf17a071f80dc, lo: 0xf96ffdf76a147ccc, ex: -4, sgn: 0x0 }, /* i=140 */
    Dint { hi: 0xce06196a692a41fb, lo: 0xbe3ccc15326765f, ex: -4, sgn: 0x0 }, /* i=141 */
    Dint { hi: 0xdb56446d6ad8deff, lo: 0xa8112e35a60e6375, ex: -4, sgn: 0x0 }, /* i=142 */
    Dint { hi: 0xe8bcbc410c9b219d, lo: 0xaf7df76ad29e5b60, ex: -4, sgn: 0x0 }, /* i=143 */
    Dint { hi: 0xf639cc185088fe5d, lo: 0x4066e87f2c0f7340, ex: -4, sgn: 0x0 }, /* i=144 */
    Dint { hi: 0x842cc5acf1d03445, lo: 0x1fecdfa819b96098, ex: -3, sgn: 0x0 }, /* i=145 */
    Dint { hi: 0x8b064012593d85a5, lo: 0x52013c7a80ad089b, ex: -3, sgn: 0x0 }, /* i=146 */
    Dint { hi: 0x91eb89524e100d23, lo: 0x8fd3df5c52d67e7b, ex: -3, sgn: 0x0 }, /* i=147 */
    Dint { hi: 0x98dcca69d27c263b, lo: 0x8e94203f336fc8c5, ex: -3, sgn: 0x0 }, /* i=148 */
    Dint { hi: 0x9fda2d2cc9465c4f, lo: 0x32b9565f5355182, ex: -3, sgn: 0x0 }, /* i=149 */
    Dint { hi: 0xa6e3dc4bde0e3cdb, lo: 0x570ff874170d2a9, ex: -3, sgn: 0x0 }, /* i=150 */
    Dint { hi: 0xab9be6480c66ea9e, lo: 0x9ae21fd871b8d27c, ex: -3, sgn: 0x0 }, /* i=151 */
    Dint { hi: 0xb2ba75f46099cf8b, lo: 0x2c3c2e77904afa78, ex: -3, sgn: 0x0 }, /* i=152 */
    Dint { hi: 0xb9e5c83a7e8a655b, lo: 0xcbffe9661fe72421, ex: -3, sgn: 0x0 }, /* i=153 */
    Dint { hi: 0xc11e0b2a8d1e0ddb, lo: 0x9a631e830fd30904, ex: -3, sgn: 0x0 }, /* i=154 */
    Dint { hi: 0xc8636dcfe5e6ca0a, lo: 0x88e72835b3292d50, ex: -3, sgn: 0x0 }, /* i=155 */
    Dint { hi: 0xcd43bc6f5d51c3e8, lo: 0xfbfb0e3f0fd23074, ex: -3, sgn: 0x0 }, /* i=156 */
    Dint { hi: 0xd49f69e456cf1b79, lo: 0x5f53bd2e406e66e7, ex: -3, sgn: 0x0 }, /* i=157 */
    Dint { hi: 0xdc08b985c11e9068, lo: 0x3b9cd767c3b1ac53, ex: -3, sgn: 0x0 }, /* i=158 */
    Dint { hi: 0xe1014558bfcda3e2, lo: 0x35470a74be1230ec, ex: -3, sgn: 0x0 }, /* i=159 */
    Dint { hi: 0xe881bf932af3dac0, lo: 0xc524848e3443e040, ex: -3, sgn: 0x0 }, /* i=160 */
    Dint { hi: 0xed89ed86a44a01aa, lo: 0x11d49f96cb88317b, ex: -3, sgn: 0x0 }, /* i=161 */
    Dint { hi: 0xf52224f82557a459, lo: 0x8dcca8d7f17fa2a9, ex: -3, sgn: 0x0 }, /* i=162 */
    Dint { hi: 0xfa3a589a6f9146d8, lo: 0x388212895529a6fb, ex: -3, sgn: 0x0 }, /* i=163 */
    Dint { hi: 0x80f572b1363487b9, lo: 0xf5bd0b5b3479d5f4, ex: -2, sgn: 0x0 }, /* i=164 */
    Dint { hi: 0x8389c3026ac3139b, lo: 0x62dda9d2270fa1f4, ex: -2, sgn: 0x0 }, /* i=165 */
    Dint { hi: 0x86216b3b0b17188b, lo: 0x163ceae88f720f1e, ex: -2, sgn: 0x0 }, /* i=166 */
    Dint { hi: 0x8a0b3f79b3bc180f, lo: 0x49b55ea7d3730d7, ex: -2, sgn: 0x0 }, /* i=167 */
    Dint { hi: 0x8cab69dcde17d2f7, lo: 0x3ad1aa142b94f16a, ex: -2, sgn: 0x0 }, /* i=168 */
    Dint { hi: 0x8f4f0b3c44cfa2a2, lo: 0x586e9343c9cfdbac, ex: -2, sgn: 0x0 }, /* i=169 */
    Dint { hi: 0x934b1089a6dc93c1, lo: 0xdf5bb3b60554e152, ex: -2, sgn: 0x0 }, /* i=170 */
    Dint { hi: 0x95f783e6e49a9cfa, lo: 0x4a5004f3ef063313, ex: -2, sgn: 0x0 }, /* i=171 */
    Dint { hi: 0x98a78f0e9ae71d85, lo: 0x2cdec34784707839, ex: -2, sgn: 0x0 }, /* i=172 */
    Dint { hi: 0x9b5b3bb5f088b766, lo: 0xd878bbe3d392be25, ex: -2, sgn: 0x0 }, /* i=173 */
    Dint { hi: 0x9e1293b9998c1daa, lo: 0x5b035eae273a855f, ex: -2, sgn: 0x0 }, /* i=174 */
    Dint { hi: 0xa22c8f029cfa45a9, lo: 0xdb5b709e0b69e773, ex: -2, sgn: 0x0 }, /* i=175 */
    Dint { hi: 0xa4ed3f9de620f666, lo: 0x9b5e973353638c11, ex: -2, sgn: 0x0 }, /* i=176 */
    Dint { hi: 0xa7b1bf5dd4c07d4e, lo: 0x699db68db75e9a7f, ex: -2, sgn: 0x0 }, /* i=177 */
    Dint { hi: 0xaa7a18dbdf0d44aa, lo: 0x604884a8dd76d08a, ex: -2, sgn: 0x0 }, /* i=178 */
    Dint { hi: 0xad4656ddf6fd070c, lo: 0x9ea10260fe452ba2, ex: -2, sgn: 0x0 }, /* i=179 */
    Dint { hi: 0xb0168457848f5f48, lo: 0xbb6f9fb246068d52, ex: -2, sgn: 0x0 }, /* i=180 */
    Dint { hi: 0xb2eaac6a67005513, lo: 0xf4b716f6fec8156b, ex: -2, sgn: 0x0 }, /* i=181 */
];

/// LOG_INV_2_2[j - 8128] approximates -log(INVERSE_2_2[j - 8128]) (C `_LOG_INV_2_2`).
#[rustfmt::skip]
static LOG_INV_2_2: [Dint; 129] = [
    Dint { hi: 0xff015358833c47e1, lo: 0xbb481c8ee141695a, ex: -8, sgn: 0x1 }, /* j=8128 */
    Dint { hi: 0xfb0933b732572a6d, lo: 0x214cca3dd1d4796a, ex: -8, sgn: 0x1 }, /* j=8129 */
    Dint { hi: 0xf710f492711d9d26, lo: 0xfbc7b38b17b2019, ex: -8, sgn: 0x1 }, /* j=8130 */
    Dint { hi: 0xf31895e84b1a6be6, lo: 0xb76782b9e88c84cb, ex: -8, sgn: 0x1 }, /* j=8131 */
    Dint { hi: 0xef2017b6cba9cf9a, lo: 0x2dc85881664025b5, ex: -8, sgn: 0x1 }, /* j=8132 */
    Dint { hi: 0xeb2779fbfdf96874, lo: 0xce4ab4e678d0ed03, ex: -8, sgn: 0x1 }, /* j=8133 */
    Dint { hi: 0xe72ebcb5ed08382b, lo: 0xb60585f4c4bb6062, ex: -8, sgn: 0x1 }, /* j=8134 */
    Dint { hi: 0xe335dfe2a3a69c2b, lo: 0x59bcffe9d5650564, ex: -8, sgn: 0x1 }, /* j=8135 */
    Dint { hi: 0xdf3ce3802c7647cd, lo: 0x3602021fa93b1e18, ex: -8, sgn: 0x1 }, /* j=8136 */
    Dint { hi: 0xdb43c78c91ea3e8c, lo: 0x9944002534d09b3d, ex: -8, sgn: 0x1 }, /* j=8137 */
    Dint { hi: 0xd74a8c05de46ce3a, lo: 0x87aa95782311a277, ex: -8, sgn: 0x1 }, /* j=8138 */
    Dint { hi: 0xd35130ea1ba18930, lo: 0xb88be10313a1303d, ex: -8, sgn: 0x1 }, /* j=8139 */
    Dint { hi: 0xcf57b63753e14083, lo: 0xad54bc31433dddba, ex: -8, sgn: 0x1 }, /* j=8140 */
    Dint { hi: 0xcb5e1beb90bdfe33, lo: 0xe1b7d813e3f825e1, ex: -8, sgn: 0x1 }, /* j=8141 */
    Dint { hi: 0xc7646204dbc0ff5e, lo: 0x14f8c1be7370f219, ex: -8, sgn: 0x1 }, /* j=8142 */
    Dint { hi: 0xc36a88813e44ae6a, lo: 0xac27c5a6139cd30c, ex: -8, sgn: 0x1 }, /* j=8143 */
    Dint { hi: 0xbf708f5ec1749d3c, lo: 0x2d23a0744e00f594, ex: -8, sgn: 0x1 }, /* j=8144 */
    Dint { hi: 0xbb76769b6e4d7f5c, lo: 0xd235e25fb9644c31, ex: -8, sgn: 0x1 }, /* j=8145 */
    Dint { hi: 0xb77c3e354d9d242b, lo: 0x361ee0bcb5db0449, ex: -8, sgn: 0x1 }, /* j=8146 */
    Dint { hi: 0xb381e62a68027106, lo: 0x18660815da3d7963, ex: -8, sgn: 0x1 }, /* j=8147 */
    Dint { hi: 0xaf876e78c5ed5b77, lo: 0x39c357b6bfdf81b5, ex: -8, sgn: 0x1 }, /* j=8148 */
    Dint { hi: 0xab8cd71e6f9ee35d, lo: 0x5076c62c951204f6, ex: -8, sgn: 0x1 }, /* j=8149 */
    Dint { hi: 0xa79220196d290d15, lo: 0x146244d643f7fa2b, ex: -8, sgn: 0x1 }, /* j=8150 */
    Dint { hi: 0xa3974967c66edba1, lo: 0x62bb0f3208d9a1bb, ex: -8, sgn: 0x1 }, /* j=8151 */
    Dint { hi: 0x9f9c530783244ad2, lo: 0x7926e92808bd580d, ex: -8, sgn: 0x1 }, /* j=8152 */
    Dint { hi: 0x9ba13cf6aace496c, lo: 0x4819e620d5fcc068, ex: -8, sgn: 0x1 }, /* j=8153 */
    Dint { hi: 0x97a6073344c2b34b, lo: 0xdc494943d427214e, ex: -8, sgn: 0x1 }, /* j=8154 */
    Dint { hi: 0x93aab1bb58284b8b, lo: 0xdf0805c4161e404c, ex: -8, sgn: 0x1 }, /* j=8155 */
    Dint { hi: 0x8faf3c8cebf6b6a8, lo: 0x2d615caaa0514c3c, ex: -8, sgn: 0x1 }, /* j=8156 */
    Dint { hi: 0x8bb3a7a606f674a0, lo: 0x85c60c12eca0aedc, ex: -8, sgn: 0x1 }, /* j=8157 */
    Dint { hi: 0x87b7f304afc0db1a, lo: 0x4c207a522524f8de, ex: -8, sgn: 0x1 }, /* j=8158 */
    Dint { hi: 0x83bc1ea6ecc00f81, lo: 0x64243e02c6215a4f, ex: -8, sgn: 0x1 }, /* j=8159 */
    Dint { hi: 0xff805515885e0250, lo: 0x435ab4da6a5bb48d, ex: -9, sgn: 0x1 }, /* j=8160 */
    Dint { hi: 0xf7882d5c7832c6cc, lo: 0x9e06fc84b6ea5e24, ex: -9, sgn: 0x1 }, /* j=8161 */
    Dint { hi: 0xef8fc61eb4b74f6e, lo: 0x91ab122ee427cfb5, ex: -9, sgn: 0x1 }, /* j=8162 */
    Dint { hi: 0xe7971f584945efae, lo: 0x5f832513e3211643, ex: -9, sgn: 0x1 }, /* j=8163 */
    Dint { hi: 0xdf9e390540da5fbe, lo: 0x5e7b48cfeeb85aa8, ex: -9, sgn: 0x1 }, /* j=8164 */
    Dint { hi: 0xd7a51321a611b0c1, lo: 0xb36a9f58eb4ccd08, ex: -9, sgn: 0x1 }, /* j=8165 */
    Dint { hi: 0xcfabada9832a4101, lo: 0x3360751e43c7af35, ex: -9, sgn: 0x1 }, /* j=8166 */
    Dint { hi: 0xc7b20898e203b01e, lo: 0x6fab78aca91193cb, ex: -9, sgn: 0x1 }, /* j=8167 */
    Dint { hi: 0xbfb823ebcc1ed344, lo: 0xeb432409cffdad8d, ex: -9, sgn: 0x1 }, /* j=8168 */
    Dint { hi: 0xb7bdff9e4a9da959, lo: 0x793b5acf3a336462, ex: -9, sgn: 0x1 }, /* j=8169 */
    Dint { hi: 0xafc39bac66434f27, lo: 0xc3ea2cd93f316b34, ex: -9, sgn: 0x1 }, /* j=8170 */
    Dint { hi: 0xa7c8f8122773f38d, lo: 0xfc679a28e9d9f212, ex: -9, sgn: 0x1 }, /* j=8171 */
    Dint { hi: 0x9fce14cb9634cba6, lo: 0xb20f215bd3b58c61, ex: -9, sgn: 0x1 }, /* j=8172 */
    Dint { hi: 0x97d2f1d4ba2c06f0, lo: 0xd1aacedcefe9d377, ex: -9, sgn: 0x1 }, /* j=8173 */
    Dint { hi: 0x8fd78f299aa0c375, lo: 0xcbef6fac33691e95, ex: -9, sgn: 0x1 }, /* j=8174 */
    Dint { hi: 0x87dbecc63e7b01ed, lo: 0xe2f1775134c8da75, ex: -9, sgn: 0x1 }, /* j=8175 */
    Dint { hi: 0xffc0154d588733c5, lo: 0x3c742a7c76356396, ex: -10, sgn: 0x1 }, /* j=8176 */
    Dint { hi: 0xefc7d18dd4485b9e, lo: 0xca47c52b7d7ffce2, ex: -10, sgn: 0x1 }, /* j=8177 */
    Dint { hi: 0xdfcf0e45fbce3e80, lo: 0x7e4cfbd830393b88, ex: -10, sgn: 0x1 }, /* j=8178 */
    Dint { hi: 0xcfd5cb6dd9ef05dd, lo: 0x7370ae83f9e72748, ex: -10, sgn: 0x1 }, /* j=8179 */
    Dint { hi: 0xbfdc08fd78c229b9, lo: 0xe6dbb624f9739782, ex: -10, sgn: 0x1 }, /* j=8180 */
    Dint { hi: 0xafe1c6ece1a058dd, lo: 0x97fa2fd0c9dc723e, ex: -10, sgn: 0x1 }, /* j=8181 */
    Dint { hi: 0x9fe705341d236102, lo: 0x7199cd06ae5d39b3, ex: -10, sgn: 0x1 }, /* j=8182 */
    Dint { hi: 0x8febc3cb332616ff, lo: 0x7b6d1248c3e1fd40, ex: -10, sgn: 0x1 }, /* j=8183 */
    Dint { hi: 0xffe0055455887de0, lo: 0x26828c92649a3a39, ex: -11, sgn: 0x1 }, /* j=8184 */
    Dint { hi: 0xdfe7839214b4e8ae, lo: 0xda6959f7f0e01bf0, ex: -11, sgn: 0x1 }, /* j=8185 */
    Dint { hi: 0xbfee023faf0c2480, lo: 0xb47505bfa5a03b06, ex: -11, sgn: 0x1 }, /* j=8186 */
    Dint { hi: 0x9ff3814d2e4a36b2, lo: 0xa8740b91c95df537, ex: -11, sgn: 0x1 }, /* j=8187 */
    Dint { hi: 0xfff0015535588833, lo: 0x3c56c598c659c2a3, ex: -12, sgn: 0x1 }, /* j=8188 */
    Dint { hi: 0xbff7008ff5e0c257, lo: 0x379eba7e6465ff63, ex: -12, sgn: 0x1 }, /* j=8189 */
    Dint { hi: 0xfff8005551558885, lo: 0xde026e271ee0549d, ex: -13, sgn: 0x1 }, /* j=8190 */
    Dint { hi: 0x0, lo: 0x0, ex: 127, sgn: 0x1 }, /* j=8191 */
    Dint { hi: 0x0, lo: 0x0, ex: 127, sgn: 0x1 }, /* j=8192 */
    Dint { hi: 0xc004802401440c26, lo: 0xdfeb485085f6f454, ex: -13, sgn: 0x0 }, /* j=8193 */
    Dint { hi: 0xa00640535a37a37a, lo: 0x6bc1e20eac8448b4, ex: -12, sgn: 0x0 }, /* j=8194 */
    Dint { hi: 0xe00c40e4bd6e4efd, lo: 0xc72446cc1bf728bd, ex: -12, sgn: 0x0 }, /* j=8195 */
    Dint { hi: 0x900a20f319a3e273, lo: 0x569b26aaa485ea5c, ex: -11, sgn: 0x0 }, /* j=8196 */
    Dint { hi: 0xb00f21bbe3e388ee, lo: 0x5f69768284463b9b, ex: -11, sgn: 0x0 }, /* j=8197 */
    Dint { hi: 0xd01522dcc4f87991, lo: 0x14d9d76196d8043a, ex: -11, sgn: 0x0 }, /* j=8198 */
    Dint { hi: 0xf01c2465c5e61b6f, lo: 0x661e135f49a47c40, ex: -11, sgn: 0x0 }, /* j=8199 */
    Dint { hi: 0x881213337898871e, lo: 0x9a31ba0cbc030353, ex: -10, sgn: 0x0 }, /* j=8200 */
    Dint { hi: 0x98169478296fad41, lo: 0x7ad1e9c315328f7e, ex: -10, sgn: 0x0 }, /* j=8201 */
    Dint { hi: 0xa81b9608fc3c50ec, lo: 0xf105b66ec4703ede, ex: -10, sgn: 0x0 }, /* j=8202 */
    Dint { hi: 0xb82117edf8832797, lo: 0xd6aef30cd312169a, ex: -10, sgn: 0x0 }, /* j=8203 */
    Dint { hi: 0xc8271a2f2689e388, lo: 0xe6e2acf8f4d4c24a, ex: -10, sgn: 0x0 }, /* j=8204 */
    Dint { hi: 0xd82d9cd48f574c00, lo: 0x28bb3cd9f2a65fb5, ex: -10, sgn: 0x0 }, /* j=8205 */
    Dint { hi: 0xe8349fe63cb35564, lo: 0x224a96f5a7471c46, ex: -10, sgn: 0x0 }, /* j=8206 */
    Dint { hi: 0xf83c236c39273972, lo: 0xd462b63756c87e80, ex: -10, sgn: 0x0 }, /* j=8207 */
    Dint { hi: 0x842213b747fec7bb, lo: 0x3ff51287882500ed, ex: -9, sgn: 0x0 }, /* j=8208 */
    Dint { hi: 0x8c2655faa6a1323f, lo: 0x1ab9679b55f78a6b, ex: -9, sgn: 0x0 }, /* j=8209 */
    Dint { hi: 0x942ad8843ee1a9cd, lo: 0x17e4b7ac6c600cb4, ex: -9, sgn: 0x0 }, /* j=8210 */
    Dint { hi: 0x9c2f9b581787cf0d, lo: 0xfd1a09c848e3950e, ex: -9, sgn: 0x0 }, /* j=8211 */
    Dint { hi: 0xa4349e7a37bc21ed, lo: 0x318b2ddd9d0a33b4, ex: -9, sgn: 0x0 }, /* j=8212 */
    Dint { hi: 0xac39e1eea7080dbc, lo: 0x9dd91e52c79fd070, ex: -9, sgn: 0x0 }, /* j=8213 */
    Dint { hi: 0xb43f65b96d55f55a, lo: 0x72de1d99ce252efd, ex: -9, sgn: 0x0 }, /* j=8214 */
    Dint { hi: 0xbc4529de92f13f58, lo: 0xd7bd1d62ef25480d, ex: -9, sgn: 0x0 }, /* j=8215 */
    Dint { hi: 0xc44b2e6220866227, lo: 0x7f921124f1ecb59e, ex: -9, sgn: 0x0 }, /* j=8216 */
    Dint { hi: 0xcc5173481f22f03f, lo: 0x271ee1cd6d5cdf9e, ex: -9, sgn: 0x0 }, /* j=8217 */
    Dint { hi: 0xd457f8949835a44e, lo: 0xfad0cc8b5faea8cc, ex: -9, sgn: 0x0 }, /* j=8218 */
    Dint { hi: 0xdc5ebe4b958e6d6b, lo: 0xe57a0acb9d5cd4df, ex: -9, sgn: 0x0 }, /* j=8219 */
    Dint { hi: 0xe465c471215e7b41, lo: 0xc81bb5a8d789f444, ex: -9, sgn: 0x0 }, /* j=8220 */
    Dint { hi: 0xec6d0b0946384a46, lo: 0x9b1beb40437575f5, ex: -9, sgn: 0x0 }, /* j=8221 */
    Dint { hi: 0xf47492180f0fafef, lo: 0x7944509046652d99, ex: -9, sgn: 0x0 }, /* j=8222 */
    Dint { hi: 0xfc7c59a18739e6e7, lo: 0x94e51ebff53a2f15, ex: -9, sgn: 0x0 }, /* j=8223 */
    Dint { hi: 0x824230d4dd36cda4, lo: 0x8bbc7f765b13ebbe, ex: -8, sgn: 0x0 }, /* j=8224 */
    Dint { hi: 0x8646551a5a617b6b, lo: 0xf61305ef7390939c, ex: -8, sgn: 0x0 }, /* j=8225 */
    Dint { hi: 0x8a4a99a34159d69f, lo: 0x3abc32a78afd4b7b, ex: -8, sgn: 0x0 }, /* j=8226 */
    Dint { hi: 0x8e4efe71988d8426, lo: 0x17596a598cb29436, ex: -8, sgn: 0x0 }, /* j=8227 */
    Dint { hi: 0x92538387669afa1b, lo: 0x1c890bee9a9d743c, ex: -8, sgn: 0x0 }, /* j=8228 */
    Dint { hi: 0x965828e6b25185ec, lo: 0xeaafbd07b543145d, ex: -8, sgn: 0x0 }, /* j=8229 */
    Dint { hi: 0x9a5cee9182b15280, lo: 0x6517bc4112d64b17, ex: -8, sgn: 0x0 }, /* j=8230 */
    Dint { hi: 0x9e61d489deeb6e53, lo: 0xdb94a1dfd653d3a5, ex: -8, sgn: 0x0 }, /* j=8231 */
    Dint { hi: 0xa266dad1ce61d1a3, lo: 0x2ada01ce7ed36080, ex: -8, sgn: 0x0 }, /* j=8232 */
    Dint { hi: 0xa66c016b58a7648c, lo: 0xd3b36c029ea7bb5d, ex: -8, sgn: 0x0 }, /* j=8233 */
    Dint { hi: 0xaa71485885800538, lo: 0x94c529f32403828, ex: -8, sgn: 0x0 }, /* j=8234 */
    Dint { hi: 0xae76af9b5ce08dfb, lo: 0xb6b6676248bba139, ex: -8, sgn: 0x0 }, /* j=8235 */
    Dint { hi: 0xb27c3735e6eedb86, lo: 0x7bdd0c2a9c7a679a, ex: -8, sgn: 0x0 }, /* j=8236 */
    Dint { hi: 0xb47f0724b1906935, lo: 0x23deb274e953a259, ex: -8, sgn: 0x0 }, /* j=8237 */
    Dint { hi: 0xb884bf4697559ffa, lo: 0xdae7e343fa859415, ex: -8, sgn: 0x0 }, /* j=8238 */
    Dint { hi: 0xbc8a97c544fdd5eb, lo: 0x17759bff5c717993, ex: -8, sgn: 0x0 }, /* j=8239 */
    Dint { hi: 0xc09090a2c35aa070, lo: 0x52e7e4dde874dace, ex: -8, sgn: 0x0 }, /* j=8240 */
    Dint { hi: 0xc496a9e11b6eb30c, lo: 0xa88971f8277a4d11, ex: -8, sgn: 0x0 }, /* j=8241 */
    Dint { hi: 0xc89ce382566de587, lo: 0x269de85f0df92588, ex: -8, sgn: 0x0 }, /* j=8242 */
    Dint { hi: 0xcca33d887dbd3a1a, lo: 0x180d255422c3377c, ex: -8, sgn: 0x0 }, /* j=8243 */
    Dint { hi: 0xd0a9b7f59af2e3a2, lo: 0x46da70925ee85c05, ex: -8, sgn: 0x0 }, /* j=8244 */
    Dint { hi: 0xd4b052cbb7d64bcf, lo: 0x37968ceafaf7b453, ex: -8, sgn: 0x0 }, /* j=8245 */
    Dint { hi: 0xd8b70e0cde601954, lo: 0x5dfba4cfdd38a059, ex: -8, sgn: 0x0 }, /* j=8246 */
    Dint { hi: 0xdcbde9bb18ba361b, lo: 0x4ae21abe75d5a19b, ex: -8, sgn: 0x0 }, /* j=8247 */
    Dint { hi: 0xe0c4e5d8713fd576, lo: 0xd3bd4fd98a1e6fe5, ex: -8, sgn: 0x0 }, /* j=8248 */
    Dint { hi: 0xe4cc0266f27d7a57, lo: 0x33cf7d5ebfb93ad3, ex: -8, sgn: 0x0 }, /* j=8249 */
    Dint { hi: 0xe8d33f68a730fd7f, lo: 0x2743c805a4928087, ex: -8, sgn: 0x0 }, /* j=8250 */
    Dint { hi: 0xecda9cdf9a4993ba, lo: 0x5dbeb9795455a5, ex: -8, sgn: 0x0 }, /* j=8251 */
    Dint { hi: 0xf0e21acdd6e7d412, lo: 0xb6ed80852ae6fd63, ex: -8, sgn: 0x0 }, /* j=8252 */
    Dint { hi: 0xf4e9b935685dbe0b, lo: 0xf237cff1acb306b3, ex: -8, sgn: 0x0 }, /* j=8253 */
    Dint { hi: 0xf8f178185a2ebfd9, lo: 0xd81648249cece4c, ex: -8, sgn: 0x0 }, /* j=8254 */
    Dint { hi: 0xfcf95778b80fbc98, lo: 0x176cd56887ac7fe9, ex: -8, sgn: 0x0 }, /* j=8255 */
    Dint { hi: 0x8080abac46f38946, lo: 0x662d417ced007a46, ex: -7, sgn: 0x0 }, /* j=8256 */
];

/// `T1_2[i]` approximates 2^(i/2^6) (C `T1_2`).
#[rustfmt::skip]
static T1_2: [Dint; 64] = [
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8164d1f3bc030773, lo: 0x7be56527bd14def5, ex: 0, sgn: 0x0 },
    Dint { hi: 0x82cd8698ac2ba1d7, lo: 0x3e2a475b46520bff, ex: 0, sgn: 0x0 },
    Dint { hi: 0x843a28c3acde4046, lo: 0x1af92eca13fd1582, ex: 0, sgn: 0x0 },
    Dint { hi: 0x85aac367cc487b14, lo: 0xc5c95b8c2154c1b2, ex: 0, sgn: 0x0 },
    Dint { hi: 0x871f61969e8d1010, lo: 0x3a1727c57b52a956, ex: 0, sgn: 0x0 },
    Dint { hi: 0x88980e8092da8527, lo: 0x5df8d76c98c67563, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8a14d575496efd9a, lo: 0x80ca1d92c3680c2, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8b95c1e3ea8bd6e6, lo: 0xfbe4628758a53c90, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8d1adf5b7e5ba9e5, lo: 0xb4c7b4968e41ad36, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8ea4398b45cd53c0, lo: 0x2dc0144c8783d4c6, ex: 0, sgn: 0x0 },
    Dint { hi: 0x9031dc431466b1dc, lo: 0x775814a8494e87e2, ex: 0, sgn: 0x0 },
    Dint { hi: 0x91c3d373ab11c336, lo: 0xfd6d8e0ae5ac9d8, ex: 0, sgn: 0x0 },
    Dint { hi: 0x935a2b2f13e6e92b, lo: 0xd339940e9d924ee7, ex: 0, sgn: 0x0 },
    Dint { hi: 0x94f4efa8fef70961, lo: 0x2e8afad12551de54, ex: 0, sgn: 0x0 },
    Dint { hi: 0x96942d3720185a00, lo: 0x48ea9b683a9c22c5, ex: 0, sgn: 0x0 },
    Dint { hi: 0x9837f0518db8a96f, lo: 0x46ad23182e42f6f6, ex: 0, sgn: 0x0 },
    Dint { hi: 0x99e0459320b7fa64, lo: 0xe43086cb34b5fcaf, ex: 0, sgn: 0x0 },
    Dint { hi: 0x9b8d39b9d54e5538, lo: 0xa2a817a2a3cc3f1f, ex: 0, sgn: 0x0 },
    Dint { hi: 0x9d3ed9a72cffb750, lo: 0xde494cf050e99b0b, ex: 0, sgn: 0x0 },
    Dint { hi: 0x9ef5326091a111ad, lo: 0xa0911f09ebb9fdd1, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa0b0510fb9714fc2, lo: 0x192dc79edb0fd9a9, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa27043030c496818, lo: 0x9b7a04ef80cfdea8, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa43515ae09e6809e, lo: 0xd1db4831781e1ef, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa5fed6a9b15138ea, lo: 0x1cbd7f621710701b, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa7cd93b4e9653569, lo: 0x9ec5b4d5039f72af, ex: 0, sgn: 0x0 },
    Dint { hi: 0xa9a15ab4ea7c0ef8, lo: 0x541e24ec3531fa73, ex: 0, sgn: 0x0 },
    Dint { hi: 0xab7a39b5a93ed337, lo: 0x658023b2759e0079, ex: 0, sgn: 0x0 },
    Dint { hi: 0xad583eea42a14ac6, lo: 0x4980a8c8f59a2ec4, ex: 0, sgn: 0x0 },
    Dint { hi: 0xaf3b78ad690a4374, lo: 0xdf26101ccbb35033, ex: 0, sgn: 0x0 },
    Dint { hi: 0xb123f581d2ac258f, lo: 0x87d037e96d215d8e, ex: 0, sgn: 0x0 },
    Dint { hi: 0xb311c412a9112489, lo: 0x3ecf14dc798a519c, ex: 0, sgn: 0x0 },
    Dint { hi: 0xb504f333f9de6484, lo: 0x597d89b3754abe9f, ex: 0, sgn: 0x0 },
    Dint { hi: 0xb6fd91e328d17791, lo: 0x7165f0ddd541a5a, ex: 0, sgn: 0x0 },
    Dint { hi: 0xb8fbaf4762fb9ee9, lo: 0x1b879778566b65a2, ex: 0, sgn: 0x0 },
    Dint { hi: 0xbaff5ab2133e45fb, lo: 0x74d519d24593838c, ex: 0, sgn: 0x0 },
    Dint { hi: 0xbd08a39f580c36be, lo: 0xa8811fb66d0faf7a, ex: 0, sgn: 0x0 },
    Dint { hi: 0xbf1799b67a731082, lo: 0xe815d0abcbf0b851, ex: 0, sgn: 0x0 },
    Dint { hi: 0xc12c4cca66709456, lo: 0x7c457d59a50087b5, ex: 0, sgn: 0x0 },
    Dint { hi: 0xc346ccda24976407, lo: 0x20ec856128b83a42, ex: 0, sgn: 0x0 },
    Dint { hi: 0xc5672a115506dadd, lo: 0x3e2ad0c964dd9f37, ex: 0, sgn: 0x0 },
    Dint { hi: 0xc78d74c8abb9b15c, lo: 0xc13a2e3976c0277e, ex: 0, sgn: 0x0 },
    Dint { hi: 0xc9b9bd866e2f27a2, lo: 0x80e1f92a0511697e, ex: 0, sgn: 0x0 },
    Dint { hi: 0xcbec14fef2727c5c, lo: 0xf4907c8f45ebf6dd, ex: 0, sgn: 0x0 },
    Dint { hi: 0xce248c151f8480e3, lo: 0xe235838f95f2c6ed, ex: 0, sgn: 0x0 },
    Dint { hi: 0xd06333daef2b2594, lo: 0xd6d45c6559a4d502, ex: 0, sgn: 0x0 },
    Dint { hi: 0xd2a81d91f12ae45a, lo: 0x12248e57c3de4028, ex: 0, sgn: 0x0 },
    Dint { hi: 0xd4f35aabcfedfa1f, lo: 0x5921deffa6262c5b, ex: 0, sgn: 0x0 },
    Dint { hi: 0xd744fccad69d6af4, lo: 0x39a68bb9902d3fde, ex: 0, sgn: 0x0 },
    Dint { hi: 0xd99d15c278afd7b5, lo: 0xfe873deca3e12bac, ex: 0, sgn: 0x0 },
    Dint { hi: 0xdbfbb797daf23755, lo: 0x3d840d5a9e29aa64, ex: 0, sgn: 0x0 },
    Dint { hi: 0xde60f4825e0e9123, lo: 0xdd07a2d9e8466859, ex: 0, sgn: 0x0 },
    Dint { hi: 0xe0ccdeec2a94e111, lo: 0x65895048dd333ca, ex: 0, sgn: 0x0 },
    Dint { hi: 0xe33f8972be8a5a51, lo: 0x9bfe90795980eed, ex: 0, sgn: 0x0 },
    Dint { hi: 0xe5b906e77c8348a8, lo: 0x1e5e8f4a4edbb0ed, ex: 0, sgn: 0x0 },
    Dint { hi: 0xe8396a503c4bdc68, lo: 0x791790d0ac70c7de, ex: 0, sgn: 0x0 },
    Dint { hi: 0xeac0c6e7dd24392e, lo: 0xd02d75b3706e54fb, ex: 0, sgn: 0x0 },
    Dint { hi: 0xed4f301ed9942b84, lo: 0x600d2db6a64bfb12, ex: 0, sgn: 0x0 },
    Dint { hi: 0xefe4b99bdcdaf5cb, lo: 0x46561cf6948db913, ex: 0, sgn: 0x0 },
    Dint { hi: 0xf281773c59ffb139, lo: 0xe8980a9cc8f47a4b, ex: 0, sgn: 0x0 },
    Dint { hi: 0xf5257d152486cc2c, lo: 0x7b9d0c7aed980fc3, ex: 0, sgn: 0x0 },
    Dint { hi: 0xf7d0df730ad13bb8, lo: 0xfe90d496d60fb6eb, ex: 0, sgn: 0x0 },
    Dint { hi: 0xfa83b2db722a033a, lo: 0x7c25bb14315d7fcd, ex: 0, sgn: 0x0 },
    Dint { hi: 0xfd3e0c0cf486c174, lo: 0x853f3a5931e0ee03, ex: 0, sgn: 0x0 },
];

/// `T2_2[i]` approximates 2^(i/2^12) (C `T2_2`).
#[rustfmt::skip]
static T2_2: [Dint; 64] = [
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80058baf7fee3b5d, lo: 0x1c718b38e549cb93, ex: 0, sgn: 0x0 },
    Dint { hi: 0x800b179c82028fd0, lo: 0x945e54e2ae18f2f0, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8010a3c708e73282, lo: 0x2b96d62d51c15a07, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8016302f17467628, lo: 0x3690dfe44d11d008, ex: 0, sgn: 0x0 },
    Dint { hi: 0x801bbcd4afcacb08, lo: 0xe23a986bd3e626f0, ex: 0, sgn: 0x0 },
    Dint { hi: 0x802149b7d51ebefb, lo: 0x7bdbadbc888aeb29, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8026d6d889ecfd69, lo: 0xb904bbfb40d3a2b7, ex: 0, sgn: 0x0 },
    Dint { hi: 0x802c6436d0e04f50, lo: 0xff8ce94a6797b3ce, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8031f1d2aca39b43, lo: 0xad9db772901d96b6, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80377fac1fe1e56a, lo: 0x61cd0bffd7cfc683, ex: 0, sgn: 0x0 },
    Dint { hi: 0x803d0dc32d464f85, lo: 0x43456f71b96affd4, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80429c17d77c18ed, lo: 0x49fc841afba9c3c6, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80482aaa212e9e95, lo: 0x86f7b54f6c45c85e, ex: 0, sgn: 0x0 },
    Dint { hi: 0x804db97a0d095b0c, lo: 0x6c9f1f7d1efcfe68, ex: 0, sgn: 0x0 },
    Dint { hi: 0x805348879db7e67d, lo: 0x171eb1ceef1d1f28, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8058d7d2d5e5f6b0, lo: 0x94d589f608ee4aa2, ex: 0, sgn: 0x0 },
    Dint { hi: 0x805e675bb83f5f0f, lo: 0x2ed38ab8472b2144, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8063f722477010a1, lo: 0xb1652de1378af1a1, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8069872686241a12, lo: 0xb4ad9233a0390cad, ex: 0, sgn: 0x0 },
    Dint { hi: 0x806f17687707a7af, lo: 0xe54ec5f966eb1872, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8074a7e81cc7036b, lo: 0x4d204ecfc11f4aab, ex: 0, sgn: 0x0 },
    Dint { hi: 0x807a38a57a0e94dc, lo: 0x9bf3ef4d9be2d1e4, ex: 0, sgn: 0x0 },
    Dint { hi: 0x807fc9a0918ae142, lo: 0x7068ab2230585d13, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80855ad965e88b83, lo: 0xa0cc0a49c10ea66b, ex: 0, sgn: 0x0 },
    Dint { hi: 0x808aec4ff9d45430, lo: 0x84099bf6830f2768, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80907e044ffb1984, lo: 0x3aa8b9cbbc65a8ab, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80960ff66b09d765, lo: 0xf7d88c0928ba3947, ex: 0, sgn: 0x0 },
    Dint { hi: 0x809ba2264dada76a, lo: 0x4a8a4f44bb703db6, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80a13493fa93c0d4, lo: 0x6699dc50dd96b774, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80a6c73f74697897, lo: 0x6e0472ed4ccfa2e0, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80ac5a28bddc4157, lo: 0xba2dc7e0c72e51ba, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80b1ed4fd999ab6c, lo: 0x25335719b6e6fd20, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80b780b4ca4f64df, lo: 0x534dfa7417846aa4, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80bd145792ab3970, lo: 0xfc41c5c2d5336ccc, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80c2a838355b1297, lo: 0x34dc28baed8f3fde, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80c83c56b50cf77f, lo: 0xb880575ea03548c1, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80cdd0b3146f0d11, lo: 0x32c1f98704428c71, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80d3654d562f95ec, lo: 0x890e222a5eb95372, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80d8fa257cfcf26e, lo: 0x24628efd9ca9d59b, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80de8f3b8b85a0af, lo: 0x3b13310f5ad57fb1, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80e4248f84783c87, lo: 0x1a9dfefaeb616564, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80e9ba216a837f8c, lo: 0x718d1151d109bf98, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80ef4ff140564116, lo: 0x996709da2e25f04c, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80f4e5ff089f763e, lo: 0xe0adc640acaa6b0b, ex: 0, sgn: 0x0 },
    Dint { hi: 0x80fa7c4ac60e31e1, lo: 0xd4eb5edc6b341283, ex: 0, sgn: 0x0 },
    Dint { hi: 0x810012d47b51a4a0, lo: 0x8ccd7223820719e3, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8105a99c2b191ce1, lo: 0xf24ebd6eb9ca4292, ex: 0, sgn: 0x0 },
    Dint { hi: 0x810b40a1d81406d4, lo: 0xcef03ab14a66550, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8110d7e584f1ec6d, lo: 0x4bf94297d1519822, ex: 0, sgn: 0x0 },
    Dint { hi: 0x81166f673462756d, lo: 0xd0d8372f966cf15e, ex: 0, sgn: 0x0 },
    Dint { hi: 0x811c0726e9156760, lo: 0xb97931db7b7be2ec, ex: 0, sgn: 0x0 },
    Dint { hi: 0x81219f24a5baa59d, lo: 0x6abd3b0eab9c7048, ex: 0, sgn: 0x0 },
    Dint { hi: 0x812737606d023148, lo: 0xdaf888e96508151a, ex: 0, sgn: 0x0 },
    Dint { hi: 0x812ccfda419c2956, lo: 0xdc8046821f46122e, ex: 0, sgn: 0x0 },
    Dint { hi: 0x813268922638ca8b, lo: 0x6846ad73a8d9027f, ex: 0, sgn: 0x0 },
    Dint { hi: 0x813801881d886f7b, lo: 0xe885724f14131287, ex: 0, sgn: 0x0 },
    Dint { hi: 0x813d9abc2a3b9090, lo: 0x83768490519df895, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8143342e4f02c405, lo: 0x661b22b45e25de18, ex: 0, sgn: 0x0 },
    Dint { hi: 0x8148cdde8e8ebdec, lo: 0xf11430fef78c6ee, ex: 0, sgn: 0x0 },
    Dint { hi: 0x814e67cceb90502c, lo: 0x99775205944eadc4, ex: 0, sgn: 0x0 },
    Dint { hi: 0x815401f968b86a87, lo: 0x7de463a40d18261, ex: 0, sgn: 0x0 },
    Dint { hi: 0x81599c6408b81a94, lo: 0x8f4a0b6748df7960, ex: 0, sgn: 0x0 },
    Dint { hi: 0x815f370cce408bc8, lo: 0xe2404468cfe5ab9f, ex: 0, sgn: 0x0 },
];

/// Degree 9 to 1 (largest first) of a polynomial for log(1+z), |z| < 0.0001221, relative
/// error below 2^-128.316 (C `P_2`).
#[rustfmt::skip]
static P_2: [Dint; 9] = [
    Dint { hi: 0xe38e3954a09e560e, lo: 0x0, ex: -4, sgn: 0x0 },
    Dint { hi: 0x800000399d09d767, lo: 0x0, ex: -3, sgn: 0x1 },
    Dint { hi: 0x9249249249248676, lo: 0x0, ex: -3, sgn: 0x0 },
    Dint { hi: 0xaaaaaaaaaaaa9fdd, lo: 0x0, ex: -3, sgn: 0x1 },
    Dint { hi: 0xcccccccccccccccc, lo: 0xcccdc5fe0ef93b8d, ex: -3, sgn: 0x0 },
    Dint { hi: 0x8000000000000000, lo: 0x600135b960d8, ex: -2, sgn: 0x1 },
    Dint { hi: 0xaaaaaaaaaaaaaaaa, lo: 0xaaaaaaaaaaa77b5e, ex: -2, sgn: 0x0 },
    Dint { hi: 0xffffffffffffffff, lo: 0xfffffffffffe33ca, ex: -2, sgn: 0x1 },
    Dint { hi: 0x8000000000000000, lo: 0x0, ex: 0, sgn: 0x0 },
];

/// Degree 7 to 0 (largest first) of a polynomial for exp(z), |z| < 0.00016923, relative
/// error below 2^-122.415 (C `Q_2`).
#[rustfmt::skip]
static Q_2: [Dint; 8] = [
    Dint { hi: 0xd00d00cd98416862, lo: 0x0, ex: -13, sgn: 0x0 },
    Dint { hi: 0xb60b60b932146a54, lo: 0x0, ex: -10, sgn: 0x0 },
    Dint { hi: 0x8888888888888897, lo: 0x0, ex: -7, sgn: 0x0 },
    Dint { hi: 0xaaaaaaaaaaaaaaa3, lo: 0x0, ex: -5, sgn: 0x0 },
    Dint { hi: 0xaaaaaaaaaaaaaaaa, lo: 0xaaaaaa6a1e0776ae, ex: -3, sgn: 0x0 },
    Dint { hi: 0x8000000000000000, lo: 0xc06f3cd29, ex: -1, sgn: 0x0 },
    Dint { hi: 0x8000000000000000, lo: 0x88, ex: 0, sgn: 0x0 },
    Dint { hi: 0xffffffffffffffff, lo: 0xffffffffffffffd0, ex: -1, sgn: 0x0 },
];

/// exp(y) for |y| < 0.00016923, relative error below 2^-122.29 (C `q_2`).
fn q_2(y: &Dint) -> Dint {
    let mut r = mul_dint_11(y, &Q_2[0]);
    r = add_dint_11(&Q_2[1], &r);
    r = mul_dint_11(y, &r);
    r = add_dint_11(&Q_2[2], &r);
    r = mul_dint_11(y, &r);
    r = add_dint(&Q_2[3], &r);
    for c in &Q_2[4..8] {
        r = mul_dint(y, &r);
        r = add_dint(c, &r);
    }
    r
}

/// log(1+z) for |z| <= 2^-13 with z a multiple of 2^-61 (C `p_2`).
fn p_2(z: &Dint) -> Dint {
    let mut r = mul_dint_11(z, &P_2[0]);
    r = add_dint_11(&P_2[1], &r);
    r = mul_dint_11(z, &r);
    r = add_dint_11(&P_2[2], &r);
    r = mul_dint_11(z, &r);
    r = add_dint_11(&P_2[3], &r);
    r = mul_dint_11(z, &r);
    r = add_dint(&P_2[4], &r);
    for c in &P_2[5..9] {
        r = mul_dint_21(&r, z);
        r = add_dint(c, &r);
    }
    mul_dint_21(&r, z)
}

/// log(x), relative error below 2^-122.88 (C `log_2`).
fn log_2(x: &Dint) -> Dint {
    let mut x = *x;
    let mut e = x.ex;
    // The first lookup index i = floor(x*2^7).
    let i = if x.hi > 0xb504f333f9de6484 {
        // hi/2^63 > sqrt(2)
        e += 1;
        (x.hi >> (63 + 1 - 7)) as u16
    } else {
        (x.hi >> (63 - 7)) as u16
    } as usize;
    x.ex -= e;
    let mut z = mul_dint_11(&x, &INVERSE_2_1[i - 90]);
    let j = (z.hi >> (63 - 13 - z.ex)) as u16 as usize;
    z = mul_dint_11(&z, &INVERSE_2_2[j - 8128]);
    z = add_dint(&M_ONE, &z);
    let r = mul_dint_int64(&LOG2, e);
    let mut p = p_2(&z);
    p = add_dint(&LOG_INV_2_2[j - 8128], &p);
    p = add_dint(&LOG_INV_2_1[i - 90], &p);
    add_dint(&p, &r)
}

/// exp(x) for |x| < 744.45, relative error below 2^-121.70 (C `exp_2`).
fn exp_2(x: &Dint) -> Dint {
    if x.ex >= 10 {
        // Underflow or overflow.
        let mut r = *x;
        r.ex = if x.sgn == 0x1 { -1076 } else { 1025 };
        r.sgn = 0;
        return r;
    }
    let big_k = mul_dint_11(x, &LOG2_INV);
    let k = dint_toi(&big_k);
    let mut big_k = mul_dint_int64(&LOG2, k);
    big_k.ex -= 12;
    big_k.sgn = u64::from(big_k.sgn == 0);
    let y = add_dint(x, &big_k);
    let m = k >> 12;
    let i2 = ((k >> 6) & 0x3f) as usize;
    let i1 = (k & 0x3f) as usize;
    let mut r = q_2(&y);
    r = mul_dint(&T1_2[i2], &r);
    r = mul_dint(&T2_2[i1], &r);
    r.ex += m;
    r
}

// ---------------------------------------------------------------------------------------------
// Phase 3: 256-bit floating point (C qint.h, and pow.h `qint_tod`). Integers only.
// ---------------------------------------------------------------------------------------------

/// (-1)^sgn * (hh + hl/2^64 + lh/2^128 + ll/2^192) * 2^(ex-63) (C `qint64_t`; `rh` there is
/// hh:hl, `rl` is lh:ll).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Qint {
    hh: u64,
    hl: u64,
    lh: u64,
    ll: u64,
    ex: i64,
    sgn: u64,
}

impl Qint {
    #[inline(always)]
    fn rh(&self) -> u128 {
        (u128::from(self.hh) << 64) | u128::from(self.hl)
    }

    #[inline(always)]
    fn rl(&self) -> u128 {
        (u128::from(self.lh) << 64) | u128::from(self.ll)
    }

    #[inline(always)]
    fn with_r(rh: u128, rl: u128, ex: i64, sgn: u64) -> Qint {
        Qint {
            hh: (rh >> 64) as u64,
            hl: rh as u64,
            lh: (rl >> 64) as u64,
            ll: rl as u64,
            ex,
            sgn,
        }
    }
}

/// -1 (exact).
const M_ONE_Q: Qint = Qint {
    hh: 0x8000000000000000,
    hl: 0x0,
    lh: 0x0,
    ll: 0x0,
    ex: 0,
    sgn: 0x1,
};
/// log(2), absolute error below 2^-256.14.
const LOG2_Q: Qint = Qint {
    hh: 0xb17217f7d1cf79ab,
    hl: 0xc9e3b39803f2f6af,
    lh: 0x40f343267298b62d,
    ll: 0x8a0d175b8baafa2b,
    ex: -1,
    sgn: 0x0,
};
/// 2^12/log(2), absolute error below 2^-52.96.
const LOG2_INV_Q: Qint = Qint {
    hh: 0xb8aa3b295c17f0bc,
    hl: 0x0,
    lh: 0x0,
    ll: 0x0,
    ex: 12,
    sgn: 0x0,
};
/// 0 (exact).
const ZERO_Q: Qint = Qint {
    hh: 0x0,
    hl: 0x0,
    lh: 0x0,
    ll: 0x0,
    ex: 0,
    sgn: 0x0,
};

/// The number of leading zeros of the 256-bit ch:cl (ch:cl nonzero).
#[inline(always)]
fn clz256(ch: u128, cl: u128) -> u32 {
    if ch != 0 {
        ch.leading_zeros()
    } else {
        128 + cl.leading_zeros()
    }
}

/// a + b, error at most 2 ulps (ulp_256); exact in the Sterbenz case (C `add_qint`).
fn add_qint(a: &Qint, b: &Qint) -> Qint {
    use core::cmp::Ordering;
    if a.rh() == 0 && a.rl() == 0 {
        return *b;
    }
    if b.rh() == 0 && b.rl() == 0 {
        return *a;
    }
    match (a.ex, a.rh(), a.rl()).cmp(&(b.ex, b.rh(), b.rl())) {
        Ordering::Equal => {
            if a.sgn ^ b.sgn != 0 {
                return ZERO_Q;
            }
            let mut r = *a;
            r.ex += 1;
            return r;
        }
        Ordering::Less => return add_qint(b, a),
        Ordering::Greater => {}
    }
    // From now on |a| > |b|.
    let (mut ah, mut al, mut bh, mut bl) = (a.rh(), a.rl(), b.rh(), b.rl());
    let k = a.ex - b.ex;
    if k > 0 {
        if k >= 128 {
            bl = if k < 256 { bh >> (k - 128) } else { 0 };
            bh = 0;
        } else {
            bl = (bl >> k) | (bh << (128 - k));
            bh >>= k;
        }
    }
    let sgn = a.sgn;
    let mut ex_r = a.ex;
    let (mut ch, mut cl);
    if a.sgn ^ b.sgn != 0 {
        // Subtraction: C = A - B (not 0: |A| > |B|).
        ch = ah.wrapping_sub(bh);
        let (l, borrow) = al.overflowing_sub(bl);
        cl = l;
        if borrow {
            ch = ch.wrapping_sub(1);
        }
        let mut ex = clz256(ch, cl);
        if ex > 0 {
            // Shift A by ex bits to the left, and B by ex - k bits to the left.
            if ex >= 128 {
                ah = al << (ex - 128);
                al = 0;
            } else {
                ah = (ah << ex) | (al >> (128 - ex));
                al <<= ex;
            }
            let sh = i64::from(ex) - k;
            bh = b.rh();
            bl = b.rl();
            if sh >= 0 {
                if sh >= 128 {
                    bh = bl << (sh - 128);
                    bl = 0;
                } else if sh > 0 {
                    bh = (bh << sh) | (bl >> (128 - sh));
                    bl <<= sh;
                }
            } else {
                // Shift B right by -sh bits.
                let j = -sh;
                if j >= 128 {
                    bl = bh.checked_shr((j - 128) as u32).unwrap_or(0);
                    bh = 0;
                } else {
                    bl = (bh << (128 - j)) | (bl >> j);
                    bh >>= j;
                }
            }
            ex_r -= i64::from(ex);
            ch = ah.wrapping_sub(bh);
            let (l, borrow) = al.overflowing_sub(bl);
            cl = l;
            if borrow {
                ch = ch.wrapping_sub(1);
            }
            ex = clz256(ch, cl);
        }
        if ex != 0 {
            if ex >= 128 {
                ch = cl << (ex - 128);
                cl = 0;
            } else {
                ch = (ch << ex) | (cl >> (128 - ex));
                cl <<= ex;
            }
        }
        ex_r -= i64::from(ex);
    } else {
        // Addition.
        let (h, c1) = ah.overflowing_add(bh);
        ch = h;
        let mut cy = u32::from(c1);
        let (l, c2) = al.overflowing_add(bl);
        cl = l;
        if c2 {
            ch = ch.wrapping_add(1);
            cy += u32::from(ch == 0);
        }
        if cy != 0 {
            cl = (ch << 127) | (cl >> 1);
            ch = (1u128 << 127) | (ch >> 1);
            ex_r += 1;
        }
    }
    Qint::with_r(ch, cl, ex_r, sgn)
}

/// a + b from the upper 2 limbs, error below 2 ulps(128) (C `add_qint_22`).
fn add_qint_22(a: &Qint, b: &Qint) -> Qint {
    use core::cmp::Ordering;
    if a.rh() == 0 {
        return *b;
    }
    if b.rh() == 0 {
        return *a;
    }
    match (a.ex, a.rh()).cmp(&(b.ex, b.rh())) {
        Ordering::Equal => {
            if a.sgn ^ b.sgn != 0 {
                return ZERO_Q;
            }
            let mut r = *a;
            r.ex += 1;
            return r;
        }
        Ordering::Less => return add_qint_22(b, a),
        Ordering::Greater => {}
    }
    let mut ah = a.rh();
    let mut bh = b.rh();
    let k = (a.ex - b.ex) as u64;
    if k > 0 {
        bh = if k >= 128 { 0 } else { bh >> k };
    }
    let sgn = a.sgn;
    let mut ex_r = a.ex;
    let ch = if a.sgn ^ b.sgn != 0 {
        let mut ch = ah.wrapping_sub(bh);
        let mut ex = ch.leading_zeros();
        if ex > 0 {
            ah <<= ex;
            // B: k bits to the right and ex to the left (k < 128 here, C comment).
            bh = if u64::from(ex) >= k {
                b.rh() << (u64::from(ex) - k)
            } else {
                b.rh() >> (k - u64::from(ex))
            };
            ex_r -= i64::from(ex);
            ch = ah.wrapping_sub(bh);
            ex = ch.leading_zeros();
        }
        ex_r -= i64::from(ex);
        ch << ex
    } else {
        let (ch, cy) = ah.overflowing_add(bh);
        if cy {
            ex_r += 1;
            (1u128 << 127) | (ch >> 1)
        } else {
            ch
        }
    };
    Qint::with_r(ch, 0, ex_r, sgn)
}

/// The end of the C `mul_qint*` routines: normalize t6:t5 and set the exponent and sign.
#[inline(always)]
fn mul_qint_end(t6: u128, t5: u128, a: &Qint, b: &Qint) -> Qint {
    let ex = u64::from(t6 >> 127 == 0);
    let (rh, rl) = if ex != 0 {
        ((t6 << 1) | (t5 >> 127), t5 << 1)
    } else {
        (t6, t5)
    };
    Qint::with_r(rh, rl, a.ex + b.ex + 1 - ex as i64, a.sgn ^ b.sgn)
}

#[inline(always)]
fn mul64(a: u64, b: u64) -> u128 {
    u128::from(a) * u128::from(b)
}

const MASK64: u128 = 0xffffffffffffffff;

/// a * b, error below 14 ulps (C `mul_qint`).
fn mul_qint(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r32 = mul64(a.hh, b.hl);
    let r23 = mul64(a.hl, b.hh);
    let r31 = mul64(a.hh, b.lh);
    let r13 = mul64(a.lh, b.hh);
    let r22 = mul64(a.hl, b.hl);
    let r30 = mul64(a.hh, b.ll);
    let r03 = mul64(a.ll, b.hh);
    let r21 = mul64(a.hl, b.lh);
    let r12 = mul64(a.lh, b.hl);
    let t3 = (r12 >> 64) + (r21 >> 64) + (r03 >> 64) + (r30 >> 64);
    let (t4, c) = r22.overflowing_add(t3);
    let mut c4 = u128::from(c);
    let (t4, c) = r13.overflowing_add(t4);
    c4 += u128::from(c);
    let (t4, c) = r31.overflowing_add(t4);
    c4 += u128::from(c);
    let (t5, c) = r23.overflowing_add(t4 >> 64);
    let mut c5 = u128::from(c);
    let (t5, c) = r32.overflowing_add(t5);
    c5 += u128::from(c);
    let t6 = r33.wrapping_add((c5 << 64) | (t5 >> 64)).wrapping_add(c4);
    let t5 = (t5 << 64) | (t4 & MASK64);
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper 3 limbs, error below 6 ulps (C `mul_qint_33`).
fn mul_qint_33(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r32 = mul64(a.hh, b.hl);
    let r23 = mul64(a.hl, b.hh);
    let r31 = mul64(a.hh, b.lh);
    let r13 = mul64(a.lh, b.hh);
    let r22 = mul64(a.hl, b.hl);
    let r21 = mul64(a.hl, b.lh);
    let r12 = mul64(a.lh, b.hl);
    let t3 = (r12 >> 64) + (r21 >> 64);
    let (t4, c) = r22.overflowing_add(t3);
    let mut c4 = u128::from(c);
    let (t4, c) = r13.overflowing_add(t4);
    c4 += u128::from(c);
    let (t4, c) = r31.overflowing_add(t4);
    c4 += u128::from(c);
    let (t5, c) = r23.overflowing_add(t4 >> 64);
    let mut c5 = u128::from(c);
    let (t5, c) = r32.overflowing_add(t5);
    c5 += u128::from(c);
    let t6 = r33.wrapping_add((c5 << 64) | (t5 >> 64)).wrapping_add(c4);
    let t5 = (t5 << 64) | (t4 & MASK64);
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper limb of b, error below 2 ulps (C `mul_qint_41`).
fn mul_qint_41(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r23 = mul64(a.hl, b.hh);
    let r13 = mul64(a.lh, b.hh);
    let r03 = mul64(a.ll, b.hh);
    let t3 = r03 >> 64;
    let (t4, c) = r13.overflowing_add(t3);
    let c4 = u128::from(c);
    let (t5, c) = r23.overflowing_add(t4 >> 64);
    let c5 = u128::from(c);
    let t6 = r33.wrapping_add((c5 << 64) | (t5 >> 64)).wrapping_add(c4);
    let t5 = (t5 << 64) | (t4 & MASK64);
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper 3 limbs of a and the upper limb of b, exact (C `mul_qint_31`).
fn mul_qint_31(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r23 = mul64(a.hl, b.hh);
    let r13 = mul64(a.lh, b.hh);
    let t4 = r13;
    let (t5, c) = r23.overflowing_add(t4 >> 64);
    let c5 = u128::from(c);
    let t6 = r33.wrapping_add((c5 << 64) | (t5 >> 64));
    let t5 = (t5 << 64) | (t4 & MASK64);
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper 2 limbs, exact (C `mul_qint_22`).
fn mul_qint_22(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r32 = mul64(a.hh, b.hl);
    let r23 = mul64(a.hl, b.hh);
    let r22 = mul64(a.hl, b.hl);
    let t4 = r22;
    let (t5, c) = r23.overflowing_add(t4 >> 64);
    let mut c5 = u128::from(c);
    let (t5, c) = r32.overflowing_add(t5);
    c5 += u128::from(c);
    let t6 = r33.wrapping_add((c5 << 64) | (t5 >> 64));
    let t5 = (t5 << 64) | (t4 & MASK64);
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper 2 limbs of a and the upper limb of b, exact (C `mul_qint_21`).
fn mul_qint_21(a: &Qint, b: &Qint) -> Qint {
    let r33 = mul64(a.hh, b.hh);
    let r23 = mul64(a.hl, b.hh);
    let t6 = r33 + (r23 >> 64);
    let t5 = r23 << 64;
    mul_qint_end(t6, t5, a, b)
}

/// a * b from the upper limbs, exact (C `mul_qint_11`).
fn mul_qint_11(a: &Qint, b: &Qint) -> Qint {
    let t6 = mul64(a.hh, b.hh);
    let ex = u64::from(t6 >> 127 == 0);
    Qint::with_r(t6 << ex, 0, a.ex + b.ex + 1 - ex as i64, a.sgn ^ b.sgn)
}

/// b * a for an integer b, error below 2 ulps (C `mul_qint_2`).
fn mul_qint_2(b: i64, a: &Qint) -> Qint {
    if b == 0 {
        return ZERO_Q;
    }
    let mut c = b.unsigned_abs();
    let sgn = u64::from(b < 0) ^ a.sgn;
    if c == 1 {
        let mut r = *a;
        r.sgn = sgn;
        return r;
    }
    let mut ex_r = a.ex + 64;
    // Scale c so that 2^63 <= c < 2^64.
    let k = c.leading_zeros();
    c <<= k;
    ex_r -= i64::from(k);
    let mut t3 = mul64(a.hh, c);
    let mut t2 = mul64(a.hl, c);
    let mut t1 = mul64(a.lh, c);
    let t0 = mul64(a.ll, c);
    let t = t0 >> 64;
    let (s, cy) = t.overflowing_add(t1);
    t1 = s;
    let t = (u128::from(cy) << 64) | (t1 >> 64);
    let (s, cy) = t.overflowing_add(t2);
    t2 = s;
    t3 = t3.wrapping_add((u128::from(cy) << 64) | (t2 >> 64));
    let ex = ((t3 >> 64) as u64).leading_zeros();
    t2 = (t2 << 64) | (t1 & MASK64);
    if ex != 0 {
        ex_r -= 1;
        Qint::with_r((t3 << 1) | (t2 >> 127), t2 << 1, ex_r, sgn)
    } else {
        Qint::with_r(t3, t2, ex_r, sgn)
    }
}

/// x as a [`Qint`], exact (C `qint_fromd`).
fn qint_fromd(b: f64) -> Qint {
    let (ex, hh) = fast_extract(b);
    let t = hh.leading_zeros();
    Qint {
        hh: hh << t,
        hl: 0,
        lh: 0,
        ll: 0,
        ex: ex - if t > 11 { i64::from(t) - 12 } else { 0 },
        sgn: u64::from(b < 0.0),
    }
}

/// a as an integer (C `qint_toi`).
fn qint_toi(a: &Qint) -> i64 {
    if a.ex < 0 {
        return 0;
    }
    let r = (a.hh >> (63 - a.ex)) as i64;
    if a.sgn != 0 { -r } else { r }
}

/// Rounds a to nearest at the subnormal precision, for a.ex <= -1023 (C `subnormalize_qint`,
/// round to nearest). The sticky test reads `hh & (~0 >> ex)`, as in C.
fn subnormalize_qint(a: &mut Qint) {
    if a.ex > -1023 {
        return;
    }
    let ex = (-(1011 + a.ex)) as u32;
    let mut hi = a.hh.checked_shr(ex).unwrap_or(0);
    let md = a.hh.checked_shr(ex.wrapping_sub(1)).unwrap_or(0) & 0x1;
    let lo =
        (a.hh & (!0u64).checked_shr(ex).unwrap_or(0)) != 0 || a.hl != 0 || a.lh != 0 || a.ll != 0;
    hi += if lo { md } else { hi & md };
    a.hh = hi.checked_shl(ex).unwrap_or(0);
    a.hl = 0;
    a.lh = 0;
    a.ll = 0;
    if a.hh == 0 {
        a.ex += 1;
        a.hh = 1 << 63;
    }
}

/// a rounded to nearest (C `qint_tod`).
fn qint_tod(a: &mut Qint) -> f64 {
    subnormalize_qint(a);
    let mut r = (a.hh >> 11) | (0x3ff << 52);
    let mut rd = 0.0;
    if a.hh & 0x400 != 0 {
        rd += hf("0x1p-53");
    }
    if a.hh & 0x3ff != 0 || a.hl != 0 || a.lh != 0 || a.ll != 0 {
        rd += hf("0x1p-54");
    }
    if a.sgn != 0 {
        rd = -rd;
    }
    r |= a.sgn << 63;
    let mut rf = f64::from_bits(r) + rd;
    let e;
    if a.ex > -1023 {
        if a.ex > 1023 {
            if a.ex == 1024 {
                rf *= 2.0;
                e = hf("0x1p+1023");
            } else {
                rf = hf("0x1.fffffffffffffp+1023");
                e = hf("0x1.fffffffffffffp+1023");
            }
        } else {
            e = f64::from_bits((((a.ex + 1023) & 0x7ff) as u64) << 52);
        }
    } else if a.ex < -1074 {
        if a.ex == -1075 {
            rf *= 0.5;
            e = hf("0x1p-1074");
        } else {
            rf = hf("0x0.0000000000001p-1022");
            e = hf("0x0.0000000000001p-1022");
        }
    } else {
        e = f64::from_bits(1u64 << (a.ex + 1074));
    }
    rf * e
}

/// For 90 <= i <= 181, INVERSE_3_1[i - 90] approximates 1/x for i/2^7 <= x < (i+1)/2^7
/// (C `_INVERSE_3_1`).
#[rustfmt::skip]
static INVERSE_3_1: [Qint; 92] = [
    Qint { hh: 0xb500000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=90 */
    Qint { hh: 0xb300000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=91 */
    Qint { hh: 0xb100000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=92 */
    Qint { hh: 0xaf00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=93 */
    Qint { hh: 0xad80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=94 */
    Qint { hh: 0xab80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=95 */
    Qint { hh: 0xaa00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=96 */
    Qint { hh: 0xa800000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=97 */
    Qint { hh: 0xa680000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=98 */
    Qint { hh: 0xa480000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=99 */
    Qint { hh: 0xa300000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=100 */
    Qint { hh: 0xa180000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=101 */
    Qint { hh: 0xa000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=102 */
    Qint { hh: 0x9e80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=103 */
    Qint { hh: 0x9d00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=104 */
    Qint { hh: 0x9b80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=105 */
    Qint { hh: 0x9a00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=106 */
    Qint { hh: 0x9880000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=107 */
    Qint { hh: 0x9700000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=108 */
    Qint { hh: 0x9580000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=109 */
    Qint { hh: 0x9480000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=110 */
    Qint { hh: 0x9300000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=111 */
    Qint { hh: 0x9180000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=112 */
    Qint { hh: 0x9080000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=113 */
    Qint { hh: 0x8f00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=114 */
    Qint { hh: 0x8e00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=115 */
    Qint { hh: 0x8c80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=116 */
    Qint { hh: 0x8b80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=117 */
    Qint { hh: 0x8a80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=118 */
    Qint { hh: 0x8900000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=119 */
    Qint { hh: 0x8800000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=120 */
    Qint { hh: 0x8700000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=121 */
    Qint { hh: 0x8580000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=122 */
    Qint { hh: 0x8480000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=123 */
    Qint { hh: 0x8380000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=124 */
    Qint { hh: 0x8280000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=125 */
    Qint { hh: 0x8180000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=126 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=127 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* i=128 */
    Qint { hh: 0xfd00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=129 */
    Qint { hh: 0xfb00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=130 */
    Qint { hh: 0xf900000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=131 */
    Qint { hh: 0xf780000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=132 */
    Qint { hh: 0xf580000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=133 */
    Qint { hh: 0xf380000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=134 */
    Qint { hh: 0xf200000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=135 */
    Qint { hh: 0xf000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=136 */
    Qint { hh: 0xee80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=137 */
    Qint { hh: 0xec80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=138 */
    Qint { hh: 0xeb00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=139 */
    Qint { hh: 0xe900000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=140 */
    Qint { hh: 0xe780000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=141 */
    Qint { hh: 0xe600000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=142 */
    Qint { hh: 0xe480000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=143 */
    Qint { hh: 0xe300000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=144 */
    Qint { hh: 0xe100000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=145 */
    Qint { hh: 0xdf80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=146 */
    Qint { hh: 0xde00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=147 */
    Qint { hh: 0xdc80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=148 */
    Qint { hh: 0xdb00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=149 */
    Qint { hh: 0xd980000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=150 */
    Qint { hh: 0xd880000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=151 */
    Qint { hh: 0xd700000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=152 */
    Qint { hh: 0xd580000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=153 */
    Qint { hh: 0xd400000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=154 */
    Qint { hh: 0xd280000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=155 */
    Qint { hh: 0xd180000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=156 */
    Qint { hh: 0xd000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=157 */
    Qint { hh: 0xce80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=158 */
    Qint { hh: 0xcd80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=159 */
    Qint { hh: 0xcc00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=160 */
    Qint { hh: 0xcb00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=161 */
    Qint { hh: 0xc980000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=162 */
    Qint { hh: 0xc880000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=163 */
    Qint { hh: 0xc700000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=164 */
    Qint { hh: 0xc600000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=165 */
    Qint { hh: 0xc500000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=166 */
    Qint { hh: 0xc380000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=167 */
    Qint { hh: 0xc280000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=168 */
    Qint { hh: 0xc180000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=169 */
    Qint { hh: 0xc000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=170 */
    Qint { hh: 0xbf00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=171 */
    Qint { hh: 0xbe00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=172 */
    Qint { hh: 0xbd00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=173 */
    Qint { hh: 0xbc00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=174 */
    Qint { hh: 0xba80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=175 */
    Qint { hh: 0xb980000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=176 */
    Qint { hh: 0xb880000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=177 */
    Qint { hh: 0xb780000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=178 */
    Qint { hh: 0xb680000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=179 */
    Qint { hh: 0xb580000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=180 */
    Qint { hh: 0xb480000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* i=181 */
];

/// For 8128 <= j <= 8256, INVERSE_3_2[j - 8128] (C `_INVERSE_3_2`).
#[rustfmt::skip]
static INVERSE_3_2: [Qint; 129] = [
    Qint { hh: 0x8100000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8128 */
    Qint { hh: 0x80fc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8129 */
    Qint { hh: 0x80f8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8130 */
    Qint { hh: 0x80f4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8131 */
    Qint { hh: 0x80f0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8132 */
    Qint { hh: 0x80ec000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8133 */
    Qint { hh: 0x80e8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8134 */
    Qint { hh: 0x80e4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8135 */
    Qint { hh: 0x80e0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8136 */
    Qint { hh: 0x80dc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8137 */
    Qint { hh: 0x80d8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8138 */ 
   Qint { hh: 0x80d4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8139 */
    Qint { hh: 0x80d0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8140 */
    Qint { hh: 0x80cc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8141 */
    Qint { hh: 0x80c8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8142 */
    Qint { hh: 0x80c4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8143 */
    Qint { hh: 0x80c0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8144 */
    Qint { hh: 0x80bc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8145 */
    Qint { hh: 0x80b8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8146 */
    Qint { hh: 0x80b4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8147 */
    Qint { hh: 0x80b0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8148 */
    Qint { hh: 0x80ac000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8149 */
    Qint { hh: 0x80a8000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8150 */
    Qint { hh: 0x80a4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8151 */
    Qint { hh: 0x80a0000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8152 */
    Qint { hh: 0x809c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8153 */
    Qint { hh: 0x8098000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8154 */
    Qint { hh: 0x8094000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8155 */
    Qint { hh: 0x8090000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8156 */
    Qint { hh: 0x808c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8157 */
    Qint { hh: 0x8088000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8158 */
    Qint { hh: 0x8084000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8159 */
    Qint { hh: 0x8080000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8160 */
    Qint { hh: 0x807c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8161 */
    Qint { hh: 0x8078000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8162 */
    Qint { hh: 0x8074000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8163 */
    Qint { hh: 0x8070000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8164 */
    Qint { hh: 0x806c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8165 */
    Qint { hh: 0x8068000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8166 */
    Qint { hh: 0x8064000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8167 */
    Qint { hh: 0x8060000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8168 */
    Qint { hh: 0x805c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8169 */
    Qint { hh: 0x8058000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8170 */
    Qint { hh: 0x8054000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8171 */
    Qint { hh: 0x8050000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8172 */
    Qint { hh: 0x804c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8173 */
    Qint { hh: 0x8048000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8174 */
    Qint { hh: 0x8044000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8175 */
    Qint { hh: 0x8040000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8176 */
    Qint { hh: 0x803c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8177 */
    Qint { hh: 0x8038000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8178 */
    Qint { hh: 0x8034000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8179 */
    Qint { hh: 0x8030000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8180 */
    Qint { hh: 0x802c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8181 */
    Qint { hh: 0x8028000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8182 */
    Qint { hh: 0x8024000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8183 */
    Qint { hh: 0x8020000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8184 */
    Qint { hh: 0x801c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8185 */
    Qint { hh: 0x8018000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8186 */
    Qint { hh: 0x8014000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8187 */
    Qint { hh: 0x8010000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8188 */
    Qint { hh: 0x800c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8189 */
    Qint { hh: 0x8008000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8190 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8191 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* j=8192 */
    Qint { hh: 0xfff4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8193 */
    Qint { hh: 0xffec000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8194 */
    Qint { hh: 0xffe4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8195 */
    Qint { hh: 0xffdc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8196 */
    Qint { hh: 0xffd4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8197 */
    Qint { hh: 0xffcc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8198 */
    Qint { hh: 0xffc4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8199 */
    Qint { hh: 0xffbc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8200 */
    Qint { hh: 0xffb4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8201 */
    Qint { hh: 0xffac000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8202 */
    Qint { hh: 0xffa4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8203 */
    Qint { hh: 0xff9c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8204 */
    Qint { hh: 0xff94000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8205 */
    Qint { hh: 0xff8c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8206 */
    Qint { hh: 0xff84000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8207 */
    Qint { hh: 0xff7c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8208 */
    Qint { hh: 0xff74000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8209 */
    Qint { hh: 0xff6c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8210 */
    Qint { hh: 0xff64000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8211 */
    Qint { hh: 0xff5c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8212 */
    Qint { hh: 0xff54000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8213 */
    Qint { hh: 0xff4c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8214 */
    Qint { hh: 0xff44000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8215 */
    Qint { hh: 0xff3c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8216 */
    Qint { hh: 0xff34000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8217 */
    Qint { hh: 0xff2c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8218 */
    Qint { hh: 0xff24000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8219 */
    Qint { hh: 0xff1c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8220 */
    Qint { hh: 0xff14000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8221 */
    Qint { hh: 0xff0c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8222 */
    Qint { hh: 0xff04000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8223 */
    Qint { hh: 0xfefc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8224 */
    Qint { hh: 0xfef4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8225 */
    Qint { hh: 0xfeec000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8226 */
    Qint { hh: 0xfee4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8227 */
    Qint { hh: 0xfedc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8228 */
    Qint { hh: 0xfed4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8229 */
    Qint { hh: 0xfecc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8230 */
    Qint { hh: 0xfec4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8231 */
    Qint { hh: 0xfebc000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8232 */
    Qint { hh: 0xfeb4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8233 */
    Qint { hh: 0xfeac000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8234 */
    Qint { hh: 0xfea4000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8235 */
    Qint { hh: 0xfe9c000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8236 */
    Qint { hh: 0xfe98000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8237 */
    Qint { hh: 0xfe90000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8238 */
    Qint { hh: 0xfe88000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8239 */
    Qint { hh: 0xfe80000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8240 */
    Qint { hh: 0xfe78000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8241 */
    Qint { hh: 0xfe70000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8242 */
    Qint { hh: 0xfe68000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8243 */
    Qint { hh: 0xfe60000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8244 */
    Qint { hh: 0xfe58000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8245 */
    Qint { hh: 0xfe50000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8246 */
    Qint { hh: 0xfe48000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8247 */
    Qint { hh: 0xfe40000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8248 */
    Qint { hh: 0xfe38000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8249 */
    Qint { hh: 0xfe30000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8250 */
    Qint { hh: 0xfe28000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8251 */
    Qint { hh: 0xfe20000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8252 */
    Qint { hh: 0xfe18000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8253 */
    Qint { hh: 0xfe10000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8254 */
    Qint { hh: 0xfe08000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8255 */
    Qint { hh: 0xfe00000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: -1, sgn: 0x0 }, /* j=8256 */
];

/// LOG_INV_3_1[i - 90] approximates -log(INVERSE_3_1[i - 90]) (C `_LOG_INV_3_1`).
#[rustfmt::skip]
static LOG_INV_3_1: [Qint; 92] = [
    Qint { hh: 0xb1641795ce3ca97b, hl: 0x7af915300e517391, lh: 0x362aee92bfa25a80, ll: 0x1646679ea2568305, ex: -2, sgn: 0x1 }, /* i=90 */
    Qint { hh: 0xabb3b8ba2ad362a4, hl: 0xd5b6506cc17a01f1, lh: 0x706866327ef7c050, ll: 0x628f2d55f109eac9, ex: -2, sgn: 0x1 }, /* i=91 */
    Qint { hh: 0xa5f2fcabbbc506da, hl: 0x64ca4fb7ec323d72, lh: 0xa68b0ce7a5e0a7ea, ll: 0xd6cad0f0b6c847ef, ex: -2, sgn: 0x1 }, /* i=92 */
    Qint { hh: 0xa0218434353f1de8, hl: 0x6093efa632530ac8, lh: 0x304fb3b2345b41a9, ll: 0xe440d92b32eac488, ex: -2, sgn: 0x1 }, /* i=93 */
    Qint { hh: 0x9bb93315fec2d792, hl: 0xa7589fba0865790d, lh: 0x82b75e91fcdfa14e, ll: 0x98d12c3138e33333, ex: -2, sgn: 0x1 }, /* i=94 */
    Qint { hh: 0x95c981d5c4e924ed, hl: 0x29404f5aa577d6b1, lh: 0xba0ea3f2ae1e1d07, ll: 0x758e4ab7f718ea9f, ex: -2, sgn: 0x1 }, /* i=95 */
    Qint { hh: 0x914a0fde7bcb2d12, hl: 0x1429ed3aea197a5d, lh: 0x355a6f4f0ec5ce8f, ll: 0xc4eca5fff76cbf20, ex: -2, sgn: 0x1 }, /* i=96 */
    Qint { hh: 0x8b3ae55d5d30701c, hl: 0xe63eab883717047e, lh: 0xcfa09487833ea69, ll: 0x8791b8732b281e2f, ex: -2, sgn: 0x1 }, /* i=97 */
    Qint { hh: 0x86a35abcd5ba5903, hl: 0xec81c3cbd925cccf, lh: 0x6a2f869f2c41ea0a, ll: 0x261913d1bbc49faf, ex: -2, sgn: 0x1 }, /* i=98 */
    Qint { hh: 0x8073622d6a80e634, hl: 0x6a97009015316070, lh: 0x9f1d0d49f7cf8122, ll: 0xc0baf08f2dd617cf, ex: -2, sgn: 0x1 }, /* i=99 */
    Qint { hh: 0xf7856e5ee2c9b290, hl: 0xc6f2a1b84190a7d6, lh: 0x94261a0e91f0e8f2, ll: 0x960d286867d7da8d, ex: -3, sgn: 0x1 }, /* i=100 */
    Qint { hh: 0xee0de5055f63eb06, hl: 0x98a33316df83ba56, lh: 0xa28f0225cea42f20, ll: 0xc8c87785c07b059d, ex: -3, sgn: 0x1 }, /* i=101 */
    Qint { hh: 0xe47fbe3cd4d10d61, hl: 0x2ec0f797fdcd1257, lh: 0x1d97a9d046b706c5, ll: 0xc3c4cfd592ff1d1b, ex: -3, sgn: 0x1 }, /* i=102 */
    Qint { hh: 0xdada8cf47dad2374, hl: 0x4ffb833c3409ee78, lh: 0x3713df786be7d79f, ll: 0xdfdccbdb9cc5e4fc, ex: -3, sgn: 0x1 }, /* i=103 */
    Qint { hh: 0xd11de0ff15ab18c9, hl: 0xb88d83d4cc613f1f, lh: 0x8db36c5996f30e02, ll: 0x67caaac70b1e203f, ex: -3, sgn: 0x1 }, /* i=104 */
    Qint { hh: 0xc74946f4436a0552, hl: 0xc4f5cb531201c0d0, lh: 0xe377c62941756dda, ll: 0xbf31ff26e7952aa7, ex: -3, sgn: 0x1 }, /* i=105 */
    Qint { hh: 0xbd5c481086c848df, hl: 0x1b596b503040323f, lh: 0xf0a4a6c408595abb, ll: 0x18b2e81de5a7413d, ex: -3, sgn: 0x1 }, /* i=106 */
    Qint { hh: 0xb3566a13956a86f6, hl: 0xff1b1e1574d9fd53, lh: 0xd790e4993973cb21, ll: 0xb570c1978023c83, ex: -3, sgn: 0x1 }, /* i=107 */
    Qint { hh: 0xa9372f1d0da1bd17, hl: 0x200eb71e58cd36de, lh: 0x631daa222aa1cc5e, ll: 0xc53df36a99bd161e, ex: -3, sgn: 0x1 }, /* i=108 */
    Qint { hh: 0x9efe158766314e54, hl: 0xc571827efe892fc4, lh: 0x5fb87ab4717a500, ll: 0x73890974d65b5cfd, ex: -3, sgn: 0x1 }, /* i=109 */
    Qint { hh: 0x981eb8c723fe97f4, hl: 0xa31c134fb702d431, lh: 0xa1267633d7a950a6, ll: 0xf1d435478be2e98d, ex: -3, sgn: 0x1 }, /* i=110 */
    Qint { hh: 0x8db956a97b3d0148, hl: 0x3023472cd739f9de, lh: 0x3f642654cbb04a9b, ll: 0x501b839196278b37, ex: -3, sgn: 0x1 }, /* i=111 */
    Qint { hh: 0x8338a89652cb7150, hl: 0xc647eb86498c2ce1, lh: 0x6fdaaacd24ed99fc, ll: 0x73619b3ac0a2580e, ex: -3, sgn: 0x1 }, /* i=112 */
    Qint { hh: 0xf85186008b15330b, hl: 0xe64b8b775997898d, lh: 0x3474d3375b525967, ll: 0x1851f0a96f698496, ex: -4, sgn: 0x1 }, /* i=113 */
    Qint { hh: 0xe2f2a47ade3a18ae, hl: 0xb0bf7c0b0d8bb4ec, lh: 0xb357c6e1bb965608, ll: 0xb1f3fb65de3326ac, ex: -4, sgn: 0x1 }, /* i=114 */
    Qint { hh: 0xd49369d256ab1b28, hl: 0x5e9154e1d5263cd4, lh: 0xfb3f11769cc680ef, ll: 0x5588fd21488d3117, ex: -4, sgn: 0x1 }, /* i=115 */
    Qint { hh: 0xbed3b36bd8966422, hl: 0x240644d7d9ed08ae, lh: 0x8bd331e0f0163a57, ll: 0xad0c8b665d0ba662, ex: -4, sgn: 0x1 }, /* i=116 */
    Qint { hh: 0xb032c549ba861d8e, hl: 0xf74e27bc92ce336a, lh: 0x476c441f8cbfb247, ll: 0xb421b4cceddb6dec, ex: -4, sgn: 0x1 }, /* i=117 */
    Qint { hh: 0xa176e5f5323781dd, hl: 0xd4f935996c92e8cb, lh: 0xb1ed0cd9e5eb16c4, ll: 0xd070037b7a65dbb6, ex: -4, sgn: 0x1 }, /* i=118 */
    Qint { hh: 0x8b29b7751bd70743, hl: 0x12e0b9ee992f236d, lh: 0x21482d3342d35569, ll: 0xbf365c4132567724, ex: -4, sgn: 0x1 }, /* i=119 */
    Qint { hh: 0xf85186008b15330b, hl: 0xe64b8b775997898d, lh: 0x3474d3375b525967, ll: 0x1851f0a96f698496, ex: -5, sgn: 0x1 }, /* i=120 */
    Qint { hh: 0xda16eb88cb8df614, hl: 0x68a63ecfb66e94ab, lh: 0xce26340fc53dc9e7, ll: 0x7778bea9e4485112, ex: -5, sgn: 0x1 }, /* i=121 */
    Qint { hh: 0xac52dd7e4726a463, hl: 0x547a963a91bb3012, lh: 0x6146c24c8704d774, ll: 0x9181c8e24fdd9bf3, ex: -5, sgn: 0x1 }, /* i=122 */
    Qint { hh: 0x8d86cc491ecbfe16, hl: 0x51776453b7e8254d, lh: 0x1fe3399d400c4228, ll: 0x939ae69b03a586bc, ex: -5, sgn: 0x1 }, /* i=123 */
    Qint { hh: 0xdcfe013d7c8cbfde, hl: 0xa32dbac46f30cffe, lh: 0xda998fa29b9bb98b, ll: 0x121d35ae45b4e2e1, ex: -6, sgn: 0x1 }, /* i=124 */
    Qint { hh: 0x9e75221a352ba779, hl: 0xa52b7ea62f2198d0, lh: 0x797189a4ceffb772, ll: 0x67b1205aea8ed5f1, ex: -6, sgn: 0x1 }, /* i=125 */
    Qint { hh: 0xbee23afc0853b6e9, hl: 0x289782c20df350a1, lh: 0x4943001d3f0647d1, ll: 0x907db46b91a9be11, ex: -7, sgn: 0x1 }, /* i=126 */
    Qint { hh: 0x0, hl: 0x0, lh: 0x0, ll: 0x0, ex: 255, sgn: 0x1 }, /* i=127 */
    Qint { hh: 0x0, hl: 0x0, lh: 0x0, ll: 0x0, ex: 255, sgn: 0x1 }, /* i=128 */
    Qint { hh: 0xc122451c45155104, hl: 0xb16137f09a002b3c, lh: 0x114425f06f494d45, ll: 0xe2fa6c6fbca3a43a, ex: -7, sgn: 0x0 }, /* i=129 */
    Qint { hh: 0xa195492cc06604e6, hl: 0x4a18dff7cdb4ae5c, lh: 0x1b120e15ca3dceb7, ll: 0x62148cc46c40db43, ex: -6, sgn: 0x0 }, /* i=130 */
    Qint { hh: 0xe31e9760a5578c63, hl: 0xf9eb2f284f31c35c, lh: 0x739276a47bc0067f, ll: 0x13d3be097b445a02, ex: -6, sgn: 0x0 }, /* i=131 */
    Qint { hh: 0x8a4f1f2002d46756, hl: 0x5be970314148c644, lh: 0xd7177b23dafc1e78, ll: 0x62496477de5b9b70, ex: -5, sgn: 0x0 }, /* i=132 */
    Qint { hh: 0xab8ae2601e777722, hl: 0x3b89d7f254f8d4d, lh: 0x7fea49aded4406bd, ll: 0xbd68cbc383b2b959, ex: -5, sgn: 0x0 }, /* i=133 */
    Qint { hh: 0xcd0c3dab9ef3dd1b, hl: 0x13b26f298aa357c8, lh: 0x307b8ee396d79ef6, ll: 0x801ddc72263552e3, ex: -5, sgn: 0x0 }, /* i=134 */
    Qint { hh: 0xe65b9e6eed965c36, hl: 0xe09f5fe2058d6005, lh: 0xb58f9a65c1043b41, ll: 0xe2faca238c300fa9, ex: -5, sgn: 0x0 }, /* i=135 */
    Qint { hh: 0x842cc5acf1d03445, hl: 0x1fecdfa819b96097, lh: 0xe362c7f8dd18e5cb, ll: 0x2c885ebb0a67bc24, ex: -4, sgn: 0x0 }, /* i=136 */
    Qint { hh: 0x9103dae3c2a4ec67, hl: 0xe0863df62ab56719, lh: 0xe0c7d4d12db021b8, ll: 0x55776e4da79f922a, ex: -4, sgn: 0x0 }, /* i=137 */
    Qint { hh: 0xa242f01edefd6a37, hl: 0x469355b78dc796e2, lh: 0xb3c575a2031956ec, ll: 0x395fd54d8695b022, ex: -4, sgn: 0x0 }, /* i=138 */
    Qint { hh: 0xaf4ad26cbc8e5be7, hl: 0xe8b8b88a14ff0cd, lh: 0x9ad6b7f2deaa8ae6, ll: 0x47e37e4affafd5a1, ex: -4, sgn: 0x0 }, /* i=139 */
    Qint { hh: 0xc0cbf17a071f80dc, hl: 0xf96ffdf76a147ccc, lh: 0x3700761ca4fb5278, ll: 0xf6961709acabf991, ex: -4, sgn: 0x0 }, /* i=140 */
    Qint { hh: 0xce06196a692a41fb, hl: 0xbe3ccc15326765f, lh: 0x733187c6d6f39bb8, ll: 0xe3f54edc6e2e0350, ex: -4, sgn: 0x0 }, /* i=141 */
    Qint { hh: 0xdb56446d6ad8deff, hl: 0xa8112e35a60e6374, lh: 0xdd62571dda9ce602, ll: 0xe44b58c81d361f18, ex: -4, sgn: 0x0 }, /* i=142 */
    Qint { hh: 0xe8bcbc410c9b219d, hl: 0xaf7df76ad29e5b5f, lh: 0xe6a09f8913389334, ll: 0x7de7a2535752b786, ex: -4, sgn: 0x0 }, /* i=143 */
    Qint { hh: 0xf639cc185088fe5d, hl: 0x4066e87f2c0f733f, lh: 0x8296a39b87519924, ll: 0x5a18333a98b0adbc, ex: -4, sgn: 0x0 }, /* i=144 */
    Qint { hh: 0x842cc5acf1d03445, hl: 0x1fecdfa819b96097, lh: 0xe362c7f8dd18e5cb, ll: 0x2c885ebb0a67bc24, ex: -3, sgn: 0x0 }, /* i=145 */
    Qint { hh: 0x8b064012593d85a5, hl: 0x52013c7a80ad089b, lh: 0x42ada32f6b02af2d, ll: 0x88cfab3b8ffc09bc, ex: -3, sgn: 0x0 }, /* i=146 */
    Qint { hh: 0x91eb89524e100d23, hl: 0x8fd3df5c52d67e7b, lh: 0x2024f18ebc9b8af6, ll: 0x81f936979eefae26, ex: -3, sgn: 0x0 }, /* i=147 */
    Qint { hh: 0x98dcca69d27c263b, hl: 0x8e94203f336fc8c4, lh: 0xcfe4e777e9932f10, ll: 0x9ed7ba1820b6ff5, ex: -3, sgn: 0x0 }, /* i=148 */
    Qint { hh: 0x9fda2d2cc9465c4f, hl: 0x32b9565f5355181, lh: 0xdc751798a72b3dc9, ll: 0x56eca518e7d2fe44, ex: -3, sgn: 0x0 }, /* i=149 */
    Qint { hh: 0xa6e3dc4bde0e3cdb, hl: 0x570ff874170d2a8, lh: 0xc7be23c834886156, ll: 0xbd706a5a2627f5aa, ex: -3, sgn: 0x0 }, /* i=150 */
    Qint { hh: 0xab9be6480c66ea9e, hl: 0x9ae21fd871b8d27c, lh: 0x7851f4e516e5c9bf, ll: 0xc1c5a79f56e96b76, ex: -3, sgn: 0x0 }, /* i=151 */
    Qint { hh: 0xb2ba75f46099cf8b, hl: 0x2c3c2e77904afa78, lh: 0x77fa400e7e689a3, ll: 0x30a5f043d61bad59, ex: -3, sgn: 0x0 }, /* i=152 */
    Qint { hh: 0xb9e5c83a7e8a655b, hl: 0xcbffe9661fe72421, lh: 0x2096b17331fac5dc, ll: 0x7775b110db2591bd, ex: -3, sgn: 0x0 }, /* i=153 */
    Qint { hh: 0xc11e0b2a8d1e0ddb, hl: 0x9a631e830fd30903, lh: 0xd59edb68f6b3f63b, ll: 0x9ede162ed215bb6b, ex: -3, sgn: 0x0 }, /* i=154 */
    Qint { hh: 0xc8636dcfe5e6ca0a, hl: 0x88e72835b3292d4f, lh: 0xd07fb98b088395ee, ll: 0x10b2726db3c15971, ex: -3, sgn: 0x0 }, /* i=155 */
    Qint { hh: 0xcd43bc6f5d51c3e8, hl: 0xfbfb0e3f0fd23074, lh: 0x435c6598be364fab, ll: 0x938d2827ce902e72, ex: -3, sgn: 0x0 }, /* i=156 */
    Qint { hh: 0xd49f69e456cf1b79, hl: 0x5f53bd2e406e66e7, lh: 0x7188af8f4f45b9ee, ll: 0x7ec0d7bfb6b65f0d, ex: -3, sgn: 0x0 }, /* i=157 */
    Qint { hh: 0xdc08b985c11e9068, hl: 0x3b9cd767c3b1ac52, lh: 0x81fdd139ee15996c, ll: 0xdf9da9ae69110ecb, ex: -3, sgn: 0x0 }, /* i=158 */
    Qint { hh: 0xe1014558bfcda3e2, hl: 0x35470a74be1230ec, lh: 0x7ea4f73313d9cef6, ll: 0x7a6dd1127f07bf9a, ex: -3, sgn: 0x0 }, /* i=159 */
    Qint { hh: 0xe881bf932af3dac0, hl: 0xc524848e3443e03f, lh: 0xc22bd8fede6ee351, ll: 0x93f48308bc589a07, ex: -3, sgn: 0x0 }, /* i=160 */
    Qint { hh: 0xed89ed86a44a01aa, hl: 0x11d49f96cb88317a, lh: 0xb09cac07eab378a8, ll: 0xe6342851611cc8be, ex: -3, sgn: 0x0 }, /* i=161 */
    Qint { hh: 0xf52224f82557a459, hl: 0x8dcca8d7f17fa2a9, lh: 0x330d7e7fe8c1c62a, ll: 0x18d25c613e0e9a6d, ex: -3, sgn: 0x0 }, /* i=162 */
    Qint { hh: 0xfa3a589a6f9146d8, hl: 0x388212895529a6fa, lh: 0x937d820ed16d615e, ll: 0x6bf1e0ae92585a10, ex: -3, sgn: 0x0 }, /* i=163 */
    Qint { hh: 0x80f572b1363487b9, hl: 0xf5bd0b5b3479d5f4, lh: 0x501b8b4a63fd6f67, ll: 0x6e9f54a7361289b3, ex: -2, sgn: 0x0 }, /* i=164 */
    Qint { hh: 0x8389c3026ac3139b, hl: 0x62dda9d2270fa1f4, lh: 0x29aec44c9ebb0731, ll: 0xee2b9479c54b01fb, ex: -2, sgn: 0x0 }, /* i=165 */
    Qint { hh: 0x86216b3b0b17188b, hl: 0x163ceae88f720f1d, lh: 0x9a8ffa0ca490b651, ll: 0x815baea454ee87ca, ex: -2, sgn: 0x0 }, /* i=166 */
    Qint { hh: 0x8a0b3f79b3bc180f, hl: 0x49b55ea7d3730d7, lh: 0x1f95d048ae9871b6, ll: 0xb7eb0007a2dfe5c0, ex: -2, sgn: 0x0 }, /* i=167 */
    Qint { hh: 0x8cab69dcde17d2f7, hl: 0x3ad1aa142b94f169, lh: 0x82d7e38d0ce7657c, ll: 0x800bbe6769b633e0, ex: -2, sgn: 0x0 }, /* i=168 */
    Qint { hh: 0x8f4f0b3c44cfa2a2, hl: 0x586e9343c9cfdbac, lh: 0x1fee2e686760d584, ll: 0xc0243088b56a6c0f, ex: -2, sgn: 0x0 }, /* i=169 */
    Qint { hh: 0x934b1089a6dc93c1, hl: 0xdf5bb3b60554e151, lh: 0x87a486e65aa1bcd5, ll: 0xad047f998c197d96, ex: -2, sgn: 0x0 }, /* i=170 */
    Qint { hh: 0x95f783e6e49a9cfa, hl: 0x4a5004f3ef063312, lh: 0xcac9f0589aff46a5, ll: 0x3b2136f4975a446e, ex: -2, sgn: 0x0 }, /* i=171 */
    Qint { hh: 0x98a78f0e9ae71d85, hl: 0x2cdec34784707839, lh: 0x4861cab8c5ee1c94, ll: 0xb0ddc91e9b86138f, ex: -2, sgn: 0x0 }, /* i=172 */
    Qint { hh: 0x9b5b3bb5f088b766, hl: 0xd878bbe3d392be25, lh: 0x24f04843d0f8f41, ll: 0xd27746bfed0adcfe, ex: -2, sgn: 0x0 }, /* i=173 */
    Qint { hh: 0x9e1293b9998c1daa, hl: 0x5b035eae273a855e, lh: 0xf58182e4db06261c, ll: 0x73db477d896b83f5, ex: -2, sgn: 0x0 }, /* i=174 */
    Qint { hh: 0xa22c8f029cfa45a9, hl: 0xdb5b709e0b69e773, lh: 0x5e171935b5381c36, ll: 0x930e7dec83978f45, ex: -2, sgn: 0x0 }, /* i=175 */
    Qint { hh: 0xa4ed3f9de620f666, hl: 0x9b5e973353638c10, lh: 0xd81763d28db5c039, ll: 0xaef301c1c91f3649, ex: -2, sgn: 0x0 }, /* i=176 */
    Qint { hh: 0xa7b1bf5dd4c07d4e, hl: 0x699db68db75e9a7e, lh: 0x8e98852150ea76b5, ll: 0x2d4a158846053fd7, ex: -2, sgn: 0x0 }, /* i=177 */
    Qint { hh: 0xaa7a18dbdf0d44aa, hl: 0x604884a8dd76d08a, lh: 0x6c40e044972eeeb2, ll: 0x8d4e70541a83bec5, ex: -2, sgn: 0x0 }, /* i=178 */
    Qint { hh: 0xad4656ddf6fd070c, hl: 0x9ea10260fe452ba2, lh: 0x59b0b64abac9cb07, ll: 0x30a999bf35a66756, ex: -2, sgn: 0x0 }, /* i=179 */
    Qint { hh: 0xb0168457848f5f48, hl: 0xbb6f9fb246068d52, lh: 0x3e567a3312c2443d, ll: 0xe963d8ddfd9f7f8b, ex: -2, sgn: 0x0 }, /* i=180 */
    Qint { hh: 0xb2eaac6a67005513, hl: 0xf4b716f6fec8156b, lh: 0x2c74b8f4ed61d394, ll: 0x1166335bb2b54fe0, ex: -2, sgn: 0x0 }, /* i=181 */
];

/// LOG_INV_3_2[j - 8128] approximates -log(INVERSE_3_2[j - 8128]) (C `_LOG_INV_3_2`).
#[rustfmt::skip]
static LOG_INV_3_2: [Qint; 129] = [
    Qint { hh: 0xff015358833c47e1, hl: 0xbb481c8ee1416959, lh: 0xed961f7cd039d43b, ll: 0x3813c435abc461e9, ex: -8, sgn: 0x1 }, /* j=8128 */
    Qint { hh: 0xfb0933b732572a6d, hl: 0x214cca3dd1d4796a, lh: 0x63275973180916, ll: 0xce33d61e9d12b379, ex: -8, sgn: 0x1 }, /* j=8129 */
    Qint { hh: 0xf710f492711d9d26, hl: 0xfbc7b38b17b2019, lh: 0x53eb4b80e74f4d9f, ll: 0xec0f1ece69a0881, ex: -8, sgn: 0x1 }, /* j=8130 */
    Qint { hh: 0xf31895e84b1a6be6, hl: 0xb76782b9e88c84cb, lh: 0x3765e2cb07bc7842, ll: 0x1b2970050550b17b, ex: -8, sgn: 0x1 }, /* j=8131 */
    Qint { hh: 0xef2017b6cba9cf9a, hl: 0x2dc85881664025b4, lh: 0xce96efdd54fdcc41, ll: 0xc953aa9864a2c806, ex: -8, sgn: 0x1 }, /* j=8132 */
    Qint { hh: 0xeb2779fbfdf96874, hl: 0xce4ab4e678d0ed03, lh: 0x7c9ecdfd8f89db96, ll: 0xa637f86e778c350c, ex: -8, sgn: 0x1 }, /* j=8133 */
    Qint { hh: 0xe72ebcb5ed08382b, hl: 0xb60585f4c4bb6062, lh: 0x1caec15031f4dc54, ll: 0x197d0a42fdccbac4, ex: -8, sgn: 0x1 }, /* j=8134 */
    Qint { hh: 0xe335dfe2a3a69c2b, hl: 0x59bcffe9d5650564, lh: 0x50c342f5fce295a8, ll: 0xac842dc0defcda11, ex: -8, sgn: 0x1 }, /* j=8135 */
    Qint { hh: 0xdf3ce3802c7647cd, hl: 0x3602021fa93b1e18, lh: 0x4f178f6a8fbb9ca5, ll: 0x4f07c79076670299, ex: -8, sgn: 0x1 }, /* j=8136 */
    Qint { hh: 0xdb43c78c91ea3e8c, hl: 0x9944002534d09b3d, lh: 0x2354c6de776e85ab, ll: 0x7bfb018bb7288d07, ex: -8, sgn: 0x1 }, /* j=8137 */
    Qint { hh: 0xd74a8c05de46ce3a, hl: 0x87aa95782311a277, lh: 0x2ba2456733804a75, ll: 0x33512ca3a3f6fe2b, ex: -8, sgn: 0x1 }, /* j=8138 */
    Qint { hh: 0xd35130ea1ba18930, hl: 0xb88be10313a1303c, lh: 0xc17153c5ab3b0225, ll: 0xe1f11ff3205d196e, ex: -8, sgn: 0x1 }, /* j=8139 */
    Qint { hh: 0xcf57b63753e14083, hl: 0xad54bc31433dddba, lh: 0x3d675807b776c8c3, ll: 0x50075ac879e03385, ex: -8, sgn: 0x1 }, /* j=8140 */
    Qint { hh: 0xcb5e1beb90bdfe33, hl: 0xe1b7d813e3f825e0, lh: 0xd24faf76303783f0, ll: 0x55ff6c2398186fbe, ex: -8, sgn: 0x1 }, /* j=8141 */
    Qint { hh: 0xc7646204dbc0ff5e, hl: 0x14f8c1be7370f218, lh: 0xb165fd239443b62c, ll: 0x237e837cc1f0bdfb, ex: -8, sgn: 0x1 }, /* j=8142 */
    Qint { hh: 0xc36a88813e44ae6a, hl: 0xac27c5a6139cd30c, lh: 0x3e814a96bc97e05e, ll: 0xa2417a23566b2cd7, ex: -8, sgn: 0x1 }, /* j=8143 */
    Qint { hh: 0xbf708f5ec1749d3c, hl: 0x2d23a0744e00f594, lh: 0x68f9c68b2e5248, ll: 0x1d701478d6f6c2b6, ex: -8, sgn: 0x1 }, /* j=8144 */
    Qint { hh: 0xbb76769b6e4d7f5c, hl: 0xd235e25fb9644c30, lh: 0x869c4dd8468b27de, ll: 0x605b72cc1d8025bc, ex: -8, sgn: 0x1 }, /* j=8145 */
    Qint { hh: 0xb77c3e354d9d242b, hl: 0x361ee0bcb5db0449, lh: 0x567cab7f031b369, ll: 0xd9bec487ee14afeb, ex: -8, sgn: 0x1 }, /* j=8146 */
    Qint { hh: 0xb381e62a68027106, hl: 0x18660815da3d7962, lh: 0xd14621e31d9f0de1, ll: 0x8fbb93f3b4e5b10e, ex: -8, sgn: 0x1 }, /* j=8147 */
    Qint { hh: 0xaf876e78c5ed5b77, hl: 0x39c357b6bfdf81b4, lh: 0xf2591df10cea40d0, ll: 0x730872806a501e6e, ex: -8, sgn: 0x1 }, /* j=8148 */
    Qint { hh: 0xab8cd71e6f9ee35d, hl: 0x5076c62c951204f5, lh: 0xb058f99c8186daa4, ll: 0xae0d7dcbc2a675ab, ex: -8, sgn: 0x1 }, /* j=8149 */
    Qint { hh: 0xa79220196d290d15, hl: 0x146244d643f7fa2a, lh: 0x90fee0b93d40db31, ll: 0xe67bce480f351d7b, ex: -8, sgn: 0x1 }, /* j=8150 */
    Qint { hh: 0xa3974967c66edba1, hl: 0x62bb0f3208d9a1ba, lh: 0xcdd5aab43dfdb463, ll: 0x9338ef1dac85e113, ex: -8, sgn: 0x1 }, /* j=8151 */
    Qint { hh: 0x9f9c530783244ad2, hl: 0x7926e92808bd580c, lh: 0x9a874314df180c72, ll: 0x2272c79abfdb422c, ex: -8, sgn: 0x1 }, /* j=8152 */
    Qint { hh: 0x9ba13cf6aace496c, hl: 0x4819e620d5fcc067, lh: 0x9066b677760637d4, ll: 0x308f6e39ffa6ca4c, ex: -8, sgn: 0x1 }, /* j=8153 */
    Qint { hh: 0x97a6073344c2b34b, hl: 0xdc494943d427214e, lh: 0x423f0610339ed04b, ll: 0x185c41879a1a0e44, ex: -8, sgn: 0x1 }, /* j=8154 */
    Qint { hh: 0x93aab1bb58284b8b, hl: 0xdf0805c4161e404b, lh: 0xb467a7c6839cc262, ll: 0x831c3f31eb48d551, ex: -8, sgn: 0x1 }, /* j=8155 */
    Qint { hh: 0x8faf3c8cebf6b6a8, hl: 0x2d615caaa0514c3b, lh: 0xf729c68cd270f129, ll: 0xaab9342f19cba76c, ex: -8, sgn: 0x1 }, /* j=8156 */
    Qint { hh: 0x8bb3a7a606f674a0, hl: 0x85c60c12eca0aedb, lh: 0xa3fb0e5c1d39e0e8, ll: 0x1dca7781b6e7202f, ex: -8, sgn: 0x1 }, /* j=8157 */
    Qint { hh: 0x87b7f304afc0db1a, hl: 0x4c207a522524f8de, lh: 0x3d54277ac7f378a6, ll: 0xa5d1bbca5392d277, ex: -8, sgn: 0x1 }, /* j=8158 */
    Qint { hh: 0x83bc1ea6ecc00f81, hl: 0x64243e02c6215a4e, lh: 0xb38c33565546e35c, ll: 0x19646b6a5fd02489, ex: -8, sgn: 0x1 }, /* j=8159 */
    Qint { hh: 0xff805515885e0250, hl: 0x435ab4da6a5bb48c, lh: 0xcd29dd6d72582491, ll: 0xba6e335a1a33227f, ex: -9, sgn: 0x1 }, /* j=8160 */
    Qint { hh: 0xf7882d5c7832c6cc, hl: 0x9e06fc84b6ea5e24, lh: 0x699801dab452e328, ll: 0x96dc3d1e75ba8032, ex: -9, sgn: 0x1 }, /* j=8161 */
    Qint { hh: 0xef8fc61eb4b74f6e, hl: 0x91ab122ee427cfb4, lh: 0x8862e24ccd48f678, ll: 0x95fa76270a5d0366, ex: -9, sgn: 0x1 }, /* j=8162 */
    Qint { hh: 0xe7971f584945efae, hl: 0x5f832513e3211642, lh: 0xa65998dde4dd76e0, ll: 0xbbb102dc658c60, ex: -9, sgn: 0x1 }, /* j=8163 */
    Qint { hh: 0xdf9e390540da5fbe, hl: 0x5e7b48cfeeb85aa7, lh: 0xbcaca74cb74df3d2, ll: 0x55bfb0c4fdd76008, ex: -9, sgn: 0x1 }, /* j=8164 */
    Qint { hh: 0xd7a51321a611b0c1, hl: 0xb36a9f58eb4ccd07, lh: 0x8770a2e82b32cf54, ll: 0xc9fdca283fb5d971, ex: -9, sgn: 0x1 }, /* j=8165 */
    Qint { hh: 0xcfabada9832a4101, hl: 0x3360751e43c7af35, lh: 0x5d10db5217ec2ab8, ll: 0xc60248447782dce7, ex: -9, sgn: 0x1 }, /* j=8166 */
    Qint { hh: 0xc7b20898e203b01e, hl: 0x6fab78aca91193cb, lh: 0x5e4d97ba155a9de6, ll: 0xcdd79f978d1afd7b, ex: -9, sgn: 0x1 }, /* j=8167 */
    Qint { hh: 0xbfb823ebcc1ed344, hl: 0xeb432409cffdad8d, lh: 0x568dc2013b32ced6, ll: 0x72b53615be59209e, ex: -9, sgn: 0x1 }, /* j=8168 */
    Qint { hh: 0xb7bdff9e4a9da959, hl: 0x793b5acf3a336461, lh: 0x9cffddada1c113a2, ll: 0xb54d414e644993a8, ex: -9, sgn: 0x1 }, /* j=8169 */
    Qint { hh: 0xafc39bac66434f27, hl: 0xc3ea2cd93f316b33, lh: 0xbfcaabcf0318ef95, ll: 0x85a480f694e7b857, ex: -9, sgn: 0x1 }, /* j=8170 */
    Qint { hh: 0xa7c8f8122773f38d, hl: 0xfc679a28e9d9f212, lh: 0x487785d971aec0af, ll: 0x61cbdae5b7255821, ex: -9, sgn: 0x1 }, /* j=8171 */
    Qint { hh: 0x9fce14cb9634cba6, hl: 0xb20f215bd3b58c60, lh: 0xd8ba6eedf272eeb0, ll: 0x574dfcf4f87b33e8, ex: -9, sgn: 0x1 }, /* j=8172 */
    Qint { hh: 0x97d2f1d4ba2c06f0, hl: 0xd1aacedcefe9d376, lh: 0xb21c7fe4cdbc5967, ll: 0xd35c35c4241c3714, ex: -9, sgn: 0x1 }, /* j=8173 */
    Qint { hh: 0x8fd78f299aa0c375, hl: 0xcbef6fac33691e95, lh: 0x466fab846a1e3e70, ll: 0xef6ef4000ffc2f4b, ex: -9, sgn: 0x1 }, /* j=8174 */
    Qint { hh: 0x87dbecc63e7b01ed, hl: 0xe2f1775134c8da75, lh: 0x134f09715ee9c6cf, ll: 0x48be7593379645aa, ex: -9, sgn: 0x1 }, /* j=8175 */
    Qint { hh: 0xffc0154d588733c5, hl: 0x3c742a7c76356395, lh: 0xb1d845d134023d8e, ll: 0x66ad982559cdd0ce, ex: -10, sgn: 0x1 }, /* j=8176 */
    Qint { hh: 0xefc7d18dd4485b9e, hl: 0xca47c52b7d7ffce2, lh: 0x13c8ea71e3d8fe30, ll: 0x8edd8e108ea7f135, ex: -10, sgn: 0x1 }, /* j=8177 */
    Qint { hh: 0xdfcf0e45fbce3e80, hl: 0x7e4cfbd830393b87, lh: 0x83304a61505642d8, ll: 0xceeccccf5069ebf8, ex: -10, sgn: 0x1 }, /* j=8178 */
    Qint { hh: 0xcfd5cb6dd9ef05dd, hl: 0x7370ae83f9e72748, lh: 0x140f4a016e0c0d28, ll: 0x5aea5efc41fa18fd, ex: -10, sgn: 0x1 }, /* j=8179 */
    Qint { hh: 0xbfdc08fd78c229b9, hl: 0xe6dbb624f9739781, lh: 0xbbd85b81581c98f8, ll: 0xe1340de77cd6a600, ex: -10, sgn: 0x1 }, /* j=8180 */
    Qint { hh: 0xafe1c6ece1a058dd, hl: 0x97fa2fd0c9dc723d, lh: 0x998985ef1e4636e0, ll: 0x306c5597c846f0b9, ex: -10, sgn: 0x1 }, /* j=8181 */
    Qint { hh: 0x9fe705341d236102, hl: 0x7199cd06ae5d39b3, lh: 0x67da60b1a110ff9, ll: 0x14c940c27f248ced, ex: -10, sgn: 0x1 }, /* j=8182 */
    Qint { hh: 0x8febc3cb332616ff, hl: 0x7b6d1248c3e1fd3f, lh: 0xd8ceb8a313143c9c, ll: 0xe1449e20d24508b2, ex: -10, sgn: 0x1 }, /* j=8183 */
    Qint { hh: 0xffe0055455887de0, hl: 0x26828c92649a3a38, lh: 0xc3585d8bbd3ac1b8, ll: 0xd315929badc83115, ex: -11, sgn: 0x1 }, /* j=8184 */
    Qint { hh: 0xdfe7839214b4e8ae, hl: 0xda6959f7f0e01bf0, lh: 0x3ff9151061ec91aa, ll: 0x8f307c96ffa40bba, ex: -11, sgn: 0x1 }, /* j=8185 */
    Qint { hh: 0xbfee023faf0c2480, hl: 0xb47505bfa5a03b06, lh: 0x2b8ff7c8377c9037, ll: 0x6cd9392c0d17528b, ex: -11, sgn: 0x1 }, /* j=8186 */
    Qint { hh: 0x9ff3814d2e4a36b2, hl: 0xa8740b91c95df537, lh: 0x5526081fe3d93a56, ll: 0x3f68c3cd00e45f8b, ex: -11, sgn: 0x1 }, /* j=8187 */
    Qint { hh: 0xfff0015535588833, hl: 0x3c56c598c659c2a2, lh: 0xf5c74f2f07e4f272, ll: 0xc451b2e04ebd63ee, ex: -12, sgn: 0x1 }, /* j=8188 */
    Qint { hh: 0xbff7008ff5e0c257, hl: 0x379eba7e6465ff63, lh: 0x3535a7e74bbb0089, ll: 0x937a324b4307d36, ex: -12, sgn: 0x1 }, /* j=8189 */
    Qint { hh: 0xfff8005551558885, hl: 0xde026e271ee0549c, lh: 0x8cd0b8002d083c9b, ll: 0x2e9198222f25f83c, ex: -13, sgn: 0x1 }, /* j=8190 */
    Qint { hh: 0x0, hl: 0x0, lh: 0x0, ll: 0x0, ex: 255, sgn: 0x1 }, /* j=8191 */
    Qint { hh: 0x0, hl: 0x0, lh: 0x0, ll: 0x0, ex: 255, sgn: 0x1 }, /* j=8192 */
    Qint { hh: 0xc004802401440c26, hl: 0xdfeb485085f6f453, lh: 0xb62f8fe41e621f91, ll: 0x274178d2188f9ea7, ex: -13, sgn: 0x0 }, /* j=8193 */
    Qint { hh: 0xa00640535a37a37a, hl: 0x6bc1e20eac8448b4, lh: 0xdf9f60ec72f7062, ll: 0x6f190de53deca5e8, ex: -12, sgn: 0x0 }, /* j=8194 */
    Qint { hh: 0xe00c40e4bd6e4efd, hl: 0xc72446cc1bf728bd, lh: 0x5e7d3be7e456c8a7, ll: 0x180e4b0fc869da77, ex: -12, sgn: 0x0 }, /* j=8195 */
    Qint { hh: 0x900a20f319a3e273, hl: 0x569b26aaa485ea5b, lh: 0xf5e4d6d9243bca81, ll: 0x2714208ba14d04c8, ex: -11, sgn: 0x0 }, /* j=8196 */
    Qint { hh: 0xb00f21bbe3e388ee, hl: 0x5f69768284463b9b, lh: 0x18baa187613466f, ll: 0x5b94041491764cd7, ex: -11, sgn: 0x0 }, /* j=8197 */
    Qint { hh: 0xd01522dcc4f87991, hl: 0x14d9d76196d8043a, lh: 0x179e870f7485c2eb, ll: 0x7931552a9aec92fb, ex: -11, sgn: 0x0 }, /* j=8198 */
    Qint { hh: 0xf01c2465c5e61b6f, hl: 0x661e135f49a47c40, lh: 0x42882a135aa4aa82, ll: 0x82166812bed98c2d, ex: -11, sgn: 0x0 }, /* j=8199 */
    Qint { hh: 0x881213337898871e, hl: 0x9a31ba0cbc030352, lh: 0xdc58134f3ce2ed6e, ll: 0x6c0826bdc4127939, ex: -10, sgn: 0x0 }, /* j=8200 */
    Qint { hh: 0x98169478296fad41, hl: 0x7ad1e9c315328f7d, lh: 0xcc0b6a758f391573, ll: 0xa0bdfad5f77c4a7f, ex: -10, sgn: 0x0 }, /* j=8201 */
    Qint { hh: 0xa81b9608fc3c50ec, hl: 0xf105b66ec4703ede, lh: 0x76406288c82a0e9c, ll: 0xc06e5c3eaccc9b65, ex: -10, sgn: 0x0 }, /* j=8202 */
    Qint { hh: 0xb82117edf8832797, hl: 0xd6aef30cd312169a, lh: 0x24ed9892618f8da1, ll: 0x6091c8f53e42724e, ex: -10, sgn: 0x0 }, /* j=8203 */
    Qint { hh: 0xc8271a2f2689e388, hl: 0xe6e2acf8f4d4c249, lh: 0x830dad0cbcb297de, ll: 0x6f8eeb67afc8978d, ex: -10, sgn: 0x0 }, /* j=8204 */
    Qint { hh: 0xd82d9cd48f574c00, hl: 0x28bb3cd9f2a65fb4, lh: 0x9c8f264a305434a2, ll: 0x719267af576b8da7, ex: -10, sgn: 0x0 }, /* j=8205 */
    Qint { hh: 0xe8349fe63cb35564, hl: 0x224a96f5a7471c45, lh: 0xd39ed746e5b2ba94, ll: 0xdb742c4b8aeaa82e, ex: -10, sgn: 0x0 }, /* j=8206 */
    Qint { hh: 0xf83c236c39273972, hl: 0xd462b63756c87e80, lh: 0x157ee3bffb879ef4, ll: 0x9e384ccad23b6068, ex: -10, sgn: 0x0 }, /* j=8207 */
    Qint { hh: 0x842213b747fec7bb, hl: 0x3ff51287882500ed, lh: 0x124d4848d57cf1e2, ll: 0xca3575b044a7e9d6, ex: -9, sgn: 0x0 }, /* j=8208 */
    Qint { hh: 0x8c2655faa6a1323f, hl: 0x1ab9679b55f78a6a, lh: 0x84963a91b59a785c, ll: 0xf77577a5e4380677, ex: -9, sgn: 0x0 }, /* j=8209 */
    Qint { hh: 0x942ad8843ee1a9cd, hl: 0x17e4b7ac6c600cb4, lh: 0x6767118f1a71745b, ll: 0x98a85668eda9496f, ex: -9, sgn: 0x0 }, /* j=8210 */
    Qint { hh: 0x9c2f9b581787cf0d, hl: 0xfd1a09c848e3950d, lh: 0xe0e66fad558345db, ll: 0xd0914651b6655eb2, ex: -9, sgn: 0x0 }, /* j=8211 */
    Qint { hh: 0xa4349e7a37bc21ed, hl: 0x318b2ddd9d0a33b3, lh: 0x9dd7a4359a20f4ae, ll: 0x176ea6faed5bf263, ex: -9, sgn: 0x0 }, /* j=8212 */
    Qint { hh: 0xac39e1eea7080dbc, hl: 0x9dd91e52c79fd070, lh: 0x184596be172aa3d1, ll: 0x8a86b1bb367fee8, ex: -9, sgn: 0x0 }, /* j=8213 */
    Qint { hh: 0xb43f65b96d55f55a, hl: 0x72de1d99ce252efd, lh: 0x149ec47368e2b10f, ll: 0xb294ccdf83577dc0, ex: -9, sgn: 0x0 }, /* j=8214 */
    Qint { hh: 0xbc4529de92f13f58, hl: 0xd7bd1d62ef25480d, lh: 0x7b1a44d708e0875b, ll: 0x83a3462cb58566da, ex: -9, sgn: 0x0 }, /* j=8215 */
    Qint { hh: 0xc44b2e6220866227, hl: 0x7f921124f1ecb59e, lh: 0x7696655fbc6715a6, ll: 0xb820b79a7e7be96f, ex: -9, sgn: 0x0 }, /* j=8216 */
    Qint { hh: 0xcc5173481f22f03f, hl: 0x271ee1cd6d5cdf9d, lh: 0x808228274a503b4b, ll: 0x3ec027918b227262, ex: -9, sgn: 0x0 }, /* j=8217 */
    Qint { hh: 0xd457f8949835a44e, hl: 0xfad0cc8b5faea8cb, lh: 0xa39dee1517d770f7, ll: 0x87b62eb6a5071ae6, ex: -9, sgn: 0x0 }, /* j=8218 */
    Qint { hh: 0xdc5ebe4b958e6d6b, hl: 0xe57a0acb9d5cd4de, lh: 0xd926113c7cf9cae4, ll: 0x2cc6a626c5980669, ex: -9, sgn: 0x0 }, /* j=8219 */
    Qint { hh: 0xe465c471215e7b41, hl: 0xc81bb5a8d789f443, lh: 0x8c30a4efa2085a91, ll: 0xd8416fa14d8a0531, ex: -9, sgn: 0x0 }, /* j=8220 */
    Qint { hh: 0xec6d0b0946384a46, hl: 0x9b1beb40437575f4, lh: 0x86e839aee0bd3623, ll: 0x447c2a52014276c5, ex: -9, sgn: 0x0 }, /* j=8221 */
    Qint { hh: 0xf47492180f0fafef, hl: 0x7944509046652d98, lh: 0xb65fa4f60aca7f76, ll: 0x4229769700e69546, ex: -9, sgn: 0x0 }, /* j=8222 */
    Qint { hh: 0xfc7c59a18739e6e7, hl: 0x94e51ebff53a2f15, lh: 0x12a476e601733b9f, ll: 0xe7eb2f0e037ceedb, ex: -9, sgn: 0x0 }, /* j=8223 */
    Qint { hh: 0x824230d4dd36cda4, hl: 0x8bbc7f765b13ebbe, lh: 0x68772abd1ce3258c, ll: 0xe7cf18b488213d56, ex: -8, sgn: 0x0 }, /* j=8224 */
    Qint { hh: 0x8646551a5a617b6b, hl: 0xf61305ef7390939c, lh: 0x1819673e1d680b66, ll: 0x3bb660c119045dcb, ex: -8, sgn: 0x0 }, /* j=8225 */
    Qint { hh: 0x8a4a99a34159d69f, hl: 0x3abc32a78afd4b7a, lh: 0xea68f0383acd9ff4, ll: 0xe7d3305627ebab51, ex: -8, sgn: 0x0 }, /* j=8226 */
    Qint { hh: 0x8e4efe71988d8426, hl: 0x17596a598cb29436, lh: 0x3dddd5d01649b409, ll: 0x314d41b7d5759903, ex: -8, sgn: 0x0 }, /* j=8227 */
    Qint { hh: 0x92538387669afa1b, hl: 0x1c890bee9a9d743c, lh: 0x7139a25d67ed976c, ll: 0x715b9888be5228e7, ex: -8, sgn: 0x0 }, /* j=8228 */
    Qint { hh: 0x965828e6b25185ec, hl: 0xeaafbd07b543145c, lh: 0xb28914b789a13376, ll: 0xc286d314221430a0, ex: -8, sgn: 0x0 }, /* j=8229 */
    Qint { hh: 0x9a5cee9182b15280, hl: 0x6517bc4112d64b17, lh: 0x3e099f46da82a60f, ll: 0x5ac8c579eeb8644c, ex: -8, sgn: 0x0 }, /* j=8230 */
    Qint { hh: 0x9e61d489deeb6e53, hl: 0xdb94a1dfd653d3a5, lh: 0x46503739bf42fb2e, ll: 0xeabea04e3ce90eed, ex: -8, sgn: 0x0 }, /* j=8231 */
    Qint { hh: 0xa266dad1ce61d1a3, hl: 0x2ada01ce7ed3607f, lh: 0x99d41a958f292523, ll: 0x2b08613964628dc6, ex: -8, sgn: 0x0 }, /* j=8232 */
    Qint { hh: 0xa66c016b58a7648c, hl: 0xd3b36c029ea7bb5d, lh: 0x36f49e73617ad152, ll: 0x967313472d10eeee, ex: -8, sgn: 0x0 }, /* j=8233 */
    Qint { hh: 0xaa71485885800538, hl: 0x94c529f32403828, lh: 0x253f332743a0ff31, ll: 0x6548b3ed5877119a, ex: -8, sgn: 0x0 }, /* j=8234 */
    Qint { hh: 0xae76af9b5ce08dfb, hl: 0xb6b6676248bba138, lh: 0xb6c98155115da0b7, ll: 0x6fcf6732c378b707, ex: -8, sgn: 0x0 }, /* j=8235 */
    Qint { hh: 0xb27c3735e6eedb86, hl: 0x7bdd0c2a9c7a679a, lh: 0x5fb1c32c4c750e98, ll: 0xb5b84cc7549d184a, ex: -8, sgn: 0x0 }, /* j=8236 */
    Qint { hh: 0xb47f0724b1906935, hl: 0x23deb274e953a258, lh: 0x990d7e42e0f2ed1b, ll: 0xe6c4e93f7ba138fe, ex: -8, sgn: 0x0 }, /* j=8237 */
    Qint { hh: 0xb884bf4697559ffa, hl: 0xdae7e343fa859415, lh: 0x5e4e41c1b5e85e4a, ll: 0xf82d9009ecaad498, ex: -8, sgn: 0x0 }, /* j=8238 */
    Qint { hh: 0xbc8a97c544fdd5eb, hl: 0x17759bff5c717992, lh: 0x949294749166f218, ll: 0x780bea6b173a668d, ex: -8, sgn: 0x0 }, /* j=8239 */
    Qint { hh: 0xc09090a2c35aa070, hl: 0x52e7e4dde874dacd, lh: 0xa952b40de6b303a6, ll: 0x46e9f250087d1466, ex: -8, sgn: 0x0 }, /* j=8240 */
    Qint { hh: 0xc496a9e11b6eb30c, hl: 0xa88971f8277a4d10, lh: 0xcc6e1825f856fe70, ll: 0x18193291e2b36381, ex: -8, sgn: 0x0 }, /* j=8241 */
    Qint { hh: 0xc89ce382566de587, hl: 0x269de85f0df92587, lh: 0xe65771fd08853e31, ll: 0xa9b4b0681339900, ex: -8, sgn: 0x0 }, /* j=8242 */
    Qint { hh: 0xcca33d887dbd3a1a, hl: 0x180d255422c3377c, lh: 0x35564b2dfc6a51cc, ll: 0xad4c9d66d29442ba, ex: -8, sgn: 0x0 }, /* j=8243 */
    Qint { hh: 0xd0a9b7f59af2e3a2, hl: 0x46da70925ee85c05, lh: 0x7b96716bbe5fc916, ll: 0x949df14f9331856d, ex: -8, sgn: 0x0 }, /* j=8244 */
    Qint { hh: 0xd4b052cbb7d64bcf, hl: 0x37968ceafaf7b452, lh: 0x866ac7791182f8a4, ll: 0x879a1365f37550b8, ex: -8, sgn: 0x0 }, /* j=8245 */
    Qint { hh: 0xd8b70e0cde601954, hl: 0x5dfba4cfdd38a058, lh: 0xc04093a8c6018f91, ll: 0x343b553466b61759, ex: -8, sgn: 0x0 }, /* j=8246 */
    Qint { hh: 0xdcbde9bb18ba361b, hl: 0x4ae21abe75d5a19a, lh: 0xd62f6d57c561e135, ll: 0x49b45337d2f46eb8, ex: -8, sgn: 0x0 }, /* j=8247 */
    Qint { hh: 0xe0c4e5d8713fd576, hl: 0xd3bd4fd98a1e6fe5, lh: 0x625d637fcaa83aa9, ll: 0x32bb34856444782c, ex: -8, sgn: 0x0 }, /* j=8248 */
    Qint { hh: 0xe4cc0266f27d7a57, hl: 0x33cf7d5ebfb93ad3, lh: 0x6a8d8f7f711ab809, ll: 0xe15b1d297f6f796c, ex: -8, sgn: 0x0 }, /* j=8249 */
    Qint { hh: 0xe8d33f68a730fd7f, hl: 0x2743c805a4928086, lh: 0x8923300474737280, ll: 0xd02a66f2f0de4bef, ex: -8, sgn: 0x0 }, /* j=8250 */
    Qint { hh: 0xecda9cdf9a4993ba, hl: 0x5dbeb9795455a5, lh: 0x5b168ca67fe36520, ll: 0xaf0629427a97e44f, ex: -8, sgn: 0x0 }, /* j=8251 */
    Qint { hh: 0xf0e21acdd6e7d412, hl: 0xb6ed80852ae6fd62, lh: 0xdfbedfa9caad16a4, ll: 0x5f822584035372e8, ex: -8, sgn: 0x0 }, /* j=8252 */
    Qint { hh: 0xf4e9b935685dbe0b, hl: 0xf237cff1acb306b3, lh: 0x8557e05d2318ee0, ll: 0x5453667bec383296, ex: -8, sgn: 0x0 }, /* j=8253 */
    Qint { hh: 0xf8f178185a2ebfd9, hl: 0xd81648249cece4c, lh: 0x3f4609735b1102e2, ll: 0x295ae12732b36daf, ex: -8, sgn: 0x0 }, /* j=8254 */
    Qint { hh: 0xfcf95778b80fbc98, hl: 0x176cd56887ac7fe8, lh: 0xaa02e8447626768d, ll: 0x553f90bb90928bc4, ex: -8, sgn: 0x0 }, /* j=8255 */
    Qint { hh: 0x8080abac46f38946, hl: 0x662d417ced007a45, lh: 0xc0be1062bd88c8e8, ll: 0xe925964e76028722, ex: -7, sgn: 0x0 }, /* j=8256 */
];

/// `T1_3[i]` approximates 2^(i/2^6) (C `T1_3`).
#[rustfmt::skip]
static T1_3: [Qint; 64] = [
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8164d1f3bc030773, hl: 0x7be56527bd14def4, lh: 0x9eb851655e2e5c4d, ll: 0xd08075ac1f200e4c, ex: 0, sgn: 0x0 },
    Qint { hh: 0x82cd8698ac2ba1d7, hl: 0x3e2a475b46520bff, lh: 0x29f1a4afbefa5d7c, ll: 0x2502f15067378a17, ex: 0, sgn: 0x0 },
    Qint { hh: 0x843a28c3acde4046, hl: 0x1af92eca13fd1582, lh: 0xd96b414ec4c9d06, ll: 0x806bddad09d9c4a3, ex: 0, sgn: 0x0 },
    Qint { hh: 0x85aac367cc487b14, hl: 0xc5c95b8c2154c1b2, lh: 0x148a0459e7585151, ll: 0x5d42b362af1ee859, ex: 0, sgn: 0x0 },
    Qint { hh: 0x871f61969e8d1010, hl: 0x3a1727c57b52a956, lh: 0x259ac58894f4fcb3, ll: 0x5229a7352c9b247b, ex: 0, sgn: 0x0 },
    Qint { hh: 0x88980e8092da8527, hl: 0x5df8d76c98c67562, lh: 0xe623d58b3772ba13, ll: 0x8bc3587fb118c94d, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8a14d575496efd9a, hl: 0x80ca1d92c3680c2, lh: 0x259c4df53d76e910, ll: 0xe9c32d22e935007d, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8b95c1e3ea8bd6e6, hl: 0xfbe4628758a53c90, lh: 0x1aa84ffbebac349f, ll: 0x91e135ee84a3f734, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8d1adf5b7e5ba9e5, hl: 0xb4c7b4968e41ad36, lh: 0x183926ae7d718dc2, ll: 0x724a166325437476, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8ea4398b45cd53c0, hl: 0x2dc0144c8783d4c5, lh: 0xa11037230b367828, ll: 0xeb90ce3700bf59b6, ex: 0, sgn: 0x0 },
    Qint { hh: 0x9031dc431466b1dc, hl: 0x775814a8494e87e2, lh: 0x43e90e15c2002132, ll: 0x6f398dfe3f7903f1, ex: 0, sgn: 0x0 },
    Qint { hh: 0x91c3d373ab11c336, hl: 0xfd6d8e0ae5ac9d8, lh: 0x1942b34816fb4f26, ll: 0xf1203caf65bfb9b9, ex: 0, sgn: 0x0 },
    Qint { hh: 0x935a2b2f13e6e92b, hl: 0xd339940e9d924ee7, lh: 0x2748c36eeaffa273, ll: 0x583eab6852a22bb1, ex: 0, sgn: 0x0 },
    Qint { hh: 0x94f4efa8fef70961, hl: 0x2e8afad12551de54, lh: 0x4856046901ff6c05, ll: 0x35fb634c2e63a0f, ex: 0, sgn: 0x0 },
    Qint { hh: 0x96942d3720185a00, hl: 0x48ea9b683a9c22c4, lh: 0xe0e68d9f200c5358, ll: 0x9a22b1526bb6a2e4, ex: 0, sgn: 0x0 },
    Qint { hh: 0x9837f0518db8a96f, hl: 0x46ad23182e42f6f6, lh: 0x5e139a1b14fa8178, ll: 0xd78b65cbefa7bb70, ex: 0, sgn: 0x0 },
    Qint { hh: 0x99e0459320b7fa64, hl: 0xe43086cb34b5fcae, lh: 0x8ac981ca9ceca6b3, ll: 0x1560e51a5df911dc, ex: 0, sgn: 0x0 },
    Qint { hh: 0x9b8d39b9d54e5538, hl: 0xa2a817a2a3cc3f1f, lh: 0x928b5fce34cdf21, ll: 0x9769d9b0a908a786, ex: 0, sgn: 0x0 },
    Qint { hh: 0x9d3ed9a72cffb750, hl: 0xde494cf050e99b0b, lh: 0x1ff17c29677589a0, ll: 0x33a6fe2d4fd53e8a, ex: 0, sgn: 0x0 },
    Qint { hh: 0x9ef5326091a111ad, hl: 0xa0911f09ebb9fdd1, lh: 0x65c15c122133e2a2, ll: 0x21f977fe7c7fa118, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa0b0510fb9714fc2, hl: 0x192dc79edb0fd9a9, lh: 0x782a0735d02b1a20, ll: 0x9f33f7bc78dc629f, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa27043030c496818, hl: 0x9b7a04ef80cfdea7, lh: 0x9da4384dbc2c8eae, ll: 0x5a7a799221808de9, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa43515ae09e6809e, hl: 0xd1db4831781e1ee, lh: 0xbae743abfbc07376, ll: 0x4c72418596cc5bd0, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa5fed6a9b15138ea, hl: 0x1cbd7f621710701b, lh: 0x1dd170ace2bcfc17, ll: 0x2589c98a8290d3f0, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa7cd93b4e9653569, hl: 0x9ec5b4d5039f72af, lh: 0x1424bd194d3999e, ll: 0xdd30939a1d1e929c, ex: 0, sgn: 0x0 },
    Qint { hh: 0xa9a15ab4ea7c0ef8, hl: 0x541e24ec3531fa73, lh: 0x3951f214c02d824a, ll: 0x325c9e2203504517, ex: 0, sgn: 0x0 },
    Qint { hh: 0xab7a39b5a93ed337, hl: 0x658023b2759e0079, lh: 0x7ad59ec00ebe6393, ll: 0x967357d6b36df9f8, ex: 0, sgn: 0x0 },
    Qint { hh: 0xad583eea42a14ac6, hl: 0x4980a8c8f59a2ec4, lh: 0x6be409407034fded, ll: 0xb165f141833a67da, ex: 0, sgn: 0x0 },
    Qint { hh: 0xaf3b78ad690a4374, hl: 0xdf26101ccbb35032, lh: 0xa4502c14f429ded9, ll: 0x5a8c73beaa946990, ex: 0, sgn: 0x0 },
    Qint { hh: 0xb123f581d2ac258f, hl: 0x87d037e96d215d8e, lh: 0x757cfb9913adc577, ll: 0x97ced890d5b0b0c0, ex: 0, sgn: 0x0 },
    Qint { hh: 0xb311c412a9112489, hl: 0x3ecf14dc798a519b, lh: 0xfa6e051d6f8bc3ff, ll: 0xba1e54cf684354df, ex: 0, sgn: 0x0 },
    Qint { hh: 0xb504f333f9de6484, hl: 0x597d89b3754abe9f, lh: 0x1d6f60ba893ba84c, ll: 0xed17ac8583339915, ex: 0, sgn: 0x0 },
    Qint { hh: 0xb6fd91e328d17791, hl: 0x7165f0ddd541a59, lh: 0xf88abbe777df360e, ll: 0x20850e774a86cd8f, ex: 0, sgn: 0x0 },
    Qint { hh: 0xb8fbaf4762fb9ee9, hl: 0x1b879778566b65a1, lh: 0xa5ab16cf451056ed, ll: 0x322d7893ed4da9a8, ex: 0, sgn: 0x0 },
    Qint { hh: 0xbaff5ab2133e45fb, hl: 0x74d519d24593838c, lh: 0x2f30d0bdcaa516d, ll: 0x6c373a75c2828202, ex: 0, sgn: 0x0 },
    Qint { hh: 0xbd08a39f580c36be, hl: 0xa8811fb66d0faf7a, lh: 0x15b34bbcb0298f41, ll: 0xd9a4be023ece032, ex: 0, sgn: 0x0 },
    Qint { hh: 0xbf1799b67a731082, hl: 0xe815d0abcbf0b850, lh: 0xa13fc7e6faf9c830, ll: 0x83ea957596be426d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xc12c4cca66709456, hl: 0x7c457d59a50087b5, lh: 0x6b2e5dd607a9969c, ll: 0xdefefee72ae7a33d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xc346ccda24976407, hl: 0x20ec856128b83a42, lh: 0x6b9f89b7dabbcb2b, ll: 0x5b718d616c4fef19, ex: 0, sgn: 0x0 },
    Qint { hh: 0xc5672a115506dadd, hl: 0x3e2ad0c964dd9f37, lh: 0x6b0f939998251a36, ll: 0xc7686006e4e6c093, ex: 0, sgn: 0x0 },
    Qint { hh: 0xc78d74c8abb9b15c, hl: 0xc13a2e3976c0277e, lh: 0x4da570a2c574a304, ll: 0xcea65224bc9900d0, ex: 0, sgn: 0x0 },
    Qint { hh: 0xc9b9bd866e2f27a2, hl: 0x80e1f92a0511697e, lh: 0x257ac0db1f419377, ll: 0xf4dd023ff93c7ffb, ex: 0, sgn: 0x0 },
    Qint { hh: 0xcbec14fef2727c5c, hl: 0xf4907c8f45ebf6dc, lh: 0xeb8a25b7b40c0426, ll: 0x639aa6f940962626, ex: 0, sgn: 0x0 },
    Qint { hh: 0xce248c151f8480e3, hl: 0xe235838f95f2c6ed, lh: 0x6f28610b8c36485a, ll: 0x2bbd398af35c079f, ex: 0, sgn: 0x0 },
    Qint { hh: 0xd06333daef2b2594, hl: 0xd6d45c6559a4d502, lh: 0x11546d3ea28976d6, ll: 0x2a33269ab05c3e5d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xd2a81d91f12ae45a, hl: 0x12248e57c3de4028, lh: 0x52029c0b81f7be57, ll: 0xfa7663033f05357b, ex: 0, sgn: 0x0 },
    Qint { hh: 0xd4f35aabcfedfa1f, hl: 0x5921deffa6262c5a, lh: 0xb8e7a32e5783da5c, ll: 0xfa628009459a2417, ex: 0, sgn: 0x0 },
    Qint { hh: 0xd744fccad69d6af4, hl: 0x39a68bb9902d3fde, lh: 0x1d733af522058b16, ll: 0xb5c13ada0e77829a, ex: 0, sgn: 0x0 },
    Qint { hh: 0xd99d15c278afd7b5, hl: 0xfe873deca3e12bab, lh: 0xc0edda4d891be43d, ll: 0xb70cfbb1bdf6eb5d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xdbfbb797daf23755, hl: 0x3d840d5a9e29aa64, lh: 0x481e1ab725b12d56, ll: 0x613b0d1dbfa0d717, ex: 0, sgn: 0x0 },
    Qint { hh: 0xde60f4825e0e9123, hl: 0xdd07a2d9e8466859, lh: 0x1438495eacdf256, ll: 0xcc2490c8643ef6b4, ex: 0, sgn: 0x0 },
    Qint { hh: 0xe0ccdeec2a94e111, hl: 0x65895048dd333ca, lh: 0x224b251b33092002, ll: 0x1cb99d3f1ff298a2, ex: 0, sgn: 0x0 },
    Qint { hh: 0xe33f8972be8a5a51, hl: 0x9bfe90795980eec, lh: 0xf358a8d368fceaea, ll: 0xfa8fcbb2e85b853f, ex: 0, sgn: 0x0 },
    Qint { hh: 0xe5b906e77c8348a8, hl: 0x1e5e8f4a4edbb0ec, lh: 0xaacd6065b6e9f6ac, ll: 0xcefcd5b62a14b818, ex: 0, sgn: 0x0 },
    Qint { hh: 0xe8396a503c4bdc68, hl: 0x791790d0ac70c7dd, lh: 0xfe312f84fa665204, ll: 0x3a1c6473409c261d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xeac0c6e7dd24392e, hl: 0xd02d75b3706e54fa, lh: 0xc4faace043b7f91c, ll: 0x17d8d1e8ca31880b, ex: 0, sgn: 0x0 },
    Qint { hh: 0xed4f301ed9942b84, hl: 0x600d2db6a64bfb12, lh: 0x3787630a764ae4c9, ll: 0xc8e7c95b06416e6d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xefe4b99bdcdaf5cb, hl: 0x46561cf6948db912, lh: 0xd4a277eaddaa925c, ll: 0x9392870834f21a53, ex: 0, sgn: 0x0 },
    Qint { hh: 0xf281773c59ffb139, hl: 0xe8980a9cc8f47a4b, lh: 0x2cf0b49df0bd70e9, ll: 0x7c43b0ea5d43228d, ex: 0, sgn: 0x0 },
    Qint { hh: 0xf5257d152486cc2c, hl: 0x7b9d0c7aed980fc3, lh: 0x6f510308677709f5, ll: 0xbdd80329364aa2a0, ex: 0, sgn: 0x0 },
    Qint { hh: 0xf7d0df730ad13bb8, hl: 0xfe90d496d60fb6ea, lh: 0xe914ffb4723793f1, ll: 0xef6797b5a11efb7c, ex: 0, sgn: 0x0 },
    Qint { hh: 0xfa83b2db722a033a, hl: 0x7c25bb14315d7fcc, lh: 0x8006fe21a95d14dc, ll: 0x4844b29bf4af18e8, ex: 0, sgn: 0x0 },
    Qint { hh: 0xfd3e0c0cf486c174, hl: 0x853f3a5931e0ee03, lh: 0x61b7bb285a60791, ll: 0x9d2285b6754edd61, ex: 0, sgn: 0x0 },
];

/// `T2_3[i]` approximates 2^(i/2^12) (C `T2_3`).
#[rustfmt::skip]
static T2_3: [Qint; 64] = [
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80058baf7fee3b5d, hl: 0x1c718b38e549cb93, lh: 0x34a318717a85d198, ll: 0x945b3ca6120b7d55, ex: 0, sgn: 0x0 },
    Qint { hh: 0x800b179c82028fd0, hl: 0x945e54e2ae18f2f0, lh: 0x36ee988aaff03620, ll: 0x76cc37ff9584ce15, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8010a3c708e73282, hl: 0x2b96d62d51c15a07, lh: 0x68b51f6090715cda, ll: 0xe7a99fea0d150e10, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8016302f17467628, hl: 0x3690dfe44d11d008, lh: 0x403605216aed73f0, ll: 0x49b8f71dcaa49423, ex: 0, sgn: 0x0 },
    Qint { hh: 0x801bbcd4afcacb08, hl: 0xe23a986bd3e626f0, lh: 0x5bdd95c213fb273c, ll: 0x9e66ef0d411e38d0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x802149b7d51ebefb, hl: 0x7bdbadbc888aeb29, lh: 0x201cf874aa8cafc4, ll: 0x92f52199af16c4de, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8026d6d889ecfd69, hl: 0xb904bbfb40d3a2b6, lh: 0x84a6d5d525029ce2, ll: 0x32bdce1e8420f0a8, ex: 0, sgn: 0x0 },
    Qint { hh: 0x802c6436d0e04f50, hl: 0xff8ce94a6797b3ce, lh: 0x345f82f5b1fae20e, ll: 0x3d0b18c06975c162, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8031f1d2aca39b43, hl: 0xad9db772901d96b5, lh: 0x8f6321e8e84c97d3, ll: 0x3cc84ae246bf5abb, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80377fac1fe1e56a, hl: 0x61cd0bffd7cfc682, lh: 0xc0432e96c959387b, ll: 0xb9c4ef247a66c427, ex: 0, sgn: 0x0 },
    Qint { hh: 0x803d0dc32d464f85, hl: 0x43456f71b96affd4, lh: 0x34c51656768b5277, ll: 0xb09b272d97fcc8, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80429c17d77c18ed, hl: 0x49fc841afba9c3c5, lh: 0xaedee98517f79365, ll: 0x836514d2d81fdf14, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80482aaa212e9e95, hl: 0x86f7b54f6c45c85e, lh: 0x14747b1b6977fb14, ll: 0xbec69e11682e0863, ex: 0, sgn: 0x0 },
    Qint { hh: 0x804db97a0d095b0c, hl: 0x6c9f1f7d1efcfe68, lh: 0x6b994b07993e3561, ll: 0x9e86080e1001781d, ex: 0, sgn: 0x0 },
    Qint { hh: 0x805348879db7e67d, hl: 0x171eb1ceef1d1f28, lh: 0x5629bb4d6d20a74a, ll: 0x7407003ab22ffa82, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8058d7d2d5e5f6b0, hl: 0x94d589f608ee4aa2, lh: 0x2adc0c3f864ba0f5, ll: 0x9dc70119154b8f9a, ex: 0, sgn: 0x0 },
    Qint { hh: 0x805e675bb83f5f0f, hl: 0x2ed38ab8472b2143, lh: 0xc40f99da125c266f, ll: 0x94d103f4365ed44b, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8063f722477010a1, hl: 0xb1652de1378af1a0, lh: 0x8e5b66f89923f0ce, ll: 0x4c9bb4d5be541cc3, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8069872686241a12, hl: 0xb4ad9233a0390cac, lh: 0x930d2b4079a002bd, ll: 0x5cf73638639bbbd6, ex: 0, sgn: 0x0 },
    Qint { hh: 0x806f17687707a7af, hl: 0xe54ec5f966eb1872, lh: 0x76754509f037248a, ll: 0xd762ffd79b46d451, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8074a7e81cc7036b, hl: 0x4d204ecfc11f4aaa, lh: 0xf02c00376690ea79, ll: 0x233e0911cc8de5f9, ex: 0, sgn: 0x0 },
    Qint { hh: 0x807a38a57a0e94dc, hl: 0x9bf3ef4d9be2d1e4, lh: 0x6dbfe64309a2b072, ll: 0x3c5cacc11f785d53, ex: 0, sgn: 0x0 },
    Qint { hh: 0x807fc9a0918ae142, hl: 0x7068ab2230585d12, lh: 0x9fe6067d9e828773, ll: 0xdc2ac4e4e300cb2c, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80855ad965e88b83, hl: 0xa0cc0a49c10ea66a, lh: 0xf0eb8fefacaf32d8, ll: 0x78899d5679c5b99e, ex: 0, sgn: 0x0 },
    Qint { hh: 0x808aec4ff9d45430, hl: 0x84099bf6830f2767, lh: 0x9a875f4408858619, ll: 0xdfa0d299eae4aa3a, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80907e044ffb1984, hl: 0x3aa8b9cbbc65a8aa, lh: 0x8b22713e014be438, ll: 0x47da7d37d3e079f6, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80960ff66b09d765, hl: 0xf7d88c0928ba3946, lh: 0xd1441da0989f9760, ll: 0xff7e8daa7390655a, ex: 0, sgn: 0x0 },
    Qint { hh: 0x809ba2264dada76a, hl: 0x4a8a4f44bb703db6, lh: 0x212bb24b9d533796, ll: 0xfcdbcb683daab7f0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80a13493fa93c0d4, hl: 0x6699dc50dd96b773, lh: 0x8712128a139dc866, ll: 0xab9445c9a773244f, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80a6c73f74697897, hl: 0x6e0472ed4ccfa2df, lh: 0xc2857930dae5bef1, ll: 0x95df5640f17d2dbe, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80ac5a28bddc4157, hl: 0xba2dc7e0c72e51ba, lh: 0x6765fb22ac558aca, ll: 0x9a33e936c809a0e7, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80b1ed4fd999ab6c, hl: 0x25335719b6e6fd20, lh: 0x1f60261b05f1202, ll: 0x3c355acba4df4fa, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80b780b4ca4f64df, hl: 0x534dfa7417846aa4, lh: 0x68164a4ae2414ea4, ll: 0x2c64cb5808ef6fa6, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80bd145792ab3970, hl: 0xfc41c5c2d5336ccc, lh: 0x65250abea5b33d49, ll: 0xad82dbaac7bfa2e3, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80c2a838355b1297, hl: 0x34dc28baed8f3fde, lh: 0x533c9eca3a17497d, ll: 0xa1b8b14b109d4838, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80c83c56b50cf77f, hl: 0xb880575ea03548c1, lh: 0x4704388d9f1b3cd2, ll: 0x86f4e188e2ca8a59, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80cdd0b3146f0d11, hl: 0x32c1f98704428c71, lh: 0x7e5ed5955b2d4887, ll: 0x5b7e292a686df542, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80d3654d562f95ec, hl: 0x890e222a5eb95372, lh: 0x1197e58ebf689d43, ll: 0x3896d92dd4431f8b, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80d8fa257cfcf26e, hl: 0x24628efd9ca9d59a, lh: 0xc5f4be776ef6a61a, ll: 0x101735de189170ec, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80de8f3b8b85a0af, hl: 0x3b13310f5ad57fb0, lh: 0x9bad68937edd6b38, ll: 0xeae4250b29447d4b, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80e4248f84783c87, hl: 0x1a9dfefaeb616563, lh: 0x94426c99024f23f0, ll: 0x235b5252cafbaa02, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80e9ba216a837f8c, hl: 0x718d1151d109bf97, lh: 0x85189bdd7ac4b012, ll: 0x9bea88f10391b325, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80ef4ff140564116, hl: 0x996709da2e25f04b, lh: 0xe18453f8dafeabf1, ll: 0xf5867174289d8d94, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80f4e5ff089f763e, hl: 0xe0adc640acaa6b0a, lh: 0x8b6d28b5eb20d2f2, ll: 0xb125fb6305bf7e6d, ex: 0, sgn: 0x0 },
    Qint { hh: 0x80fa7c4ac60e31e1, hl: 0xd4eb5edc6b341283, lh: 0x370761b5ce7d7e44, ll: 0xf1eb5df89b727f7c, ex: 0, sgn: 0x0 },
    Qint { hh: 0x810012d47b51a4a0, hl: 0x8ccd7223820719e3, lh: 0x118525e07f78529c, ll: 0x97f6dffe47385081, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8105a99c2b191ce1, hl: 0xf24ebd6eb9ca4292, lh: 0x70f4efb7d5c90568, ll: 0x5301745d3b39c4d0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x810b40a1d81406d4, hl: 0xcef03ab14a6654f, lh: 0xa9c9ffc2ca67ffde, ll: 0xada5b6f36036c85a, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8110d7e584f1ec6d, hl: 0x4bf94297d1519822, lh: 0x9ee96b903910b0f, ll: 0x45fe2b1237a101fc, ex: 0, sgn: 0x0 },
    Qint { hh: 0x81166f673462756d, hl: 0xd0d8372f966cf15d, lh: 0xb70c0ef050a08aa9, ll: 0xd499953c6b9aa8f0, ex: 0, sgn: 0x0 },
    Qint { hh: 0x811c0726e9156760, hl: 0xb97931db7b7be2ec, lh: 0x135c526104fa1c29, ll: 0x215ef11d179cc996, ex: 0, sgn: 0x0 },
    Qint { hh: 0x81219f24a5baa59d, hl: 0x6abd3b0eab9c7047, lh: 0xa7712808fe956328, ll: 0x717f9b1d39438323, ex: 0, sgn: 0x0 },
    Qint { hh: 0x812737606d023148, hl: 0xdaf888e965081519, lh: 0xada38ad7502e18a9, ll: 0x6ebf0e93981c95f2, ex: 0, sgn: 0x0 },
    Qint { hh: 0x812ccfda419c2956, hl: 0xdc8046821f46122d, lh: 0x8b2f742bd9d4370a, ll: 0x5718a10a231edabf, ex: 0, sgn: 0x0 },
    Qint { hh: 0x813268922638ca8b, hl: 0x6846ad73a8d9027f, lh: 0x1163a8bcf6bffce3, ll: 0x2e1b37721d94b76, ex: 0, sgn: 0x0 },
    Qint { hh: 0x813801881d886f7b, hl: 0xe885724f14131286, lh: 0x9cea3c3530355654, ll: 0xf9c7f1fa9145fa7f, ex: 0, sgn: 0x0 },
    Qint { hh: 0x813d9abc2a3b9090, hl: 0x83768490519df895, lh: 0x605362ea89eb07d4, ll: 0x4794c4e3bcb98244, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8143342e4f02c405, hl: 0x661b22b45e25de17, lh: 0xa82b3121936ae61d, ll: 0x91fe35aa6124aefb, ex: 0, sgn: 0x0 },
    Qint { hh: 0x8148cdde8e8ebdec, hl: 0xf11430fef78c6ee, lh: 0x932801def6b0fb, ll: 0x8aa35adbcc33b28e, ex: 0, sgn: 0x0 },
    Qint { hh: 0x814e67cceb90502c, hl: 0x99775205944eadc4, lh: 0x2555ab2151b96f7c, ll: 0x1ab6ca4ae6eda941, ex: 0, sgn: 0x0 },
    Qint { hh: 0x815401f968b86a87, hl: 0x7de463a40d18260, lh: 0xdc941f1fd7a051c0, ll: 0xdddfd0f8f59dec56, ex: 0, sgn: 0x0 },
    Qint { hh: 0x81599c6408b81a94, hl: 0x8f4a0b6748df795f, lh: 0x988da3f28bde163d, ll: 0x951855dd23786b9c, ex: 0, sgn: 0x0 },
    Qint { hh: 0x815f370cce408bc8, hl: 0xe2404468cfe5ab9f, lh: 0x4db5f07dc6319207, ll: 0x221ce2379e877086, ex: 0, sgn: 0x0 },
];

/// Degree 18 to 1 (largest first) of a polynomial for log(1+z), |z| < 2^-13, relative error
/// below 2^-255.0786 (C `P_3`).
#[rustfmt::skip]
static P_3: [Qint; 18] = [
    Qint { hh: 0xe38e39d490f62b2f, hl: 0x0, lh: 0x0, ll: 0x0, ex: -5, sgn: 0x1 }, /* degree 18 */
    Qint { hh: 0xf0f0f1e1e1d4e1cf, hl: 0xbbb343000334fd0f, lh: 0x0, ll: 0x0, ex: -5, sgn: 0x0 }, /* degree 17 */
    Qint { hh: 0xffffffffffff88b8, hl: 0xc17633c5a3181e76, lh: 0x0, ll: 0x0, ex: -5, sgn: 0x1 }, /* degree 16 */
    Qint { hh: 0x8888888888885088, hl: 0x8f6a4426b02f93be, lh: 0x0, ll: 0x0, ex: -4, sgn: 0x0 }, /* degree 15 */
    Qint { hh: 0x9249249249249249, hl: 0x24a2676c009fc980, lh: 0x0, ll: 0x0, ex: -4, sgn: 0x1 }, /* degree 14 */
    Qint { hh: 0x9d89d89d89d89d89, hl: 0xd8ab89d5a96621f1, lh: 0x0, ll: 0x0, ex: -4, sgn: 0x0 }, /* degree 13 */
    Qint { hh: 0xaaaaaaaaaaaaaaaa, hl: 0xaaaaaaaaa815192a, lh: 0x0, ll: 0x0, ex: -4, sgn: 0x1 }, /* degree 12 */
    Qint { hh: 0xba2e8ba2e8ba2e8b, hl: 0xa2e8ba2e899ae964, lh: 0x0, ll: 0x0, ex: -4, sgn: 0x0 }, /* degree 11 */
    Qint { hh: 0xcccccccccccccccc, hl: 0xcccccccccccccccd, lh: 0xcc491481418dc51, ll: 0x1cebff4e21be093e, ex: -4, sgn: 0x1 }, /* degree 10 */
    Qint { hh: 0xe38e38e38e38e38e, hl: 0x38e38e38e38e38e3, lh: 0xba38ce3dcedbed7d, ll: 0xfa2cc6f77565683f, ex: -4, sgn: 0x0 }, /* degree 9 */
    Qint { hh: 0xffffffffffffffff, hl: 0xffffffffffffffff, lh: 0xfffffffc6072860a, ll: 0x786bd58754911c58, ex: -4, sgn: 0x1 }, /* degree 8 */
    Qint { hh: 0x9249249249249249, hl: 0x2492492492492492, lh: 0x492492481c930545, ll: 0xf0298bcd6e1b2310, ex: -3, sgn: 0x0 }, /* degree 7 */
    Qint { hh: 0xaaaaaaaaaaaaaaaa, hl: 0xaaaaaaaaaaaaaaaa, lh: 0xaaaaaaaaaaaaaab8, ll: 0xd8b61a619485f089, ex: -3, sgn: 0x1 }, /* degree 6 */
    Qint { hh: 0xcccccccccccccccc, hl: 0xcccccccccccccccc, lh: 0xccccccccccccccd2, ll: 0xccc65d183d01d5ef, ex: -3, sgn: 0x0 }, /* degree 5 */
    Qint { hh: 0xffffffffffffffff, hl: 0xffffffffffffffff, lh: 0xffffffffffffffff, ll: 0xffffffcdc3a6a23c, ex: -3, sgn: 0x1 }, /* degree 4 */
    Qint { hh: 0xaaaaaaaaaaaaaaaa, hl: 0xaaaaaaaaaaaaaaaa, lh: 0xaaaaaaaaaaaaaaaa, ll: 0xaaaaaaa4aab50b70, ex: -2, sgn: 0x0 }, /* degree 3 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0xd, ex: -1, sgn: 0x1 }, /* degree 2 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* degree 1 */
];

/// Degree 14 to 0 (largest first) of a polynomial for exp(z), |z| < 0.00016923, relative
/// error below 2^-242.180 (C `Q_3`).
#[rustfmt::skip]
static Q_3: [Qint; 15] = [
    Qint { hh: 0xc9cba547af749429, hl: 0x0, lh: 0x0, ll: 0x0, ex: -37, sgn: 0x0 }, /* degree 14 */
    Qint { hh: 0xb092309ec73dd7db, hl: 0x0, lh: 0x0, ll: 0x0, ex: -33, sgn: 0x0 }, /* degree 13 */
    Qint { hh: 0x8f76c77fc6c4bda8, hl: 0xcd9aab7578033f6d, lh: 0x0, ll: 0x0, ex: -29, sgn: 0x0 }, /* degree 12 */
    Qint { hh: 0xd7322b3faa271c7d, hl: 0xb3537cbfd60dcb9, lh: 0x0, ll: 0x0, ex: -26, sgn: 0x0 }, /* degree 11 */
    Qint { hh: 0x93f27dbbc4fae397, hl: 0x780b69f6554de3d9, lh: 0x0, ll: 0x0, ex: -22, sgn: 0x0 }, /* degree 10 */
    Qint { hh: 0xb8ef1d2ab6399c7d, hl: 0x560e44741a6a8e66, lh: 0x0, ll: 0x0, ex: -19, sgn: 0x0 }, /* degree 9 */
    Qint { hh: 0xd00d00d00d00d00d, hl: 0xd00d00d00d00cf, lh: 0x0, ll: 0x0, ex: -16, sgn: 0x0 }, /* degree 8 */
    Qint { hh: 0xd00d00d00d00d00d, hl: 0xd00d00d00d00ce, lh: 0xca5a80878f19216b, ll: 0xd236a7fa15252936, ex: -13, sgn: 0x0 }, /* degree 7 */
    Qint { hh: 0xb60b60b60b60b60b, hl: 0x60b60b60b60b60b6, lh: 0xb60b60be6ac2e60, ll: 0x800a9987617257e3, ex: -10, sgn: 0x0 }, /* degree 6 */
    Qint { hh: 0x8888888888888888, hl: 0x8888888888888888, lh: 0x888888890ac16c5a, ll: 0xf78e687c535a714, ex: -7, sgn: 0x0 }, /* degree 5 */
    Qint { hh: 0xaaaaaaaaaaaaaaaa, hl: 0xaaaaaaaaaaaaaaaa, lh: 0xaaaaaaaaaaaaaaaa, ll: 0x6b3ad4c251cd03d5, ex: -5, sgn: 0x0 }, /* degree 4 */
    Qint { hh: 0xaaaaaaaaaaaaaaaa, hl: 0xaaaaaaaaaaaaaaaa, lh: 0xaaaaaaaaaaaaaaaa, ll: 0x4df8c3de374c499e, ex: -3, sgn: 0x0 }, /* degree 3 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x1446e270, ex: -1, sgn: 0x0 }, /* degree 2 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x262ce809, ex: 0, sgn: 0x0 }, /* degree 1 */
    Qint { hh: 0x8000000000000000, hl: 0x0, lh: 0x0, ll: 0x0, ex: 0, sgn: 0x0 }, /* degree 0 */
];

/// exp(y) for |y| < 0.00016923, error below 2^-241.11 (C `q_3`).
fn q_3(y: &Qint) -> Qint {
    let mut r = mul_qint_11(y, &Q_3[0]);
    r = add_qint_22(&Q_3[1], &r);
    for c in &Q_3[2..7] {
        r = mul_qint_22(y, &r);
        r = add_qint_22(c, &r);
    }
    for c in &Q_3[7..12] {
        r = mul_qint_33(y, &r);
        r = add_qint(c, &r);
    }
    for c in &Q_3[12..15] {
        r = mul_qint(y, &r);
        r = add_qint(c, &r);
    }
    r
}

/// log(1+z) for |z| < 2^-13 (C `p_3`).
fn p_3(z: &Qint) -> Qint {
    let mut r = mul_qint_11(&P_3[0], z);
    r = add_qint_22(&P_3[1], &r);
    for c in &P_3[2..4] {
        r = mul_qint_11(&r, z);
        r = add_qint_22(c, &r);
    }
    for c in &P_3[4..8] {
        r = mul_qint_21(&r, z);
        r = add_qint_22(c, &r);
    }
    for c in &P_3[8..14] {
        r = mul_qint_31(&r, z);
        r = add_qint(c, &r);
    }
    for c in &P_3[14..18] {
        r = mul_qint_41(&r, z);
        r = add_qint(c, &r);
    }
    mul_qint_41(&r, z)
}

/// log(x), relative error below 2^-250.74 (C `log_3`).
fn log_3(x: &Qint) -> Qint {
    let mut x = *x;
    let mut e = x.ex;
    let i = if x.hh > 0xb504f333f9de6484 {
        e += 1;
        (x.hh >> (63 + 1 - 7)) as u16
    } else {
        (x.hh >> (63 - 7)) as u16
    } as usize;
    x.ex -= e;
    let mut z = mul_qint(&x, &INVERSE_3_1[i - 90]);
    let j = (z.hh >> (63 - 13 - z.ex)) as u16 as usize;
    z = mul_qint(&z, &INVERSE_3_2[j - 8128]);
    z = add_qint(&M_ONE_Q, &z);
    let r = mul_qint_2(e, &LOG2_Q);
    let mut p = p_3(&z);
    p = add_qint(&LOG_INV_3_2[j - 8128], &p);
    p = add_qint(&LOG_INV_3_1[i - 90], &p);
    add_qint(&p, &r)
}

/// exp(x), relative error below 2^-241.10 (C `exp_3`).
fn exp_3(x: &Qint) -> Qint {
    let big_k = mul_qint_11(x, &LOG2_INV_Q);
    let k = qint_toi(&big_k);
    let mut big_k = mul_qint_2(k, &LOG2_Q);
    big_k.ex -= 12;
    big_k.sgn = u64::from(big_k.sgn == 0);
    let y = add_qint(x, &big_k);
    let m = k >> 12;
    let i2 = ((k >> 6) & 0x3f) as usize;
    let i1 = (k & 0x3f) as usize;
    let mut r = q_3(&y);
    r = mul_qint(&T1_3[i2], &r);
    r = mul_qint(&T2_3[i1], &r);
    r.ex += m;
    r
}

// ---------------------------------------------------------------------------------------------
// Exact and midpoint cases (C `exact_pow`, `is_exact`), and phases 2 and 3 of `cr_pow`.
// ---------------------------------------------------------------------------------------------

/// x^y when it is exact or a midpoint (C `exact_pow`), given the phase-2 approximation z.
fn exact_pow(x: f64, y: f64, z: &Dint, exact: bool) -> Option<f64> {
    let s: i64 = if z.sgn != 0 { -1 } else { 1 };
    let (big_e, mut m) = extract(x);
    if m == 1 {
        // x = 2^E: x^y = 2^(E*y).
        let big_g = big_e as f64 * y;
        if is_int(big_g) {
            let r = if z.sgn != 0 { -1.0 } else { 1.0 };
            return Some(pow2(r, big_g as i64));
        }
        return None;
    }
    if !(0.0..=34.0).contains(&y) {
        return None;
    }
    let (big_f, n) = extract(y);
    if n > 34 || big_f < -5 {
        return None;
    }
    if big_f < 0 {
        // y = n/2^-F, n odd: E must be a multiple of 2^-F.
        if (big_e as u64) & (!0u64 >> (64 + big_f)) != 0 {
            return None;
        }
        let g = (big_e >> -big_f) * n as i64;
        let (big_g, k) = round_54(z);
        if !exact {
            let cnt = (k as u64).leading_zeros();
            let mut d = Dint {
                hi: (k as u64) << cnt,
                lo: 0,
                ex: big_g + 63 - i64::from(cnt),
                sgn: 1 - z.sgn,
            };
            d = add_dint(z, &d);
            d.ex += 116;
            if cmp_dint_abs(&d, z) != core::cmp::Ordering::Less {
                return None;
            }
        }
        if big_g > g {
            return None;
        }
        let sh = (g - big_g) as u32;
        // k < 2^55, so 2^sh with sh >= 55 never matches (C shifts by sh < 64 only then).
        if sh < 64 && (k as u64 & !((!1u64) << sh)) == (1u64 << sh) {
            let r = ((k >> sh) * s) as f64;
            return Some(pow2(r, g));
        }
        return None;
    }
    let mut t = n << big_f;
    let mut k: i64 = 1;
    while t != 0 {
        if t & 0x1 != 0 {
            k = i64::try_from(i128::from(m) * i128::from(k)).ok()?;
        }
        t >>= 1;
        if t != 0 {
            m = m.checked_mul(m)?;
        }
    }
    if k >> 54 != 0 {
        return None;
    }
    let r = (k * s) as f64;
    let big_g = big_e * (n << big_f) as i64;
    Some(pow2(r, big_g))
}

/// For y >= 0 an integer: the largest x mantissa m (odd) with m^y below 2^64 (C `xmax`).
#[rustfmt::skip]
const XMAX: [u64; 34] = [
    0, 0xffffffffffffffff, 94906265, 208063, 9741, 1551, 455, 189, 97, 59, 39, 27, 21, 15, 13,
    11, 9, 7, 7, 5, 5, 5, 5, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
];

/// Whether x^y is exact in binary64 (x > 0 finite, y finite) (C `is_exact`).
fn is_exact(x: f64, y: f64) -> bool {
    let v = x.to_bits();
    let w = y.to_bits();
    if (v << 1) != 0x7fe0000000000000 && (w << 22) != 0 {
        return false;
    }
    if (v << 1) == 0x7fe0000000000000 {
        return true; // x = ±1
    }
    if y >= 0.0 && is_int(y) {
        let mut m = v & 0xfffffffffffff;
        let mut e = ((v << 1) >> 53) as i64 - 0x433;
        if e >= -1074 {
            m |= 0x10000000000000;
        } else {
            e += 1;
        }
        let t = m.trailing_zeros();
        m >>= t;
        e += i64::from(t);
        if y == 0.0 || y == 1.0 {
            return true;
        }
        if m == 1 {
            let ye = y * e as f64;
            return (-1074.0..1024.0).contains(&ye);
        }
        if !(0.0..=33.0).contains(&y) {
            return false;
        }
        let y_int = y as usize;
        if m > XMAX[y_int] {
            return false;
        }
        let t = 64 - i64::from(m.leading_zeros());
        let ez = e * y_int as i64 + t;
        if ez <= -1074 || 1024 < ez {
            return false;
        }
        return e * y_int as i64 >= -1074;
    }
    let mut n = w & 0xfffffffffffff;
    let mut f = ((w << 1) >> 53) as i64 - 0x433;
    if f >= -1074 {
        n |= 0x10000000000000;
    } else {
        f += 1;
    }
    let t = n.trailing_zeros();
    n >>= t;
    f += i64::from(t);
    let mut m = v & 0xfffffffffffff;
    let mut e = ((v << 1) >> 53) as i64 - 0x433;
    if e >= -1074 {
        m |= 0x10000000000000;
    } else {
        e += 1;
    }
    let t = m.trailing_zeros();
    m >>= t;
    e += i64::from(t);
    if y < 0.0 {
        if m != 1 {
            return false;
        }
        let ez: i64 = if f >= 0 {
            if f < 12 {
                (n.wrapping_neg().wrapping_mul(e as u64) << f) as i64
            } else {
                1024
            }
        } else {
            // e != 0 here (x = 1 returned above).
            let t = e.trailing_zeros();
            if -f > i64::from(t) {
                return false;
            }
            ((-e >> -f) as u64).wrapping_mul(n) as i64
        };
        return (-1074..1024).contains(&ez);
    }
    while f != 0 {
        f += 1;
        if e & 1 != 0 {
            return false;
        }
        e /= 2;
        let dm = m as f64;
        // C: round(sqrt(dm)); sqrt(dm) < 2^27 here, so +0.5 and a conversion round it.
        let s = ((dm.sqrt() + 0.5) as u64) as f64;
        if s * s != dm {
            return false;
        }
        m = s as u64;
    }
    if m > 1 {
        if 33 < n {
            return false;
        }
        if m > XMAX[n as usize] {
            return false;
        }
    }
    let mut my = m;
    let mut n0 = n;
    while n0 > 1 {
        n0 -= 1;
        my = my.wrapping_mul(m);
    }
    let t = 64 - i64::from(my.leading_zeros());
    let en = e * n as i64;
    -1074 <= en && en + t <= 1024
}

/// Phases 2 and 3 of `cr_pow` (x0 the original x, x = |x0|, s the sign of the result).
#[cold]
#[inline(never)]
fn pow_accurate(x0: f64, x: f64, y: f64, s: f64) -> f64 {
    let exact = is_exact(x, y);
    // Phase 2.
    let mut big_x = dint_fromd(x); // exact
    let big_y = dint_fromd(y); // exact
    big_x.sgn = 0x0;
    let mut r = log_2(&big_x); // relative error below 2^-122.88
    r = mul_dint_21(&r, &big_y);
    r = exp_2(&r); // relative error below 2^-121.70; in all below 2^-113.17
    if r.ex < -1075 {
        // Underflow.
        return 0.5 * (s * hf("0x1p-1074"));
    }
    let rd = if r.ex < -1022 {
        // Subnormal: shift the significand right by ex bits (1 <= ex <= 53).
        let ex = (-(1022 + r.ex)) as u32;
        let m = (r.lo >> (10 + ex)) | (r.hi << (54 - ex));
        m.wrapping_add(14) > 2 * 14
    } else {
        // The 64 bits after the round bit; 28 = floor(29126/2^10) (C comments).
        const ERR_BND_2: u64 = 28;
        let lo = (r.lo >> 10) | (r.hi << 54);
        lo.wrapping_add(ERR_BND_2) > 2 * ERR_BND_2
    };
    r.sgn = u64::from(s == -1.0);
    if rd {
        return dint_tod(&r);
    }
    if let Some(e) = exact_pow(x0, y, &r, exact) {
        return e;
    }
    // Phase 3: the hard-to-round cases.
    let mut qx = qint_fromd(x);
    let qy = qint_fromd(y);
    qx.sgn = 0x0;
    let mut qr = log_3(&qx); // relative error below 2^-250.74
    qr = mul_qint_41(&qr, &qy);
    let mut qz = exp_3(&qr); // in all, below 62433 ulps
    const ERR_BND_3: u64 = 60; // floor(62433/2^10)
    let r1 = (qz.hh << 54) | (qz.hl >> 10);
    let r2 = (qz.hl << 54) | (qz.lh >> 10);
    let r3 = (qz.lh << 54) | (qz.ll >> 10);
    let rd = !((r1 == 0 && r2 == 0 && r3 <= ERR_BND_3)
        || (!r1 == 0 && !r2 == 0 && r3.wrapping_add(2 * ERR_BND_3) <= ERR_BND_3));
    if rd {
        qz.sgn = u64::from(s == -1.0);
        qz.ll &= !0u64 << 10;
        return qint_tod(&mut qz);
    }
    // x^y very close to 1: for |qR| < 2^-55, exp(qR) rounds to 1 (to nearest).
    if qr.ex < -56 {
        return if qr.sgn == 0x0 {
            1.0 + hf("0x1p-100")
        } else {
            1.0 - hf("0x1p-100")
        };
    }
    // C reports an unexpected worst case and exits; every hard case is known, so this is not
    // reached. Return the phase-3 rounding.
    qz.sgn = u64::from(s == -1.0);
    qz.ll &= !0u64 << 10;
    qint_tod(&mut qz)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f64 = f64::INFINITY;
    const NEG_INF: f64 = f64::NEG_INFINITY;

    /// Same bits, or both NaN.
    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    fn check(x: f64, y: f64, want: f64) {
        let got = pow_js(x, y);
        assert!(
            same(got, want),
            "pow_js({x:e}, {y:e}) = {got:e}, want {want:e}"
        );
    }

    /// A C99 hexadecimal literal (Python `float.hex` form) or inf/nan.
    fn v(s: &str) -> f64 {
        match s {
            "inf" => INF,
            "-inf" => NEG_INF,
            "nan" => f64::NAN,
            _ => hf(s),
        }
    }

    /// The ep9_vm numconv.rs test of `pow_exact`, moved unchanged (M0 week 4).
    #[test]
    fn pow_exact_cases() {
        assert_eq!(pow_exact(2.0, 10.0), Some(1024.0));
        assert_eq!(pow_exact(-3.0, 3.0), Some(-27.0));
        assert_eq!(pow_exact(7.5, 0.0), Some(1.0));
        assert!(pow_exact(f64::NAN, 0.0) == Some(1.0));
        assert!(pow_exact(2.0, f64::NAN).unwrap().is_nan());
        assert_eq!(pow_exact(1.25, 1.0), Some(1.25));
        assert!(pow_exact(-0.0, 3.0).unwrap().is_sign_negative());
        assert_eq!(pow_exact(2.0, 54.0), None);
        assert_eq!(pow_exact(2.0, 0.5), None);
        assert_eq!(pow_exact(1.5, 2.0), None);
    }

    /// The M0 `numconv::exponentiate` (pow_exact plus the infinite-exponent rules), kept as the
    /// reference: wherever it answered, `pow_js` gives the same bits.
    fn m0_exponentiate(base: f64, exp: f64) -> Option<f64> {
        if let Some(r) = pow_exact(base, exp) {
            return Some(r);
        }
        if exp.is_infinite() && !base.is_nan() {
            let a = base.abs();
            return Some(if a == 1.0 {
                f64::NAN
            } else if (a > 1.0) == (exp > 0.0) {
                f64::INFINITY
            } else {
                0.0
            });
        }
        None
    }

    fn sample_values() -> Vec<f64> {
        let mut v = vec![
            0.0,
            1.0,
            2.0,
            3.0,
            0.5,
            0.25,
            1.5,
            7.0,
            10.0,
            64.0,
            65.0,
            53.0,
            0.1,
            1e-300,
            1e300,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            9_007_199_254_740_992.0,
            9_007_199_254_740_994.0,
            0.999_999_999_999_999,
            1.000_000_000_000_001,
            std::f64::consts::PI,
            INF,
            f64::NAN,
        ];
        let neg: Vec<f64> = v.iter().map(|x| -x).collect();
        v.extend(neg);
        v
    }

    #[test]
    fn pow_js_keeps_every_m0_answer() {
        let vals = sample_values();
        let mut answered = 0;
        for &x in &vals {
            for &y in &vals {
                if let Some(old) = m0_exponentiate(x, y) {
                    answered += 1;
                    check(x, y, old);
                }
            }
        }
        assert!(answered > 300, "{answered}");
        for x in -40i32..=40 {
            for y in 0..=64 {
                let (x, y) = (f64::from(x), f64::from(y));
                if let Some(old) = m0_exponentiate(x, y) {
                    check(x, y, old);
                }
            }
        }
    }

    /// test262 language/expressions/exponentiation applying-the-exp-operator_A1-A23 and the
    /// ECMA-262 Number::exponentiate steps, one row each.
    #[test]
    fn pow_js_spec_special_values() {
        let max = f64::MAX;
        let pi = std::f64::consts::PI;
        // A1-A3: NaN exponent, zero exponent, NaN base.
        check(1.0, f64::NAN, f64::NAN);
        check(f64::NAN, -0.0, 1.0);
        check(f64::NAN, 1.5, f64::NAN);
        // A4-A9: y = ±∞.
        for x in [1.0, -1.0] {
            check(x, INF, f64::NAN);
            check(x, NEG_INF, f64::NAN);
        }
        for x in [1.000_000_000_000_001, 2.0, max, -2.0, NEG_INF, INF] {
            check(x, INF, INF);
            check(x, NEG_INF, 0.0);
        }
        for x in [
            0.999_999_999_999_999,
            0.5,
            0.0,
            -0.0,
            -0.5,
            -0.999_999_999_999_999,
        ] {
            check(x, INF, 0.0);
            check(x, NEG_INF, INF);
        }
        // A11-A12: x = +∞.
        for y in [1e-15, 1.0, max, INF, 2.0, 0.5] {
            check(INF, y, INF);
            check(INF, -y, 0.0);
        }
        // A13-A16: x = -∞.
        for y in [1.0, 111.0, 111_111.0] {
            check(NEG_INF, y, NEG_INF);
            check(NEG_INF, -y, -0.0);
        }
        for y in [1e-15, 2.0, pi, max, INF, 0.5] {
            check(NEG_INF, y, INF);
            check(NEG_INF, -y, 0.0);
        }
        // A17-A22: x = ±0.
        for y in [1e-15, 1.0, max, INF, 2.0, 0.5] {
            check(0.0, y, 0.0);
            check(0.0, -y, INF);
        }
        for y in [1.0, 111.0, 111_111.0] {
            check(-0.0, y, -0.0);
            check(-0.0, -y, NEG_INF);
        }
        for y in [1e-15, 2.0, pi, max, INF, 0.5] {
            check(-0.0, y, 0.0);
            check(-0.0, -y, INF);
        }
        // A23: negative finite base, finite non-integer exponent.
        for x in [-max, -pi, -1.0, -0.5, -5e-324] {
            for y in [-pi, -0.1, 0.1, pi, 1e-15] {
                check(x, y, f64::NAN);
            }
        }
        // ±1 bases with finite exponents.
        check(1.0, 1e300, 1.0);
        check(1.0, -0.1, 1.0);
        check(-1.0, 1e300, 1.0);
        check(-1.0, 9_007_199_254_740_991.0, -1.0);
        check(-1.0, -3.0, -1.0);
        // Odd-integer test at the 2^53 edge: 2^53 + 2 is even, 2^53 - 1 is odd.
        check(-0.0, 9_007_199_254_740_994.0, 0.0);
        check(-0.0, 9_007_199_254_740_991.0, -0.0);
        check(NEG_INF, -9_007_199_254_740_991.0, -0.0);
    }

    /// H36 step 5, and inputs that v0 left to the correctly rounded core (results from MPFR
    /// 4.2 through gmpy2, binary64 context, round to nearest).
    #[test]
    fn pow_js_step5_and_cr_core() {
        check(1.5, 2.0, 2.25);
        check(-3.0, 2.0, 9.0);
        check(1e200, 2.0, INF);
        check(0.1, 2.0, 0.1 * 0.1);
        check(2.0, 0.5, std::f64::consts::SQRT_2);
        check(-0.0, 0.5, 0.0);
        check(-4.0, 0.5, f64::NAN);
        check(NEG_INF, 0.5, INF);
        check(2.0, -1.0, 0.5);
        check(-0.0, -1.0, NEG_INF);
        check(3.0, -1.0, 1.0 / 3.0);
        check(0.1, 1.0, 0.1);
        // Exact integer path.
        check(2.0, 32.0, 4_294_967_296.0);
        check(-2.0, 53.0, -9_007_199_254_740_992.0);
        check(10.0, 15.0, 1e15);
        // The correctly rounded core (v0 gave None here).
        check(2.0, 0.3, v("0x1.3b2c47bff8329p+0"));
        check(2.0, 54.0, v("0x1p+54"));
        check(2.0, -2.0, 0.25);
        check(10.0, -1.5, v("0x1.030dc4ea03a72p-5"));
        check(1.5, 3.0, 3.375);
        check(-8.0, 1.0 / 3.0, f64::NAN); // negative base, non-integer exponent
        check(-1.5, 3.0, -3.375);
        check(0.5, 1e300, 0.0);
        check(3.0, 0.3, v("0x1.63f08b6f382aep+0"));
        check(std::f64::consts::E, 2.5, v("0x1.85d6fd931e0bap+3"));
        // Overflow and underflow at the edges.
        check(10.0, 308.0, v("0x1.1ccf385ebc8a0p+1023"));
        check(10.0, 309.0, INF);
        check(10.0, -323.0, v("0x0.0000000000002p-1022"));
        check(2.0, -1074.0, v("0x0.0000000000001p-1022"));
        check(2.0, -1075.0, 0.0);
        check(-2.0, -1075.0, -0.0);
    }

    /// H36 Consequences: `10 ** k === 1ek` for k in -30..30 (fdlibm pow fails 10 of these).
    #[test]
    fn ten_to_the_k_is_the_decimal_literal() {
        for k in -30i32..=30 {
            let want: f64 = format!("1e{k}").parse().unwrap();
            check(10.0, f64::from(k), want);
        }
    }

    /// CORE-MATH worst cases (`src/binary64/pow/pow.wc`: the first 4 and 12 drawn at random),
    /// with the MPFR results.
    #[test]
    fn pow_core_math_worst_cases() {
        for (x, y, want) in [
            (
                "0x1.c4269c893fd34p+50",
                "0x1.4000000000000p-2",
                "0x1.d79ca618b9632p+15",
            ),
            (
                "0x1.ffffffffffffcp-1",
                "0x1.fffffffffffffp-2",
                "0x1.ffffffffffffep-1",
            ),
            (
                "0x1.ffffffffffffcp-1",
                "-0x1.ffffffffffffdp-2",
                "0x1.0000000000001p+0",
            ),
            (
                "0x1.ffffffffffff0p-1",
                "-0x1.ffffffffffff6p-3",
                "0x1.0000000000002p+0",
            ),
            (
                "0x1.de2e74ff76878p-1",
                "0x1.6100000000000p+11",
                "0x1.828079b304a96p-279",
            ),
            (
                "0x1.489854f9520c6p-1",
                "0x1.7100000000000p+8",
                "0x1.de3db977be98fp-237",
            ),
            ("0x1.d853b33ccaa53p+0", "0x1.f020000000000p+12", "inf"),
            ("0x1.2460b18459394p+0", "0x1.6dd0000000000p+12", "inf"),
            (
                "0x1.16d26e97e5fbfp+0",
                "0x1.b800000000000p+5",
                "0x1.b660d4c96d517p+6",
            ),
            ("0x1.841ae5269e68bp-1", "0x1.8528000000000p+13", "0x0.0p+0"),
            (
                "0x1.fd506bcd0a5cdp-1",
                "0x1.ff20000000000p+12",
                "0x1.ece485332e98dp-63",
            ),
            (
                "0x1.923c0c57b413fp-1",
                "0x1.c880000000000p+9",
                "0x1.21b93570494a9p-318",
            ),
            (
                "0x1.fffffec000000p+509",
                "0x1.0000000000000p+1",
                "0x1.fffffd800000cp+1019",
            ),
            (
                "0x1.add79f2b12b41p-1",
                "0x1.3dc0000000000p+11",
                "0x1.799d4645c0af3p-642",
            ),
            ("0x1.b5c1123519c27p-1", "0x1.20d8000000000p+13", "0x0.0p+0"),
            ("0x1.2fa9fc92705f9p-1", "0x1.1f40000000000p+13", "0x0.0p+0"),
        ] {
            check(v(x), v(y), v(want));
        }
    }

    /// The hardware and the software FMA give the same bits (H36: one result on every target),
    /// in phase 1 and through the accurate phases.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn pow_soft_equals_hw() {
        let mut s = 0x2545f4914f6cdd1du64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for i in 0..200_000 {
            let x = f64::from_bits(next() >> 1);
            // y such that x^y is in range, or a raw y; every 8th a near-1 x.
            let x = if i % 8 == 0 {
                1.0 + (x.to_bits() as f64) * hf("0x1p-100")
            } else {
                x
            };
            let r = (next() >> 11) as f64 * hf("0x1p-53");
            let y = if i % 2 == 0 {
                (r * 1400.0 - 700.0) / crate::log(x)
            } else {
                f64::from_bits(next())
            };
            if !x.is_finite() || !y.is_finite() || x == 0.0 || x == 1.0 || y == 0.0 {
                continue;
            }
            let hw = pow_cr_with::<crate::exact::HwFma>(x, y);
            let sw = pow_cr_with::<crate::exact::SoftFma>(x, y);
            assert!(same(hw, sw), "pow({x:e}, {y:e}): {hw:e} vs {sw:e}");
        }
    }
}
