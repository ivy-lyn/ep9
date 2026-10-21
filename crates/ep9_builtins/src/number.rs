//! Number builtins (§21.1): module `number`, block 4. Targets `%Number%` (made here:
//! `IntrinsicDef`) and `%Number.prototype%` (`IntrinsicId::Number`,
//! `IntrinsicId::NumberPrototype`).
//!
//! ```text
//! Number(v)          ToNumber(v) (+0 without v); with new: a Number wrapper whose prototype
//!                    comes from new.target (GetPrototypeFromConstructor, Cx::new_wrapper)
//! Number.<constant>  MAX_VALUE, MIN_VALUE, NaN, NEGATIVE_INFINITY, POSITIVE_INFINITY,
//!                    MAX_SAFE_INTEGER, MIN_SAFE_INTEGER, EPSILON ({}; V8's key order)
//! %Number.prototype% toString(radix): radix 10 is ToString; 2-36 is V8's
//!                    DoubleToRadixCString (`radix_string`); valueOf. Both read
//!                    thisNumberValue (Cx::this_primitive_value)
//! global             Number {w, c}
//! ```
//!
//! Not in week 4: `isFinite`, `isInteger`, `isNaN`, `isSafeInteger`, `parseFloat`,
//! `parseInt`, `toExponential`, `toFixed`, `toPrecision`, `toLocaleString`.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-number.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 4;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Number", 1, number), // 0
    BuiltinDef::method("toString", 1, to_string), // 1
    BuiltinDef::method("valueOf", 0, value_of),   // 2
];

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
            // §21.1.2.1-14, in V8's order.
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
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 1),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Number ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("Number"),
            IntrinsicId::Number,
            PropAttrs::METHOD,
        )],
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
