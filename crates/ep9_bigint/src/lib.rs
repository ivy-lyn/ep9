//! BigInt digit arithmetic (ECMA-262 §6.1.6.2) on digit slices.
//!
//! ```text
//!   ep9_vm::bigint (BIGINT cells, kind 9) --View (sign, &[u64])--> ep9_bigint --Big--> new cell
//!                                                                     |
//!                                                                dashu-int 0.6.2
//! ```
//!
//! Only `ep9_vm` depends on this crate (`.github/scripts/dep-guards.sh` check 17). The API takes
//! and gives digit slices (64-bit digits, least significant first, no zero top digit), so the
//! M8 port of V8 `src/bigint` (plan 06 §5) replaces only this crate.
//!
//! | Rule | Where |
//! |---|---|
//! | Values are canonical: no zero top digit; zero is no digits and `neg = false` (no `-0n`) | [`View`], [`Big`] |
//! | A result above [`MAX_BITS`] bits is [`Error::TooBig`]; the check runs before the work for `*`, `**`, `<<`, parse and `asUintN` | each operation |
//! | Signed operations follow the spec: `/` and `%` truncate, `>>` rounds toward -infinity, `&` `\|` `^` `~` act on infinite two's complement | [`div_trunc`], [`shr`], [`and`] |
//! | No float math except [`to_f64`], [`cmp_f64`] and [`from_f64_exact`], which use bit operations and the u64 -> f64 conversion (correctly rounded) only (H36) | the `f64` section |
#![forbid(unsafe_code)]

use core::cmp::Ordering;

use dashu_int::{IBig, Sign, UBig, Word};

// The digit slices are `&[u64]`, which is `&[Word]` only on 64-bit targets (all ep9 targets).
const _: () = assert!(core::mem::size_of::<Word>() == 8);

/// The largest BigInt in bits (V8 `BigInt::kMaxLengthBits`). A larger result is a RangeError
/// "Maximum BigInt size exceeded" (H23 "BigInt size"). The input of an operation may be larger
/// (a literal from a unit is not checked).
pub const MAX_BITS: u64 = 1 << 30;

/// A borrowed canonical BigInt: sign and magnitude digits (u64, least significant first, no
/// zero top digit). Zero is `neg = false` and no digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View<'a> {
    pub neg: bool,
    pub digits: &'a [u64],
}

/// An owned canonical BigInt (the same rules as [`View`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Big {
    pub neg: bool,
    pub digits: Vec<u64>,
}

/// Why an operation has no result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The result would be above [`MAX_BITS`] bits: RangeError "Maximum BigInt size exceeded".
    TooBig,
    /// `/` or `%` by `0n`: RangeError "Division by zero".
    DivByZero,
    /// `**` with a negative exponent: RangeError "Exponent must be non-negative".
    NegExponent,
}

impl<'a> View<'a> {
    /// A view of `digits` with the sign `neg`. Debug-checks that it is canonical.
    pub fn new(neg: bool, digits: &'a [u64]) -> View<'a> {
        debug_assert!(digits.last() != Some(&0), "a zero top digit");
        debug_assert!(!(neg && digits.is_empty()), "-0n");
        View { neg, digits }
    }

    pub const ZERO: View<'static> = View {
        neg: false,
        digits: &[],
    };

    pub fn is_zero(self) -> bool {
        self.digits.is_empty()
    }

    /// The bit length of the magnitude (0 for `0n`).
    pub fn bit_len(self) -> u64 {
        bit_len(self.digits)
    }

    pub fn to_big(self) -> Big {
        Big {
            neg: self.neg,
            digits: self.digits.to_vec(),
        }
    }

    fn ibig(self) -> IBig {
        IBig::from_sign_words(sign_of(self.neg), self.digits)
    }

    fn ubig(self) -> UBig {
        UBig::from_words(self.digits)
    }

    /// The magnitude as a u64, if it fits.
    fn small_magnitude(self) -> Option<u64> {
        match self.digits {
            [] => Some(0),
            [d] => Some(*d),
            _ => None,
        }
    }
}

impl Big {
    pub fn zero() -> Big {
        Big::default()
    }

    pub fn view(&self) -> View<'_> {
        View::new(self.neg, &self.digits)
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Makes a Big from any sign and digits: strips zero top digits, `-0` becomes `0`.
    pub fn from_parts(neg: bool, mut digits: Vec<u64>) -> Big {
        while digits.last() == Some(&0) {
            digits.pop();
        }
        let neg = neg && !digits.is_empty();
        Big { neg, digits }
    }

    fn from_ibig(x: &IBig) -> Big {
        let (sign, words) = x.as_sign_words();
        Big::from_parts(sign == Sign::Negative, words.to_vec())
    }

    fn from_ubig(neg: bool, x: &UBig) -> Big {
        Big::from_parts(neg, x.as_words().to_vec())
    }
}

fn sign_of(neg: bool) -> Sign {
    if neg { Sign::Negative } else { Sign::Positive }
}

fn bit_len(digits: &[u64]) -> u64 {
    match digits.last() {
        None => 0,
        Some(top) => 64 * (digits.len() as u64 - 1) + u64::from(64 - top.leading_zeros()),
    }
}

/// `Ok(b)` if `b` has at most [`MAX_BITS`] bits.
fn checked(b: Big) -> Result<Big, Error> {
    if bit_len(&b.digits) > MAX_BITS {
        Err(Error::TooBig)
    } else {
        Ok(b)
    }
}

/// `1n` with the sign `neg` (`-1n` when `neg`).
fn one(neg: bool) -> Big {
    Big {
        neg,
        digits: vec![1],
    }
}

// ---- arithmetic (§6.1.6.2.1-§6.1.6.2.12) -----------------------------------------------------

/// BigInt::unaryMinus. `-0n` is `0n`.
pub fn neg(a: View) -> Big {
    Big::from_parts(!a.neg, a.digits.to_vec())
}

/// BigInt::bitwiseNOT: `-a - 1n`.
pub fn not(a: View) -> Result<Big, Error> {
    checked(Big::from_ibig(&(-a.ibig() - IBig::ONE)))
}

/// BigInt::add.
pub fn add(a: View, b: View) -> Result<Big, Error> {
    checked(Big::from_ibig(&(a.ibig() + b.ibig())))
}

/// BigInt::subtract.
pub fn sub(a: View, b: View) -> Result<Big, Error> {
    checked(Big::from_ibig(&(a.ibig() - b.ibig())))
}

/// BigInt::multiply. Size check first: the product has at least `bits(a) + bits(b) - 1` bits.
pub fn mul(a: View, b: View) -> Result<Big, Error> {
    if a.is_zero() || b.is_zero() {
        return Ok(Big::zero());
    }
    if a.bit_len() + b.bit_len() - 1 > MAX_BITS {
        return Err(Error::TooBig);
    }
    checked(Big::from_ibig(&(a.ibig() * b.ibig())))
}

/// BigInt::divide: the quotient truncated toward zero. `b = 0n` is [`Error::DivByZero`].
pub fn div_trunc(a: View, b: View) -> Result<Big, Error> {
    if b.is_zero() {
        return Err(Error::DivByZero);
    }
    let (q, _) = div_rem_mag(a, b);
    Ok(Big::from_ubig(a.neg != b.neg, &q))
}

/// BigInt::remainder: the remainder of the truncated division; it has the sign of `a`.
pub fn rem_trunc(a: View, b: View) -> Result<Big, Error> {
    if b.is_zero() {
        return Err(Error::DivByZero);
    }
    let (_, r) = div_rem_mag(a, b);
    Ok(Big::from_ubig(a.neg, &r))
}

/// Quotient and remainder of the magnitudes.
fn div_rem_mag(a: View, b: View) -> (UBig, UBig) {
    let (x, y) = (a.ubig(), b.ubig());
    let q = &x / &y;
    let r = x - &q * &y;
    (q, r)
}

/// BigInt::exponentiate. A negative `exp` is [`Error::NegExponent`]; `0n ** 0n` is `1n`.
pub fn pow(base: View, exp: View) -> Result<Big, Error> {
    if exp.neg {
        return Err(Error::NegExponent);
    }
    if exp.is_zero() {
        return Ok(one(false));
    }
    if base.is_zero() {
        return Ok(Big::zero());
    }
    // An odd exponent keeps the sign of the base.
    let odd = exp.digits[0] & 1 == 1;
    if base.digits == [1] {
        return Ok(one(base.neg && odd));
    }
    // |base| >= 2, so the result has more than `(bits(base) - 1) * exp` bits.
    let e = match exp.small_magnitude() {
        Some(e) if e <= MAX_BITS => e,
        _ => return Err(Error::TooBig),
    };
    if (base.bit_len() - 1).saturating_mul(e) >= MAX_BITS {
        return Err(Error::TooBig);
    }
    let m = base.ubig().pow(e as usize);
    checked(Big::from_ubig(base.neg && odd, &m))
}

// ---- shifts (§6.1.6.2.9-§6.1.6.2.11) ---------------------------------------------------------

/// BigInt::leftShift: `a * 2^n`; a negative `n` shifts right.
pub fn shl(a: View, n: View) -> Result<Big, Error> {
    if n.neg {
        Ok(shr_mag(a, n))
    } else {
        shl_mag(a, n)
    }
}

/// BigInt::signedRightShift: `floor(a / 2^n)`; a negative `n` shifts left.
pub fn shr(a: View, n: View) -> Result<Big, Error> {
    if n.neg {
        shl_mag(a, n)
    } else {
        Ok(shr_mag(a, n))
    }
}

/// `a << |n|`.
fn shl_mag(a: View, n: View) -> Result<Big, Error> {
    if a.is_zero() {
        return Ok(Big::zero());
    }
    let s = match n.small_magnitude() {
        Some(s) if s <= MAX_BITS => s,
        _ => return Err(Error::TooBig),
    };
    if a.bit_len() + s > MAX_BITS {
        return Err(Error::TooBig);
    }
    let m = a.ubig() << s as usize;
    checked(Big::from_ubig(a.neg, &m))
}

/// `floor(a / 2^|n|)`.
fn shr_mag(a: View, n: View) -> Big {
    let bits = a.bit_len();
    let s = match n.small_magnitude() {
        Some(s) if s < bits => s,
        // Every bit goes: 0n, or -1n for a negative `a` (floor).
        _ => return if a.neg { one(true) } else { Big::zero() },
    };
    let q = a.ubig() >> s as usize;
    if a.neg {
        // floor(-m / 2^s) = -ceil(m / 2^s).
        let q = if any_bit_below(a.digits, s) {
            q + UBig::ONE
        } else {
            q
        };
        Big::from_ubig(true, &q)
    } else {
        Big::from_ubig(false, &q)
    }
}

// ---- bitwise (§6.1.6.2.18-§6.1.6.2.20: infinite two's complement) -----------------------------

/// BigInt::bitwiseAND.
pub fn and(a: View, b: View) -> Big {
    bitwise(a, b, |x, y| x & y)
}

/// BigInt::bitwiseOR.
pub fn or(a: View, b: View) -> Big {
    bitwise(a, b, |x, y| x | y)
}

/// BigInt::bitwiseXOR.
pub fn xor(a: View, b: View) -> Big {
    bitwise(a, b, |x, y| x ^ y)
}

/// Applies `op` to each 64-bit digit of the two's complement forms of `a` and `b`, with one
/// extra digit of sign extension, and reads the result back as two's complement.
fn bitwise(a: View, b: View, op: impl Fn(u64, u64) -> u64) -> Big {
    let n = a.digits.len().max(b.digits.len()) + 1;
    let x = twos(a, n);
    let y = twos(b, n);
    let r: Vec<u64> = x.iter().zip(&y).map(|(p, q)| op(*p, *q)).collect();
    from_twos(r)
}

/// The `n`-digit two's complement form of `a` (`n` is more than its digit count).
fn twos(a: View, n: usize) -> Vec<u64> {
    let mut v = a.digits.to_vec();
    v.resize(n, 0);
    if a.neg {
        negate_digits(&mut v);
    }
    v
}

/// Reads an `n`-digit two's complement form (the top bit is the sign).
fn from_twos(mut v: Vec<u64>) -> Big {
    let neg = v.last().is_some_and(|d| d >> 63 == 1);
    if neg {
        negate_digits(&mut v);
    }
    Big::from_parts(neg, v)
}

/// Two's complement negation in place: invert, add 1.
fn negate_digits(v: &mut [u64]) {
    let mut carry = true;
    for d in v.iter_mut() {
        let (s, c) = (!*d).overflowing_add(u64::from(carry));
        *d = s;
        carry = c;
    }
}

// ---- comparison (§6.1.6.2.12-§6.1.6.2.13, §7.2.13) -------------------------------------------

/// Compares two BigInts by value.
pub fn cmp(a: View, b: View) -> Ordering {
    match (a.neg, b.neg) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => cmp_mag(a.digits, b.digits),
        (true, true) => cmp_mag(b.digits, a.digits),
    }
}

fn cmp_mag(a: &[u64], b: &[u64]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

/// The value of a finite f64 split at the binary point: the integer part (truncated toward
/// zero) and whether a fraction was cut off. Bit operations only.
fn f64_trunc(x: f64) -> (Big, bool) {
    debug_assert!(x.is_finite());
    let bits = x.to_bits();
    let neg = bits >> 63 == 1;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1 << 52) - 1);
    if exp == 0 {
        // Zero or subnormal: |x| < 1.
        return (Big::zero(), frac != 0);
    }
    let mant = frac | (1 << 52);
    // x = mant * 2^(exp - 1075)
    let e = exp - 1075;
    if e >= 0 {
        let m = UBig::from(mant) << e as usize;
        (Big::from_ubig(neg, &m), false)
    } else {
        let s = -e;
        if s >= 64 {
            return (Big::zero(), true);
        }
        let int = mant >> s;
        let cut = mant & ((1u64 << s) - 1) != 0;
        (Big::from_parts(neg, vec![int]), cut)
    }
}

/// Compares a BigInt with a Number exactly (no float rounding). `None` for NaN.
pub fn cmp_f64(a: View, x: f64) -> Option<Ordering> {
    if x.is_nan() {
        return None;
    }
    if x.is_infinite() {
        return Some(if x > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let (t, cut) = f64_trunc(x);
    let c = cmp(a, t.view());
    if c != Ordering::Equal || !cut {
        return Some(c);
    }
    // a == trunc(x) and x has a fraction: x is above a when positive, below when negative.
    Some(if x > 0.0 {
        Ordering::Less
    } else {
        Ordering::Greater
    })
}

/// NumberToBigInt (§21.2.1.1.1) without the error: `None` if `x` is not an integral Number.
pub fn from_f64_exact(x: f64) -> Option<Big> {
    if !x.is_finite() {
        return None;
    }
    let (t, cut) = f64_trunc(x);
    if cut { None } else { Some(t) }
}

/// 𝔽(ℝ(a)): the Number nearest to `a`, ties to even; past the f64 range, ±Infinity.
pub fn to_f64(a: View) -> f64 {
    let bits = a.bit_len();
    let mag = if bits <= 64 {
        // The u64 -> f64 conversion rounds to nearest, ties to even.
        a.digits.first().copied().unwrap_or(0) as f64
    } else {
        // The top 64 bits, with every lower bit folded into bit 0 (a sticky bit): bits 0-10
        // are cut by the one rounding of the conversion, so the tie test stays exact.
        let shift = bits - 64;
        let top = top_bits(a.digits, shift);
        let sticky = any_bit_below(a.digits, shift);
        let m = (top | u64::from(sticky)) as f64;
        // Exact scaling by 2^shift (shift <= 960 below), or Infinity past the range.
        if bits > 1024 {
            f64::INFINITY
        } else {
            m * f64::from_bits((1023 + shift) << 52)
        }
    };
    if a.neg { -mag } else { mag }
}

/// The 64 bits of `digits` from bit `shift` (the caller makes sure they are the top bits).
fn top_bits(digits: &[u64], shift: u64) -> u64 {
    let w = (shift / 64) as usize;
    let b = (shift % 64) as u32;
    let lo = digits[w] >> b;
    if b == 0 {
        lo
    } else {
        lo | digits.get(w + 1).map_or(0, |d| d << (64 - b))
    }
}

/// Whether a bit below bit `shift` is set.
fn any_bit_below(digits: &[u64], shift: u64) -> bool {
    let w = (shift / 64) as usize;
    let b = (shift % 64) as u32;
    digits[..w].iter().any(|d| *d != 0) || (b > 0 && digits[w] & ((1u64 << b) - 1) != 0)
}

// ---- strings (§7.1.14 StringToBigInt, §21.2.3.3 toString) ------------------------------------

/// StringToBigInt (§7.1.14) of a string that is already trimmed of JS white space (the caller
/// trims, as `StringToNumber`; a non-ASCII char left over makes the string invalid there).
/// The StringIntegerLiteral grammar: empty is `0n`; `0x`/`0o`/`0b` (any case) with at least one
/// digit and no sign; else an optional sign and at least one decimal digit. No `_`, no `n`, no
/// `.`, no exponent. `Ok(None)` for a string that does not match; `Err(TooBig)` above
/// [`MAX_BITS`].
pub fn parse_literal(s: &[u8]) -> Result<Option<Big>, Error> {
    if s.is_empty() {
        return Ok(Some(Big::zero()));
    }
    if s.len() >= 2 && s[0] == b'0' {
        let radix = match s[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(r) = radix {
            return parse_digits(&s[2..], r, false);
        }
    }
    match s[0] {
        b'+' => parse_digits(&s[1..], 10, false),
        b'-' => parse_digits(&s[1..], 10, true),
        _ => parse_digits(s, 10, false),
    }
}

/// Digits of `radix` (2-36, either letter case) with the sign `neg`. `Ok(None)` for an empty
/// string or a char that is not a digit of `radix`.
pub fn parse_digits(s: &[u8], radix: u32, neg: bool) -> Result<Option<Big>, Error> {
    debug_assert!((2..=36).contains(&radix));
    if s.is_empty() || !s.iter().all(|c| char::from(*c).is_digit(radix)) {
        return Ok(None);
    }
    let first = s.iter().position(|c| *c != b'0');
    let Some(first) = first else {
        return Ok(Some(Big::zero()));
    };
    let body = &s[first..];
    // The value is at least radix^(len - 1) >= 2^((len - 1) * floor(log2 radix)).
    let log2_floor = u64::from(31 - radix.leading_zeros());
    if (body.len() as u64 - 1).saturating_mul(log2_floor) >= MAX_BITS {
        return Err(Error::TooBig);
    }
    let text = core::str::from_utf8(body).expect("ASCII digits");
    let m = UBig::from_str_radix(text, radix).expect("digits checked above");
    checked(Big::from_ubig(neg, &m)).map(Some)
}

/// `ceil(2^32 / log2(r))` for `r` = 2..=36: digits per bit, as a 32-bit fixed-point fraction
/// rounded up (from a 60-digit decimal computation).
const DIGITS_PER_BIT: [u64; 35] = [
    4294967296, 2709822658, 2147483648, 1849741733, 1661520156, 1529898220, 1431655766, 1354911329,
    1292913987, 1241523976, 1198050830, 1160664036, 1128071164, 1099331346, 1073741824, 1050766078,
    1029986702, 1011073585, 993761859, 977836273, 963119892, 949465784, 936750802, 924870867,
    913737343, 903274220, 893415895, 884105414, 875293063, 866935226, 858993460, 851433730,
    844225783, 837342624, 830760078,
];

/// An upper bound of the length of [`to_string_radix`] (the sign included), computed from the
/// bit length only, so a caller can refuse a string above its length limit before the work.
pub fn string_len_bound(a: View, radix: u32) -> u64 {
    debug_assert!((2..=36).contains(&radix));
    let k = DIGITS_PER_BIT[radix as usize - 2];
    // A value below 2^bits has at most ceil(bits / log2(radix)) digits.
    ((a.bit_len() * k) >> 32) + 1 + u64::from(a.neg)
}

/// BigInt::toString(a, radix): lower-case digits, a `-` for a negative value.
pub fn to_string_radix(a: View, radix: u32) -> String {
    debug_assert!((2..=36).contains(&radix));
    if a.is_zero() {
        return "0".to_string();
    }
    let digits = a.ubig().in_radix(radix as u8).to_string();
    if a.neg { format!("-{digits}") } else { digits }
}

// ---- asIntN, asUintN (§21.2.2.1, §21.2.2.2) ---------------------------------------------------

/// `a` modulo 2^bits as a value in [0, 2^bits) (`bits` > 0; the caller keeps `bits` small enough
/// that the mask fits in memory).
fn mod_pow2(a: View, bits: u64) -> Big {
    let words = bits.div_ceil(64) as usize;
    let mut v = twos(a, words.max(a.digits.len() + 1));
    v.truncate(words);
    let rem = bits % 64;
    if rem != 0 {
        v[words - 1] &= (1u64 << rem) - 1;
    }
    Big::from_parts(false, v)
}

/// BigInt.asIntN: `a` modulo 2^bits, read as a signed `bits`-bit integer.
pub fn as_int_n(a: View, bits: u64) -> Result<Big, Error> {
    if bits == 0 || a.is_zero() {
        return Ok(Big::zero());
    }
    // |a| < 2^(bits - 1): `a` is already in range.
    if a.bit_len() < bits {
        return Ok(a.to_big());
    }
    // Here bits <= bit_len(a), so the mask is no larger than `a`.
    let m = mod_pow2(a, bits);
    let top = bits - 1;
    let set = m
        .digits
        .get((top / 64) as usize)
        .is_some_and(|d| (d >> (top % 64)) & 1 == 1);
    if set {
        // m - 2^bits = -(2^bits - m): the two's complement of m in `bits` bits.
        let words = bits.div_ceil(64) as usize;
        let mut v = m.digits.clone();
        v.resize(words, 0);
        negate_digits(&mut v);
        let rem = bits % 64;
        if rem != 0 {
            v[words - 1] &= (1u64 << rem) - 1;
        }
        Ok(Big::from_parts(true, v))
    } else {
        Ok(m)
    }
}

/// BigInt.asUintN: `a` modulo 2^bits. A negative `a` with `bits` above [`MAX_BITS`] is
/// [`Error::TooBig`] (the result would have `bits` bits).
pub fn as_uint_n(a: View, bits: u64) -> Result<Big, Error> {
    if bits == 0 || a.is_zero() {
        return Ok(Big::zero());
    }
    if !a.neg && a.bit_len() <= bits {
        return Ok(a.to_big());
    }
    if a.neg && bits > MAX_BITS {
        return Err(Error::TooBig);
    }
    Ok(mod_pow2(a, bits))
}

// ---- i128, hash -------------------------------------------------------------------------------

/// The BigInt of `n`.
pub fn from_i128(n: i128) -> Big {
    let m = n.unsigned_abs();
    Big::from_parts(n < 0, vec![m as u64, (m >> 64) as u64])
}

/// The BigInt of `n`.
pub fn from_i64(n: i64) -> Big {
    from_i128(i128::from(n))
}

/// `a` as an i128, if it fits.
pub fn to_i128(a: View) -> Option<i128> {
    let m: u128 = match a.digits {
        [] => 0,
        [lo] => u128::from(*lo),
        [lo, hi] => u128::from(*lo) | (u128::from(*hi) << 64),
        _ => return None,
    };
    if a.neg {
        if m <= 1u128 << 127 {
            Some((m as i128).wrapping_neg())
        } else {
            None
        }
    } else {
        i128::try_from(m).ok()
    }
}

/// A hash of the value only (sign and digits; H12-Q2: no address input), mixed with `seed`.
/// Equal values give equal hashes.
pub fn hash(a: View, seed: u64) -> u32 {
    // FNV-1a over 64-bit words, then a final avalanche (splitmix64).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
    let mut mix = |w: u64| {
        h ^= w;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(u64::from(a.neg));
    mix(a.digits.len() as u64);
    for d in a.digits {
        mix(*d);
    }
    let mut z = h;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z ^ (z >> 32)) as u32
}
