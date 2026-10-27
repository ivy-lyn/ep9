//! One Rust Math (H36, `docs/adr/H36-fp-semantics.md`): every engine crate gets libm-class
//! results, `pow_js`, exact `fmod` and `fma_exact` from here, so each target computes the same
//! bits. This crate has no dependencies (`.github/scripts/dep-guards.sh` check 6).
//!
//! | Item | JS use | Status |
//! |---|---|---|
//! | [`fmod()`] | `%` (Number::remainder) | exact; musl port |
//! | [`pow_js`] | `**`, `Math.pow` | the JS special cases, then the correctly rounded `pow` (CORE-MATH `cr_pow` port, M4-P1) |
//! | [`exp()`] | `Math.exp` | correctly rounded (CORE-MATH `cr_exp` port, M4-P1) |
//! | [`log()`] | `Math.log` | correctly rounded (CORE-MATH `cr_log` port, M4-P1) |
//! | [`round_js`], [`fround`] | `Math.round`, `Math.fround` | exact |
//! | [`fma_exact`], [`two_prod`], [`two_sum`], [`fma_soft`] | building blocks of the CR functions | exact |
//!
//! Rules (H36): every public function is `#[inline(never)]`, so FMA instructions stay inside
//! `ep9_math::` symbols (`cargo xtask fp-check` allows them only there). No libm call, no
//! `mul_add` outside [`exact::HwFma`]. The only `unsafe` code is the FMA dispatch: the
//! `multiversion!` macro of `exact.rs`, which carries the one `#[allow(unsafe_code)]`; the CR
//! functions use it for their entry points, so each one runs with the hardware FMA where the CPU
//! has it and with the exact software FMA elsewhere (the same bits).
//!
//! The CR ports keep the CORE-MATH tables as written in C: `hf` (crate-private) reads each C99 hexadecimal
//! floating literal at compile time. Provenance: `THIRD-PARTY-LICENSE` ("CORE-MATH").
#![deny(unsafe_code)]

#[macro_use]
pub mod exact;
mod acos;
mod acosh;
mod asin;
mod asinh;
mod atan;
mod atan2;
mod atanh;
mod cbrt;
mod cos;
mod cosh;
mod exp;
mod expm1;
mod fmod;
mod log;
mod log10;
mod log1p;
mod log2;
mod pow;
mod round;
mod sin;
mod sinh;
mod tan;
mod tanh;
mod trig;

pub use acos::acos;
pub use acosh::acosh;
pub use asin::asin;
pub use asinh::asinh;
pub use atan::atan;
pub use atan2::atan2;
pub use atanh::atanh;
pub use cbrt::cbrt;
pub use cos::cos;
pub use cosh::cosh;
pub use exact::{fma_exact, fma_soft, two_prod, two_sum};
pub use exp::exp;
pub use expm1::expm1;
pub use fmod::fmod;
pub use log::log;
pub use log1p::log1p;
pub use log2::log2;
pub use log10::log10;
pub use pow::pow_js;
pub use round::{fround, round_js};
pub use sin::sin;
pub use sinh::sinh;
pub use tan::tan;
pub use tanh::tanh;

/// The value of a C99 hexadecimal floating literal such as `"0x1.62e42fefa39efp-13"`,
/// `"-0x1p-1074"` or `"0x0p+0"`, at compile time. The CORE-MATH ports write their tables with
/// it, so each line keeps its C text. A literal that is not exact in binary64 (more bits than
/// the format holds, or out of range) is a compile error.
pub(crate) const fn hf(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    let neg = b[0] == b'-';
    if b[0] == b'-' || b[0] == b'+' {
        i = 1;
    }
    assert!(b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X'));
    i += 2;
    // The significand digits as one integer m, and the number of fraction bits.
    let mut m: u64 = 0;
    let mut frac_bits: i32 = 0;
    let mut point = false;
    while i < b.len() && b[i] != b'p' && b[i] != b'P' {
        let c = b[i];
        if c == b'.' {
            assert!(!point);
            point = true;
        } else {
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("hf: not a hex digit"),
            };
            assert!(m < (1u64 << 59), "hf: too many digits");
            m = m * 16 + d as u64;
            if point {
                frac_bits += 4;
            }
        }
        i += 1;
    }
    // The binary exponent (required in C99 hexadecimal literals).
    assert!(i < b.len(), "hf: no exponent");
    i += 1;
    let eneg = b[i] == b'-';
    if b[i] == b'-' || b[i] == b'+' {
        i += 1;
    }
    let mut e: i32 = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit());
        e = e * 10 + (b[i] - b'0') as i32;
        i += 1;
    }
    if eneg {
        e = -e;
    }
    let sign = if neg { 1u64 << 63 } else { 0 };
    if m == 0 {
        return f64::from_bits(sign);
    }
    // value = m * 2^exp2; normalize m to [2^52, 2^53).
    let mut exp2 = e - frac_bits;
    let shift = m.leading_zeros() as i32 - 11;
    if shift >= 0 {
        m <<= shift;
        exp2 -= shift;
    } else {
        let s = (-shift) as u32;
        assert!(m & ((1u64 << s) - 1) == 0, "hf: not exact in binary64");
        m >>= s;
        exp2 += -shift;
    }
    let biased = exp2 + 52 + 1023;
    assert!(biased < 2047, "hf: overflow");
    if biased <= 0 {
        // Subnormal: the shifted-out bits must be 0.
        let s = (1 - biased) as u32;
        assert!(
            s < 64 && m & ((1u64 << s) - 1) == 0,
            "hf: not exact in binary64"
        );
        return f64::from_bits(sign | (m >> s));
    }
    f64::from_bits(sign | ((biased as u64) << 52) | (m & ((1u64 << 52) - 1)))
}

#[cfg(test)]
mod tests {
    use super::hf;

    #[test]
    fn hf_reads_c_hex_literals() {
        assert_eq!(hf("0x1p+0"), 1.0);
        assert_eq!(hf("0x1.8p+1"), 3.0);
        assert_eq!(hf("-0x1p-1"), -0.5);
        assert_eq!(hf("0x1.62e42fefa39efp-1").to_bits(), 0x3fe62e42fefa39ef);
        assert_eq!(hf("0x1.fffffffffffffp+1023"), f64::MAX);
        assert_eq!(hf("0x1p-1022"), f64::MIN_POSITIVE);
        assert_eq!(hf("0x1p-1074").to_bits(), 1);
        assert_eq!(hf("0x0.0000000000001p-1022").to_bits(), 1);
        assert_eq!(hf("0x1.698p+0").to_bits(), 0x3ff6980000000000);
        assert_eq!(hf("0x0p+0").to_bits(), 0);
        assert_eq!(hf("-0x0p+0").to_bits(), 1 << 63);
        assert_eq!(hf("0x1.71547652b82fep+12").to_bits(), 0x40b71547652b82fe);
        assert_eq!(hf("0X1P1"), 2.0);
        assert_eq!(hf("0x10p-4"), 1.0);
    }
}
