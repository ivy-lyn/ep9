//! Literal helpers: Smi checks, BigInt limbs, RegExp flags, strings and property names.

use ep9_bytecode::{BigIntConst, UnitString};
use oxc_ast::ast::{PropertyKey, RegExpFlags, TemplateElementValue};

/// A property name known at compile time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PropertyName {
    /// The property key string (`ToPropertyKey` gives the same string at run time).
    Static(UnitString),
}

/// `Some(i)` if `v` is an int32 and not -0 (the `LdaSmi` and `*Smi` forms).
pub(crate) fn as_smi(v: f64) -> Option<i32> {
    if v.fract() != 0.0 || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v) {
        return None;
    }
    if v == 0.0 && v.is_sign_negative() {
        return None;
    }
    Some(v as i32)
}

/// A BigInt literal value (base 10, no separators, no `n`) as limbs.
pub(crate) fn bigint_from_decimal(digits: &str) -> BigIntConst {
    let mut limbs: Vec<u64> = Vec::new();
    for d in digits.bytes().filter(u8::is_ascii_digit) {
        let mut carry = u128::from(d - b'0');
        for limb in &mut limbs {
            let v = u128::from(*limb) * 10 + carry;
            *limb = v as u64;
            carry = v >> 64;
        }
        if carry != 0 {
            limbs.push(carry as u64);
        }
    }
    BigIntConst::new(false, &limbs)
}

/// `CreateRegExpLiteral` flag bits: bit i is the i-th letter of `dgimsuvy`.
pub(crate) fn regexp_flags(f: RegExpFlags) -> u16 {
    let table = [
        (RegExpFlags::D, 0),
        (RegExpFlags::G, 1),
        (RegExpFlags::I, 2),
        (RegExpFlags::M, 3),
        (RegExpFlags::S, 4),
        (RegExpFlags::U, 5),
        (RegExpFlags::V, 6),
        (RegExpFlags::Y, 7),
    ];
    table
        .iter()
        .filter(|(flag, _)| f.contains(*flag))
        .fold(0u16, |acc, &(_, bit)| acc | (1 << bit))
}

/// A JS string value from its UTF-16 code units (`JSStr::encode_utf16` keeps lone
/// surrogates).
pub(crate) fn js_string(units: impl Iterator<Item = u16>) -> UnitString {
    let units: Vec<u16> = units.collect();
    UnitString::from_utf16(&units)
}

/// [`js_string`] of an oxc `JSStr`, given as its `as_str()` (`None` when it holds a lone
/// surrogate) and its `encode_utf16()`. Text with no lone surrogate goes straight through
/// [`UnitString::new`] (no `u16` copy); the other text takes the `js_string` path. Both give
/// the same `UnitString`.
pub(crate) fn js_text(text: Option<&str>, units: impl Iterator<Item = u16>) -> UnitString {
    match text {
        Some(t) => UnitString::new(t),
        None => js_string(units),
    }
}

/// The cooked string of a template element (`None` = undefined), through [`js_text`].
pub(crate) fn template_cooked(v: &TemplateElementValue<'_>) -> Option<UnitString> {
    v.cooked.map(|c| js_text(c.as_str(), c.encode_utf16()))
}

/// The TRV of a template element: line terminators CR LF and CR become LF.
pub(crate) fn template_raw(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn is_proto_key(key: &UnitString) -> bool {
    // "__proto__" is Latin-1, so its canonical form is always `Latin1` (no allocation here).
    matches!(key, UnitString::Latin1(b) if **b == *b"__proto__")
}

/// `prefix + key` (the name of an accessor function).
pub(crate) fn prefixed(prefix: &str, key: &UnitString) -> UnitString {
    let mut units: Vec<u16> = prefix.encode_utf16().collect();
    units.extend(key.to_utf16());
    UnitString::from_utf16(&units)
}

/// ToString of an integer Number below 2^53 (exact digits, as Number::toString gives).
fn small_integer_name(v: f64) -> Option<UnitString> {
    const LIMIT: f64 = 9_007_199_254_740_992.0;
    if v.fract() != 0.0 || !(0.0..LIMIT).contains(&v) {
        return None;
    }
    // -0 is "0" too.
    Some(UnitString::new(&format!("{}", v as u64)))
}

/// The property name of a non-computed key, if it is known at compile time. Numeric keys other
/// than small non-negative integers are left to `ToName` at run time.
pub(crate) fn property_name(key: &PropertyKey<'_>) -> Option<PropertyName> {
    match key {
        PropertyKey::StaticIdentifier(id) => {
            Some(PropertyName::Static(UnitString::new(id.name.as_str())))
        }
        PropertyKey::PrivateIdentifier(_) => None,
        PropertyKey::StringLiteral(s) => Some(PropertyName::Static(js_text(
            s.value.as_str(),
            s.value.encode_utf16(),
        ))),
        PropertyKey::NumericLiteral(n) => small_integer_name(n.value).map(PropertyName::Static),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smi() {
        assert_eq!(as_smi(0.0), Some(0));
        assert_eq!(as_smi(-0.0), None);
        assert_eq!(as_smi(1.5), None);
        assert_eq!(as_smi(2_147_483_647.0), Some(i32::MAX));
        assert_eq!(as_smi(2_147_483_648.0), None);
        assert_eq!(as_smi(-2_147_483_648.0), Some(i32::MIN));
        assert_eq!(as_smi(f64::NAN), None);
        assert_eq!(as_smi(f64::INFINITY), None);
    }

    #[test]
    fn bigint() {
        assert_eq!(bigint_from_decimal("0"), BigIntConst::new(false, &[]));
        assert_eq!(bigint_from_decimal("42"), BigIntConst::new(false, &[42]));
        assert_eq!(
            bigint_from_decimal("18446744073709551616"),
            BigIntConst::new(false, &[0, 1])
        );
        assert_eq!(
            bigint_from_decimal("340282366920938463463374607431768211457"),
            BigIntConst::new(false, &[1, 0, 1])
        );
    }

    #[test]
    fn flags() {
        assert_eq!(regexp_flags(RegExpFlags::D), 1);
        assert_eq!(regexp_flags(RegExpFlags::G | RegExpFlags::Y), 0b1000_0010);
        assert_eq!(regexp_flags(RegExpFlags::empty()), 0);
    }

    #[test]
    fn js_text_equals_js_string() {
        // PREALLOC K4: the `as_str` path gives the `UnitString` of the `u16` path, for ASCII,
        // Latin-1, BMP, astral and lone-surrogate literals and template cooked strings.
        use oxc_allocator::Allocator;
        use oxc_ast::ast::{Expression, Statement};
        use oxc_parser::Parser;
        use oxc_span::SourceType;
        // `0;` first: a string statement at the start would be a directive, not a literal.
        let src = r#"0; "abc"; ""; "caf\xe9"; "\u20ac"; "\u{1F600}"; "a\uD800"; "\uDC00\xff";
            `t\u00e9${0}\u{1F600}${1}\uD83D`; `plain`;"#;
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, src, SourceType::script()).parse();
        assert!(!parsed.diagnostics.has_errors(), "{src}");
        let (mut n, mut lone) = (0, 0);
        let mut check = |text: Option<&str>, units: &mut dyn Iterator<Item = u16>| {
            let units: Vec<u16> = units.collect();
            let new = js_text(text, units.iter().copied());
            assert_eq!(new, js_string(units.iter().copied()), "{units:x?}");
            assert_eq!(new, UnitString::from_utf16(&units), "{units:x?}");
            n += 1;
            lone += usize::from(text.is_none());
        };
        for st in &parsed.program.body {
            let Statement::ExpressionStatement(es) = st else {
                panic!("an expression statement");
            };
            match &es.expression {
                Expression::StringLiteral(s) => {
                    check(s.value.as_str(), &mut s.value.encode_utf16())
                }
                Expression::TemplateLiteral(t) => {
                    for q in &t.quasis {
                        let c = q.value.cooked.expect("a cooked string");
                        check(c.as_str(), &mut c.encode_utf16());
                    }
                }
                Expression::NumericLiteral(_) => {}
                e => panic!("unexpected {e:?}"),
            }
        }
        assert_eq!((n, lone), (11, 3));
    }

    #[test]
    fn proto_key() {
        assert!(is_proto_key(&UnitString::new("__proto__")));
        assert!(is_proto_key(&UnitString::from_utf16(
            &"__proto__".encode_utf16().collect::<Vec<_>>()
        )));
        assert!(!is_proto_key(&UnitString::new("__proto_")));
        assert!(!is_proto_key(&UnitString::new("__proto__x")));
        assert!(!is_proto_key(&UnitString::new("__pro\u{20ac}__")));
        assert!(!is_proto_key(&UnitString::new("")));
    }

    #[test]
    fn names() {
        assert_eq!(small_integer_name(1.0), Some(UnitString::new("1")));
        assert_eq!(small_integer_name(-0.0), Some(UnitString::new("0")));
        assert_eq!(small_integer_name(1.5), None);
        assert_eq!(small_integer_name(-1.0), None);
        assert_eq!(small_integer_name(1e300), None);
        assert_eq!(template_raw("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(
            prefixed("get ", &UnitString::new("x")),
            UnitString::new("get x")
        );
    }
}
