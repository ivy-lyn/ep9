//! IEEE 754-2019 binary16 (half precision) conversions for Float16Array and the DataView
//! `getFloat16` and `setFloat16` methods (TypedArray TA-0, design TA-D12; H36: one Rust math
//! crate for all tiers). [`to_bits`] is NumericToRawBytes for Float16 (one rounding from the
//! double, ties to even: the same value as [`crate::f16round`]); [`from_bits`] is
//! RawBytesToNumeric for Float16 (exact). Integer steps only: no `f32` step (that would round
//! twice), no libm, no FMA.
//!
//! ```text
//! binary16: sign (bit 15) | exponent field e (bits 10-14, bias 15) | fraction f (bits 0-9)
//!   e = 0: ±f × 2^-24 (zero, subnormal)   e = 1..30: ±(1024 + f) × 2^(e - 25)   e = 31: ±∞, NaN
//! ```

/// The binary16 pattern that [`to_bits`] stores for every NaN: the quiet NaN 0x7E00. The spec
/// lets an implementation pick any NaN pattern (NumericToRawBytes); ep9 also boxes every
/// double NaN as one pattern (H36), so one NaN in, one NaN out.
pub const NAN_BITS: u16 = 0x7E00;

/// The binary16 pattern of `x` rounded to the nearest binary16 value, ties to even
/// (NumericToRawBytes for Float16, roundTiesToEven). A magnitude of 65520 (the midpoint above
/// the largest finite value, 65504) or more gives ±∞; a magnitude of 2^-25 or less gives ±0
/// (2^-25 is a tie that goes to the even 0); the sign of a zero is kept; every NaN gives
/// [`NAN_BITS`]. `from_bits(to_bits(x))` is `f16round(x)` for every `x` (NaN aside: both NaN).
#[inline(never)]
pub fn to_bits(x: f64) -> u16 {
    if x.is_nan() {
        return NAN_BITS;
    }
    let b = x.to_bits();
    let sign = ((b >> 48) & 0x8000) as u16;
    let field = ((b >> 52) & 0x7ff) as i32;
    if field == 0x7ff {
        // ±∞ (NaN is above).
        return sign | 0x7c00;
    }
    if field == 0 {
        // ±0, or a subnormal double (below 2^-1022): ±0.
        return sign;
    }
    // |x| = m × 2^(e - 52), with 2^52 <= m < 2^53.
    let e = field - 1023;
    if e > 15 {
        // |x| >= 2^16 > 65520.
        return sign | 0x7c00;
    }
    let m = (b & ((1 << 52) - 1)) | (1 << 52);
    // The binary16 step at |x|: 2^(e - 10) for a normal value (11 significant bits), and the
    // subnormal step 2^-24 below 2^-14.
    let q = if e >= -14 { e - 10 } else { -24 };
    // Drop s >= 42 low bits of m, round to nearest, ties to even: |x| ≈ r × 2^q.
    let s = (q - (e - 52)) as u32;
    let r: u64 = if s >= 64 {
        // |x| < 2^(q - 11): far below half a step.
        0
    } else {
        let r = m >> s;
        let rest = m & ((1 << s) - 1);
        let half = 1 << (s - 1);
        if rest > half || (rest == half && r & 1 == 1) {
            r + 1
        } else {
            r
        }
    };
    // A subnormal step (q = -24): the pattern is r itself (r = 1024 is the smallest normal,
    // 0x0400). A normal step: 1024 <= r <= 2048 and the pattern is (e + 15) << 10 | (r - 1024),
    // that is ((e + 14) << 10) + r, so r = 2048 carries into the exponent; at e = 15 that carry
    // gives 0x7c00, ±∞ (the round-up past 65504).
    let mag = if q == -24 {
        r as u16
    } else {
        ((((e + 14) as u64) << 10) + r) as u16
    };
    sign | mag
}

/// The double value of the binary16 pattern `h` (RawBytesToNumeric for Float16): exact, since
/// every binary16 value is a double. A NaN pattern (exponent field 31, fraction not 0) gives the
/// double NaN.
#[inline(never)]
pub fn from_bits(h: u16) -> f64 {
    let sign = u64::from(h & 0x8000) << 48;
    let field = i32::from((h >> 10) & 0x1f);
    let f = u64::from(h & 0x3ff);
    if field == 0x1f {
        return if f == 0 {
            f64::from_bits(sign | (0x7ff << 52))
        } else {
            f64::NAN
        };
    }
    // f × 2^-24 for a subnormal, (1024 + f) × 2^(field - 25) for a normal value: an integer
    // below 2^11 times a power of 2 that is a normal double, so the product is exact.
    let (n, k) = if field == 0 {
        (f, -24)
    } else {
        (1024 + f, field - 25)
    };
    let mag = n as f64 * f64::from_bits(((k + 1023) as u64) << 52);
    f64::from_bits(sign | mag.to_bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::f16round;

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    /// The value of a binary16 pattern by an independent route: the fraction as a fraction of
    /// 1024, scaled by repeated halving and doubling (exact for these small values).
    fn value_of(h: u16) -> f64 {
        let field = (h >> 10) & 0x1f;
        let f = f64::from(h & 0x3ff);
        let mut v = if field == 0 {
            f / 1024.0
        } else {
            1.0 + f / 1024.0
        };
        let mut k = if field == 0 {
            -14
        } else {
            i32::from(field) - 15
        };
        while k > 0 {
            v *= 2.0;
            k -= 1;
        }
        while k < 0 {
            v /= 2.0;
            k += 1;
        }
        if h & 0x8000 != 0 { -v } else { v }
    }

    /// Every one of the 65536 patterns: a finite one reads as its value and stores back as
    /// itself; ±∞ stay; a NaN pattern reads as NaN and stores as [`NAN_BITS`].
    #[test]
    fn every_pattern_round_trips() {
        for h in 0..=u16::MAX {
            let v = from_bits(h);
            let field = (h >> 10) & 0x1f;
            if field == 0x1f && h & 0x3ff != 0 {
                assert!(v.is_nan(), "{h:#06x}");
                assert_eq!(to_bits(v), NAN_BITS);
                continue;
            }
            if field == 0x1f {
                assert_eq!(
                    v,
                    if h & 0x8000 != 0 {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }
                );
            } else {
                assert_eq!(v.to_bits(), value_of(h).to_bits(), "{h:#06x}");
            }
            assert_eq!(to_bits(v), h, "{h:#06x} -> {v:e}");
            assert!(same(f16round(v), v), "{h:#06x}");
        }
    }

    fn next_up(x: f64) -> f64 {
        f64::from_bits(x.to_bits() + 1)
    }

    fn next_down(x: f64) -> f64 {
        f64::from_bits(x.to_bits() - 1)
    }

    /// Each midpoint between two neighbouring finite patterns goes to the even one, and one
    /// double ulp off it to the nearer one; both signs; the same values as `f16round`.
    #[test]
    fn midpoints_go_to_even_as_f16round() {
        for h in 0u16..0x7bff {
            let (lo, hi) = (from_bits(h), from_bits(h + 1));
            let mid = (lo + hi) / 2.0; // exact: 12 significant bits at most
            let tie = if h & 1 == 0 { h } else { h + 1 };
            for neg in [0, 0x8000] {
                let s = |x: f64| if neg == 0 { x } else { -x };
                assert_eq!(to_bits(s(mid)), tie | neg, "tie {mid:e}");
                assert_eq!(to_bits(s(next_up(mid))), (h + 1) | neg, "above {mid:e}");
                assert_eq!(to_bits(s(next_down(mid))), h | neg, "below {mid:e}");
                for x in [mid, next_up(mid), next_down(mid)] {
                    assert!(same(from_bits(to_bits(s(x))), f16round(s(x))), "{x:e}");
                }
            }
        }
    }

    /// The edges, with node v26.13.0 results of `new Float16Array([x])[0]` (the store and
    /// load) and of `Math.f16round(x)` (design §4.1).
    #[test]
    fn edges_match_node() {
        let cases: &[(f64, u16)] = &[
            (0.0, 0x0000),
            (-0.0, 0x8000),
            (1.0, 0x3c00),
            (-2.0, 0xc000),
            (1.0 / 3.0, 0x3555), // 0.333251953125
            (65504.0, 0x7bff),
            (65519.99999999999, 0x7bff),
            (65520.0, 0x7c00), // the tie goes to the even 2^16: ∞
            (-65520.0, 0xfc00),
            (1e300, 0x7c00),
            (f64::INFINITY, 0x7c00),
            (f64::NEG_INFINITY, 0xfc00),
            (5.960464477539063e-8, 0x0001), // 2^-24, the smallest subnormal
            (2.9802322387695312e-8, 0x0000), // 2^-25: a tie, to the even 0
            (2.980232238769532e-8, 0x0001),
            (-1e-10, 0x8000),
            (5e-324, 0x0000),
            (-5e-324, 0x8000),
            (6.103515625e-5, 0x0400),       // 2^-14, the smallest normal
            (6.097555160522461e-5, 0x03ff), // the largest subnormal
            (1.00048828125, 0x3c00),        // 1 + 2^-11: a tie, to the even 1
            (1.0014648437500002, 0x3c02),
        ];
        for &(x, h) in cases {
            assert_eq!(to_bits(x), h, "to_bits({x:e})");
            assert!(same(from_bits(h), f16round(x)), "{x:e}");
        }
        assert_eq!(from_bits(0x3555), 0.333251953125);
        assert_eq!(to_bits(f64::NAN), NAN_BITS);
        assert_eq!(to_bits(-f64::NAN), NAN_BITS);
        assert!(from_bits(0xfe01).is_nan());
        // 1 + 2^-11 + 2^-40 is above the midpoint 1 + 2^-11: one rounding gives 1 + 2^-10, where
        // a double rounding through f32 would give 1.
        let x = 1.0 + f64::from_bits((1023 - 11) << 52) + f64::from_bits((1023 - 40) << 52);
        assert_eq!(to_bits(x), 0x3c01);
    }

    /// `from_bits(to_bits(x))` equals `f16round(x)` on 200,000 doubles from every binade
    /// (SplitMix64 bits with the exponent spread over the binary16 range and past it).
    #[test]
    #[cfg_attr(miri, ignore)]
    fn store_and_load_equal_f16round_on_many_doubles() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        for i in 0..200_000u32 {
            let r = next();
            let x = if i % 4 == 0 {
                f64::from_bits(r)
            } else {
                // Exponents 2^-30 .. 2^20 around the binary16 range.
                let e = 1023 - 30 + (r >> 58) % 51;
                f64::from_bits((r & 0x800f_ffff_ffff_ffff) | (e << 52))
            };
            assert!(
                same(from_bits(to_bits(x)), f16round(x)),
                "{x:e} ({:#018x})",
                x.to_bits()
            );
        }
    }
}
