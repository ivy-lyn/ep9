//! Number conversions and the number operations that need more than one machine instruction.
//!
//! | Function | Spec | v0 status |
//! |---|---|---|
//! | [`number_to_string`] | Number::toString(x, 10) (ECMA-262 §6.1.6.1.20) | digits from `core::fmt` `{:e}` (shortest round trip). v0: `ep9_numconv` (dragonbox_ecma) replaces it |
//! | [`string_to_number`] | StringToNumber (§7.1.4.1.1) | decimal through `str::parse::<f64>` after a JS grammar check (correctly rounded); hex/octal/binary exact up to 2^128 |
//! | [`fmod`] | Number::remainder (§6.1.6.1.6) | exact, bit-level (musl `fmod.c`, MIT). H36: never the platform libm. v0: moves to `ep9_math::fmod` |
//! | [`pow_exact`] | Number::exponentiate (§6.1.6.1.3) | v0: special cases and exact integer results only; else `None`. H36: `ep9_math::pow_js` replaces it |
//! | [`exponentiate`] | Number::exponentiate (§6.1.6.1.3) | `pow_exact` plus the infinite-exponent rules; else `None` (`**` throws "not implemented") |

/// Number::toString(x) in radix 10.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x < 0.0 {
        let mut s = String::from("-");
        s.push_str(&number_to_string(-x));
        return s;
    }
    if x.is_infinite() {
        return "Infinity".into();
    }
    if x < 2_147_483_648.0 && x.fract() == 0.0 {
        return (x as u32).to_string();
    }
    // v0: shortest round-trip digits from core::fmt. Format: "d[.ddd]e[-]x".
    let e = format!("{x:e}");
    let (mant, exp) = e.split_once('e').expect("{:e} has an exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let exp: i32 = exp.parse().expect("{:e} exponent");
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut out = String::new();
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(core::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(core::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n > 0 { '+' } else { '-' });
        out.push_str(&(n - 1).abs().to_string());
    }
    out
}

/// JS WhiteSpace or LineTerminator (§12.2, §12.3), the set StringToNumber trims.
pub fn is_js_whitespace(c: u16) -> bool {
    matches!(
        c,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// StringToNumber of UTF-16 code units. NaN for text that is not a StringNumericLiteral.
pub fn string_to_number(units: &[u16]) -> f64 {
    let mut s = units;
    while let [first, rest @ ..] = s
        && is_js_whitespace(*first)
    {
        s = rest;
    }
    while let [rest @ .., last] = s
        && is_js_whitespace(*last)
    {
        s = rest;
    }
    if s.is_empty() {
        return 0.0;
    }
    // Every valid literal is ASCII.
    if s.iter().any(|&u| u > 0x7F) {
        return f64::NAN;
    }
    let t: String = s.iter().map(|&u| u as u8 as char).collect();
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return parse_radix_integer(&t[2..], radix);
        }
    }
    let (sign, body) = match b[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, &t[..]),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(body.as_bytes()) {
        return f64::NAN;
    }
    // The grammar check above rules out Rust-only forms ("inf", "NaN"). Rust's parse is
    // correctly rounded.
    match body.parse::<f64>() {
        Ok(v) => sign * v,
        Err(_) => f64::NAN,
    }
}

/// StrUnsignedDecimalLiteral without `Infinity`: `digits [. digits?] | . digits`, then an
/// optional exponent.
fn is_decimal_literal(b: &[u8]) -> bool {
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let f = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - f;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return false;
        }
    }
    i == b.len()
}

/// A non-decimal integer literal body. Exact while the value fits 128 bits; v0 accumulates in
/// f64 beyond that.
fn parse_radix_integer(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let mut exact: Option<u128> = Some(0);
    let mut approx = 0.0f64;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return f64::NAN;
        };
        exact = exact
            .and_then(|v| v.checked_mul(u128::from(radix)))
            .and_then(|v| v.checked_add(u128::from(d)));
        approx = approx * f64::from(radix) + f64::from(d);
    }
    match exact {
        // `as f64` rounds to nearest, ties to even: the spec's rounding.
        Some(v) => v as f64,
        // v0: double rounding is possible beyond 2^128.
        None => approx,
    }
}

/// Exact IEEE-754 remainder with the sign of `x` (JS `%`). A port of musl `src/math/fmod.c`
/// (MIT, Copyright 2005-2020 Rich Felker et al.).
pub fn fmod(x: f64, y: f64) -> f64 {
    let mut uxi = x.to_bits();
    let mut uyi = y.to_bits();
    let mut ex = ((uxi >> 52) & 0x7ff) as i64;
    let mut ey = ((uyi >> 52) & 0x7ff) as i64;
    let sx = uxi >> 63;

    if uyi << 1 == 0 || y.is_nan() || ex == 0x7ff {
        return f64::NAN;
    }
    if uxi << 1 <= uyi << 1 {
        if uxi << 1 == uyi << 1 {
            return 0.0 * x;
        }
        return x;
    }

    // Normalize x and y.
    if ex == 0 {
        let mut i = uxi << 12;
        while i >> 63 == 0 {
            ex -= 1;
            i <<= 1;
        }
        uxi <<= (-ex + 1) as u32;
    } else {
        uxi &= u64::MAX >> 12;
        uxi |= 1 << 52;
    }
    if ey == 0 {
        let mut i = uyi << 12;
        while i >> 63 == 0 {
            ey -= 1;
            i <<= 1;
        }
        uyi <<= (-ey + 1) as u32;
    } else {
        uyi &= u64::MAX >> 12;
        uyi |= 1 << 52;
    }

    // x mod y.
    while ex > ey {
        let i = uxi.wrapping_sub(uyi);
        if i >> 63 == 0 {
            if i == 0 {
                return 0.0 * x;
            }
            uxi = i;
        }
        uxi <<= 1;
        ex -= 1;
    }
    let i = uxi.wrapping_sub(uyi);
    if i >> 63 == 0 {
        if i == 0 {
            return 0.0 * x;
        }
        uxi = i;
    }
    while uxi >> 52 == 0 {
        uxi <<= 1;
        ex -= 1;
    }

    // Scale the result.
    if ex > 0 {
        uxi -= 1 << 52;
        uxi |= (ex as u64) << 52;
    } else {
        uxi >>= (-ex + 1) as u32;
    }
    uxi |= sx << 63;
    f64::from_bits(uxi)
}

/// Number::exponentiate for the cases v0 can answer exactly: the spec's special cases for NaN
/// and zero exponents, exponent 1, and integer bases with small non-negative integer exponents
/// whose result is an integer of at most 2^53. `None` for everything else (H36: no libm `pow`).
pub fn pow_exact(base: f64, exp: f64) -> Option<f64> {
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
        let odd = fmod(exp, 2.0) == 1.0;
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

/// Number::exponentiate (§6.1.6.1.3). v0: [`pow_exact`] plus the infinity and NaN rules of
/// the spec; `None` for the rest (H36: `ep9_math::pow_js` replaces it).
pub fn exponentiate(base: f64, exp: f64) -> Option<f64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn number_to_string_matches_js() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (0.000001, "0.000001"),
            (0.0000001, "1e-7"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (2147483648.0, "2147483648"),
            (4294967296.5, "4294967296.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1.5e-10, "1.5e-10"),
            (123.456, "123.456"),
        ];
        for &(x, want) in cases {
            assert_eq!(number_to_string(x), want, "{x:?}");
        }
    }

    #[test]
    fn string_to_number_grammar() {
        let cases: &[(&str, f64)] = &[
            ("", 0.0),
            ("  \n\t ", 0.0),
            ("42", 42.0),
            (" -1.5e3 ", -1500.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("+Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
            ("0x1F", 31.0),
            ("0b101", 5.0),
            ("0o17", 15.0),
            ("1e1000", f64::INFINITY),
            ("0.1", 0.1),
            ("9007199254740993", 9007199254740992.0),
        ];
        for &(s, want) in cases {
            assert_eq!(string_to_number(&u(s)), want, "{s:?}");
        }
        for bad in [
            "inf", "NaN", "infinity", "1e", "e5", ".", "0x", "-0x10", "1_0", "12a", "+-1",
        ] {
            assert!(string_to_number(&u(bad)).is_nan(), "{bad:?}");
        }
        assert!(string_to_number(&u("-0")).is_sign_negative());
        assert_eq!(string_to_number(&u("\u{FEFF}7\u{3000}")), 7.0);
    }

    #[test]
    fn fmod_is_exact_and_follows_js() {
        assert_eq!(fmod(5.5, 2.0), 1.5);
        assert_eq!(fmod(-5.5, 2.0), -1.5);
        assert_eq!(fmod(5.0, -3.0), 2.0);
        assert!(fmod(-4.0, 2.0).is_sign_negative());
        assert!(fmod(1.0, 0.0).is_nan());
        assert!(fmod(f64::INFINITY, 2.0).is_nan());
        assert!(fmod(f64::NAN, 2.0).is_nan());
        assert_eq!(fmod(3.0, f64::INFINITY), 3.0);
        assert!(fmod(-0.0, 5.0).is_sign_negative());
        // Reference values: node 0.1 % 0.01, 1e300 % 3 (exact remainders).
        assert_eq!(fmod(0.1, 0.01), 3.469446951953614e-18);
        assert_eq!(fmod(1e300, 3.0), 0.0);
        assert_eq!(fmod(5e-324, 2e-323), 5e-324);
        // A plain reference: exact for these integer pairs.
        for a in -50i32..50 {
            for b in [-7i32, -3, 1, 2, 5, 11] {
                let r = fmod(f64::from(a), f64::from(b));
                assert_eq!(r, f64::from(a % b), "{a} % {b}");
            }
        }
    }

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
}
