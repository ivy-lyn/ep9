//! Number conversions and the number operations that need more than one machine instruction.
//!
//! | Function | Spec | v0 status |
//! |---|---|---|
//! | [`number_to_string`] | Number::toString(x, 10) (ECMA-262 §6.1.6.1.20) | digits from `core::fmt` `{:e}` (shortest round trip). v0: `ep9_numconv` (dragonbox_ecma) replaces it |
//! | [`string_to_number`] | StringToNumber (§7.1.4.1.1) | decimal through `str::parse::<f64>` after a JS grammar check (correctly rounded); hex/octal/binary exact up to 2^128 |
//! | [`fmod`] | Number::remainder (§6.1.6.1.6) | `ep9_math::fmod`: exact, bit-level (musl `fmod.c`, MIT). H36: never the platform libm |
//! | [`exponentiate`] | Number::exponentiate (§6.1.6.1.3) | `ep9_math::pow_js` v0: every special case and exact result; else `None` (`**` throws "not implemented") until the correctly rounded core lands (month 2, H36) |

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

/// Exact IEEE-754 remainder with the sign of `x` (JS `%`). H36: the one Rust Math.
pub use ep9_math::fmod;

/// Number::exponentiate (§6.1.6.1.3) for `**`. `None` where `ep9_math::pow_js` v0 needs the
/// correctly rounded core (month 2); the caller throws "not implemented" then.
#[inline]
pub fn exponentiate(base: f64, exp: f64) -> Option<f64> {
    ep9_math::pow_js(base, exp)
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

    /// `%` and `**` reach ep9_math (the vectors moved there with the code, M0 week 4).
    #[test]
    fn remainder_and_exponent_use_ep9_math() {
        assert_eq!(fmod(-5.5, 2.0), -1.5);
        assert_eq!(fmod(0.1, 0.01), 3.469446951953614e-18);
        assert_eq!(exponentiate(2.0, 32.0), Some(4_294_967_296.0));
        assert_eq!(exponentiate(2.0, 0.5), Some(std::f64::consts::SQRT_2));
        assert_eq!(
            exponentiate(f64::NEG_INFINITY, -3.0).map(f64::to_bits),
            Some((-0.0f64).to_bits())
        );
        assert_eq!(exponentiate(2.0, 0.3), None);
    }
}
