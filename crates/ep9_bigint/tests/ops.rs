//! ep9_bigint against i128 arithmetic and the spec cases (design §2.4).

use core::cmp::Ordering;

use ep9_bigint::{self as b, Big, Error, MAX_BITS, View};

/// The operations with a poll callback that never stops them.
fn pow(x: View, y: View) -> Result<Big, Error> {
    b::pow(x, y, &mut || true)
}

fn parse_literal(s: &[u8]) -> Result<Option<Big>, Error> {
    b::parse_literal(s, &mut || true)
}

fn parse_digits(s: &[u8], radix: u32, neg: bool) -> Result<Option<Big>, Error> {
    b::parse_digits(s, radix, neg, &mut || true)
}

fn to_string_radix(x: View, radix: u32) -> String {
    b::to_string_radix(x, radix, &mut || true).unwrap()
}

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
    parse_literal(s.as_bytes()).unwrap().unwrap()
}

fn text(x: &Big) -> String {
    to_string_radix(x.view(), 10)
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
            assert_eq!(
                int(&b::and(bx.view(), by.view()).unwrap()),
                x & y,
                "{x} & {y}"
            );
            assert_eq!(
                int(&b::or(bx.view(), by.view()).unwrap()),
                x | y,
                "{x} | {y}"
            );
            assert_eq!(
                int(&b::xor(bx.view(), by.view()).unwrap()),
                x ^ y,
                "{x} ^ {y}"
            );
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
    assert_eq!(int(&b::and(big(-5).view(), big(3).view()).unwrap()), 3);
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
    assert_eq!(int(&pow(View::ZERO, View::ZERO).unwrap()), 1);
    assert_eq!(int(&pow(big(-1).view(), big(3).view()).unwrap()), -1);
    assert_eq!(int(&pow(big(-1).view(), big(4).view()).unwrap()), 1);
    assert_eq!(int(&pow(big(-3).view(), big(3).view()).unwrap()), -27);
    assert_eq!(pow(big(2).view(), big(-1).view()), Err(Error::NegExponent));
    assert_eq!(
        int(&pow(
            big(1).view(),
            parse("123456789012345678901234567890").view()
        )
        .unwrap()),
        1
    );
    assert_eq!(
        text(&pow(big(2).view(), big(100).view()).unwrap()),
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
        assert_eq!(parse_literal(s.as_bytes()), Ok(None), "{s:?}");
    }
    // Leading zeros do not count toward the size check. (A string past MAX_BITS needs 2^28
    // hex digits or more: too large for a unit test; the check is one compare.)
    let zeros = format!("{}1", "0".repeat(2_000_000));
    assert_eq!(text(&parse(&zeros)), "1");
}

#[test]
fn to_string_radixes() {
    assert_eq!(to_string_radix(big(255).view(), 16), "ff");
    assert_eq!(to_string_radix(big(-255).view(), 2), "-11111111");
    assert_eq!(to_string_radix(big(35).view(), 36), "z");
    assert_eq!(to_string_radix(View::ZERO, 7), "0");
    let x = parse("-123456789012345678901234567890");
    for r in 2..=36u32 {
        let s = to_string_radix(x.view(), r);
        assert!(
            s.len() as u64 <= b::string_len_bound(x.view(), r),
            "radix {r}"
        );
        let back = parse_digits(&s.as_bytes()[1..], r, true).unwrap().unwrap();
        assert_eq!(back, x, "radix {r}");
    }
    // The bound is tight enough: radix 2 of a 1000-bit value.
    let p = pow2(999);
    assert_eq!(b::string_len_bound(p.view(), 2), 1001);
    assert_eq!(to_string_radix(p.view(), 2).len(), 1000);
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
        pow(big(2).view(), big(i128::from(MAX_BITS)).view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        pow(big(2).view(), parse("99999999999999999999999").view()),
        Err(Error::TooBig)
    );
    assert_eq!(
        pow(big(2).view(), big(i128::from(MAX_BITS) - 1).view()).unwrap(),
        top
    );
    // `~`, `^` and `&` can give one bit more than the larger operand (Codex r1 B1).
    let ones = b::as_uint_n(big(-1).view(), MAX_BITS).unwrap(); // MAX_BITS bits
    assert_eq!(ones.view().bit_len(), MAX_BITS);
    assert_eq!(b::not(ones.view()), Err(Error::TooBig));
    assert_eq!(b::xor(ones.view(), big(-1).view()), Err(Error::TooBig));
    assert_eq!(b::xor(ones.view(), View::ZERO).unwrap(), ones);
    let neg_ones = b::neg(ones.view()); // -(2^MAX - 1)
    let neg_top = b::neg(top.view()); // -2^(MAX-1)
    assert_eq!(b::and(neg_ones.view(), neg_top.view()), Err(Error::TooBig));
    assert!(
        b::or(neg_ones.view(), neg_top.view())
            .unwrap()
            .view()
            .bit_len()
            <= MAX_BITS
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

// ---- long work: the divide-and-conquer paths and the poll callback (MC-SPEC-BIGPOLL) ---------

/// `"1"` and `n` zeros, and `n` nines.
fn one_zeros(n: usize) -> String {
    format!("1{}", "0".repeat(n))
}

#[test]
fn long_parse_and_to_string_give_the_exact_digits() {
    // Lengths around the one-call sizes (PARSE_LEAF = 8192 chars, to_string 1024 digits of
    // 64 bits) and above them (leaves of 16 words of digits: 304 decimal digits), so every
    // level pads its low part.
    for n in [
        255usize, 256, 257, 1023, 1024, 1025, 8191, 8192, 8193, 16385, 20000, 65537, 100_000,
    ] {
        for (r, digit) in [(10u32, '9'), (3, '2'), (7, '6'), (36, 'z')] {
            // r^n in radix r is "1" and n zeros; r^n - 1 is n top digits.
            let rn = pow(big(i128::from(r)).view(), big(n as i128).view()).unwrap();
            assert_eq!(
                to_string_radix(rn.view(), r),
                one_zeros(n),
                "radix {r}, n {n}"
            );
            let all = b::sub(rn.view(), big(1).view()).unwrap();
            let top: String = core::iter::repeat_n(digit, n).collect();
            assert_eq!(to_string_radix(all.view(), r), top, "radix {r}, n {n}");
            assert_eq!(
                parse_digits(one_zeros(n).as_bytes(), r, false).unwrap(),
                Some(rn)
            );
            assert_eq!(
                parse_digits(top.as_bytes(), r, true).unwrap(),
                Some(b::neg(all.view()))
            );
        }
    }
    // A value with mixed digits: the round trip through each radix, and the decimal text
    // against the radix-16 text (one is divide and conquer, the other digit by digit).
    let mut x = parse("12345678901234567890123456789");
    for _ in 0..12 {
        let sq = b::mul(x.view(), x.view()).unwrap();
        x = b::add(sq.view(), big(987654321).view()).unwrap();
    }
    assert!(x.digits.len() > 4 * b::STRING_LEAF_WORDS);
    for r in 2..=36u32 {
        let s = to_string_radix(x.view(), r);
        assert_eq!(
            parse_digits(s.as_bytes(), r, false).unwrap(),
            Some(x.clone()),
            "radix {r}"
        );
    }
    let hex = to_string_radix(x.view(), 16);
    let dec = to_string_radix(x.view(), 10);
    assert_eq!(
        parse_digits(hex.as_bytes(), 16, false).unwrap().unwrap(),
        parse(&dec)
    );
}

/// A long operation run with a poll callback.
type LongOp<'a> = &'a dyn Fn(&mut dyn FnMut() -> bool) -> Result<(), Error>;

/// A poll callback that counts its calls and says stop at call `stop` (never: `u64::MAX`).
fn counter(stop: u64) -> (std::rc::Rc<std::cell::Cell<u64>>, impl FnMut() -> bool) {
    let n = std::rc::Rc::new(std::cell::Cell::new(0));
    let m = n.clone();
    (n, move || {
        m.set(m.get() + 1);
        m.get() < stop
    })
}

#[test]
fn small_work_does_not_poll() {
    let (n, mut poll) = counter(u64::MAX);
    let x = b::pow(big(3).view(), big(1000).view(), &mut poll).unwrap();
    b::to_string_radix(x.view(), 10, &mut poll).unwrap();
    b::to_string_radix(x.view(), 2, &mut poll).unwrap();
    b::parse_literal("123456789".as_bytes(), &mut poll).unwrap();
    assert_eq!(n.get(), 0);
}

#[test]
fn long_work_polls_once_per_poll_words() {
    // 2^20 bits = 16,384 digits: toString(2) reads 256 polls of 64 digits.
    let x = b::sub(pow2(1 << 20).view(), big(1).view()).unwrap();
    let (n, mut poll) = counter(u64::MAX);
    let s = b::to_string_radix(x.view(), 2, &mut poll).unwrap();
    assert_eq!(s.len(), 1 << 20);
    assert_eq!(n.get(), (1 << 20) / (64 * b::POLL_WORDS));
    // The other long paths poll too, at least once per POLL_WORDS digits they make or read.
    for f in [
        &|p: &mut dyn FnMut() -> bool| b::to_string_radix(x.view(), 10, p).map(|_| ()),
        &|p: &mut dyn FnMut() -> bool| b::pow(big(3).view(), big(400_000).view(), p).map(|_| ()),
        &|p: &mut dyn FnMut() -> bool| {
            b::parse_literal("7".repeat(100_000).as_bytes(), p).map(|_| ())
        },
    ] as [LongOp; 3]
    {
        let (n, mut poll) = counter(u64::MAX);
        f(&mut poll).unwrap();
        assert!(n.get() >= 5_000 / b::POLL_WORDS, "{}", n.get());
    }
}

#[test]
fn a_false_poll_stops_long_work() {
    let x = b::sub(pow2(1 << 20).view(), big(1).view()).unwrap();
    let nines = "9".repeat(100_000);
    let cases: [LongOp; 6] = [
        &|p| b::to_string_radix(x.view(), 2, p).map(|_| ()),
        &|p| b::to_string_radix(x.view(), 8, p).map(|_| ()),
        &|p| b::to_string_radix(x.view(), 10, p).map(|_| ()),
        &|p| b::pow(big(3).view(), big(400_000).view(), p).map(|_| ()),
        &|p| b::parse_literal(nines.as_bytes(), p).map(|_| ()),
        &|p| b::parse_digits(nines.as_bytes(), 36, false, p).map(|_| ()),
    ];
    for (i, f) in cases.iter().enumerate() {
        let (n, mut poll) = counter(3);
        assert_eq!(f(&mut poll), Err(Error::Interrupted), "case {i}");
        // It stops at the call that said stop.
        assert_eq!(n.get(), 3, "case {i}");
    }
}

#[test]
fn pow_refuses_a_too_big_result_before_the_last_squares() {
    // 3^(2^30 - 1) has about 1.58 x 2^30 bits. The lower bound refuses it after a few
    // small squares (before: about 15 s of squaring, then the size check).
    let (n, mut poll) = counter(u64::MAX);
    let e = big(i128::from(MAX_BITS) - 1);
    assert_eq!(
        b::pow(big(3).view(), e.view(), &mut poll),
        Err(Error::TooBig)
    );
    assert_eq!(n.get(), 0);
    // A large base: 2^20 bits squared 2^10 times is 2^30 bits at least, refused at once.
    let x = b::add(pow2(1 << 20).view(), big(1).view()).unwrap();
    let (n, mut poll) = counter(u64::MAX);
    assert_eq!(
        b::pow(x.view(), big(1 << 10).view(), &mut poll),
        Err(Error::TooBig)
    );
    assert_eq!(n.get(), 0);
}

#[test]
fn a_parse_above_the_limit_is_refused_before_the_work() {
    // 323,228,498 decimal digits make more than 2^30 bits (10^323228497 > 2^1073741825):
    // refused from the length, before the work and with no poll.
    let (n, mut poll) = counter(u64::MAX);
    let nines = "9".repeat(323_228_498);
    assert_eq!(
        b::parse_literal(nines.as_bytes(), &mut poll),
        Err(Error::TooBig)
    );
    let hex = format!("0x1{}", "0".repeat(268_435_456)); // 2^1073741824
    assert_eq!(
        b::parse_literal(hex.as_bytes(), &mut poll),
        Err(Error::TooBig)
    );
    assert_eq!(n.get(), 0);
}
