//! ToIntlMathematicalValue ( value ) (ECMA-402 §16.5.17) for `select` and `selectRange`.
//!
//! A local copy of the NF-CORE helper (`number_format::decimal::to_intl_mv` and its parts,
//! INTL W2 NF-CORE): PLURAL starts from the W2 skeleton, which does not have it. The W2 fix run
//! replaces this file with the NF-CORE function (the same code).
//!
//! ```text
//! Number   digits of Number::toString ("1e+21", "5e-324") -> Decimal::try_from_str (no ryu,
//!          INTL-V2-FP2)
//! BigInt   BigInt::toString -> Decimal
//! String   ToNumber (NaN, +-Infinity, +-0) then the exact decimal text (INTL-V2-D20: the
//!          trailing fraction zeros are cut: "1.500" is the value 1.5)
//! ```
//!
//! No f64 arithmetic: every digit comes from a text (H36).

use ep9_vm::{Cx, JsResult, Local, ToPrimitiveHint};
use fixed_decimal::{Decimal, Sign};

/// An Intl mathematical value: a finite value (negative zero is a zero with
/// [`Sign::Negative`]), an infinity or not-a-number.
#[derive(Clone, Debug, PartialEq)]
pub enum IntlMv {
    Finite(Decimal),
    PosInf,
    NegInf,
    NaN,
}

/// The JS StrWhiteSpaceChar code points (WhiteSpace and LineTerminator, ECMA-262 §7.2, §7.3).
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'
            | '\u{B}'
            | '\u{C}'
            | ' '
            | '\u{A0}'
            | '\u{FEFF}'
            | '\u{A}'
            | '\u{D}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// The Decimal of a finite decimal literal `s` (ASCII; an optional sign, digits with at most
/// one '.', an optional exponent with an optional sign), with no leading or trailing zeros.
/// `None` if `s` is not such a literal.
pub fn decimal_from_literal(s: &str) -> Option<Decimal> {
    let (neg, body) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int, frac) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut text = String::with_capacity(s.len() + 2);
    if neg {
        text.push('-');
    }
    text.push_str(if int.is_empty() { "0" } else { int });
    if !frac.is_empty() {
        text.push('.');
        text.push_str(frac);
    }
    if let Some(e) = exponent {
        let digits = e.strip_prefix('+').unwrap_or(e);
        let unsigned = digits.strip_prefix('-').unwrap_or(digits);
        if unsigned.is_empty() || !unsigned.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.push('e');
        text.push_str(digits);
    }
    let mut d = Decimal::try_from_str(&text).ok()?;
    d.absolute.trim_start();
    d.absolute.trim_end();
    if d.sign == Sign::Positive {
        d.sign = Sign::None;
    }
    Some(d)
}

/// The Decimal of the unsigned integer in base `radix` (2, 8 or 16) with the digits `s`.
fn decimal_from_radix(s: &str, radix: u32) -> Option<Decimal> {
    if s.is_empty() {
        return None;
    }
    // Decimal digits, least significant first.
    let mut digits: Vec<u8> = vec![0];
    for c in s.chars() {
        let mut carry = c.to_digit(radix)?;
        for d in digits.iter_mut() {
            let v = u32::from(*d) * radix + carry;
            *d = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let text: String = digits.iter().rev().map(|d| char::from(b'0' + d)).collect();
    let mut d = Decimal::try_from_str(&text).ok()?;
    d.absolute.trim_start();
    Some(d)
}

/// The Intl mathematical value of the Number `x`: its digits are the digits of
/// Number::toString(x) (INTL-V2-FP2), read by `Decimal::try_from_str`.
pub fn mv_of_number(cx: &mut Cx<'_>, x: f64) -> JsResult<IntlMv> {
    if x.is_nan() {
        return Ok(IntlMv::NaN);
    }
    if x.is_infinite() {
        return Ok(if x > 0.0 {
            IntlMv::PosInf
        } else {
            IntlMv::NegInf
        });
    }
    if x == 0.0 {
        let mut d = Decimal::from(0);
        if x.is_sign_negative() {
            d.sign = Sign::Negative;
        }
        return Ok(IntlMv::Finite(d));
    }
    let text = cx.to_rust_string(Local::number(x))?;
    Ok(decimal_from_literal(&text).map_or(IntlMv::NaN, IntlMv::Finite))
}

/// ToIntlMathematicalValue ( value ) (§16.5.17).
pub fn to_intl_mv<'s>(cx: &mut Cx<'s>, value: Local<'s>) -> JsResult<IntlMv> {
    // 1. If value is a Number, return ℝ(value) (or the special values).
    if let Some(x) = value.as_number() {
        return mv_of_number(cx, x);
    }
    // 2. Let primValue be ? ToPrimitive(value, number).
    let prim = cx.to_primitive(value, ToPrimitiveHint::Number)?;
    // 3. If primValue is a BigInt, return ℝ(primValue).
    if cx.is_bigint(prim) {
        let s = cx.bigint_to_string(prim, 10)?;
        let text = cx.to_rust_string(s)?;
        return Ok(decimal_from_literal(&text).map_or(IntlMv::NaN, IntlMv::Finite));
    }
    // 4-5. If primValue is not a String, return ℝ(? ToNumber(primValue)).
    if !prim.is_string() {
        let x = cx.to_number(prim)?;
        return mv_of_number(cx, x);
    }
    // 6-11. A String: StringIntlMV of its StringNumericLiteral. ToNumber gives NaN for a text
    // that is not one, and the result of RoundMVResult for the infinities and the zeros.
    let x = cx.to_number(prim)?;
    if x.is_nan() {
        return Ok(IntlMv::NaN);
    }
    if x.is_infinite() {
        return Ok(if x > 0.0 {
            IntlMv::PosInf
        } else {
            IntlMv::NegInf
        });
    }
    let text = cx.to_rust_string(prim)?;
    let text = text.trim_matches(is_js_space);
    if x == 0.0 {
        let mut d = Decimal::from(0);
        if text.starts_with('-') {
            d.sign = Sign::Negative;
        }
        return Ok(IntlMv::Finite(d));
    }
    let lower = text.get(..2).map(str::to_ascii_lowercase);
    let d = match lower.as_deref() {
        Some("0x") => decimal_from_radix(&text[2..], 16),
        Some("0o") => decimal_from_radix(&text[2..], 8),
        Some("0b") => decimal_from_radix(&text[2..], 2),
        _ => decimal_from_literal(text),
    };
    // A text that ToNumber reads but Decimal cannot hold keeps the Number's digits.
    match d {
        Some(d) => Ok(IntlMv::Finite(d)),
        None => mv_of_number(cx, x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        let s = |t: &str| decimal_from_literal(t).map(|d| d.to_string());
        assert_eq!(s("1e+21").as_deref(), Some("1000000000000000000000"));
        assert_eq!(s("1.500").as_deref(), Some("1.5"));
        assert_eq!(s("-0.0012").as_deref(), Some("-0.0012"));
        assert_eq!(s("5.").as_deref(), Some("5"));
        assert_eq!(s(".5").as_deref(), Some("0.5"));
        assert_eq!(s("1x"), None);
        assert_eq!(
            decimal_from_radix("ff", 16)
                .map(|d| d.to_string())
                .as_deref(),
            Some("255")
        );
    }
}
