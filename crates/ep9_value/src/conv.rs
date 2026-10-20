//! Exact ToInt32 and ToUint32 on a JS number (ECMA-262 §7.1.6, §7.1.7; 06-object-model §4,
//! H36: "ToInt32 and IEEE-754 stay exact").
//!
//! ```text
//! ToInt32(d):  NaN, +-0, +-inf -> 0
//!              t = sign(d) * floor(|d|)        (truncate toward zero)
//!              u = t mod 2^32                  (0 ..= 2^32 - 1)
//!              u >= 2^31 ? u - 2^32 : u
//! ToUint32(d): the same u
//! ```
//!
//! | Range of d | Method |
//! |---|---|
//! | i32::MIN ..= i32::MAX | `d as i32` (truncates toward zero) |
//! | other finite | Shift the 53-bit significand by the exponent; keep the low 32 bits; negate if d < 0 (V8 `DoubleToInt32`) |
//! | NaN, +-inf | 0 (the biased exponent is 0x7FF, so the shift rule gives 0) |

/// ECMAScript ToInt32 of a number. Exact for every f64.
#[inline]
pub const fn to_int32(d: f64) -> i32 {
    // In range: the cast truncates toward zero, which is the spec rounding. NaN fails both
    // compares, so it takes the slow path.
    if d >= i32::MIN as f64 && d <= i32::MAX as f64 {
        return d as i32;
    }
    to_int32_slow(d)
}

/// ECMAScript ToUint32 of a number. Exact for every f64.
#[inline]
pub const fn to_uint32(d: f64) -> u32 {
    // ToUint32(d) and ToInt32(d) have the same low 32 bits (both are `t mod 2^32`).
    to_int32(d) as u32
}

/// |d| > 2^31 - 1, or NaN, or an infinity. Such a d is a normal number or a non-finite one,
/// never a subnormal, so the significand has its hidden bit.
const fn to_int32_slow(d: f64) -> i32 {
    const SIGNIFICAND_BITS: u32 = 52;
    const EXPONENT_BIAS: i32 = 1023;
    let bits = d.to_bits();
    let biased = ((bits >> SIGNIFICAND_BITS) & 0x7FF) as i32;
    // d = significand * 2^exponent, with the significand as an integer of 53 bits.
    let exponent = biased - EXPONENT_BIAS - SIGNIFICAND_BITS as i32;
    let significand = (bits & ((1u64 << SIGNIFICAND_BITS) - 1)) | (1u64 << SIGNIFICAND_BITS);
    let magnitude = if exponent > 31 {
        // A multiple of 2^32 (also NaN and the infinities: biased 0x7FF gives 972).
        return 0;
    } else if exponent >= 0 {
        // The low 32 bits survive a shift of at most 31; bits shifted out above 2^64 do not
        // matter for `mod 2^32`.
        significand << exponent
    } else if exponent > -64 {
        // Drops the fraction: truncation toward zero of |d|.
        significand >> -exponent
    } else {
        // Not reached: |d| > 2^31 means exponent >= -21. Kept so the function is total.
        0
    };
    let low = magnitude as u32;
    let low = if bits >> 63 == 1 {
        low.wrapping_neg()
    } else {
        low
    };
    low as i32
}
