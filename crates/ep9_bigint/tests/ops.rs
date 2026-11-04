//! ep9_bigint against i128 arithmetic and the spec cases (design §2.4).

use core::cmp::Ordering;

use ep9_bigint::{self as b, Big, Error, MAX_BITS, View};

fn big(n: i128) -> Big {
    b::from_i128(n)
}

fn int(x: &Big) -> i128 {
    b::to_i128(x.view()).expect("fits i128")
}

fn pow2(k: u64) -> Big {
    b::shl(big(1).view(), big(i128::from(k)).view()).unwrap()
}

fn parse(s: &str) -> Big {
    b::parse_literal(s.as_bytes()).unwrap().unwrap()
}

fn text(x: &Big) -> String {
    b::to_string_radix(x.view(), 10)
}

/// Small values and values near the 64-bit and 63-bit edges.
fn samples() -> Vec<i128> {
    let mut v = vec![0, 1, 2, 3, 5, 7, 64, 255, 256, 1000, 12345];
    for e in [31u32, 32, 53, 62, 63, 64, 65, 100] {
        let p = 1i128 << e;
        v.extend([p - 1, p, p + 1]);
    }
    let neg: Vec<i128> = v.iter().map(|x| -x).collect();
    v.extend(neg);
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn arithmetic_matches_i128() {
    let s = samples();
    for &x in &s {
        for &y in &s {
            let (bx, by) = (big(x), big(y));
            assert_eq!(int(&b::add(bx.view(), by.view()).unwrap()), x + y);
            assert_eq!(int(&b::sub(bx.view(), by.view()).unwrap()), x - y);
            if x.unsigned_abs() < 1 << 62 && y.unsigned_abs() < 1 << 62 {
                assert_eq!(int(&b::mul(bx.view(), by.view()).unwrap()), x * y);
            }
            if y != 0 {
                // Rust's / and % truncate, as the spec's BigInt::divide and ::remainder.
                assert_eq!(int(&b::div_trunc(bx.view(), by.view()).unwrap()), x / y);
                assert_eq!(int(&b::rem_trunc(bx.view(), by.view()).unwrap()), x % y);
            }
            assert_eq!(int(&b::and(bx.view(), by.view())), x & y, "{x} & {y}");
            assert_eq!(int(&b::or(bx.view(), by.view())), x | y, "{x} | {y}");
            assert_eq!(int(&b::xor(bx.view(), by.view())), x ^ y, "{x} ^ {y}");
            assert_eq!(b::cmp(bx.view(), by.view()), x.cmp(&y));
        }
        assert_eq!(int(&b::neg(big(x).view())), -x);
        assert_eq!(int(&b::not(big(x).view()).unwrap()), !x);
        for sh in [0i128, 1, 3, 63, 64, 65, 120, 200] {
            // >> rounds toward -infinity (Rust's >> on i128 is arithmetic: floor).
            let want = if sh >= 127 {
                if x < 0 { -1 } else { 0 }
            } else {
                x >> sh
            };
            assert_eq!(int(&b::shr(big(x).view(), big(sh).view()).unwrap()), want);
            // A negative count shifts the other way.
            assert_eq!(int(&b::shl(big(x).view(), big(-sh).view()).unwrap()), want);
        }
    }
}

#[test]
fn spec_cases() {
    // design §2.4: two's complement of negatives, >> floor, / and % signs.
    assert_eq!(int(&b::and(big(-5).view(), big(3).view())), 3);
    assert_eq!(int(&b::shr(big(-5).view(), big(1).view()).unwrap()), -3);
    assert_eq!(
        int(&b::div_trunc(big(-7).view(), big(2).view()).unwrap()),
        -3
    );
    assert_eq!(
        int(&b::rem_trunc(big(-7).view(), big(2).view()).unwrap()),
        -1
    );
    assert_eq!(
        int(&b::rem_trunc(big(7).view(), big(-2).view()).unwrap()),
        1
    );
    assert_eq!(
        b::div_trunc(big(1).view(), View::ZERO),
        Err(Error::DivByZero)
    );
    assert_eq!(
        b::rem_trunc(big(1).view(), View::ZERO),
        Err(Error::DivByZero)
    );
    // -0n does not exist.
    let z = b::neg(View::ZERO);
    assert!(!z.neg && z.digits.is_empty());
    let z = b::rem_trunc(big(-4).view(), big(2).view()).unwrap();
    assert!(!z.neg && z.digits.is_empty());
    // ** rules.
    assert_eq!(int(&b::pow(View::ZERO, View::ZERO).unwrap()), 1);
    assert_eq!(int(&b::pow(big(-1).view(), big(3).view()).unwrap()), -1);
    assert_eq!(int(&b::pow(big(-1).view(), big(4).view()).unwrap()), 1);
    assert_eq!(int(&b::pow(big(-3).view(), big(3).view()).unwrap()), -27);
    assert_eq!(
        b::pow(big(2).view(), big(-1).view()),
        Err(Error::NegExponent)
    );
    assert_eq!(
        int(&b::pow(
            big(1).view(),
            parse("123456789012345678901234567890").view()
        )
        .unwrap()),
        1
    );
    assert_eq!(
        text(&b::pow(big(2).view(), big(100).view()).unwrap()),
        "1267650600228229401496703205376"
    );
    // asIntN, asUintN.
    let p63 = pow2(63);
    assert_eq!(b::as_int_n(p63.view(), 64).unwrap(), b::neg(p63.view()));
    assert_eq!(
        int(&b::as_uint_n(big(-1).view(), 64).unwrap()),
        i128::from(u64::MAX)
    );
    assert_eq!(int(&b::as_int_n(big(255).view(), 8).unwrap()), -1);
    assert_eq!(int(&b::as_int_n(big(128).view(), 8).unwrap()), -128);
    assert_eq!(int(&b::as_int_n(big(-128).view(), 8).unwrap()), -128);
    assert_eq!(int(&b::as_int_n(big(-129).view(), 8).unwrap()), 127);
    assert_eq!(int(&b::as_int_n(big(127).view(), 8).unwrap()), 127);
    assert_eq!(int(&b::as_int_n(big(5).view(), 0).unwrap()), 0);
    assert_eq!(int(&b::as_uint_n(big(-5).view(), 0).unwrap()), 0);
    assert_eq!(int(&b::as_uint_n(big(-5).view(), 3).unwrap()), 3);
    assert_eq!(int(&b::as_uint_n(big(257).view(), 8).unwrap()), 1);
    assert_eq!(int(&b::as_int_n(big(-1).view(), 1).unwrap()), -1);
    assert_eq!(int(&b::as_uint_n(big(-1).view(), 1).unwrap()), 1);
    assert_eq!(int(&b::as_int_n(big(1).view(), 1).unwrap()), -1);
    // asIntN with a huge bit count keeps the value; asUintN of a negative needs the bits.
    assert_eq!(
        int(&b::as_int_n(big(-9).view(), (1 << 53) - 1).unwrap()),
        -9
    );
    assert_eq!(int(&b::as_uint_n(big(9).view(), (1 << 53) - 1).unwrap()), 9);
    assert_eq!(
        b::as_uint_n(big(-9).view(), MAX_BITS + 1),
        Err(Error::TooBig)
    );
    // Wider than i128.
    let x = parse("-340282366920938463463374607431768211457"); // -(2^128 + 1)
    assert_eq!(
        text(&b::as_uint_n(x.view(), 130).unwrap()),
        "1020847100762815390390123822295304634367" // 2^130 - 2^128 - 1
    );
    assert_eq!(
        text(&b::as_int_n(x.view(), 130).unwrap()),
        "-340282366920938463463374607431768211457"
    );
    assert_eq!(
        text(&b::as_int_n(x.view(), 129).unwrap()),
        "340282366920938463463374607431768211455"
    );
}

#[test]
fn parse_every_form() {
    let ok = [
        ("", "0"),
        ("0", "0"),
        ("-0", "0"),
        ("+12", "12"),
        ("-12", "-12"),
        ("000123", "123"),
        ("0x1F", "31"),
        ("0XfF", "255"),
        ("0o17", "15"),
        ("0O7", "7"),
        ("0b101", "5"),
        ("0B0", "0"),
        ("0x0000", "0"),
        (
            "123456789012345678901234567890",
            "123456789012345678901234567890",
        ),
    ];
    for (s, want) in ok {
        assert_eq!(text(&parse(s)), want, "{s:?}");
    }
    let bad = [
        "-", "+", "0x", "0b", "0o", "-0x1", "+0x1", "1n", "1_0", "1.0", "1e3", "0x1g", "0b2",
        "0o8", " 1", "1 ", "Infinity", "--1", "0xx1", "١",
    ];
    for s in bad {
        assert_eq!(b::parse_literal(s.as_bytes()), Ok(None), "{s:?}");
    }
    // Leading zeros do not count toward the size check. (A string past MAX_BITS needs 2^28
    // hex digits or more: too large for a unit test; the check is one compare.)
    let zeros = format!("{}1", "0".repeat(2_000_000));
    assert_eq!(text(&parse(&zeros)), "1");
}

#[test]
fn to_string_radixes() {
    assert_eq!(b::to_string_radix(big(255).view(), 16), "ff");
    assert_eq!(b::to_string_radix(big(-255).view(), 2), "-11111111");
    assert_eq!(b::to_string_radix(big(35).view(), 36), "z");
    assert_eq!(b::to_string_radix(View::ZERO, 7), "0");
    let x = parse("-123456789012345678901234567890");
    for r in 2..=36u32 {
        let s = b::to_string_radix(x.view(), r);
        assert!(
            s.len() as u64 <= b::string_len_bound(x.view(), r),
            "radix {r}"
        );
        let back = b::parse_digits(&s.as_bytes()[1..], r, true)
            .unwrap()
            .unwrap();
        assert_eq!(back, x, "radix {r}");
    }
    // The bound is tight enough: radix 2 of a 1000-bit value.
    let p = pow2(999);
    assert_eq!(b::string_len_bound(p.view(), 2), 1001);
    assert_eq!(b::to_string_radix(p.view(), 2).len(), 1000);
}

#[test]
fn to_f64_rounds_to_nearest_even() {
    let cases = [
        ("9007199254740993", 9007199254740992.0), // 2^53 + 1: tie, to even
        ("9007199254740995", 9007199254740996.0), // 2^53 + 3: tie, to even (up)
        ("-9007199254740993", -9007199254740992.0),
        ("18446744073709551615", 18446744073709551616.0), // 2^64 - 1
        ("18446744073709553664", 18446744073709551616.0), // 2^64 + 2^11: tie, to even
        ("18446744073709553665", 18446744073709555712.0), // just above the tie
        ("18446744073709555712", 18446744073709555712.0), // exact
        ("36893488147419107328", 36893488147419103232.0), // 2^65 + 2^12: tie, even down
        ("36893488147419111424", 36893488147419111424.0), // 2^65 + 2*2^12: exact
        ("36893488147419115520", 36893488147419119616.0), // 2^65 + 3*2^12: tie, even up
    ];
    for (s, want) in cases {
        assert_eq!(b::to_f64(parse(s).view()), want, "{s}");
    }
    // A sticky bit far below the top 64 bits breaks a tie upward.
    let tie = b::add(pow2(200).view(), pow2(200 - 53).view()).unwrap();
    assert_eq!(b::to_f64(tie.view()), f64::from_bits((1023 + 200) << 52));
    let above = b::add(tie.view(), big(1).view()).unwrap();
    assert_eq!(
        b::to_f64(above.view()),
        f64::from_bits(((1023 + 200) << 52) | 1)
    );
    // The top of the f64 range: MAX, the tie to 2^1024 (Infinity), past it.
    let max = b::from_f64_exact(f64::MAX).unwrap();
    assert_eq!(b::to_f64(max.view()), f64::MAX);
    let half_ulp = pow2(1024 - 54);
    let tie = b::add(max.view(), half_ulp.view()).unwrap();
    assert_eq!(b::to_f64(tie.view()), f64::INFINITY);
    let below = b::sub(tie.view(), big(1).view()).unwrap();
    assert_eq!(b::to_f64(below.view()), f64::MAX);
    assert_eq!(b::to_f64(pow2(1024).view()), f64::INFINITY);
    assert_eq!(
        b::to_f64(b::neg(pow2(1100).view()).view()),
        f64::NEG_INFINITY
    );
    assert_eq!(b::to_f64(View::ZERO).to_bits(), 0);
}

#[test]
fn f64_compare_and_exact() {
    use Ordering::*;
    let one = big(1);
    assert_eq!(b::cmp_f64(one.view(), f64::NAN), None);
    assert_eq!(b::cmp_f64(one.view(), f64::INFINITY), Some(Less));
    assert_eq!(b::cmp_f64(one.view(), f64::NEG_INFINITY), Some(Greater));
    assert_eq!(b::cmp_f64(one.view(), 1.0), Some(Equal));
    assert_eq!(b::cmp_f64(one.view(), 1.5), Some(Less));
    assert_eq!(b::cmp_f64(one.view(), 0.5), Some(Greater));
    assert_eq!(b::cmp_f64(big(-1).view(), -1.5), Some(Greater));
    assert_eq!(b::cmp_f64(big(-1).view(), -0.5), Some(Less));
    assert_eq!(b::cmp_f64(View::ZERO, -0.0), Some(Equal));
    assert_eq!(b::cmp_f64(View::ZERO, 5e-324), Some(Less));
    assert_eq!(b::cmp_f64(View::ZERO, -5e-324), Some(Greater));
    // 2^53 + 1 is not a double: exact compare, no rounding.
    let x = parse("9007199254740993");
    assert_eq!(b::cmp_f64(x.view(), 9007199254740992.0), Some(Greater));
    assert_eq!(b::cmp_f64(x.view(), 9007199254740994.0), Some(Less));
    assert_eq!(b::from_f64_exact(0.5), None);
    assert_eq!(b::from_f64_exact(f64::NAN), None);
    assert_eq!(b::from_f64_exact(f64::INFINITY), None);
    assert_eq!(b::from_f64_exact(-0.0), Some(Big::zero()));
    assert_eq!(
        int(&b::from_f64_exact(-1e20).unwrap()),
        -100000000000000000000
    );
    assert_eq!(text(&b::from_f64_exact(1e300).unwrap()).len(), 301);
}

#[test]
fn size_limit_edges() {
    let top = pow2(MAX_BITS - 1); // MAX_BITS bits
    assert_eq!(top.view().bit_len(), MAX_BITS);
    assert_eq!(
        b::shl(big(1).view(), big(i128::from(MAX_BITS)).view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        b::shl(big(1).view(), parse("100000000000000000000000").view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        int(&b::shl(View::ZERO, parse("100000000000000000000000").view()).unwrap()),
        0
    );
    assert_eq!(b::add(top.view(), top.view()), Err(Error::TooBig));
    assert_eq!(b::mul(top.view(), big(2).view()), Err(Error::TooBig));
    assert_eq!(b::mul(top.view(), big(1).view()).unwrap(), top);
    assert_eq!(
        b::pow(big(2).view(), big(i128::from(MAX_BITS)).view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        b::pow(big(2).view(), parse("99999999999999999999999").view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        b::pow(big(2).view(), big(i128::from(MAX_BITS) - 1).view()).unwrap(),
        top
    );
    // A right shift by a huge count is 0n or -1n.
    let huge = parse("99999999999999999999999");
    assert_eq!(int(&b::shr(big(5).view(), huge.view()).unwrap()), 0);
    assert_eq!(int(&b::shr(big(-5).view(), huge.view()).unwrap()), -1);
    assert_eq!(
        int(&b::shl(big(-5).view(), b::neg(huge.view()).view()).unwrap()),
        -1
    );
}

#[test]
fn i128_round_trip_and_hash() {
    for x in [
        0i128,
        1,
        -1,
        i128::MAX,
        i128::MIN,
        i128::MIN + 1,
        1 << 64,
        -(1 << 64),
    ] {
        assert_eq!(b::to_i128(big(x).view()), Some(x));
    }
    let over = b::add(big(i128::MAX).view(), big(1).view()).unwrap();
    assert_eq!(b::to_i128(over.view()), None);
    assert_eq!(b::to_i128(b::neg(over.view()).view()), Some(i128::MIN));
    let under = b::sub(big(i128::MIN).view(), big(1).view()).unwrap();
    assert_eq!(b::to_i128(under.view()), None);
    assert_eq!(b::from_i64(-7), big(-7));
    // Equal values, equal hashes; the sign and the seed take part.
    let a = parse("123456789012345678901234567890");
    let c = parse("123456789012345678901234567890");
    assert_eq!(b::hash(a.view(), 9), b::hash(c.view(), 9));
    assert_ne!(b::hash(a.view(), 9), b::hash(b::neg(a.view()).view(), 9));
    assert_ne!(b::hash(a.view(), 9), b::hash(a.view(), 10));
}
