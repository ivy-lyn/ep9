//! Math builtins (§21.3): module `math`, block 8. Makes `%Math%` (`IntrinsicId::Math`, an
//! ordinary object whose prototype is `%Object.prototype%`).
//!
//! ```text
//! %Math%     E, LN10, LN2, LOG10E, LOG2E, PI, SQRT1_2, SQRT2 ({}), [@@toStringTag] "Math" ({c})
//!            abs, ceil, floor, fround, max, min, pow, round, sign, sqrt, trunc ({w, c})
//! global     Math {w, c}
//! ```
//!
//! | Function | Computed by (H36: one Rust Math, no libm) |
//! |---|---|
//! | abs, ceil, floor, trunc, sqrt | the exact `f64` operations (IEEE 754; allowed by clippy.toml) |
//! | sign, max, min | the spec steps |
//! | round, fround | `ep9_math::round_js`, `ep9_math::fround` |
//! | pow | `ep9_math::pow_js` v0: `None` (needs the correctly rounded core, month 2) throws "not implemented" with the text of `**` |
//!
//! Not in week 4: the libm-class functions (acos ... tanh, exp, log, hypot, cbrt), clz32,
//! imul, random, f16round: they need ep9_math's correctly rounded cores (H36, month 2).
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-math.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 8;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("abs", 1, abs),       // 0
    BuiltinDef::method("ceil", 1, ceil),     // 1
    BuiltinDef::method("floor", 1, floor),   // 2
    BuiltinDef::method("fround", 1, fround), // 3
    BuiltinDef::method("max", 2, max),       // 4
    BuiltinDef::method("min", 2, min),       // 5
    BuiltinDef::method("pow", 2, pow),       // 6
    BuiltinDef::method("round", 1, round),   // 7
    BuiltinDef::method("sign", 1, sign),     // 8
    BuiltinDef::method("sqrt", 1, sqrt),     // 9
    BuiltinDef::method("trunc", 1, trunc),   // 10
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Math,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// A value property of `%Math%` ({}: §21.3.1).
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
        target: IntrinsicId::Math,
        // V8's order: the functions, then the constants, then @@toStringTag.
        props: &[
            PropDef::method(PropKey::Str("abs"), 0),
            PropDef::method(PropKey::Str("ceil"), 1),
            PropDef::method(PropKey::Str("floor"), 2),
            PropDef::method(PropKey::Str("fround"), 3),
            PropDef::method(PropKey::Str("max"), 4),
            PropDef::method(PropKey::Str("min"), 5),
            PropDef::method(PropKey::Str("pow"), 6),
            PropDef::method(PropKey::Str("round"), 7),
            PropDef::method(PropKey::Str("sign"), 8),
            PropDef::method(PropKey::Str("sqrt"), 9),
            PropDef::method(PropKey::Str("trunc"), 10),
            // §21.3.1.1-8: the Number values closest to the real constants (Rust's `consts`
            // are those values: the node texts are in tests/math.rs).
            constant("E", core::f64::consts::E),
            constant("LN10", core::f64::consts::LN_10),
            constant("LN2", core::f64::consts::LN_2),
            constant("LOG10E", core::f64::consts::LOG10_E),
            constant("LOG2E", core::f64::consts::LOG2_E),
            constant("PI", core::f64::consts::PI),
            constant("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
            constant("SQRT2", core::f64::consts::SQRT_2),
            // §21.3.1.9 Math [ %Symbol.toStringTag% ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Math"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.4.3 Math.
        props: &[PropDef::intrinsic(
            PropKey::Str("Math"),
            IntrinsicId::Math,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "math",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The not-implemented text of `**` (ep9_vm conv.rs): Math.pow and `**` share
/// Number::exponentiate, so they fail on the same inputs with the same cause.
const POW_NOT_IMPLEMENTED: &str = "** with a result that is not an exact integer: needs the correctly rounded pow core (H36, month 2)";

/// `Math.f(x)` for a function of one Number that cannot fail after ToNumber.
fn unary<'s>(cx: &mut Cx<'s>, args: Args<'s>, f: fn(f64) -> f64) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    let n = cx.to_number(args.get(0))?;
    Ok(Local::number(f(n)))
}

/// Math.abs ( x ) (§21.3.2.1).
fn abs<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is NaN, return NaN.
    // 3. If n is -0𝔽, return +0𝔽.
    // 4. If n is -∞𝔽, return +∞𝔽.
    // 5. If n < -0𝔽, return -n.
    // 6. Return n.
    // (`f64::abs` clears the sign bit: all of steps 2-6.)
    unary(cx, args, f64::abs)
}

/// Math.ceil ( x ) (§21.3.2.10).
fn ceil<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is not finite or n is either +0𝔽 or -0𝔽, return n.
    // 3. If n < -0𝔽 and n > -1𝔽, return -0𝔽.
    // 4. If n is an integral Number, return n.
    // 5. Return the smallest (closest to -∞) integral Number value that is not less than n.
    // (IEEE roundToIntegralTowardPositive: `f64::ceil`, -0.5 gives -0.)
    unary(cx, args, f64::ceil)
}

/// Math.floor ( x ) (§21.3.2.16).
fn floor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is not finite or n is either +0𝔽 or -0𝔽, return n.
    // 3. If n < 1𝔽 and n > +0𝔽, return +0𝔽.
    // 4. If n is an integral Number, return n.
    // 5. Return the greatest (closest to +∞) integral Number value that is not greater than
    //    n.
    // (IEEE roundToIntegralTowardNegative: `f64::floor`.)
    unary(cx, args, f64::floor)
}

/// Math.fround ( x ) (§21.3.2.17).
fn fround<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is NaN, return NaN.
    // 3. If n is one of +0𝔽, -0𝔽, +∞𝔽, or -∞𝔽, return n.
    // 4. Let n32 be the result of converting n to IEEE 754-2019 binary32 format using
    //    roundTiesToEven mode.
    // 5. Let n64 be the result of converting n32 to IEEE 754-2019 binary64 format.
    // 6. Return the ECMAScript Number value corresponding to n64.
    unary(cx, args, ep9_math::fround)
}

fn is_plus_zero(x: f64) -> bool {
    x == 0.0 && x.is_sign_positive()
}

fn is_minus_zero(x: f64) -> bool {
    x == 0.0 && x.is_sign_negative()
}

/// Math.max ( ...args ) (§21.3.2.24).
fn max<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let coerced be a new empty List.
    // 2. For each element arg of args, do
    //    a. Let n be ? ToNumber(arg).
    //    b. Append n to coerced.
    // 3. Let highest be -∞𝔽.
    // 4. For each element number of coerced, do
    //    a. If number is NaN, return NaN.
    //    b. If number is +0𝔽 and highest is -0𝔽, set highest to +0𝔽.
    //    c. If number > highest, set highest to number.
    // 5. Return highest.
    // (Steps 2 and 4 in one loop: every argument is still converted, in order, before a
    // NaN is returned, and the fold does not depend on the order.)
    let mut nan = false;
    let mut highest = f64::NEG_INFINITY;
    for arg in args.iter() {
        let number = cx.to_number(arg)?;
        if number.is_nan() {
            nan = true;
        } else if (is_plus_zero(number) && is_minus_zero(highest)) || number > highest {
            highest = number;
        }
    }
    Ok(Local::number(if nan { f64::NAN } else { highest }))
}

/// Math.min ( ...args ) (§21.3.2.25).
fn min<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let coerced be a new empty List.
    // 2. For each element arg of args, do
    //    a. Let n be ? ToNumber(arg).
    //    b. Append n to coerced.
    // 3. Let lowest be +∞𝔽.
    // 4. For each element number of coerced, do
    //    a. If number is NaN, return NaN.
    //    b. If number is -0𝔽 and lowest is +0𝔽, set lowest to -0𝔽.
    //    c. If number < lowest, set lowest to number.
    // 5. Return lowest.
    // (As in `max`: one loop.)
    let mut nan = false;
    let mut lowest = f64::INFINITY;
    for arg in args.iter() {
        let number = cx.to_number(arg)?;
        if number.is_nan() {
            nan = true;
        } else if (is_minus_zero(number) && is_plus_zero(lowest)) || number < lowest {
            lowest = number;
        }
    }
    Ok(Local::number(if nan { f64::NAN } else { lowest }))
}

/// Math.pow ( base, exponent ) (§21.3.2.26).
fn pow<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set base to ? ToNumber(base).
    let base = cx.to_number(args.get(0))?;
    // 2. Set exponent to ? ToNumber(exponent).
    let exponent = cx.to_number(args.get(1))?;
    // 3. Return Number::exponentiate(base, exponent).
    match ep9_math::pow_js(base, exponent) {
        Some(r) => Ok(Local::number(r)),
        None => Err(cx.not_implemented(POW_NOT_IMPLEMENTED)),
    }
}

/// Math.round ( x ) (§21.3.2.28).
fn round<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is not finite or n is an integral Number, return n.
    // 3. If n < 0.5𝔽 and n > +0𝔽, return +0𝔽.
    // 4. If n < -0𝔽 and n ≥ -0.5𝔽, return -0𝔽.
    // 5. Return the integral Number closest to n, preferring the Number closer to +∞ in the
    //    case of a tie.
    unary(cx, args, ep9_math::round_js)
}

/// Math.sign ( x ) (§21.3.2.29).
fn sign<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    unary(cx, args, |n| {
        // 2. If n is one of NaN, +0𝔽, or -0𝔽, return n.
        if n.is_nan() || n == 0.0 {
            n
        // 3. If n < -0𝔽, return -1𝔽.
        } else if n < 0.0 {
            -1.0
        // 4. Return 1𝔽.
        } else {
            1.0
        }
    })
}

/// Math.sqrt ( x ) (§21.3.2.32).
fn sqrt<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is not finite or n is either +0𝔽 or -0𝔽, return n.
    // 3. If n < -0𝔽, return NaN.
    // 4. Return 𝔽(the square root of ℝ(n)).
    // (IEEE squareRoot, correctly rounded: `f64::sqrt`; sqrt(-0) is -0, sqrt(+∞) is +∞.)
    unary(cx, args, f64::sqrt)
}

/// Math.trunc ( x ) (§21.3.2.35).
fn trunc<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToNumber(x).
    // 2. If n is not finite or n is either +0𝔽 or -0𝔽, return n.
    // 3. If n < 1𝔽 and n > +0𝔽, return +0𝔽.
    // 4. If n < -0𝔽 and n > -1𝔽, return -0𝔽.
    // 5. Return the integral Number nearest n in the direction of +0𝔽.
    // (IEEE roundToIntegralTowardZero: `f64::trunc`.)
    unary(cx, args, f64::trunc)
}
