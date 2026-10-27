//! Number builtins (§21.1): module `number`, block 4. Targets `%Number%` (made here:
//! `IntrinsicDef`) and `%Number.prototype%` (`IntrinsicId::Number`,
//! `IntrinsicId::NumberPrototype`).
//!
//! ```text
//! Number(v)          ToNumber(v) (+0 without v); with new: a Number wrapper whose prototype
//!                    comes from new.target (GetPrototypeFromConstructor, Cx::new_wrapper)
//! Number.<static>    isFinite, isInteger, isNaN, isSafeInteger (no conversion), parseFloat,
//!                    parseInt ({w, c}; after `prototype`, before the constants: V8's order)
//! Number.<constant>  MAX_VALUE, MIN_VALUE, NaN, NEGATIVE_INFINITY, POSITIVE_INFINITY,
//!                    MAX_SAFE_INTEGER, MIN_SAFE_INTEGER, EPSILON ({}; V8's key order)
//! %Number.prototype% toExponential, toFixed, toPrecision: exact digits, rounded half up
//!                    (`exact_decimal`, `round_half_up`); toString(radix): radix 10 is
//!                    ToString; 2-36 is V8's DoubleToRadixCString (`radix_string`); valueOf.
//!                    All read thisNumberValue (Cx::this_primitive_value)
//! global             Number, parseFloat, parseInt {w, c} (node's order: right after Number)
//! ```
//!
//! parseFloat and parseInt (§19.2.4, §19.2.5) are definitions of this module, installed on
//! both `%Number%` and the global object, so `Number.parseInt === parseInt` (§21.1.2.12-13)
//! with one function object (M2-P5-PLAN-D4). They trim StrWhiteSpaceChar with this module's
//! copy of the table ([`is_str_white_space`], M2-P5-PLAN-D5). Their values follow V8:
//!
//! | Input | Value |
//! |---|---|
//! | parseFloat | the longest StrDecimalLiteral prefix; `f64::from_str` on it, correctly rounded; `Infinity` by hand |
//! | parseInt, radix 10 | the digits through `f64::from_str` (V8: strtod), correctly rounded |
//! | parseInt, radix 2, 4, 8, 16, 32 | exact bits, rounded half to even (V8 `InternalStringToIntDouble`) |
//! | parseInt, other radices | V8's `HandleGenericCase`: 32-bit parts, then `result * multiplier + part` as one fused multiply-add (§19.2.5 step 14 allows an approximation) |
//!
//! toExponential, toFixed and toPrecision (§21.1.3.2, 3, 5) take the exact decimal value of
//! the double (it always has a finite expansion: `m × 2^e` is `m × 5^-e × 10^e`) and round it
//! half up: "if there are two such n, pick the larger n". V8 (node) prints the same digits.
//! toExponential without an argument uses the shortest digits of Number::toString (the VM's
//! `Cx::to_string`, with its even-tie rule), as V8 does. These helpers live here, not in the
//! VM's numconv.rs: that module is private to ep9_vm.
//!
//! Not built yet: `toLocaleString`.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-number.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, FlatStr, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 4;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Number", 1, number),   // 0
    BuiltinDef::method("toString", 1, to_string),   // 1
    BuiltinDef::method("valueOf", 0, value_of),     // 2
    BuiltinDef::method("isFinite", 1, is_finite),   // 3
    BuiltinDef::method("isInteger", 1, is_integer), // 4
    BuiltinDef::method("isNaN", 1, is_nan),         // 5
    BuiltinDef::method("isSafeInteger", 1, is_safe_integer), // 6
    BuiltinDef::method("parseFloat", 1, parse_float), // 7
    BuiltinDef::method("parseInt", 2, parse_int),   // 8
    BuiltinDef::method("toExponential", 1, to_exponential), // 9
    BuiltinDef::method("toFixed", 1, to_fixed),     // 10
    BuiltinDef::method("toPrecision", 1, to_precision), // 11
];

/// Definition indices (ids `BLOCK.i`) of the statics.
mod def {
    pub const IS_FINITE: u16 = 3;
    pub const IS_INTEGER: u16 = 4;
    pub const IS_NAN: u16 = 5;
    pub const IS_SAFE_INTEGER: u16 = 6;
    pub const PARSE_FLOAT: u16 = 7;
    pub const PARSE_INT: u16 = 8;
    pub const TO_EXPONENTIAL: u16 = 9;
    pub const TO_FIXED: u16 = 10;
    pub const TO_PRECISION: u16 = 11;
}

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Number,
    value: IntrinsicValue::Builtin(0),
}];

/// A value property of `%Number%` ({}: §21.1.2).
const fn constant(name: &'static str, value: f64) -> PropDef {
    PropDef::constant(
        PropKey::Str(name),
        ConstValue::Number(value),
        PropAttrs::NONE,
    )
}

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Number,
        props: &[
            // §21.1.2.15 Number.prototype ({}): the first key of a constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::NumberPrototype,
                PropAttrs::NONE,
            ),
            // §21.1.2.2-5, 12-13 ({w, c}), in V8's order: before the constants.
            PropDef::method(PropKey::Str("isFinite"), def::IS_FINITE),
            PropDef::method(PropKey::Str("isInteger"), def::IS_INTEGER),
            PropDef::method(PropKey::Str("isNaN"), def::IS_NAN),
            PropDef::method(PropKey::Str("isSafeInteger"), def::IS_SAFE_INTEGER),
            PropDef::method(PropKey::Str("parseFloat"), def::PARSE_FLOAT),
            PropDef::method(PropKey::Str("parseInt"), def::PARSE_INT),
            // §21.1.2.1, 6-11, 14, in V8's order.
            constant("MAX_VALUE", f64::MAX),
            // 5 × 10^-324: the smallest positive denormal (not f64::MIN_POSITIVE).
            constant("MIN_VALUE", 5e-324),
            constant("NaN", f64::NAN),
            constant("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            constant("POSITIVE_INFINITY", f64::INFINITY),
            constant("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
            constant("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
            constant("EPSILON", f64::EPSILON),
        ],
    },
    ObjectProps {
        target: IntrinsicId::NumberPrototype,
        props: &[
            // §21.1.3.1 Number.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Number,
                PropAttrs::METHOD,
            ),
            // §21.1.3.2, 3, 5 ({w, c}), in V8's order: before toString.
            PropDef::method(PropKey::Str("toExponential"), def::TO_EXPONENTIAL),
            PropDef::method(PropKey::Str("toFixed"), def::TO_FIXED),
            PropDef::method(PropKey::Str("toPrecision"), def::TO_PRECISION),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 1),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Number ( . . . ), then §19.2.4 parseFloat and §19.2.5 parseInt: the same
        // function objects as Number.parseFloat and Number.parseInt (node's order).
        props: &[
            PropDef::intrinsic(
                PropKey::Str("Number"),
                IntrinsicId::Number,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Str("parseFloat"), def::PARSE_FLOAT),
            PropDef::method(PropKey::Str("parseInt"), def::PARSE_INT),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "number",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Number ( value ) (§21.1.1.1).
fn number<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If value is present, then
    //    a. Let prim be ? ToNumeric(value).
    //    b. If prim is a BigInt, let n be 𝔽(ℝ(prim)).
    //    c. Otherwise, let n be prim.
    // 2. Else,
    //    a. Let n be +0𝔽.
    // (v0 has no BigInt: ToNumeric is ToNumber.)
    let n = if args.is_empty() {
        0.0
    } else {
        cx.to_number(args.get(0))?
    };
    let n = Local::number(n);
    // 3. If NewTarget is undefined, return n.
    let Some(nt) = new_target.as_object() else {
        return Ok(n);
    };
    // 4. Let O be ? OrdinaryCreateFromConstructor(NewTarget, "%Number.prototype%",
    //    « [[NumberData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::NumberPrototype)?;
    // 5. Set O.[[NumberData]] to n.
    let o = cx.new_wrapper(proto, n)?;
    // 6. Return O.
    Ok(o.value())
}

/// thisNumberValue ( value ) (§21.1.3.7.1) as an `f64`.
fn this_number_value<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<f64> {
    let x = cx.this_primitive_value(this, PrimitiveType::Number)?;
    Ok(x.as_number().expect("thisNumberValue gives a number"))
}

/// `Number.prototype.toString ( [ radix ] )` (§21.1.3.6).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisNumberValue(this value).
    let x = this_number_value(cx, this)?;
    // 2. If radix is undefined, let radixMV be 10.
    // 3. Else, let radixMV be ? ToIntegerOrInfinity(radix).
    let radix = args.get(0);
    let radix = if radix.is_undefined() {
        10.0
    } else {
        cx.to_integer_or_infinity(radix)?
    };
    // 4. If radixMV is not in the inclusive interval from 2 to 36, throw a RangeError
    //    exception.
    if !(2.0..=36.0).contains(&radix) {
        return Err(cx.throw_range_error("toString() radix argument must be between 2 and 36"));
    }
    // 5. Return Number::toString(x, radixMV).
    if radix == 10.0 {
        return Ok(cx.to_string(Local::number(x))?.value());
    }
    let text = radix_string(x, radix as u32);
    Ok(cx.new_string(&text)?.value())
}

/// Number.prototype.valueOf ( ) (§21.1.3.7).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisNumberValue(this value).
    cx.this_primitive_value(this, PrimitiveType::Number)
}

// ---- toExponential, toFixed, toPrecision (§21.1.3.2, 3, 5) --------------------------------------

/// Number.prototype.toExponential ( fractionDigits ) (§21.1.3.2).
fn to_exponential<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisNumberValue(this value).
    let x = this_number_value(cx, this)?;
    // 2. Let f be ? ToIntegerOrInfinity(fractionDigits).
    // 3. Assert: If fractionDigits is undefined, then f is 0.
    let fraction_digits = args.get(0);
    let f = cx.to_integer_or_infinity(fraction_digits)?;
    // 4. If x is not finite, return Number::toString(x, 10).
    if !x.is_finite() {
        return Ok(cx.to_string(Local::number(x))?.value());
    }
    // 5. If f < 0 or f > 100, throw a RangeError exception.
    if !(0.0..=100.0).contains(&f) {
        return Err(cx.throw_range_error("toExponential() argument must be between 0 and 100"));
    }
    // 10.b. If fractionDigits is undefined: the fewest digits that give back x (the digits
    //       of Number::toString).
    let shortest = if fraction_digits.is_undefined() && x != 0.0 {
        let text = cx.to_rust_string(Local::number(x.abs()))?;
        Some(shortest_digits(&text))
    } else {
        None
    };
    let text = exponential_string(x, f as u32, shortest);
    Ok(cx.new_string(&text)?.value())
}

/// Number.prototype.toFixed ( fractionDigits ) (§21.1.3.3).
fn to_fixed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisNumberValue(this value).
    let x = this_number_value(cx, this)?;
    // 2. Let f be ? ToIntegerOrInfinity(fractionDigits).
    // 3. Assert: If fractionDigits is undefined, then f is 0.
    let f = cx.to_integer_or_infinity(args.get(0))?;
    // 4. If f is not finite, throw a RangeError exception.
    // 5. If f < 0 or f > 100, throw a RangeError exception.
    if !(0.0..=100.0).contains(&f) {
        return Err(cx.throw_range_error("toFixed() digits argument must be between 0 and 100"));
    }
    // 6. If x is not finite, return Number::toString(x, 10).
    // 10. If x ≥ 10^21, let m be ! ToString(𝔽(x)). (With the sign of steps 8-9.)
    if !x.is_finite() || x.abs() >= 1e21 {
        return Ok(cx.to_string(Local::number(x))?.value());
    }
    let text = fixed_string(x, f as u32);
    Ok(cx.new_string(&text)?.value())
}

/// Number.prototype.toPrecision ( precision ) (§21.1.3.5).
fn to_precision<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisNumberValue(this value).
    let x = this_number_value(cx, this)?;
    // 2. If precision is undefined, return ! ToString(x).
    let precision = args.get(0);
    if precision.is_undefined() {
        return Ok(cx.to_string(Local::number(x))?.value());
    }
    // 3. Let p be ? ToIntegerOrInfinity(precision).
    let p = cx.to_integer_or_infinity(precision)?;
    // 4. If x is not finite, return Number::toString(x, 10).
    if !x.is_finite() {
        return Ok(cx.to_string(Local::number(x))?.value());
    }
    // 5. If p < 1 or p > 100, throw a RangeError exception.
    if !(1.0..=100.0).contains(&p) {
        return Err(cx.throw_range_error("toPrecision() argument must be between 1 and 100"));
    }
    let text = precision_string(x, p as u32);
    Ok(cx.new_string(&text)?.value())
}

/// The exact decimal value of the positive finite double `x`: `(d, point)` with
/// `x = 0.d₁d₂…dₖ × 10^point`, `d₁ ≠ 0` and `dₖ ≠ 0` (each `dᵢ` a digit 0-9).
///
/// `x = m × 2^e` with an integer `m`. For `e ≥ 0` the digits are those of `m × 2^e`; for
/// `e < 0`, `x = m × 5^-e × 10^e`, so they are those of `m × 5^-e`. Both are integers of at most
/// 1,024 bits (2^1024 and 2^53 × 5^1074 < 10^767), kept in base-10^9 limbs.
pub fn exact_decimal(x: f64) -> (Vec<u8>, i32) {
    debug_assert!(x > 0.0 && x.is_finite());
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (m, e) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    const BASE: u64 = 1_000_000_000;
    // Little-endian limbs < 10^9.
    let mut limbs: Vec<u64> = vec![m % BASE, (m / BASE) % BASE, m / (BASE * BASE)];
    let mul = |limbs: &mut Vec<u64>, k: u64| {
        let mut carry = 0;
        for l in limbs.iter_mut() {
            let v = *l * k + carry;
            *l = v % BASE;
            carry = v / BASE;
        }
        while carry > 0 {
            limbs.push(carry % BASE);
            carry /= BASE;
        }
    };
    // Factors below 2^32, so limb × factor + carry stays below 2^64.
    let (factor, step, mut count) = if e >= 0 {
        (1u64 << 31, 31, e)
    } else {
        (1_220_703_125, 13, -e) // 5^13
    };
    while count >= step {
        mul(&mut limbs, factor);
        count -= step;
    }
    let last = if e >= 0 {
        1u64 << count
    } else {
        5u64.pow(count as u32)
    };
    mul(&mut limbs, last);
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
    let mut text = limbs.last().expect("x > 0").to_string();
    for l in limbs.iter().rev().skip(1) {
        text.push_str(&format!("{l:09}"));
    }
    let mut d: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
    // The integer has `d.len()` digits and is x × 10^max(0, -e).
    let point = d.len() as i32 + e.min(0);
    while d.last() == Some(&0) {
        d.pop();
    }
    (d, point)
}

/// `0.d₁…dₖ × 10^point` rounded half up to `keep` digits (the larger of two equally close
/// values, as §21.1.3.2, 3 and 5 ask): `(r, point')` with `r` the `keep` leading digits (zeros
/// added past `k`) and the rounded value `0.r × 10^point'`. A carry out of the first digit
/// gives `r = 1 0…0` (`keep` digits) and `point' = point + 1`. `keep ≤ 0` rounds at a place
/// above the first digit: `([1], point + 1)` when that rounds up (only for `keep = 0` and
/// `d₁ ≥ 5`), else `([], point)` for zero.
pub fn round_half_up(d: &[u8], point: i32, keep: i32) -> (Vec<u8>, i32) {
    if keep < 0 {
        return (Vec::new(), point);
    }
    let keep = keep as usize;
    if keep >= d.len() {
        let mut r = d.to_vec();
        r.resize(keep, 0);
        return (r, point);
    }
    let mut r = d[..keep].to_vec();
    // The value past the cut is at least half a unit exactly when its first digit is ≥ 5.
    if d[keep] < 5 {
        return (r, point);
    }
    for digit in r.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return (r, point);
        }
    }
    // All nines (or keep = 0): 10^point.
    r.insert(0, 1);
    r.truncate(keep.max(1));
    (r, point + 1)
}

/// The digits and the point of Number::toString's text of a positive finite number (the
/// shortest round-trip digits, §6.1.6.1.20 step 5): "123", "1.5", "0.00012", "1.2e+21",
/// "5e-324".
fn shortest_digits(text: &str) -> (Vec<u8>, i32) {
    let (mantissa, exp) = match text.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().expect("an exponent")),
        None => (text, 0),
    };
    let int_len = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut d: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let mut point = int_len + exp;
    let lead = d.iter().take_while(|&&c| c == 0).count();
    d.drain(..lead);
    point -= lead as i32;
    while d.last() == Some(&0) {
        d.pop();
    }
    (d, point)
}

/// The digits `d` as text.
fn digit_text(d: &[u8]) -> String {
    d.iter().map(|&c| char::from(b'0' + c)).collect()
}

/// §21.1.3.2 steps 6-15 for a finite `x` and `0 ≤ f ≤ 100`; `shortest` holds the digits of
/// step 10.b when fractionDigits is undefined (and `x ≠ 0`).
pub fn exponential_string(x: f64, f: u32, shortest: Option<(Vec<u8>, i32)>) -> String {
    // 7. Let s be the empty String.
    // 8. If x < 0, then set s to "-" and x to -x.
    let mut s = String::new();
    if x < 0.0 {
        s.push('-');
    }
    let x = x.abs();
    let (m, e) = if x == 0.0 {
        // 9. If x = 0: m is f + 1 zeros, e = 0.
        (vec![0; f as usize + 1], 0)
    } else if let Some((d, point)) = shortest {
        // 10.b. The fewest digits n (f + 1 of them) with 𝔽(n × 10^(e - f)) = x.
        (d, point - 1)
    } else {
        // 10.a. 10^f ≤ n < 10^(f+1), n × 10^(e - f) - x closest to zero; the larger of two.
        let (d, point) = exact_decimal(x);
        let (r, point) = round_half_up(&d, point, f as i32 + 1);
        (r, point - 1)
    };
    // 11. If f ≠ 0 (the digit count after the first), m = a "." b.
    let m = digit_text(&m);
    s.push_str(&m[..1]);
    if m.len() > 1 {
        s.push('.');
        s.push_str(&m[1..]);
    }
    // 12-14. "e", the sign of e ("+" for 0), and the digits of abs(e).
    s.push('e');
    s.push(if e < 0 { '-' } else { '+' });
    s.push_str(&e.unsigned_abs().to_string());
    s
}

/// §21.1.3.3 steps 7-12 for a finite `x` with `abs(x) < 10^21` and `0 ≤ f ≤ 100`.
pub fn fixed_string(x: f64, f: u32) -> String {
    // 8. Let s be the empty String.
    // 9. If x < 0, then set s to "-" and x to -x. (-0 is not below 0.)
    let mut s = String::new();
    if x < 0.0 {
        s.push('-');
    }
    let x = x.abs();
    // 11.a. n: n / 10^f - x closest to zero; the larger of two.
    // 11.b. If n = 0, m = "0"; else the digits of n.
    let mut m = if x == 0.0 {
        String::from("0")
    } else {
        let (d, point) = exact_decimal(x);
        let (mut r, point) = round_half_up(&d, point, point + f as i32);
        if r.is_empty() {
            String::from("0")
        } else {
            // n = 0.r × 10^(point + f): point + f digits (a carry adds one).
            r.resize((point + f as i32) as usize, 0);
            digit_text(&r)
        }
    };
    // 11.c. If f ≠ 0: pad m with zeros to f + 1 digits, then a "." b with f digits in b.
    let f = f as usize;
    if f != 0 {
        if m.len() <= f {
            m = format!("{}{m}", "0".repeat(f + 1 - m.len()));
        }
        let k = m.len();
        m.insert(k - f, '.');
    }
    s.push_str(&m);
    s
}

/// §21.1.3.5 steps 6-14 for a finite `x` and `1 ≤ p ≤ 100`.
pub fn precision_string(x: f64, p: u32) -> String {
    // 7. Let s be the empty String.
    // 8. If x < 0, then set s to "-" and x to -x.
    let mut s = String::new();
    if x < 0.0 {
        s.push('-');
    }
    let x = x.abs();
    let p = p as i32;
    let (m, e) = if x == 0.0 {
        // 9. m is p zeros, e = 0.
        (vec![0; p as usize], 0)
    } else {
        // 10.a. 10^(p-1) ≤ n < 10^p, n × 10^(e-p+1) - x closest to zero; the larger of two.
        let (d, point) = exact_decimal(x);
        let (r, point) = round_half_up(&d, point, p);
        (r, point - 1)
    };
    let m = digit_text(&m);
    // 10.c. If e < -6 or e ≥ p: exponential notation.
    if e < -6 || e >= p {
        s.push_str(&m[..1]);
        if p != 1 {
            s.push('.');
            s.push_str(&m[1..]);
        }
        s.push('e');
        s.push(if e < 0 { '-' } else { '+' });
        s.push_str(&e.unsigned_abs().to_string());
        return s;
    }
    if e == p - 1 {
        // 11. All p digits before the point.
        s.push_str(&m);
    } else if e >= 0 {
        // 12. The first e + 1 digits, ".", the rest.
        let k = e as usize + 1;
        s.push_str(&m[..k]);
        s.push('.');
        s.push_str(&m[k..]);
    } else {
        // 13. "0.", -(e + 1) zeros, m.
        s.push_str("0.");
        s.push_str(&"0".repeat((-(e + 1)) as usize));
        s.push_str(&m);
    }
    s
}

// ---- Number statics (§21.1.2) ------------------------------------------------------------------

/// The number in `v`, if `v` is a Number (the statics below do not convert).
fn number_arg(v: Local<'_>) -> Option<f64> {
    v.as_number()
}

/// Number.isFinite ( number ) (§21.1.2.2).
fn is_finite<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If number is not a Number, return false.
    // 2. If number is not finite, return false.
    // 3. Otherwise, return true.
    Ok(Local::boolean(
        number_arg(args.get(0)).is_some_and(f64::is_finite),
    ))
}

/// IsIntegralNumber ( argument ) (§7.2.6) for a number.
fn is_integral(x: f64) -> bool {
    // 2. If argument is not finite, return false.
    // 3. If truncate(ℝ(argument)) ≠ ℝ(argument), return false.
    x.is_finite() && x.trunc() == x
}

/// Number.isInteger ( number ) (§21.1.2.3).
fn is_integer<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return IsIntegralNumber(number). (Step 1 of it: not a Number gives false.)
    Ok(Local::boolean(
        number_arg(args.get(0)).is_some_and(is_integral),
    ))
}

/// Number.isNaN ( number ) (§21.1.2.4).
fn is_nan<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If number is not a Number, return false.
    // 2. If number is NaN, return true.
    // 3. Otherwise, return false.
    Ok(Local::boolean(
        number_arg(args.get(0)).is_some_and(f64::is_nan),
    ))
}

/// Number.isSafeInteger ( number ) (§21.1.2.5).
fn is_safe_integer<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If IsIntegralNumber(number) is true, then
    //    a. If abs(ℝ(number)) ≤ 2^53 - 1, return true.
    // 2. Return false.
    Ok(Local::boolean(number_arg(args.get(0)).is_some_and(|x| {
        is_integral(x) && x.abs() <= 9_007_199_254_740_991.0
    })))
}

// ---- parseFloat and parseInt (§19.2.4, §19.2.5) -------------------------------------------------

/// StrWhiteSpaceChar (§7.1.4.1.1): WhiteSpace (§12.2: TAB, VT, FF, ZWNBSP and the Unicode
/// "Space_Separator" (Zs) code points of Unicode 17, all in the BMP) and LineTerminator
/// (§12.3: LF, CR, LS, PS). A copy of the table of `String.prototype.trim`
/// (string/proto.rs, not reachable from here: M2-P5-PLAN-D5); `tests/number.rs` checks the two
/// agree on all 65,536 code units. U+180E is not in it (Unicode 6.3 moved it out of Zs).
pub fn is_str_white_space(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

/// The code units of a flat string, read one at a time (no copy).
struct Units<'a, 's> {
    cx: &'a Cx<'s>,
    flat: Local<'s, FlatStr>,
    len: usize,
}

impl Units<'_, '_> {
    /// Code unit `i`, or `None` past the end.
    fn at(&self, i: usize) -> Option<u16> {
        if i < self.len {
            self.cx.code_unit_at(self.flat, i)
        } else {
            None
        }
    }

    /// The index of the first code unit at or after `i` that is not StrWhiteSpaceChar:
    /// TrimString(S, start) (§22.1.3.32.1).
    fn skip_white_space(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(is_str_white_space) {
            i += 1;
        }
        i
    }
}

/// ToString(v), flattened: its code units can be read.
fn flat_string<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<(Local<'s, FlatStr>, usize)> {
    let s = cx.to_string(v)?;
    let len = cx.string_len(s);
    Ok((cx.flatten(s)?, len))
}

/// `u` as an ASCII byte, if it is one.
fn ascii(u: Option<u16>) -> Option<u8> {
    u.and_then(|u| u8::try_from(u).ok()).filter(u8::is_ascii)
}

/// parseFloat ( string ) (§19.2.4).
fn parse_float<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let inputString be ? ToString(string).
    let (flat, len) = flat_string(cx, args.get(0))?;
    let s = Units { cx, flat, len };
    // 2. Let trimmedString be ! TrimString(inputString, start).
    let start = s.skip_white_space(0);
    // 3-4. The longest prefix of trimmedString that satisfies the syntax of a
    //      StrDecimalLiteral (§7.1.4.1), or NaN if there is none.
    Ok(Local::number(str_decimal_literal_prefix(&s, start)))
}

/// The value of the longest StrDecimalLiteral prefix of `s` from `start` (§19.2.4 steps 3-6),
/// or NaN without one:
///
/// ```text
/// StrDecimalLiteral         ::: ( + | - )? StrUnsignedDecimalLiteral
/// StrUnsignedDecimalLiteral ::: Infinity
///                             | DecimalDigits ( . DecimalDigits? )? ExponentPart?
///                             | . DecimalDigits ExponentPart?
/// ExponentPart              ::: ( e | E ) ( + | - )? DecimalDigits      (no separators)
/// ```
fn str_decimal_literal_prefix(s: &Units<'_, '_>, start: usize) -> f64 {
    let mut text = String::new();
    let mut i = start;
    let mut negative = false;
    if let Some(c @ (b'+' | b'-')) = ascii(s.at(i)) {
        negative = c == b'-';
        text.push(char::from(c));
        i += 1;
    }
    // Infinity (case-sensitive): f64::from_str also takes "inf" and "nan", so it never
    // sees these letters.
    if "Infinity"
        .bytes()
        .enumerate()
        .all(|(k, c)| ascii(s.at(i + k)) == Some(c))
    {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let digits = |text: &mut String, mut i: usize| -> usize {
        while let Some(c) = ascii(s.at(i)).filter(u8::is_ascii_digit) {
            text.push(char::from(c));
            i += 1;
        }
        i
    };
    let int_end = digits(&mut text, i);
    let mut any_digit = int_end > i;
    i = int_end;
    if ascii(s.at(i)) == Some(b'.') {
        let mut with_dot = text.clone();
        with_dot.push('.');
        let frac_end = digits(&mut with_dot, i + 1);
        // "1." is a literal; "." alone is not.
        if any_digit || frac_end > i + 1 {
            any_digit = true;
            text = with_dot;
            i = frac_end;
        }
    }
    if !any_digit {
        return f64::NAN;
    }
    // ExponentPart only when at least one digit follows the e and its sign.
    if let Some(e @ (b'e' | b'E')) = ascii(s.at(i)) {
        let mut exp = String::from(char::from(e));
        let mut j = i + 1;
        if let Some(c @ (b'+' | b'-')) = ascii(s.at(j)) {
            exp.push(char::from(c));
            j += 1;
        }
        if digits(&mut exp, j) > j {
            text.push_str(&exp);
        }
    }
    // 5-6. The StringNumericValue of the prefix (§7.1.4.1.2), rounded to the nearest double:
    // f64::from_str rounds correctly, and keeps the sign of "-0".
    text.parse::<f64>().unwrap_or(f64::NAN)
}

/// parseInt ( string, radix ) (§19.2.5).
fn parse_int<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let inputString be ? ToString(string).
    let (flat, len) = flat_string(cx, args.get(0))?;
    // 6. Let R be ℝ(? ToInt32(radix)). (Before the scan: it may run JS, so the scan reads
    //    the flat string after it.)
    let r = cx.to_int32(args.get(1))?;
    let s = Units { cx, flat, len };
    Ok(Local::number(parse_int_units(&s, r)))
}

/// The value of digit `u` in `radix`, if it is one.
fn digit_value(u: Option<u16>, radix: u32) -> Option<u32> {
    char::from(ascii(u)?).to_digit(radix)
}

/// §19.2.5 steps 2-16 on the code units of `s` with the radix argument `r`.
fn parse_int_units(s: &Units<'_, '_>, r: i32) -> f64 {
    // 2. Let S be ! TrimString(inputString, start).
    let mut i = s.skip_white_space(0);
    // 3. Let sign be 1.
    // 4. If S is not empty and the first code unit of S is the code unit 0x002D (HYPHEN-MINUS),
    //    set sign to -1.
    // 5. If S is not empty and the first code unit of S is either the code unit 0x002B (PLUS
    //    SIGN) or the code unit 0x002D (HYPHEN-MINUS), set S to the substring of S from index 1.
    let mut negative = false;
    if let Some(c @ (b'+' | b'-')) = ascii(s.at(i)) {
        negative = c == b'-';
        i += 1;
    }
    // 7. Let stripPrefix be true.
    let mut strip_prefix = true;
    // 8. If R ≠ 0, then
    //    a. If R < 2 or R > 36, return NaN.
    //    b. If R ≠ 16, set stripPrefix to false.
    // 9. Else,
    //    a. Set R to 10.
    let mut radix = match r {
        0 => 10,
        2..=36 => {
            strip_prefix = r == 16;
            r as u32
        }
        _ => return f64::NAN,
    };
    // 10. If stripPrefix is true, then
    //     a. If the length of S is at least 2 and the first two code units of S are either
    //        "0x" or "0X", then
    //        i. Set S to the substring of S from index 2.
    //        ii. Set R to 16.
    if strip_prefix
        && ascii(s.at(i)) == Some(b'0')
        && matches!(ascii(s.at(i + 1)), Some(b'x' | b'X'))
    {
        i += 2;
        radix = 16;
    }
    // 11. If S contains a code unit that is not a radix-R digit, let end be the index within
    //     S of the first such code unit; otherwise, let end be the length of S.
    // 12. Let Z be the substring of S from 0 to end.
    let mut end = i;
    while digit_value(s.at(end), radix).is_some() {
        end += 1;
    }
    // 13. If Z is empty, return NaN.
    if end == i {
        return f64::NAN;
    }
    // Leading zeros add nothing (V8 skips them).
    while i < end && s.at(i) == Some(u16::from(b'0')) {
        i += 1;
    }
    // 14. Let mathInt be the integer value that is represented by Z in radix-R notation...
    let digits = (i..end).map(|k| digit_value(s.at(k), radix).expect("a radix-R digit"));
    let math_int = if radix == 10 {
        // V8: strtod on the digits, correctly rounded.
        let text: String = digits
            .map(|d| char::from_digit(d, 10).expect("a decimal digit"))
            .collect();
        if text.is_empty() {
            0.0
        } else {
            text.parse::<f64>().expect("decimal digits")
        }
    } else if radix.is_power_of_two() {
        power_of_two_radix_value(digits, radix.trailing_zeros())
    } else {
        generic_radix_value(digits, radix)
    };
    // 15. If mathInt = 0, then
    //     a. If sign = -1, return -0𝔽.
    //     b. Return +0𝔽.
    // 16. Return 𝔽(sign × mathInt).
    if negative { -math_int } else { math_int }
}

/// The digits (most significant first, no leading zero) in radix `2^log2`, rounded to the
/// nearest double, ties to even (V8 `InternalStringToIntDouble`).
fn power_of_two_radix_value(mut digits: impl Iterator<Item = u32>, log2: u32) -> f64 {
    let mut number: u64 = 0;
    let mut exponent: i64 = 0;
    while let Some(d) = digits.next() {
        number = (number << log2) | u64::from(d);
        let overflow = number >> 53;
        if overflow == 0 {
            continue;
        }
        // Past 53 bits: drop the low bits, and remember them and the rest of the digits for
        // the rounding.
        let dropped_count = 64 - overflow.leading_zeros();
        let dropped_mask = (1u64 << dropped_count) - 1;
        let dropped = number & dropped_mask;
        number >>= dropped_count;
        exponent = i64::from(dropped_count);
        let mut zero_tail = true;
        for d in digits.by_ref() {
            zero_tail &= d == 0;
            exponent += i64::from(log2);
        }
        let middle = 1u64 << (dropped_count - 1);
        if dropped > middle || (dropped == middle && (number & 1 != 0 || !zero_tail)) {
            number += 1;
        }
        // Rounding up may carry into bit 53.
        if number & (1u64 << 53) != 0 {
            exponent += 1;
            number >>= 1;
        }
        break;
    }
    // `number` < 2^53 is exact as a double, and so is the scale 2^exponent (exponent ≤ 1023,
    // a normal double); past that, `number` ≥ 2^52 makes the value overflow to Infinity.
    if exponent > 1023 {
        return f64::INFINITY;
    }
    number as f64 * f64::from_bits(((1023 + exponent) as u64) << 52)
}

/// The digits (most significant first) in a radix that is not 10 or a power of 2: V8's
/// `HandleGenericCase` (src/numbers/conversions.cc). It reads parts whose multiplier stays
/// below 2^32 / 36 in 32-bit integers, then folds each part into a double; above about 2^56
/// the rounding errors add up, which §19.2.5 step 14 allows for these radices.
///
/// The fold `result * multiplier + part` is one fused multiply-add (`ep9_math::fma_exact`, the
/// only FMA H36 allows): node v24.22.1 on arm64 gives the fused values. With a separate
/// multiply and add, 103 of 1,400 long strings in radices 2-36 and 76 of
/// 30,000 shorter random ones differed from node in the last digits; fused, none did. A fused
/// fold also gives the same value on every ep9 target.
fn generic_radix_value(digits: impl Iterator<Item = u32>, radix: u32) -> f64 {
    const MAXIMUM_MULTIPLIER: u32 = u32::MAX / 36;
    let mut result = 0.0f64;
    let (mut part, mut multiplier) = (0u32, 1u32);
    for d in digits {
        let m = multiplier * radix;
        if m > MAXIMUM_MULTIPLIER {
            result = ep9_math::fma_exact(result, f64::from(multiplier), f64::from(part));
            (part, multiplier) = (d, radix);
        } else {
            part = part * radix + d;
            multiplier = m;
        }
    }
    ep9_math::fma_exact(result, f64::from(multiplier), f64::from(part))
}

/// The biased exponent of `x` minus 1075 (V8 `Double::Exponent`): above 0 exactly when the
/// finite `x` is at least 2^53, so the units digit of `x` in any radix is not represented.
fn exponent(x: f64) -> i32 {
    ((x.to_bits() >> 52) & 0x7ff) as i32 - 1075
}

/// Number::toString(x, radix) for a radix other than 10 (§6.1.6.1.20 leaves the digits to
/// the implementation, "a generalization of" the radix-10 algorithm). This is V8's
/// `DoubleToRadixCString` (src/numbers/conversions.cc), so the text matches node's:
///
/// ```text
/// fraction digits: shift up one digit at a time while the remaining fraction is at least
///   delta (half the distance to the next double, scaled with it); round half to even, and
///   when the rounded-up value passes the next double, carry into the digits already written
/// integer digits:  divide by the radix while the quotient is >= 2^53, writing '0' for each
///   unrepresented digit; then take remainders (fmod) down to 0
/// ```
fn radix_string(x: f64, radix: u32) -> String {
    debug_assert!((2..=36).contains(&radix) && radix != 10);
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let r = f64::from(radix);
    let negative = x < 0.0;
    let value = x.abs();
    // Split the value into an integer part and a fractional part.
    let mut integer = value.floor();
    let mut fraction = value - integer;
    // Only compute fraction digits up to the input's precision: half the gap to the next
    // double, at least the smallest denormal.
    let next = f64::from_bits(value.to_bits() + 1);
    let mut delta = (0.5 * (next - value)).max(f64::from_bits(1));
    let mut fraction_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            // Shift up by one digit.
            fraction *= r;
            delta *= r;
            // Write the digit (fraction < radix, so the cast truncates in range).
            let digit = fraction as u32;
            fraction_digits.push(digit as u8);
            // The remainder.
            fraction -= f64::from(digit);
            // Round to even.
            if (fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1)) && fraction + delta > 1.0 {
                // Carry into the digits already written; past the first one, into the
                // integer part.
                loop {
                    match fraction_digits.pop() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(d) if u32::from(d) + 1 < radix => {
                            fraction_digits.push(d + 1);
                            break;
                        }
                        Some(_) => {}
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    // Integer digits, least significant first. Unrepresented digits are zeros.
    let mut integer_digits: Vec<u8> = Vec::new();
    while exponent(integer / r) > 0 {
        integer /= r;
        integer_digits.push(b'0');
    }
    loop {
        let remainder = ep9_math::fmod(integer, r);
        integer_digits.push(CHARS[remainder as usize]);
        integer = (integer - remainder) / r;
        if integer <= 0.0 {
            break;
        }
    }
    let mut s = String::with_capacity(integer_digits.len() + fraction_digits.len() + 2);
    if negative {
        s.push('-');
    }
    s.extend(integer_digits.iter().rev().map(|&c| char::from(c)));
    if !fraction_digits.is_empty() {
        s.push('.');
        s.extend(
            fraction_digits
                .iter()
                .map(|&d| char::from(CHARS[usize::from(d)])),
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::radix_string;

    /// Expected texts from node 24 (`x.toString(radix)`).
    #[test]
    fn radix_strings_match_node() {
        let cases: &[(f64, u32, &str)] = &[
            (0.0, 2, "0"),
            (-0.0, 16, "0"),
            (255.0, 16, "ff"),
            (-255.0, 16, "-ff"),
            (255.5, 16, "ff.8"),
            (0.5, 2, "0.1"),
            (
                0.1,
                2,
                "0.0001100110011001100110011001100110011001100110011001101",
            ),
            (0.1, 3, "0.0022002200220022002200220022002201"),
            (1e21, 36, "5v1j4f4ds7c000"),
            (
                9007199254740992.0,
                2,
                "100000000000000000000000000000000000000000000000000000",
            ),
            (core::f64::consts::PI, 36, "3.53i5ab8p5f"),
            (-1.5, 7, "-1.3333333333333333334"),
            (0.3, 36, "0.asssssssssp"),
            (123.456, 8, "173.3513615237574734"),
            (1e-7, 16, "0.000001ad7f29abcaf48"),
            (-0.000001, 5, "-0.0000000014340322421131001434033"),
            (4294967295.0, 32, "3vvvvvv"),
            (
                0.9999999999999999,
                2,
                "0.11111111111111111111111111111111111111111111111111111",
            ),
            (2.5, 5, "2.2222222222222222222222"),
            (0.5, 3, "0.1111111111111111111111111111111112"),
            (0.3333333333333333, 3, "0.1"),
            (0.6666666666666666, 3, "0.2"),
            (35.0, 36, "z"),
            (36.0, 36, "10"),
            (-35.99, 36, "-z.zn1fu8n1g"),
            (f64::NAN, 2, "NaN"),
            (f64::NEG_INFINITY, 36, "-Infinity"),
        ];
        for &(x, radix, want) in cases {
            assert_eq!(radix_string(x, radix), want, "({x:e}).toString({radix})");
        }
        // Long texts: MAX_VALUE in radix 36 is "1a1e4vngaiqo" and 187 zeros; 5e-324 in
        // radix 2 is "0." and 1073 zeros, then "1".
        let max = radix_string(f64::MAX, 36);
        assert_eq!(max, format!("1a1e4vngaiqo{}", "0".repeat(187)));
        let min = radix_string(5e-324, 2);
        assert_eq!(min, format!("0.{}1", "0".repeat(1073)));
    }
}
