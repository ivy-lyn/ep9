//! Number conversions and the number operations that need more than one machine instruction.
//!
//! | Function | Spec | v0 status |
//! |---|---|---|
//! | [`number_to_string`], [`number_to_ascii`] (no heap allocation) | Number::toString(x, 10) (ECMA-262 §6.1.6.1.20) | digits from `core::fmt` `{:e}` (shortest round trip), then the even one of two closest (step 5, `even_tie`). v0: `ep9_numconv` (dragonbox_ecma) replaces it |
//! | [`string_to_number`] | StringToNumber (§7.1.4.1.1) | decimal through `str::parse::<f64>` after a JS grammar check (correctly rounded); hex/octal/binary exact up to 2^128 |
//! | [`fmod`] | Number::remainder (§6.1.6.1.6) | `ep9_math::fmod`: exact, bit-level (musl `fmod.c`, MIT). H36: never the platform libm |
//! | [`exponentiate`] | Number::exponentiate (§6.1.6.1.3) | `ep9_math::pow_js`: every special case and exact result, else the correctly rounded pow (CORE-MATH `cr_pow` port, M4-P1, H36) |

/// Number::toString(x) in radix 10, as a new `String`. One allocation of the exact length;
/// the text comes from [`number_to_ascii`].
pub fn number_to_string(x: f64) -> String {
    let mut buf = [0u8; NUMBER_TEXT_CAP];
    String::from(number_to_ascii(x, &mut buf))
}

/// The buffer size for [`number_to_ascii`]. The longest Number::toString text is 25 bytes:
/// "-0.0000012345678901234567" (a sign, "0.", 5 zeros and 17 digits). The other forms are
/// shorter: the exponent form is at most 24 ("-1.2345678901234567e-308"), the integer form
/// at most 22 (a sign and 21 digits), the fraction form at most 19 (a sign, 17 digits and a
/// point).
pub const NUMBER_TEXT_CAP: usize = 32;

/// ASCII text in a fixed stack buffer. A write past the end is an error (`core::fmt`) or a
/// panic (`push`); the length bound above makes both unreachable.
struct AsciiBuf<'a, const N: usize> {
    buf: &'a mut [u8; N],
    len: usize,
}

impl<const N: usize> AsciiBuf<'_, N> {
    fn push(&mut self, b: u8) {
        self.buf[self.len] = b;
        self.len += 1;
    }

    fn push_bytes(&mut self, b: &[u8]) {
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
    }

    fn push_zeros(&mut self, n: usize) {
        self.buf[self.len..self.len + n].fill(b'0');
        self.len += n;
    }

    /// The decimal digits of `v`, most significant first.
    fn push_u32(&mut self, mut v: u32) {
        let mut tmp = [0u8; 10];
        let mut at = tmp.len();
        loop {
            at -= 1;
            tmp[at] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.push_bytes(&tmp[at..]);
    }
}

impl<const N: usize> core::fmt::Write for AsciiBuf<'_, N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        if b.len() > N - self.len {
            return Err(core::fmt::Error);
        }
        self.push_bytes(b);
        Ok(())
    }
}

/// Number::toString(x) in radix 10, written into `buf` with no heap allocation. Returns the
/// ASCII text (a part of `buf`). The same text as [`number_to_string`].
///
/// The one heap allocation left is in `even_tie`, and only when the exact decimal value of
/// `x` has exactly one digit more than its shortest form and that digit is 5 (a possible
/// tie). A double with a binary exponent below -26 or a value of 10^18 or more never reaches
/// it; for example no `Math.random()` result with a nonzero bit below 2^-26 does.
pub fn number_to_ascii(x: f64, buf: &mut [u8; NUMBER_TEXT_CAP]) -> &str {
    let len = {
        let mut out = AsciiBuf { buf, len: 0 };
        write_number(x, &mut out);
        out.len
    };
    debug_assert!(len <= 25, "Number::toString text of {len} bytes");
    core::str::from_utf8(&buf[..len]).expect("Number::toString text is ASCII")
}

/// A new heap string of Number::toString(d) (one Latin-1 cell). The text goes through a
/// stack buffer ([`number_to_ascii`]), so no temporary Rust `String`. GC point. For
/// `conv::number_string` (the ToString cache miss).
pub(crate) fn number_cell(rt: &crate::runtime::Runtime, d: f64) -> u64 {
    let mut buf = [0u8; NUMBER_TEXT_CAP];
    crate::string::new_string_latin1(rt, number_to_ascii(d, &mut buf).as_bytes())
}

/// The atom of Number::toString(d), interned from a stack buffer ([`number_to_ascii`]), so
/// no temporary Rust `String`. GC point. For `key::number_key` (a non-index number key).
pub(crate) fn intern_number(rt: &crate::runtime::Runtime, d: f64) -> u64 {
    let mut buf = [0u8; NUMBER_TEXT_CAP];
    crate::atoms::intern_latin1(rt, number_to_ascii(d, &mut buf).as_bytes())
}

fn write_number(mut x: f64, out: &mut AsciiBuf<'_, NUMBER_TEXT_CAP>) {
    if x.is_nan() {
        out.push_bytes(b"NaN");
        return;
    }
    if x == 0.0 {
        out.push(b'0');
        return;
    }
    if x < 0.0 {
        out.push(b'-');
        x = -x;
    }
    if x.is_infinite() {
        out.push_bytes(b"Infinity");
        return;
    }
    if x < 2_147_483_648.0 && x.fract() == 0.0 {
        out.push_u32(x as u32);
        return;
    }
    // v0: shortest round-trip digits from core::fmt. Format: "d[.ddd]e[-]x", at most 23
    // bytes (17 digits, a point, 'e', '-' and 3 exponent digits).
    let mut ebuf = [0u8; NUMBER_TEXT_CAP];
    let mut e = AsciiBuf {
        buf: &mut ebuf,
        len: 0,
    };
    core::fmt::Write::write_fmt(&mut e, format_args!("{x:e}")).expect("{:e} fits 32 bytes");
    let e = &e.buf[..e.len];
    let at = e
        .iter()
        .position(|&c| c == b'e')
        .expect("{:e} has an exponent");
    let (mant, exp_text) = (&e[..at], &e[at + 1..]);
    // The digits of the mantissa without the point: at most 17.
    let mut dbuf = [0u8; NUMBER_TEXT_CAP];
    let mut k = 0;
    for &c in mant {
        if c != b'.' {
            dbuf[k] = c;
            k += 1;
        }
    }
    let (neg, abs) = match exp_text {
        [b'-', rest @ ..] => (true, rest),
        _ => (false, exp_text),
    };
    let mut exp: i32 = 0;
    for &c in abs {
        debug_assert!(c.is_ascii_digit(), "{{:e}} exponent");
        exp = exp * 10 + i32::from(c - b'0');
    }
    if neg {
        exp = -exp;
    }
    // Step 5's last rule: of two closest digit strings, the even one. The rare path that
    // allocates (see `number_to_ascii`).
    if let Some((even, even_exp)) = even_tie(x, k, exp) {
        k = even.len();
        dbuf[..k].copy_from_slice(even.as_bytes());
        exp = even_exp;
    }
    let digits = &dbuf[..k];
    let k = k as i32;
    let n = exp + 1;
    if k <= n && n <= 21 {
        out.push_bytes(digits);
        out.push_zeros((n - k) as usize);
    } else if 0 < n && n <= 21 {
        out.push_bytes(&digits[..n as usize]);
        out.push(b'.');
        out.push_bytes(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_bytes(b"0.");
        out.push_zeros((-n) as usize);
        out.push_bytes(digits);
    } else {
        out.push(digits[0]);
        if k > 1 {
            out.push(b'.');
            out.push_bytes(&digits[1..]);
        }
        out.push(b'e');
        out.push(if n > 0 { b'+' } else { b'-' });
        out.push_u32((n - 1).unsigned_abs());
    }
}

/// The tie rule of Number::toString (§6.1.6.1.20 step 5): "If there are two such possible
/// values of s, choose the one that is even." `core::fmt`'s shortest digits pick the closer
/// of two shortest candidates, and the upper one when `x` lies exactly halfway between them
/// (`900719925474099.25`: `...099.3`; node and the spec: `...099.2`).
///
/// `x` (positive, finite) with `k` shortest digits and scientific exponent `exp` is such a
/// tie when its exact decimal value has exactly `k + 1` significant digits and the last is 5.
/// Then the candidates are the first `k` digits and that plus one; if both round-trip, the
/// even one wins: `Some((digits, exponent))`. `None` when there is no tie, or when only one
/// candidate round-trips (`core::fmt` already chose it).
///
/// The exact value `m × 2^e` (`m` odd) has more than 18 significant digits when `e < -26`
/// (`m × 5^-e` then has more than 0.69 × 27 digits after the first) or when it is at least
/// 10^18, and the shortest form has at most 17 digits, so only the rest is checked, in u128.
fn even_tie(x: f64, k: usize, exp: i32) -> Option<(String, i32)> {
    debug_assert!(x > 0.0 && x.is_finite());
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7FF) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mut m, mut e) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let tz = m.trailing_zeros();
    m >>= tz;
    e += tz as i32;
    // x = d × 10^-t exactly.
    let (mut d, mut t): (u128, i32) = if e >= 0 {
        if x >= 1e18 {
            return None;
        }
        (u128::from(m) << e, 0)
    } else if e >= -26 {
        (u128::from(m) * 5u128.pow((-e) as u32), -e)
    } else {
        return None;
    };
    while d % 10 == 0 {
        d /= 10;
        t -= 1;
    }
    let c = d.ilog10() as usize + 1;
    if c != k + 1 || d % 10 != 5 {
        return None;
    }
    let lower = d / 10;
    let upper = lower + 1;
    if upper == 10u128.pow(k as u32) {
        return None;
    }
    let sci_exp = c as i32 - 1 - t;
    debug_assert_eq!(sci_exp, exp, "core::fmt chose one of the two");
    let round_trips = |s: u128| format!("{s}e{}", sci_exp - (k as i32 - 1)).parse::<f64>() == Ok(x);
    if !(round_trips(lower) && round_trips(upper)) {
        return None;
    }
    let even = if lower % 2 == 0 { lower } else { upper };
    Some((even.to_string(), sci_exp))
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
    ascii_to_number(&t)
}

/// StringToNumber of the code units of a flat string (H19 `JsStr`) with no copy: Latin-1
/// text is trimmed, checked and parsed as its own bytes; UTF-16 text goes to
/// [`string_to_number`] as it is. The same result as [`string_to_number`] of the units. Not a
/// GC point. For `conv::to_number` (PREALLOC K12).
pub(crate) fn string_to_number_text(text: crate::string::JsStr<'_>) -> f64 {
    let mut s = match text {
        crate::string::JsStr::Latin1(b) => b,
        crate::string::JsStr::Utf16(u) => return string_to_number(u),
    };
    while let [first, rest @ ..] = s
        && is_js_whitespace(u16::from(*first))
    {
        s = rest;
    }
    while let [rest @ .., last] = s
        && is_js_whitespace(u16::from(*last))
    {
        s = rest;
    }
    if s.is_empty() {
        return 0.0;
    }
    // Every valid literal is ASCII (and ASCII bytes are UTF-8).
    if !s.is_ascii() {
        return f64::NAN;
    }
    ascii_to_number(core::str::from_utf8(s).expect("ASCII text"))
}

/// StringToNumber of `t`: ASCII text, not empty, with no leading or trailing white space.
fn ascii_to_number(t: &str) -> f64 {
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
        _ => (1.0, t),
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

/// Number::exponentiate (§6.1.6.1.3) for `**`: `ep9_math::pow_js`, the exact or correctly
/// rounded result (H36). `Math.pow` calls the same function.
#[inline]
pub fn exponentiate(base: f64, exp: f64) -> f64 {
    ep9_math::pow_js(base, exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The body of `number_to_string` before PREALLOC K1 (temporary Strings), kept as the
    /// oracle of `number_to_ascii`.
    fn old_number_to_string(x: f64) -> String {
        if x.is_nan() {
            return "NaN".into();
        }
        if x == 0.0 {
            return "0".into();
        }
        if x < 0.0 {
            let mut s = String::from("-");
            s.push_str(&old_number_to_string(-x));
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
        let mut digits: String = mant.chars().filter(|c| *c != '.').collect();
        let mut exp: i32 = exp.parse().expect("{:e} exponent");
        // Step 5's last rule: of two closest digit strings, the even one.
        if let Some((even, even_exp)) = even_tie(x, digits.len(), exp) {
            digits = even;
            exp = even_exp;
        }
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

    /// xorshift64* (fixed seed): the same inputs on every run.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    fn same_as_old(x: f64) {
        let mut buf = [0u8; NUMBER_TEXT_CAP];
        let new = number_to_ascii(x, &mut buf);
        let old = old_number_to_string(x);
        assert_eq!(new, old, "{x:e} (bits {:#018x})", x.to_bits());
        assert!(new.len() <= 25, "{new}");
        assert_eq!(number_to_string(x), old, "{x:e}");
    }

    /// PREALLOC K1: the buffer form gives the same text as the old body, for 1,000,000 seeded
    /// random doubles (random bit patterns, `Math.random`-like [0, 1), integers near 2^31 and
    /// 2^53) and the edge cases.
    #[test]
    #[allow(clippy::excessive_precision)]
    fn number_to_ascii_matches_the_old_body() {
        let mut fixed: Vec<f64> = vec![
            0.0,
            -0.0,
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            5e-324,
            -5e-324,
            f64::from_bits(0x000F_FFFF_FFFF_FFFF),
            f64::from_bits(0x0008_0000_0000_0000),
            1e21,
            -1e21,
            1e21 * (1.0 + f64::EPSILON),
            999999999999999900000.0,
            1e20,
            1e-7,
            -1e-7,
            1e-6,
            0.000001234567890123456,
            -0.0000012345678901234567,
            9.999999999999999e-7,
            1.0000000000000001e-7,
            -1.2345678901234567e-308,
            2147483647.0,
            2147483648.0,
            2147483649.0,
            4294967295.0,
            4294967296.0,
            9007199254740991.0,
            9007199254740992.0,
            9007199254740993.0,
            0.1 + 0.2,
            // The even-tie cases (`number_to_string_ties_go_to_even`).
            900719925474099.25,
            0.185214996337890625,
            1.82361602783203125,
            104.471893310546875,
            156512588.544921875,
            1071568155368862.75,
            0.74469757080078125,
            249274520119823.375,
            -900719925474099.25,
            0.5,
            2.5e-7,
        ];
        for p in -330..=310 {
            let t: f64 = format!("1e{p}").parse().unwrap();
            fixed.extend([t, -t, t * 1.5, t * 0.999_999_999_999_999_9]);
        }
        for i in 0..4096u64 {
            fixed.push(f64::from_bits(i)); // subnormals near 0
            fixed.push(f64::from_bits(0x000F_FFFF_FFFF_FFFF - i)); // near the normal limit
        }
        for &x in &fixed {
            same_as_old(x);
        }
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for i in 0..1_000_000u32 {
            let r = rng.next();
            let half = r >> 63 == 1;
            let x = match i % 4 {
                // Any bit pattern: subnormals, NaNs, infinities, every exponent.
                0 => f64::from_bits(r),
                // Math.random: 53 random bits in [0, 1).
                1 => (r >> 11) as f64 / (1u64 << 53) as f64,
                // Integers (and halves) near 2^31: both sides of the integer fast path.
                2 => 2_147_483_648.0 + ((r % 4096) as f64 - 2048.0) * if half { 0.5 } else { 1.0 },
                // Integers (and quarters) at and below 2^53.
                _ => {
                    9_007_199_254_740_992.0 - (r % 1_000_000) as f64 / if half { 4.0 } else { 1.0 }
                }
            };
            same_as_old(x);
        }
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

    /// §6.1.6.1.20 step 5: of two closest shortest digit strings, the even one (node v24's
    /// output for each value; `core::fmt` gives the upper one at an exact half). The full
    /// check is the node golden of
    /// `ep9_builtins/tests/json.rs` (`number_to_string_matches_the_node_golden`).
    #[test]
    // The literals are the exact values of the doubles (each one halfway between two digit
    // strings), longer than core::fmt's shortest form.
    #[allow(clippy::excessive_precision)]
    fn number_to_string_ties_go_to_even() {
        let cases: &[(f64, &str)] = &[
            (900719925474099.25, "900719925474099.2"),
            (0.185214996337890625, "0.18521499633789062"),
            (1.82361602783203125, "1.8236160278320312"),
            (104.471893310546875, "104.47189331054688"),
            (156512588.544921875, "156512588.54492188"),
            (1071568155368862.75, "1071568155368862.8"),
            (0.74469757080078125, "0.7446975708007812"),
            (249274520119823.375, "249274520119823.38"),
            (-900719925474099.25, "-900719925474099.2"),
            // No tie: core::fmt's digits stay.
            (0.5, "0.5"),
            (2.5e-7, "2.5e-7"),
            (9007199254740993.0, "9007199254740992"),
        ];
        for &(x, want) in cases {
            assert_eq!(number_to_string(x), want, "{x:e}");
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

    /// `string_to_number_text` of `units` as Latin-1 bytes (when every unit fits) and as
    /// UTF-16 units: both give the bits of `string_to_number(units)`.
    fn same_text_result(units: &[u16]) {
        use crate::string::JsStr;
        let want = string_to_number(units).to_bits();
        assert_eq!(
            string_to_number_text(JsStr::Utf16(units)).to_bits(),
            want,
            "UTF-16 {units:x?}"
        );
        if units.iter().all(|&c| c <= 0xFF) {
            let bytes: Vec<u8> = units.iter().map(|&c| c as u8).collect();
            assert_eq!(
                string_to_number_text(JsStr::Latin1(&bytes)).to_bits(),
                want,
                "Latin-1 {bytes:x?}"
            );
        }
    }

    /// PREALLOC K12: the no-copy form (Latin-1 bytes, or the UTF-16 units as they are) gives
    /// the same Number as `string_to_number` of the units: the grammar cases above, every
    /// Latin-1 white space and non-ASCII byte around a literal, and 200,000 seeded random
    /// texts over an alphabet of literal parts, white space and Latin-1 bytes.
    #[test]
    fn string_to_number_text_matches_the_units_form() {
        // '|' separates the cases (the first one is "").
        let fixed = "|  \n\t |42| -1.5e3 |.5|5.|+Infinity|-Infinity|Infinity|0x1F|0X1f|0b101|0B2|\
                     0o17|0O8|1e1000|-1e-400|0.1|-0|+0|9007199254740993|inf|NaN|infinity|1e|e5|.|\
                     0x|-0x10|1_0|12a|+-1|00|007|1.e5|.e5|1E+5|1e-5";
        for t in fixed.split('|') {
            same_text_result(&u(t));
        }
        // Each Latin-1 byte as a prefix, a suffix and alone: the white space set (0x09-0x0D,
        // 0x20, 0xA0) trims, every other byte (0x85, 0xFF, 0x00, ...) makes NaN.
        for c in 0u16..=0xFF {
            same_text_result(&[c]);
            same_text_result(&[c, u16::from(b'7')]);
            same_text_result(&[u16::from(b'7'), c]);
            same_text_result(&[c, u16::from(b'1'), u16::from(b'.'), u16::from(b'5'), c]);
        }
        // UTF-16 only: white space above 0xFF trims, other units make NaN.
        same_text_result(&u("\u{FEFF}7\u{3000}"));
        same_text_result(&u("\u{2028}0x10\u{2029}"));
        same_text_result(&u("1\u{100}"));
        let alphabet: Vec<u16> = "0123456789+-.eExXoObBaAfFIinNtyl_ "
            .encode_utf16()
            .chain([
                0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0xA0, 0x85, 0xFF, 0x00, 0x1680, 0xFEFF,
            ])
            .collect();
        let mut rng = XorShift(0x4B31_2D50_5245_414C);
        for _ in 0..200_000 {
            let n = (rng.next() % 12) as usize;
            let t: Vec<u16> = (0..n)
                .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
                .collect();
            same_text_result(&t);
        }
        // Mostly valid literals: white space, a sign, then a decimal (digits, point, digits,
        // exponent) or a radix integer, then white space.
        let ws: [&[u16]; 4] = [&[], &[0x20], &[0x09, 0x0A], &[0xA0]];
        let digit = |rng: &mut XorShift, radix: u64| -> u16 {
            let d = (rng.next() % radix) as u8;
            u16::from(if d < 10 { b'0' + d } else { b'a' + d - 10 })
        };
        for _ in 0..200_000 {
            let mut t: Vec<u16> = ws[(rng.next() % 4) as usize].to_vec();
            let r = rng.next();
            if r.is_multiple_of(4) {
                let (p, radix) = [(b'x', 16), (b'o', 8), (b'b', 2)][(r / 4 % 3) as usize];
                t.extend([u16::from(b'0'), u16::from(p)]);
                for _ in 0..rng.next() % 30 {
                    t.push(digit(&mut rng, radix));
                }
            } else {
                t.extend(match r / 4 % 3 {
                    0 => None,
                    1 => Some(u16::from(b'+')),
                    _ => Some(u16::from(b'-')),
                });
                for _ in 0..rng.next() % 20 {
                    t.push(digit(&mut rng, 10));
                }
                if rng.next().is_multiple_of(2) {
                    t.push(u16::from(b'.'));
                    for _ in 0..rng.next() % 20 {
                        t.push(digit(&mut rng, 10));
                    }
                }
                if rng.next().is_multiple_of(3) {
                    t.push(u16::from(b'e'));
                    if rng.next().is_multiple_of(2) {
                        t.push(u16::from(b'-'));
                    }
                    for _ in 0..rng.next() % 4 {
                        t.push(digit(&mut rng, 10));
                    }
                }
            }
            t.extend(ws[(rng.next() % 4) as usize]);
            same_text_result(&t);
        }
    }

    /// `%` and `**` reach ep9_math (the vectors moved there with the code, M0 week 4).
    #[test]
    fn remainder_and_exponent_use_ep9_math() {
        assert_eq!(fmod(-5.5, 2.0), -1.5);
        assert_eq!(fmod(0.1, 0.01), 3.469446951953614e-18);
        assert_eq!(exponentiate(2.0, 32.0), 4_294_967_296.0);
        assert_eq!(exponentiate(2.0, 0.5), std::f64::consts::SQRT_2);
        assert_eq!(
            exponentiate(f64::NEG_INFINITY, -3.0).to_bits(),
            (-0.0f64).to_bits()
        );
        // The correctly rounded core (M4-P1): 2^0.3 = 0x1.3b2c47bff8329p+0 (MPFR).
        assert_eq!(exponentiate(2.0, 0.3).to_bits(), 0x3ff3b2c47bff8329);
    }
}
