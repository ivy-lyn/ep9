//! Exact building blocks (H36): [`fma_exact`] (`a * b + c` rounded once), [`two_prod`] and
//! [`two_sum`] (error-free transformations).
//!
//! An FMA is allowed only as one of these exact operations, never as a contraction of `a * b + c`
//! (H36 Decision). The algorithms are generic over [`FmaOps`], with two implementations that give
//! the same bits on every input:
//!
//! | Implementation | What | Used on |
//! |---|---|---|
//! | [`HwFma`] | `f64::mul_add`: the hardware instruction. The crate's only `mul_add` | aarch64 (FMA is base ISA); x86-64 when the CPU has FMA |
//! | [`SoftFma`] | integer-only port of the libm generic `fma` (MIT, musl origin) | x86-64 without FMA; every other target |
//!
//! On x86-64, the `multiversion!` macro compiles an entry point twice, a
//! `#[target_feature(enable = "fma")]` copy with [`HwFma`] and a [`SoftFma`] copy, and picks one
//! per call with `is_x86_feature_detected!("fma")` (std caches the CPUID answer). Rust's default
//! x86-64 target has no FMA, so a plain `mul_add` there would call the C `fma`, which
//! `cargo xtask fp-check` bans. The dispatch is the crate's only `unsafe` code.

/// `a * b + c` with one rounding (IEEE-754 fusedMultiplyAdd, round to nearest, ties to even).
pub trait FmaOps {
    fn fma(a: f64, b: f64, c: f64) -> f64;
}

/// The hardware FMA instruction. Inline it only into code that runs on a CPU with FMA: aarch64,
/// or an x86-64 function compiled with `#[target_feature(enable = "fma")]`. Elsewhere `mul_add`
/// becomes a call to the C `fma`.
pub struct HwFma;

/// FMA in integer arithmetic: the same bits as [`HwFma`] on every input, on every target.
pub struct SoftFma;

impl FmaOps for HwFma {
    #[inline(always)]
    // H36: the one allowed `mul_add`. It is an exact operation, not a contraction.
    #[allow(clippy::disallowed_methods)]
    fn fma(a: f64, b: f64, c: f64) -> f64 {
        a.mul_add(b, c)
    }
}

impl FmaOps for SoftFma {
    #[inline(always)]
    fn fma(a: f64, b: f64, c: f64) -> f64 {
        soft_fma(a, b, c)
    }
}

/// Defines a public entry point `$name(args) -> ret` that calls `$generic::<F: FmaOps>(args)`
/// with the FMA implementation of the running CPU (module doc). The entry point is
/// `#[inline(never)]`, so every FMA instruction stays inside an `ep9_math::` symbol, where
/// `cargo xtask fp-check` allows it. `$generic` must be `#[inline(always)]`.
///
/// Both copies are exact, so they give the same bits (H36 allows a hardware/software dispatch
/// only between exact implementations).
macro_rules! multiversion {
    ($(#[$m:meta])* $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty = $generic:ident;) => {
        $(#[$m])*
        #[inline(never)]
        $vis fn $name($($arg: $ty),*) -> $ret {
            #[cfg(target_arch = "x86_64")]
            {
                #[target_feature(enable = "fma")]
                fn hw($($arg: $ty),*) -> $ret {
                    $generic::<$crate::exact::HwFma>($($arg),*)
                }
                if ::std::is_x86_feature_detected!("fma") {
                    #[allow(unsafe_code)]
                    // SAFETY: `hw` only needs the `fma` target feature, and the CPU has it
                    // (checked on the line above).
                    return unsafe { hw($($arg),*) };
                }
                $generic::<$crate::exact::SoftFma>($($arg),*)
            }
            #[cfg(target_arch = "aarch64")]
            {
                $generic::<$crate::exact::HwFma>($($arg),*)
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                $generic::<$crate::exact::SoftFma>($($arg),*)
            }
        }
    };
}

#[inline(always)]
fn fma_with<F: FmaOps>(a: f64, b: f64, c: f64) -> f64 {
    F::fma(a, b, c)
}

#[inline(always)]
fn two_prod_with<F: FmaOps>(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    (p, F::fma(a, b, -p))
}

multiversion! {
    /// `a * b + c` rounded once (IEEE-754 fusedMultiplyAdd). The same bits on every target.
    pub fn fma_exact(a: f64, b: f64, c: f64) -> f64 = fma_with;
}

multiversion! {
    /// TwoProduct: `(p, e)` with `p = a * b` rounded and `p + e = a * b` exactly, while
    /// `a * b` neither overflows nor underflows (the error term must be representable).
    pub fn two_prod(a: f64, b: f64) -> (f64, f64) = two_prod_with;
}

/// TwoSum (Knuth): `(s, e)` with `s = a + b` rounded and `s + e = a + b` exactly, for finite
/// inputs whose sum does not overflow. No FMA, no branch.
#[inline(never)]
pub fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    (s, e)
}

/// The software FMA on every target: the same bits as [`fma_exact`]. For tests, and for code
/// that must not depend on the CPU.
#[inline(never)]
pub fn fma_soft(a: f64, b: f64, c: f64) -> f64 {
    SoftFma::fma(a, b, c)
}

// ---------------------------------------------------------------------------------------------
// SoftFma: the libm generic fma, for f64.
//
// SPDX-License-Identifier: MIT
// origin: musl src/math/fma.c. Ported to generic Rust algorithm in 2025, TG.
// Ported for ep9 from libm (rust-lang/compiler-builtins) libm/src/math/generic/fma.rs and
// libm/src/math/generic/scalbn.rs at libm 0.2.16 (dfd2203a4d6110820ad7bb65cafe1bf331a03a3d).
// ep9 changes: monomorphized to f64 (the generic `F` constants are spelled out), no status
// flags, rounding to nearest only. Licence and copyright: THIRD-PARTY-LICENSE ("libm").
// ---------------------------------------------------------------------------------------------

const BITS: i32 = 64;
const SIG_BITS: u32 = 52;
const EXP_BITS: u32 = 11;
const EXP_SAT: u32 = (1 << EXP_BITS) - 1;
const EXP_BIAS: u32 = EXP_SAT >> 1;
const EXP_MAX: i32 = EXP_BIAS as i32;
const EXP_MIN: i32 = -(EXP_MAX - 1);
const SIG_MASK: u64 = (1 << SIG_BITS) - 1;
const IMPLICIT_BIT: u64 = 1 << SIG_BITS;

/// `f64` from its sign, biased exponent and significand bits.
#[inline(always)]
fn from_parts(negative: bool, exponent: u32, significand: u64) -> f64 {
    f64::from_bits((u64::from(negative) << 63) | (u64::from(exponent) << SIG_BITS) | significand)
}

/// The biased exponent field.
#[inline(always)]
fn ex(x: f64) -> u32 {
    (x.to_bits() >> SIG_BITS) as u32 & EXP_SAT
}

/// Fused multiply-add that works when there is not a larger float size available. Computes
/// `(x * y) + z`.
fn soft_fma(x: f64, y: f64, z: f64) -> f64 {
    let one = 1u64;
    let zero = 0u64;

    // Normalize such that the top of the mantissa is zero and we have a guard bit.
    let nx = Norm::from_float(x);
    let ny = Norm::from_float(y);
    let nz = Norm::from_float(z);

    if nx.is_zero_nan_inf() || ny.is_zero_nan_inf() {
        // Value will overflow, defer to non-fused operations.
        return x * y + z;
    }

    if nz.is_zero_nan_inf() {
        if nz.is_zero() {
            // Empty add component means we only need to multiply.
            return x * y;
        }
        // `z` is NaN or infinity, which sets the result.
        return z;
    }

    // multiply: r = x * y
    let zhi: u64;
    let zlo: u64;
    let wide = u128::from(nx.m) * u128::from(ny.m);
    let (mut rlo, mut rhi) = (wide as u64, (wide >> 64) as u64);

    // Exponent result of multiplication
    let mut e: i32 = nx.e + ny.e;
    // Needed shift to align `z` to the multiplication result
    let mut d: i32 = nz.e - e;
    let sbits = BITS;

    // Scale `z`. Shift `z <<= kz`, `r >>= kr`, so `kz+kr == d`, set `e = e+kr` (== ez-kz)
    if d > 0 {
        // The magnitude of `z` is larger than `x * y`
        if d < sbits {
            // Maximum shift of one `F::BITS` means shifted `z` will fit into `2 * F::BITS`. Shift
            // it into `(zhi, zlo)`. No exponent adjustment necessary.
            zlo = nz.m << d;
            zhi = nz.m >> (sbits - d);
        } else {
            // Shift larger than `sbits`, `z` only needs the top half `zhi`. Place it there (acts
            // as a shift by `sbits`).
            zlo = zero;
            zhi = nz.m;
            d -= sbits;

            // `z`'s exponent is large enough that it now needs to be taken into account.
            e = nz.e - sbits;

            if d == 0 {
                // Exactly `sbits`, nothing to do
            } else if d < sbits {
                // Remaining shift fits within `sbits`. Leave `z` in place, shift `x * y`
                rlo = (rhi << (sbits - d)) | (rlo >> d);
                // Set the sticky bit
                rlo |= u64::from((rlo << (sbits - d)) != zero);
                rhi >>= d;
            } else {
                // `z`'s magnitude is enough that `x * y` is irrelevant. It was nonzero, so set
                // the sticky bit.
                rlo = one;
                rhi = zero;
            }
        }
    } else {
        // `z`'s magnitude once shifted fits entirely within `zlo`
        zhi = zero;
        d = -d;
        if d == 0 {
            // No shift needed
            zlo = nz.m;
        } else if d < sbits {
            // Shift s.t. `nz.m` fits into `zlo`
            let sticky = u64::from((nz.m << (sbits - d)) != zero);
            zlo = (nz.m >> d) | sticky;
        } else {
            // Would be entirely shifted out, only set the sticky bit
            zlo = one;
        }
    }

    /* addition */

    let mut neg = nx.neg ^ ny.neg;
    let samesign: bool = !neg ^ nz.neg;
    let mut rhi_nonzero = true;

    if samesign {
        // r += z
        rlo = rlo.wrapping_add(zlo);
        rhi += zhi + u64::from(rlo < zlo);
    } else {
        // r -= z
        let (res, borrow) = rlo.overflowing_sub(zlo);
        rlo = res;
        rhi = rhi.wrapping_sub(zhi.wrapping_add(u64::from(borrow)));
        if (rhi >> (BITS - 1)) != zero {
            rlo = (rlo as i64).wrapping_neg() as u64;
            rhi = (rhi as i64).wrapping_neg() as u64 - u64::from(rlo != zero);
            neg = !neg;
        }
        rhi_nonzero = rhi != zero;
    }

    /* Construct result */

    // Shift result into `rhi`, left-aligned. Last bit is sticky
    if rhi_nonzero {
        // `d` > 0, need to shift both `rhi` and `rlo` into result
        e += sbits;
        d = rhi.leading_zeros() as i32 - 1;
        rhi = (rhi << d) | (rlo >> (sbits - d));
        // Update sticky
        rhi |= u64::from((rlo << d) != zero);
    } else if rlo != zero {
        // `rhi` is zero, `rlo` is the entire result and needs to be shifted
        d = rlo.leading_zeros() as i32 - 1;
        if d < 0 {
            // Shift and set sticky
            rhi = (rlo >> 1) | (rlo & one);
        } else {
            rhi = rlo << d;
        }
    } else {
        // exact +/- 0.0
        return x * y + z;
    }

    e -= d;

    // Use int->float conversion to populate the significand.
    // i is in [1 << (BITS - 2), (1 << (BITS - 1)) - 1]
    let mut i: i64 = rhi as i64;

    if neg {
        i = -i;
    }

    // `|r|` is in `[0x1p62,0x1p63]` for `f64`
    let mut r: f64 = i as f64;

    /* Account for subnormal and rounding */

    // Unbiased exponent for the maximum value of `r`
    let max_pow = BITS as u32 - 1 + EXP_BIAS;

    if e < -(max_pow as i32 - 2) {
        // Result is subnormal before rounding
        if e == -(max_pow as i32 - 1) {
            let mut c = from_parts(false, max_pow, zero);
            if neg {
                c = -c;
            }

            if r == c {
                // Min normal after rounding,
                return f64::MIN_POSITIVE.copysign(r);
            }

            if (rhi << (SIG_BITS + 1)) != zero {
                // Account for truncated bits. One bit will be lost in the `scalbn` call, add
                // another top bit to avoid double rounding if inexact.
                let iu: u64 = (rhi >> 1) | (rhi & one) | (one << (BITS - 2));
                i = iu as i64;

                if neg {
                    i = -i;
                }

                r = i as f64;

                // Remove the top bit
                r = 2.0 * r - c;
            }
        } else {
            // Only round once when scaled
            d = EXP_BITS as i32 - 1;
            let sticky = u64::from(rhi << (BITS - d) != zero);
            i = (((rhi >> d) | sticky) << d) as i64;

            if neg {
                i = -i;
            }

            r = i as f64;
        }
    }

    // Use our exponent to scale the final value.
    scalbn(r, e)
}

/// Representation of `F` that has handled subnormals.
#[derive(Clone, Copy, Debug)]
struct Norm {
    /// Normalized significand with one guard bit, unsigned.
    m: u64,
    /// Exponent of the mantissa such that `m * 2^e = x`. Accounts for the shift in the mantissa
    /// and the guard bit; that is, 1.0 will normalize as `m = 1 << 53` and `e = -53`.
    e: i32,
    neg: bool,
}

impl Norm {
    /// Unbias the exponent and account for the mantissa's precision, including the guard bit.
    const EXP_UNBIAS: u32 = EXP_BIAS + SIG_BITS + 1;

    /// Values greater than this had a saturated exponent (infinity or NaN), OR were zero and we
    /// adjusted the exponent such that it exceeds this threashold.
    const ZERO_INF_NAN: u32 = EXP_SAT - Self::EXP_UNBIAS;

    fn from_float(x: f64) -> Self {
        let mut ix = x.to_bits();
        let mut e = ex(x) as i32;
        let neg = x.is_sign_negative();
        if e == 0 {
            // Normalize subnormals by multiplication
            let scale_i = BITS as u32 - 1;
            let scale_f = from_parts(false, scale_i + EXP_BIAS, 0);
            let scaled = x * scale_f;
            ix = scaled.to_bits();
            e = ex(scaled) as i32;
            e = if e == 0 {
                // If the exponent is still zero, the input was zero. Artifically set this value
                // such that the final `e` will exceed `ZERO_INF_NAN`.
                1 << EXP_BITS
            } else {
                // Otherwise, account for the scaling we just did.
                e - scale_i as i32
            };
        }

        e -= Self::EXP_UNBIAS as i32;

        // Absolute  value, set the implicit bit, and shift to create a guard bit
        ix &= SIG_MASK;
        ix |= IMPLICIT_BIT;
        ix <<= 1;

        Self { m: ix, e, neg }
    }

    /// True if the value was zero, infinity, or NaN.
    fn is_zero_nan_inf(self) -> bool {
        self.e >= Self::ZERO_INF_NAN as i32
    }

    /// The only value we have
    fn is_zero(self) -> bool {
        // The only exponent that strictly exceeds this value is our sentinel value for zero.
        self.e > Self::ZERO_INF_NAN as i32
    }
}

/// Scale the exponent: `x * 2^n`, exact unless the result overflows or underflows (then rounded
/// once). From libm generic/scalbn.rs, for f64.
fn scalbn(mut x: f64, mut n: i32) -> f64 {
    // Bits including the implicit bit
    let sig_total_bits = SIG_BITS + 1;

    // 2 ^ Emax, maximum positive with null significand (0x1p1023 for f64)
    let f_exp_max = from_parts(false, EXP_BIAS << 1, 0);

    // 2 ^ Emin, minimum positive normal with null significand (0x1p-1022 for f64)
    let f_exp_min = from_parts(false, 1, 0);

    // 2 ^ sig_total_bits, moltiplier to normalize subnormals (0x1p53 for f64)
    let f_pow_subnorm = from_parts(false, sig_total_bits + EXP_BIAS, 0);

    /*
     * The goal is to multiply `x` by a scale factor that applies `n`. However, there are cases
     * where `2^n` is not representable by `F` but the result should be, e.g. `x = 2^Emin` with
     * `n = -EMin + 2` (one out of range of 2^Emax). To get around this, reduce the magnitude of
     * the final scale operation by prescaling by the max/min power representable by `F`.
     */

    if n > EXP_MAX {
        // Worse case positive `n`: `x`  is the minimum subnormal value, the result is `F::MAX`.
        // This can be reached by three scaling multiplications (two here and one final).
        x *= f_exp_max;
        n -= EXP_MAX;
        if n > EXP_MAX {
            x *= f_exp_max;
            n -= EXP_MAX;
            if n > EXP_MAX {
                n = EXP_MAX;
            }
        }
    } else if n < EXP_MIN {
        // When scaling toward 0, the prescaling is limited to a value that does not allow `x` to
        // go subnormal. This avoids double rounding.
        // `mul` s.t. `!(x * mul).is_subnormal() ∀ x`
        let mul = f_exp_min * f_pow_subnorm;
        let add = -EXP_MIN - sig_total_bits as i32;

        // Worse case negative `n`: `x`  is the maximum positive value, the result is `F::MIN`.
        // This must be reachable by three scaling multiplications (two here and one final).
        x *= mul;
        n += add;

        if n < EXP_MIN {
            x *= mul;
            n += add;

            if n < EXP_MIN {
                n = EXP_MIN;
            }
        }
    }

    let scale = from_parts(false, (EXP_BIAS as i32 + n) as u32, 0);
    x * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same bits, or both NaN (NaN payloads differ by CPU; JS has one NaN).
    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    type Fma3 = fn(f64, f64, f64) -> f64;

    /// The hardware FMA through the same dispatch as `fma_exact`: `(name, function)`.
    fn hw() -> (&'static str, Fma3) {
        #[cfg(target_arch = "x86_64")]
        {
            #[target_feature(enable = "fma")]
            fn hw_fma(a: f64, b: f64, c: f64) -> f64 {
                HwFma::fma(a, b, c)
            }
            #[allow(unsafe_code)]
            fn checked(a: f64, b: f64, c: f64) -> f64 {
                // SAFETY: `hw()` returns this function only when the CPU has FMA.
                unsafe { hw_fma(a, b, c) }
            }
            if std::is_x86_feature_detected!("fma") {
                return ("HwFma (vfmadd)", checked);
            }
            // No FMA unit: HwFma calls the C `fma` (exact in glibc, macOS and the MSVC CRT).
            ("HwFma (C fma; this CPU has no FMA)", HwFma::fma)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            ("HwFma", HwFma::fma)
        }
    }

    /// SplitMix64 (Steele, Lea, Flood 2014): a fixed, portable input stream.
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A finite double with sign and significand random and the unbiased exponent in
        /// `lo..=hi` (normal range only).
        fn with_exp(&mut self, lo: i32, hi: i32) -> f64 {
            let r = self.next();
            let span = (hi - lo + 1) as u64;
            let e = lo + ((r >> 53) % span) as i32;
            let bits = (r & (1 << 63)) | (((e + 1023) as u64) << 52) | (self.next() & SIG_MASK);
            f64::from_bits(bits)
        }
    }

    /// Edge values: zeros, infinities, NaN, subnormal and normal limits, values next to 1 and to
    /// powers of two.
    fn edges() -> Vec<f64> {
        let mut v = vec![
            0.0,
            1.0,
            2.0,
            0.5,
            3.0,
            f64::INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::EPSILON,
            f64::from_bits(1),                     // min subnormal
            f64::from_bits(SIG_MASK),              // max subnormal
            f64::from_bits(1 << 51),               // 2^-1023
            f64::from_bits(0x3FEF_FFFF_FFFF_FFFF), // 1 - ulp/2
            f64::from_bits(0x3FF0_0000_0000_0001), // 1 + ulp
            f64::from_bits(0x7FEF_FFFF_FFFF_FFFE), // MAX - ulp
            f64::from_bits(0x0010_0000_0000_0001), // MIN_POSITIVE + ulp
            1e300,
            1e-300,
            0.1,
            1.0 / 3.0,
            9_007_199_254_740_994.0,               // 2^53 + 2
            f64::from_bits(0x10),                  // 2^-1070
            f64::from_bits(0x000F_FFFF_FFFF_FFF8), // 0x1.ffffffffffffp-1023
        ];
        let neg: Vec<f64> = v.iter().map(|x| -x).collect();
        v.extend(neg);
        v
    }

    /// Runs `f(a, b, c)` on edge triples and on `n` random triples from several input shapes.
    fn corpus(n: u64, mut f: impl FnMut(f64, f64, f64)) {
        let e = edges();
        for &a in &e {
            for &b in &e {
                for &c in &e {
                    f(a, b, c);
                }
            }
        }
        let mut r = SplitMix64(0x0EF9_4A7E_0000_0001);
        for i in 0..n {
            let (a, b, c) = match i % 8 {
                // Any bit pattern: NaN, infinities, subnormals, huge exponent gaps.
                0 => (
                    f64::from_bits(r.next()),
                    f64::from_bits(r.next()),
                    f64::from_bits(r.next()),
                ),
                // Ordinary magnitudes, close exponents (carries across the product halves).
                1 | 2 => (
                    r.with_exp(-40, 40),
                    r.with_exp(-40, 40),
                    r.with_exp(-80, 80),
                ),
                // Exact or near cancellation: c = -(a*b) rounded, nudged by a few ulps.
                3 | 4 => {
                    let (a, b) = (r.with_exp(-200, 200), r.with_exp(-200, 200));
                    let p = -(a * b);
                    let nudge = (r.next() % 5) as i64 - 2;
                    (a, b, f64::from_bits((p.to_bits() as i64 + nudge) as u64))
                }
                // Results near or below the subnormal limit.
                5 => (
                    r.with_exp(-540, -500),
                    r.with_exp(-560, -510),
                    r.with_exp(-1022, -1000),
                ),
                // Results near overflow.
                6 => (
                    r.with_exp(500, 520),
                    r.with_exp(500, 520),
                    r.with_exp(1000, 1023),
                ),
                // z far from x*y in either direction (sticky-bit paths).
                _ => (
                    r.with_exp(-30, 30),
                    r.with_exp(-30, 30),
                    r.with_exp(-200, 200),
                ),
            };
            f(a, b, c);
        }
    }

    fn soft_vs_hw_run(n: u64) {
        let (name, hw) = hw();
        let mut mismatches = 0u64;
        let mut total = 0u64;
        corpus(n, |a, b, c| {
            total += 1;
            let (s, h) = (fma_soft(a, b, c), hw(a, b, c));
            if !same(s, h) {
                mismatches += 1;
                if mismatches <= 10 {
                    eprintln!(
                        "mismatch: fma({a:e} [{:#018x}], {b:e} [{:#018x}], {c:e} [{:#018x}]): soft {s:e} [{:#018x}], {name} {h:e} [{:#018x}]",
                        a.to_bits(),
                        b.to_bits(),
                        c.to_bits(),
                        s.to_bits(),
                        h.to_bits()
                    );
                }
            }
            // The dispatched entry point is one of the two.
            assert!(same(fma_exact(a, b, c), s));
        });
        eprintln!("soft_vs_hw: {total} inputs, {mismatches} mismatches, hardware = {name}");
        assert_eq!(mismatches, 0);
    }

    /// SoftFma equals the hardware FMA bit for bit (NaN: both NaN). The acceptance run:
    /// `cargo test -p ep9_math --release soft_vs_hw -- --ignored` (10^7 random inputs plus the
    /// edge triples).
    #[test]
    #[ignore = "10^7 inputs; run with --release -- --ignored"]
    fn soft_vs_hw() {
        soft_vs_hw_run(10_000_000);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn soft_vs_hw_quick() {
        soft_vs_hw_run(100_000);
    }

    /// Vectors from the libm 0.2.16 fma tests (src/math/fma.rs) and IEEE-754 zero signs.
    #[test]
    fn soft_fma_known_values() {
        let tiny = f64::from_bits(0x10);
        let sub = f64::from_bits(0x000F_FFFF_FFFF_FFF8);
        let cases: &[(f64, f64, f64, f64)] = &[
            // 0x1p-1070 * 0x1p-1070 + 0x1.ffffffffffffp-1023 = 0x0.ffffffffffff8p-1022
            (tiny, tiny, sub, sub),
            // 0x1p-1070 * 0x1p-1070 - 0x1p-1022 = -0x1p-1022
            (tiny, tiny, -f64::MIN_POSITIVE, -f64::MIN_POSITIVE),
            (
                -0.0000000000000002220446049250313,
                -0.0000000000000002220446049250313,
                -0.0000000000000002220446049250313,
                -0.00000000000000022204460492503126,
            ),
            (-0.992, -0.992, -0.992, -0.007936000000000007),
            (
                -(1.0 - f64::EPSILON),
                f64::MIN,
                f64::MIN,
                -3991680619069439e277,
            ),
            (
                1.1102230246251565e-16,
                -9.812526705433188e-305,
                1.0894e-320,
                0.0,
            ),
            // 0.1 * 10 - 1 is the rounding error of 0.1, not 0 (no double rounding).
            (0.1, 10.0, -1.0, 5.551115123125783e-17),
        ];
        for &(a, b, c, want) in cases {
            assert_eq!(
                fma_soft(a, b, c).to_bits(),
                want.to_bits(),
                "fma({a:e}, {b:e}, {c:e})"
            );
            assert_eq!(
                fma_exact(a, b, c).to_bits(),
                want.to_bits(),
                "fma({a:e}, {b:e}, {c:e})"
            );
        }
        // "When the exact result of (a × b) + c is non-zero yet the result of fusedMultiplyAdd
        // is zero because of rounding, the zero result takes the sign of the exact result."
        let t = f64::from_bits(1);
        for (x, y, sign) in [(t, t, 0), (t, -t, 1), (-t, t, 1), (-t, -t, 0)] {
            assert_eq!(fma_soft(x, y, 0.0).to_bits(), sign << 63);
        }
        // An exact zero sum is +0 (round to nearest), -0 only for (-0) + (-0).
        assert_eq!(fma_soft(2.0, 3.0, -6.0).to_bits(), 0);
        assert_eq!(fma_soft(-0.0, 1.0, -0.0).to_bits(), 1 << 63);
        assert!(fma_soft(f64::INFINITY, 0.0, 1.0).is_nan());
        assert!(fma_soft(f64::INFINITY, 1.0, f64::NEG_INFINITY).is_nan());
        // 2 * MAX - MAX is exact: no overflow inside an FMA.
        assert_eq!(fma_soft(f64::MAX, 2.0, -f64::MAX), f64::MAX);
    }

    /// `v` as `m * 2^e` with an odd or zero integer `m` (finite `v` only).
    fn decompose(v: f64) -> (i128, i32) {
        if v == 0.0 {
            return (0, 0);
        }
        let bits = v.to_bits();
        let be = ((bits >> 52) & 0x7ff) as i32;
        let (mut m, mut e) = if be == 0 {
            ((bits & SIG_MASK) as i128, -1074)
        } else {
            (((bits & SIG_MASK) | IMPLICIT_BIT) as i128, be - 1075)
        };
        while m & 1 == 0 {
            m >>= 1;
            e += 1;
        }
        (if v < 0.0 { -m } else { m }, e)
    }

    /// `m * 2^(e - base)` as an integer (`e >= base`, small shifts only).
    fn scaled(m: i128, e: i32, base: i32) -> i128 {
        m << (e - base)
    }

    /// TwoProduct and TwoSum are error-free: checked in exact integer arithmetic.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn two_prod_and_two_sum_are_exact() {
        let mut r = SplitMix64(7);
        for _ in 0..200_000 {
            let (a, b) = (r.with_exp(-300, 300), r.with_exp(-300, 300));
            let (p, e) = two_prod(a, b);
            assert_eq!(p, a * b);
            let ((ma, ea), (mb, eb)) = (decompose(a), decompose(b));
            let ((mp, ep), (me, ee)) = (decompose(p), decompose(e));
            let base = ea + eb;
            let exact = ma * mb;
            let got = scaled(mp, ep, base) + if me == 0 { 0 } else { scaled(me, ee, base) };
            assert_eq!(got, exact, "two_prod({a:e}, {b:e})");

            let (a, b) = (r.with_exp(-20, 20), r.with_exp(-20, 20));
            let (s, e) = two_sum(a, b);
            assert_eq!(s, a + b);
            let ((ma, ea), (mb, eb)) = (decompose(a), decompose(b));
            let ((ms, es), (me, ee)) = (decompose(s), decompose(e));
            let base = ea.min(eb).min(if me == 0 { ea } else { ee });
            let exact = scaled(ma, ea, base) + scaled(mb, eb, base);
            let got = (if ms == 0 { 0 } else { scaled(ms, es, base) })
                + if me == 0 { 0 } else { scaled(me, ee, base) };
            assert_eq!(got, exact, "two_sum({a:e}, {b:e})");
        }
    }

    #[test]
    fn scalbn_limits() {
        assert_eq!(scalbn(1.0, 1023), f64::from_bits(0x7FE0_0000_0000_0000));
        assert_eq!(scalbn(1.0, 1024), f64::INFINITY);
        assert_eq!(
            scalbn(f64::from_bits(1), 2097),
            f64::from_bits(0x7FE0_0000_0000_0000)
        );
        assert_eq!(scalbn(1.0, -1074), f64::from_bits(1));
        assert_eq!(scalbn(1.0, -1075), 0.0);
        assert_eq!(scalbn(1.5, -1074), f64::from_bits(2)); // ties to even
        assert_eq!(scalbn(f64::MAX, -2097), f64::from_bits(2)); // (2 - 2^-52) * 2^-1074, rounded once
        assert_eq!(scalbn(-0.0, 5).to_bits(), (-0.0f64).to_bits());
    }
}
