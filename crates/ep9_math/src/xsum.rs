//! `Math.sumPrecise` (ECMA-262, sec-math.sumprecise; H36 row `sumPrecise`): the exact sum of a
//! list of Numbers, rounded once (ties to even). [`ExactSum`] keeps the spec's state (minus-zero,
//! finite, plus-infinity, minus-infinity, not-a-number) and, for the finite values, Radford M.
//! Neal's "small superaccumulator" (xsum): 67 signed 64-bit chunks of 32 bits each, which hold
//! every double exactly. Integer operations only: no FP rounding happens before the one in
//! [`ExactSum::sum`], so the result is the same on every target (H36).
//!
//! Ported for ep9 from V8 `src/builtins/builtins-math-xsum.h` and
//! `src/builtins/builtins-math-xsum.cc` (main at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0,
//! 2026-11-02; the files last changed in b76957af9c52bc8566a34dbd870637bbb5eed25a), which V8
//! adapted from xsum.c and xsum.h by Radford M. Neal (<https://gitlab.com/radfordneal/xsum>):
//!
//! ```text
//! Copyright 2026 the V8 project authors. All rights reserved.
//! Use of this source code is governed by a BSD-style license that can be
//! found in the LICENSE file.
//!
//! Adapted from xsum.c and xsum.h by Radford M. Neal.
//! https://gitlab.com/radfordneal/xsum
//!
//! Original license header:
//!
//! Copyright 2015, 2018, 2021 Radford M. Neal
//!
//! Permission is hereby granted, free of charge, to any person obtaining
//! a copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so, subject to
//! the following conditions:
//!
//! The above copyright notice and this permission notice shall be
//! included in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
//! EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
//! LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
//! OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
//! WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//! ```
//!
//! Licence text of V8: `THIRD-PARTY-LICENSE` ("V8"). ep9 changes:
//! - The spec state machine (sumPrecise steps 7.b.vi and 8-12) replaces V8's `inf_`, `nan_`,
//!   `minus_zero_` fields and `AddInfNan`, so only finite values that are not -0 reach the
//!   chunks. V8's `only_ints` fast path (in `MathSumPrecise`) is not ported: it gives the same
//!   result.
//! - `CarryPropagate` finds the top and the bottom non-zero chunk with plain loops (V8 unrolls
//!   them in groups of 4); the result is the same.
//! - A carry out of the top chunk sets `overflow`, and [`ExactSum::sum`] then gives NaN, where
//!   V8 adds a NaN through `AddInfNan` (the same result). It needs more than 2^45 values near
//!   the top of the double range, so no real program reaches it.

/// Bits of the fraction field of a double.
const MANTISSA_BITS: u32 = 52;
/// Bits of the exponent field of a double.
const EXP_BITS: u32 = 11;
const MANTISSA_MASK: i64 = (1 << MANTISSA_BITS) - 1;
const EXP_MASK: i64 = (1 << EXP_BITS) - 1;
const EXP_BIAS: i64 = (1 << (EXP_BITS - 1)) - 1;
const SIGN_MASK: u64 = 1 << 63;

/// The low 5 bits of the exponent pick the bit position in a chunk; the high 6 bits pick the
/// chunk.
const LOW_EXP_BITS: u32 = 5;
const LOW_EXP_MASK: i64 = (1 << LOW_EXP_BITS) - 1;
const HIGH_EXP_BITS: u32 = EXP_BITS - LOW_EXP_BITS;
/// The number of chunks: 64 for the exponents, plus 3 for carries.
const SCHUNKS: usize = (1 << HIGH_EXP_BITS) + 3;

/// Each chunk holds 32 bits of the sum when the carries are propagated.
const LOW_MANTISSA_BITS: u32 = 1 << LOW_EXP_BITS;
const LOW_MANTISSA_MASK: i64 = (1 << LOW_MANTISSA_BITS) - 1;

/// One value adds less than 2^53 to a chunk, so 2^11 - 1 values fit in the 63 bits of a chunk
/// before the carries must move up.
const SMALL_CARRY_BITS: u32 = (64 - 1) - MANTISSA_BITS;
const SMALL_CARRY_TERMS: u32 = (1 << SMALL_CARRY_BITS) - 1;

/// The `state` of Math.sumPrecise (steps 3 and 7.b.vi).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    MinusZero,
    Finite,
    PlusInfinity,
    MinusInfinity,
    NotANumber,
}

/// The running sum of Math.sumPrecise: [`ExactSum::add`] each Number in order, then
/// [`ExactSum::sum`]. The type checks, the count limit and the iterator are the caller's
/// (steps 1-2 and 7.a-7.b.v).
#[derive(Clone, Debug)]
pub struct ExactSum {
    /// The superaccumulator: chunk `k`, bit `b` has the weight 2^(32k + b - 1075).
    chunk: [i64; SCHUNKS],
    /// Adds left before the carries must move up.
    adds_until_propagate: u32,
    state: State,
    /// A carry left the top chunk (see the module doc).
    overflow: bool,
}

impl Default for ExactSum {
    fn default() -> Self {
        Self::new()
    }
}

impl ExactSum {
    /// The empty sum (state minus-zero; its [`ExactSum::sum`] is -0).
    pub const fn new() -> Self {
        ExactSum {
            chunk: [0; SCHUNKS],
            adds_until_propagate: SMALL_CARRY_TERMS,
            state: State::MinusZero,
            overflow: false,
        }
    }

    /// Math.sumPrecise step 7.b.vi for the Number `n`.
    #[inline(never)]
    pub fn add(&mut self, n: f64) {
        // vi. If state is not not-a-number, then
        if self.state == State::NotANumber {
            return;
        }
        if n.is_nan() {
            // 1. If n is NaN, set state to not-a-number.
            self.state = State::NotANumber;
        } else if n == f64::INFINITY {
            // 2. Else if n is +∞𝔽: not-a-number after minus-infinity, else plus-infinity.
            self.state = if self.state == State::MinusInfinity {
                State::NotANumber
            } else {
                State::PlusInfinity
            };
        } else if n == f64::NEG_INFINITY {
            // 3. Else if n is -∞𝔽: not-a-number after plus-infinity, else minus-infinity.
            self.state = if self.state == State::PlusInfinity {
                State::NotANumber
            } else {
                State::MinusInfinity
            };
        } else if n.to_bits() != SIGN_MASK && matches!(self.state, State::MinusZero | State::Finite)
        {
            // 4. Else if n is not -0𝔽 and state is either minus-zero or finite, then
            //    a. Set state to finite.
            //    b. Set sum to sum + ℝ(n).
            self.state = State::Finite;
            if self.adds_until_propagate == 0 {
                self.carry_propagate();
            }
            self.add1_no_carry(n);
            self.adds_until_propagate -= 1;
        }
    }

    /// Math.sumPrecise steps 8-12: NaN, ±∞, -0, or 𝔽(sum) (the exact sum rounded to the
    /// nearest double, ties to even; ±∞ when it is too large).
    #[inline(never)]
    pub fn sum(&mut self) -> f64 {
        match self.state {
            // 8. If state is not-a-number, return NaN.
            State::NotANumber => f64::NAN,
            // 9. If state is plus-infinity, return +∞𝔽.
            State::PlusInfinity => f64::INFINITY,
            // 10. If state is minus-infinity, return -∞𝔽.
            State::MinusInfinity => f64::NEG_INFINITY,
            // 11. If state is minus-zero, return -0𝔽.
            State::MinusZero => -0.0,
            // 12. Return 𝔽(sum).
            State::Finite => self.round(),
        }
    }

    /// V8 `Xsum::Add1NoCarry`: adds the finite double `value` (not -0) to the chunks. The
    /// 53-bit significand, shifted by the low 5 bits of the exponent, spans two chunks.
    fn add1_no_carry(&mut self, value: f64) {
        let ivalue = value.to_bits() as i64;
        let exp = (ivalue >> MANTISSA_BITS) & EXP_MASK;
        let mut mantissa = ivalue & MANTISSA_MASK;
        let high_exp = (exp >> LOW_EXP_BITS) as usize;
        let mut low_exp = exp & LOW_EXP_MASK;
        if exp == 0 {
            // Zero or subnormal: the weight of exponent field 1, without the hidden bit.
            if mantissa == 0 {
                return;
            }
            low_exp = 1;
        } else {
            // Normal: the hidden bit.
            mantissa |= 1 << MANTISSA_BITS;
        }
        let low = (((mantissa as u64) << low_exp) as i64) & LOW_MANTISSA_MASK;
        let high = mantissa >> (i64::from(LOW_MANTISSA_BITS) - low_exp);
        if ivalue < 0 {
            self.chunk[high_exp] -= low;
            self.chunk[high_exp + 1] -= high;
        } else {
            self.chunk[high_exp] += low;
            self.chunk[high_exp + 1] += high;
        }
    }

    /// V8 `Xsum::CarryPropagate`: moves the bits above 32 of each chunk into the next chunk, so
    /// every chunk below the top one is in 0..2^32. Returns the index of the top non-zero chunk
    /// (0 for a zero sum). The top chunk keeps the sign; a top chunk of -1 is merged into the
    /// chunk below it.
    fn carry_propagate(&mut self) -> usize {
        self.adds_until_propagate = SMALL_CARRY_TERMS - 1;
        // The uppermost non-zero chunk.
        let mut u = SCHUNKS - 1;
        while self.chunk[u] == 0 {
            if u == 0 {
                // The sum is zero.
                return 0;
            }
            u -= 1;
        }
        // The lowest non-zero chunk (one exists: chunk u).
        let mut i = 0;
        while self.chunk[i] == 0 {
            i += 1;
        }
        // The index of the uppermost non-zero chunk found so far.
        let mut uix: Option<usize> = None;
        loop {
            let mut c = self.chunk[i];
            if c == 0 {
                // The next non-zero chunk; u is an upper bound, but that chunk may be 0 now.
                i += 1;
                while i <= u && self.chunk[i] == 0 {
                    i += 1;
                }
                if i > u {
                    break;
                }
                c = self.chunk[i];
            }
            let chigh = c >> LOW_MANTISSA_BITS;
            if chigh == 0 {
                uix = Some(i);
                i += 1;
                if i > u {
                    break;
                }
                continue;
            }
            if u == i {
                if chigh == -1 {
                    // Do not move -1 into the zero chunks above.
                    uix = Some(i);
                    break;
                }
                u = i + 1;
            }
            let clow = c & LOW_MANTISSA_MASK;
            if clow != 0 {
                uix = Some(i);
            }
            self.chunk[i] = clow;
            if i + 1 >= SCHUNKS {
                self.overflow = true;
                u = i;
            } else {
                self.chunk[i + 1] += chigh;
            }
            i += 1;
            if i > u {
                break;
            }
        }
        let Some(mut uix) = uix else {
            return 0;
        };
        // While the uppermost chunk is -1, combine it with the chunk below.
        while self.chunk[uix] == -1 && uix > 0 {
            self.chunk[uix - 1] -= 1 << LOW_MANTISSA_BITS;
            self.chunk[uix] = 0;
            uix -= 1;
        }
        uix
    }

    /// V8 `Xsum::Round` (finite part): the chunks rounded to the nearest double, ties to even.
    /// It takes the top 55 bits (53 plus a half bit and one more), then looks at the lower bits
    /// only for a tie.
    fn round(&mut self) -> f64 {
        let i = self.carry_propagate();
        if self.overflow {
            return f64::NAN;
        }
        let mut ivalue = self.chunk[i];

        // A possible subnormal result, zero included.
        if i <= 1 {
            if ivalue == 0 {
                // An exact zero sum of finite values is +0 (𝔽(0)).
                return 0.0;
            }
            if i == 0 {
                let mut intv = ivalue.unsigned_abs() >> 1;
                if ivalue < 0 {
                    intv |= SIGN_MASK;
                }
                return f64::from_bits(intv);
            }
            let intv = ivalue * (1 << (LOW_MANTISSA_BITS - 1)) + (self.chunk[0] >> 1);
            if intv < 0 {
                if intv > -(1 << MANTISSA_BITS) {
                    return f64::from_bits((-intv) as u64 | SIGN_MASK);
                }
            } else if (intv as u64) < (1 << MANTISSA_BITS) {
                return f64::from_bits(intv as u64);
            }
        }

        // |ivalue| < 2^33, so the conversion is exact; its exponent places the top bit.
        let fltv = ivalue as f64;
        let mut e = ((fltv.to_bits() >> MANTISSA_BITS) as i64) & EXP_MASK;
        let mut more = 2 + i64::from(MANTISSA_BITS) + EXP_BIAS - e;

        // ivalue now has 55 significant bits: 53, the half bit, and one more bit.
        ivalue *= 1 << more;
        let mut j = i as isize - 1;
        let mut lower = self.chunk[j as usize];
        if more >= i64::from(LOW_MANTISSA_BITS) {
            more -= i64::from(LOW_MANTISSA_BITS);
            ivalue += lower << more;
            j -= 1;
            lower = if j < 0 { 0 } else { self.chunk[j as usize] };
        }
        ivalue += lower >> (i64::from(LOW_MANTISSA_BITS) - more);
        lower &= (1 << (i64::from(LOW_MANTISSA_BITS) - more)) - 1;

        let mut intv: u64;
        let mut round_away = false;
        if ivalue >= 0 {
            intv = 0;
            if ivalue & 2 != 0 {
                if ivalue & 1 != 0 || ivalue & 4 != 0 {
                    // Above the midpoint, or on it with an odd last bit.
                    round_away = true;
                } else {
                    // On the midpoint as far as ivalue goes: any lower bit is above it.
                    if lower == 0 {
                        while j > 0 {
                            j -= 1;
                            if self.chunk[j as usize] != 0 {
                                lower = 1;
                                break;
                            }
                        }
                    }
                    if lower != 0 {
                        round_away = true;
                    }
                }
            }
        } else {
            // A negative sum: the lower bits (positive) make its magnitude smaller.
            if (-ivalue) & (1 << (MANTISSA_BITS + 2)) == 0 {
                // One bit short of 55 significant bits: take one more from `lower`.
                let pos = 1i64 << (i64::from(LOW_MANTISSA_BITS) - 1 - more);
                ivalue *= 2;
                if lower & pos != 0 {
                    ivalue += 1;
                    lower &= !pos;
                }
                e -= 1;
            }
            intv = SIGN_MASK;
            ivalue = -ivalue;
            if ivalue & 3 == 3 {
                round_away = true;
            } else if ivalue & 3 > 1 && ivalue & 4 != 0 {
                // On the midpoint as far as ivalue goes, with an odd last bit: a tie only if
                // no lower bit is set (a lower bit puts the magnitude below the midpoint).
                if lower == 0 {
                    while j > 0 {
                        j -= 1;
                        if self.chunk[j as usize] != 0 {
                            lower = 1;
                            break;
                        }
                    }
                }
                if lower == 0 {
                    round_away = true;
                }
            }
        }

        if round_away {
            ivalue += 4;
            if ivalue & (1 << (MANTISSA_BITS + 3)) != 0 {
                ivalue >>= 1;
                e += 1;
            }
        }
        ivalue >>= 2;
        e += ((i as i64) << LOW_EXP_BITS) - EXP_BIAS - i64::from(MANTISSA_BITS);

        if e >= EXP_MASK {
            // Too large: ±∞.
            intv |= (EXP_MASK as u64) << MANTISSA_BITS;
            return f64::from_bits(intv);
        }
        intv += (e as u64) << MANTISSA_BITS;
        intv += (ivalue & MANTISSA_MASK) as u64;
        f64::from_bits(intv)
    }
}

/// `Math.sumPrecise` of the Numbers `values` in order (steps 3-12): [`ExactSum::add`] each one,
/// then [`ExactSum::sum`].
#[inline(never)]
pub fn sum_precise(values: &[f64]) -> f64 {
    let mut s = ExactSum::new();
    for &v in values {
        s.add(v);
    }
    s.sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    /// An independent exact sum: a two's-complement integer in units of 2^-1074 (35 limbs of
    /// 64 bits: the double range needs 2098 bits, and the rest is room for carries), rounded to
    /// the nearest double, ties to even, by plain bit steps.
    struct Reference {
        limbs: [u64; 35],
    }

    impl Reference {
        fn new() -> Self {
            Reference { limbs: [0; 35] }
        }

        /// Adds (or subtracts) the integer `m * 2^shift`.
        fn add_shifted(&mut self, m: u64, shift: u32, negative: bool) {
            let mut v = [0u64; 35];
            let w = (shift / 64) as usize;
            let b = shift % 64;
            v[w] = m << b;
            if b != 0 {
                v[w + 1] = m >> (64 - b);
            }
            if negative {
                // v = -v: invert and add 1.
                let mut carry = 1u64;
                for x in v.iter_mut() {
                    let (s, c) = (!*x).overflowing_add(carry);
                    *x = s;
                    carry = u64::from(c);
                }
            }
            let mut carry = 0u64;
            for (a, &x) in self.limbs.iter_mut().zip(v.iter()) {
                let (s1, c1) = a.overflowing_add(x);
                let (s2, c2) = s1.overflowing_add(carry);
                *a = s2;
                carry = u64::from(c1) + u64::from(c2);
            }
        }

        fn add(&mut self, x: f64) {
            let b = x.to_bits();
            let e = ((b >> 52) & 0x7ff) as u32;
            let f = b & ((1 << 52) - 1);
            let negative = b >> 63 == 1;
            if e == 0 {
                self.add_shifted(f, 0, negative);
            } else {
                self.add_shifted(f | (1 << 52), e - 1, negative);
            }
        }

        fn bit(mag: &[u64; 35], k: u32) -> bool {
            (mag[(k / 64) as usize] >> (k % 64)) & 1 == 1
        }

        fn round(&self) -> f64 {
            let negative = self.limbs[34] >> 63 == 1;
            let mut mag = self.limbs;
            if negative {
                let mut carry = 1u64;
                for x in mag.iter_mut() {
                    let (s, c) = (!*x).overflowing_add(carry);
                    *x = s;
                    carry = u64::from(c);
                }
            }
            let sign = if negative { SIGN_MASK } else { 0 };
            // p: the position of the top set bit.
            let Some(p) = (0..35 * 64).rev().find(|&k| Self::bit(&mag, k)) else {
                return 0.0;
            };
            if p < 53 {
                // Exact: a subnormal, or a normal with exponent field 1.
                return f64::from_bits(sign | mag[0]);
            }
            let s = p - 52;
            let mut m = 0u64;
            for k in (s..=p).rev() {
                m = (m << 1) | u64::from(Self::bit(&mag, k));
            }
            let half = Self::bit(&mag, s - 1);
            let sticky = (0..s - 1).any(|k| Self::bit(&mag, k));
            let mut biased = u64::from(s) + 1;
            if half && (sticky || m & 1 == 1) {
                m += 1;
                if m == 1 << 53 {
                    m >>= 1;
                    biased += 1;
                }
            }
            if biased >= 2047 {
                return f64::from_bits(sign | (0x7ff << 52));
            }
            f64::from_bits(sign | (biased << 52) | (m & ((1 << 52) - 1)))
        }
    }

    fn reference(values: &[f64]) -> f64 {
        let mut r = Reference::new();
        for &v in values {
            r.add(v);
        }
        r.round()
    }

    /// SplitMix64.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        /// A finite double with a random sign and a biased exponent field in `lo..=hi`.
        fn finite(&mut self, lo: u64, hi: u64) -> f64 {
            let e = lo + self.below(hi - lo + 1);
            let r = self.next();
            f64::from_bits((r & SIGN_MASK) | (e << 52) | (r & ((1 << 52) - 1)))
        }

        /// One list element: wide, narrow, near the top, tiny, or the negation of an earlier one.
        fn element(&mut self, earlier: &[f64]) -> f64 {
            match self.below(8) {
                0 => self.finite(0, 2046),
                1 | 2 => self.finite(1000, 1100),
                3 => self.finite(2030, 2046),
                4 => self.finite(0, 40),
                5 if !earlier.is_empty() => {
                    let x = earlier[self.below(earlier.len() as u64) as usize];
                    // -x, moved by a few ulps.
                    let d = self.below(9) as i64 - 4;
                    let y = -f64::from_bits((x.to_bits() as i64).wrapping_add(d) as u64);
                    // (MAX moved up is ∞ or NaN: keep the lists finite.)
                    if y.is_finite() { y } else { -x }
                }
                6 => (self.below(2001) as f64) - 1000.0,
                _ => self.finite(1, 2046),
            }
        }
    }

    #[test]
    fn the_test262_cases() {
        // test/built-ins/Math/sumPrecise/sum.js
        let cases: &[(&[f64], f64)] = &[
            (&[1.0, 2.0, 3.0], 6.0),
            (&[1e308], 1e308),
            (&[1e308, -1e308], 0.0),
            (&[0.1], 0.1),
            (&[0.1, 0.1], 0.2),
            (&[0.1, -0.1], 0.0),
            (
                &[1e308, 1e308, 0.1, 0.1, 1e30, 0.1, -1e30, -1e308, -1e308],
                0.30000000000000004,
            ),
            (&[1e30, 0.1, -1e30], 0.1),
            (
                &[
                    8.98846567431158e+307,
                    8.988465674311579e+307,
                    -1.7976931348623157e+308,
                ],
                9.9792015476736e+291,
            ),
            (
                &[
                    -5.630637621603525e+255,
                    9.565271205476345e+307,
                    2.9937604643020797e+292,
                ],
                9.565271205476347e+307,
            ),
            (
                &[
                    6.739986666787661e+66,
                    2.0,
                    -1.2689709186578243e-116,
                    1.7046015739467354e+308,
                    -9.979201547673601e+291,
                    6.160926733208294e+307,
                    -3.179557053031852e+234,
                    -7.027282978772846e+307,
                    -0.7500000000000001,
                ],
                1.61796594939028e+308,
            ),
            (
                &[
                    0.31150493246968836,
                    -8.988465674311582e+307,
                    1.8315037361673755e-270,
                    -15.999999999999996,
                    2.9999999999999996,
                    7.345200721499384e+164,
                    -2.033582473639399,
                    -8.98846567431158e+307,
                    -3.5737295155405993e+292,
                    4.13894772383715e-124,
                    -3.6111186457260667e-35,
                    2.387234887098013e+180,
                    7.645295562778372e-298,
                    3.395189016861822e-103,
                    -2.6331611115768973e-149,
                ],
                f64::NEG_INFINITY,
            ),
            (
                &[
                    -1.1442589134409902e+308,
                    9.593842098384855e+138,
                    4.494232837155791e+307,
                    -1.3482698511467367e+308,
                    4.494232837155792e+307,
                ],
                -1.5936821971565685e+308,
            ),
            (
                &[
                    -1.1442589134409902e+308,
                    4.494232837155791e+307,
                    -1.3482698511467367e+308,
                    4.494232837155792e+307,
                ],
                -1.5936821971565687e+308,
            ),
            (
                &[
                    9.593842098384855e+138,
                    -6.948356297254111e+307,
                    -1.3482698511467367e+308,
                    4.494232837155792e+307,
                ],
                -1.5936821971565685e+308,
            ),
            (
                &[
                    -2.534858246857893e+115,
                    8.988465674311579e+307,
                    8.98846567431158e+307,
                ],
                1.7976931348623157e+308,
            ),
            (
                &[
                    1.3588124894186193e+308,
                    1.4803986201152006e+223,
                    6.741349255733684e+307,
                ],
                f64::INFINITY,
            ),
            (
                &[
                    6.741349255733684e+307,
                    1.7976931348623155e+308,
                    -7.388327292663961e+41,
                ],
                f64::INFINITY,
            ),
            (
                &[
                    -1.9807040628566093e+28,
                    1.7976931348623157e+308,
                    9.9792015476736e+291,
                ],
                1.7976931348623157e+308,
            ),
            (
                &[
                    -1.0214557991173964e+61,
                    1.7976931348623157e+308,
                    8.98846567431158e+307,
                    -8.988465674311579e+307,
                ],
                1.7976931348623157e+308,
            ),
            (
                &[
                    1.7976931348623157e+308,
                    7.999999999999999,
                    -1.908963895403937e-230,
                    1.6445950082320264e+292,
                    2.0734856707605806e+205,
                ],
                f64::INFINITY,
            ),
            (
                &[
                    6.197409167220438e-223,
                    -9.979201547673601e+291,
                    -1.7976931348623157e+308,
                ],
                f64::NEG_INFINITY,
            ),
            (
                &[
                    4.49423283715579e+307,
                    8.944251746776101e+307,
                    -0.0002441406250000001,
                    1.1752060710043817e+308,
                    4.940846717201632e+292,
                    -1.6836699406454528e+308,
                ],
                8.353845887521184e+307,
            ),
            (
                &[
                    8.988465674311579e+307,
                    7.999999999999998,
                    7.029158107234023e-308,
                    -2.2303483759420562e-172,
                    -1.7976931348623157e+308,
                    -8.98846567431158e+307,
                ],
                -1.7976931348623157e+308,
            ),
            (
                &[8.98846567431158e+307, 8.98846567431158e+307],
                f64::INFINITY,
            ),
        ];
        for (values, want) in cases {
            assert_eq!(
                bits(sum_precise(values)),
                bits(*want),
                "sumPrecise({values:?})"
            );
            assert_eq!(
                bits(reference(values)),
                bits(*want),
                "reference({values:?})"
            );
        }
    }

    #[test]
    fn the_states() {
        // sum-is-minus-zero.js, sum-is-NaN.js, sum-is-infinite.js
        assert_eq!(bits(sum_precise(&[])), bits(-0.0));
        assert_eq!(bits(sum_precise(&[-0.0])), bits(-0.0));
        assert_eq!(bits(sum_precise(&[-0.0, -0.0])), bits(-0.0));
        assert_eq!(bits(sum_precise(&[-0.0, 0.0])), bits(0.0));
        assert_eq!(bits(sum_precise(&[0.0, -0.0])), bits(0.0));
        assert_eq!(bits(sum_precise(&[1.0, -1.0])), bits(0.0));
        assert_eq!(bits(sum_precise(&[-1.0, 1.0, -0.0])), bits(0.0));
        assert!(sum_precise(&[f64::NAN]).is_nan());
        assert!(sum_precise(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
        assert!(sum_precise(&[f64::NEG_INFINITY, f64::INFINITY]).is_nan());
        assert!(sum_precise(&[f64::NAN, f64::INFINITY]).is_nan());
        assert!(sum_precise(&[f64::INFINITY, 1.0, f64::NAN]).is_nan());
        assert_eq!(sum_precise(&[f64::INFINITY]), f64::INFINITY);
        assert_eq!(sum_precise(&[f64::INFINITY, f64::INFINITY]), f64::INFINITY);
        assert_eq!(sum_precise(&[f64::NEG_INFINITY]), f64::NEG_INFINITY);
        assert_eq!(
            sum_precise(&[f64::NEG_INFINITY, f64::NEG_INFINITY]),
            f64::NEG_INFINITY
        );
        // An infinity wins over any finite value, also over a finite sum that overflows.
        assert_eq!(
            sum_precise(&[f64::MAX, f64::MAX, f64::NEG_INFINITY]),
            f64::NEG_INFINITY
        );
        assert_eq!(sum_precise(&[-0.0, f64::INFINITY, -0.0]), f64::INFINITY);
        // The finite sum overflows only after the rounding: MAX + MAX is +∞, but
        // MAX + MAX - MAX is MAX (the sum is exact before the one rounding).
        assert_eq!(sum_precise(&[f64::MAX, f64::MAX]), f64::INFINITY);
        assert_eq!(sum_precise(&[f64::MAX, f64::MAX, -f64::MAX]), f64::MAX);
        assert_eq!(sum_precise(&[-f64::MAX, -f64::MAX]), f64::NEG_INFINITY);
    }

    #[test]
    fn the_rounding_edges() {
        let ulp_max = f64::MAX - f64::from_bits(f64::MAX.to_bits() - 1); // 2^971
        // MAX + half an ulp is the midpoint to 2^1024: a tie, and MAX is odd, so +∞.
        assert_eq!(sum_precise(&[f64::MAX, ulp_max / 2.0]), f64::INFINITY);
        // Just below the midpoint: MAX.
        assert_eq!(sum_precise(&[f64::MAX, ulp_max / 2.0, -5e-324]), f64::MAX);
        // 1 + 2^-53 is a tie between 1 and 1 + 2^-52: to even (1); with any lower bit, up.
        let e53 = f64::from_bits((1023 - 53) << 52); // 2^-53
        assert_eq!(sum_precise(&[1.0, e53]), 1.0);
        assert_eq!(sum_precise(&[1.0, e53, 5e-324]), 1.0 + 2.0 * e53);
        assert_eq!(sum_precise(&[1.0, e53, -5e-324]), 1.0);
        assert_eq!(sum_precise(&[-1.0, -e53]), -1.0);
        assert_eq!(sum_precise(&[-1.0, -e53, -5e-324]), -1.0 - 2.0 * e53);
        assert_eq!(sum_precise(&[-1.0, -e53, 5e-324]), -1.0);
        // (1 + 2^-52) + 2^-53 is a tie with an odd last bit: up.
        let one_ulp = 1.0 + 2.0 * e53;
        assert_eq!(sum_precise(&[one_ulp, e53]), 1.0 + 4.0 * e53);
        assert_eq!(sum_precise(&[-one_ulp, -e53]), -1.0 - 4.0 * e53);
        // Subnormal sums are exact.
        assert_eq!(bits(sum_precise(&[5e-324, 5e-324])), 2);
        assert_eq!(bits(sum_precise(&[-5e-324, -5e-324])), SIGN_MASK | 2);
        assert_eq!(
            sum_precise(&[f64::MIN_POSITIVE, -5e-324]),
            f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1)
        );
        assert_eq!(sum_precise(&[f64::MIN_POSITIVE, -f64::MIN_POSITIVE]), 0.0);
    }

    #[test]
    fn long_lists_need_the_carries() {
        // 10,000 adds: more than SMALL_CARRY_TERMS, so carry_propagate runs on the way.
        let mut v = Vec::new();
        for k in 0..10_000 {
            v.push(f64::MAX / 4.0);
            v.push(if k % 2 == 0 { 0.1 } else { -1e-300 });
        }
        assert_eq!(sum_precise(&v), f64::INFINITY);
        assert_eq!(bits(sum_precise(&v)), bits(reference(&v)));
        let w: Vec<f64> = (0..10_000)
            .map(|k| if k % 2 == 0 { 0.1 } else { -0.1 })
            .collect();
        assert_eq!(bits(sum_precise(&w)), bits(0.0));
        let u: Vec<f64> = (0..5_000).map(|_| 0.1).collect();
        assert_eq!(bits(sum_precise(&u)), bits(reference(&u)));
        assert_eq!(sum_precise(&u), 500.0);
        // 2^20 copies of -MAX / 2^20 and of MAX / 2^20, then one 1: the sum is 1.
        let mut big: Vec<f64> = (0..1 << 12).map(|_| -f64::MAX / 4096.0).collect();
        big.extend((0..1 << 12).map(|_| f64::MAX / 4096.0));
        big.push(1.0);
        assert_eq!(sum_precise(&big), 1.0);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn equals_the_reference_on_random_lists() {
        let mut r = Rng(0x5eed_5ca1_ab1e);
        let mut list = Vec::new();
        for n in 0..200_000u32 {
            list.clear();
            let len = if n % 1000 == 0 {
                3000
            } else {
                1 + r.below(12) as usize
            };
            for _ in 0..len {
                let x = r.element(&list);
                list.push(x);
            }
            let got = sum_precise(&list);
            let want = reference(&list);
            assert_eq!(bits(got), bits(want), "sumPrecise({list:?})");
        }
    }

    /// Ties and near-ties on random values: `a` plus or minus half an ulp of `a`, with or
    /// without a small value of either sign far below, and with `a` split in two parts.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn ties_equal_the_reference() {
        let mut r = Rng(0x7135);
        for _ in 0..20_000 {
            let a = r.finite(54, 2046);
            let field = (a.to_bits() >> 52) & 0x7ff;
            let half = f64::from_bits((field - 53) << 52); // half an ulp of a
            let tiny = r.finite(0, field - 54);
            let part = r.finite(1, field);
            for list in [
                vec![a, half],
                vec![a, -half],
                vec![a, half, tiny],
                vec![a, -half, tiny],
                vec![a, half, -tiny],
                vec![tiny, a, -half],
                vec![a - part, part, half],
                vec![-part, a + part, -half, tiny],
            ] {
                if list.iter().all(|x| x.is_finite()) {
                    assert_eq!(bits(sum_precise(&list)), bits(reference(&list)), "{list:?}");
                }
            }
        }
    }

    #[test]
    fn the_order_does_not_matter() {
        let mut r = Rng(7);
        for _ in 0..2_000 {
            let mut list: Vec<f64> = Vec::new();
            for _ in 0..1 + r.below(20) {
                let x = r.element(&list);
                list.push(x);
            }
            let a = sum_precise(&list);
            list.reverse();
            assert_eq!(bits(a), bits(sum_precise(&list)));
        }
    }
}
