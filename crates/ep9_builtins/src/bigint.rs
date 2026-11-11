//! BigInt builtins (§21.2): module `bigint`, block 18 (spec-temporal B4). Makes `%BigInt%`
//! and `%BigInt.prototype%` (`IntrinsicId::BigInt`, `IntrinsicId::BigIntPrototype`). The
//! digits stay in ep9_vm: every step goes through the BigInt `Cx` methods (H20 part 2).
//!
//! ```text
//! BigInt(v)               not with new (TypeError); ToPrimitive(v, number); a Number:
//!                         NumberToBigInt (Cx::number_to_bigint); else ToBigInt (Cx::to_bigint)
//! BigInt.asIntN(b, x)     ToIndex(b), ToBigInt(x), then Cx::bigint_as_int_n
//! BigInt.asUintN(b, x)    ToIndex(b), ToBigInt(x), then Cx::bigint_as_uint_n
//! %BigInt.prototype%      an ordinary object: toLocaleString (ECMA-402: Intl.NumberFormat),
//!                         toString(radix), valueOf, @@toStringTag "BigInt"
//! global                  BigInt {w, c}
//! ```
//!
//! BigInt64Array, BigUint64Array, the DataView BigInt methods and Atomics on BigInts wait for
//! the TypedArray lane (design SPEC-T-D9).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-bigint.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, ToPrimitiveHint,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 18;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("BigInt", 1, bigint), // 0
    BuiltinDef::method("asIntN", 2, as_int_n),    // 1
    BuiltinDef::method("asUintN", 2, as_uint_n),  // 2
    BuiltinDef::method("toString", 0, to_string), // 3
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 4
    BuiltinDef::method("valueOf", 0, value_of),   // 5
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::BigInt,
        value: IntrinsicValue::Builtin(0),
    },
    // §21.2.3: %BigInt.prototype% is an ordinary object (not a BigInt wrapper).
    IntrinsicDef {
        id: IntrinsicId::BigIntPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::BigInt,
        props: &[
            // §21.2.2.3 BigInt.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::BigIntPrototype,
                PropAttrs::NONE,
            ),
            // §21.2.2.1 BigInt.asIntN, §21.2.2.2 BigInt.asUintN.
            PropDef::method(PropKey::Str("asIntN"), 1),
            PropDef::method(PropKey::Str("asUintN"), 2),
        ],
    },
    ObjectProps {
        target: IntrinsicId::BigIntPrototype,
        props: &[
            // §21.2.3.1 BigInt.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::BigInt,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Str("toLocaleString"), 4),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 3),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 5),
            // §21.2.3.5 BigInt.prototype [ @@toStringTag ] ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("BigInt"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 BigInt ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("BigInt"),
            IntrinsicId::BigInt,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "bigint",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// BigInt ( value ) (§21.2.1.1).
fn bigint<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is not undefined, throw a TypeError exception.
    if !new_target.is_undefined() {
        return Err(cx.throw_type_error("BigInt is not a constructor"));
    }
    // 2. Let prim be ? ToPrimitive(value, number).
    let prim = cx.to_primitive(args.get(0), ToPrimitiveHint::Number)?;
    // 3. If prim is a Number, return ? NumberToBigInt(prim).
    if let Some(n) = prim.as_number() {
        return cx.number_to_bigint(n);
    }
    // 4. Otherwise, return ? ToBigInt(prim).
    cx.to_bigint(prim)
}

/// ToIndex (§7.1.22): an integer in [0, 2^53 - 1], else a RangeError.
fn to_index<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<u64> {
    // 1. Let integer be ? ToIntegerOrInfinity(value).
    let integer = cx.to_integer_or_infinity(v)?;
    // 2. If integer is not in the inclusive interval from 0 to 2^53 - 1, throw a RangeError.
    if !(0.0..=9_007_199_254_740_991.0).contains(&integer) {
        return Err(cx.throw_range_error("Invalid value: not (convertible to) a safe integer"));
    }
    // 3. Return integer.
    Ok(integer as u64)
}

/// BigInt.asIntN ( bits, bigint ) (§21.2.2.1).
fn as_int_n<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set bits to ? ToIndex(bits).
    let bits = to_index(cx, args.get(0))?;
    // 2. Set bigint to ? ToBigInt(bigint).
    let b = cx.to_bigint(args.get(1))?;
    // 3-4. Let mod be ℝ(bigint) modulo 2^bits; if mod >= 2^(bits - 1), return
    //      ℤ(mod - 2^bits); otherwise, return ℤ(mod).
    cx.bigint_as_int_n(bits, b)
}

/// BigInt.asUintN ( bits, bigint ) (§21.2.2.2).
fn as_uint_n<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set bits to ? ToIndex(bits).
    let bits = to_index(cx, args.get(0))?;
    // 2. Set bigint to ? ToBigInt(bigint).
    let b = cx.to_bigint(args.get(1))?;
    // 3. Return ℤ(ℝ(bigint) modulo 2^bits).
    cx.bigint_as_uint_n(bits, b)
}

/// BigInt.prototype.toString ( \[ radix \] ) (§21.2.3.3).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisBigIntValue(this value).
    let x = cx.this_bigint_value(this)?;
    let radix = args.get(0);
    // 2. If radix is undefined, let radixMV be 10.
    let radix_mv = if radix.is_undefined() {
        10.0
    } else {
        // 3. Else, let radixMV be ? ToIntegerOrInfinity(radix).
        cx.to_integer_or_infinity(radix)?
    };
    // 4. If radixMV is not in the inclusive interval from 2 to 36, throw a RangeError.
    if !(2.0..=36.0).contains(&radix_mv) {
        return Err(cx.throw_range_error("toString() radix must be between 2 and 36"));
    }
    // 5. Return BigInt::toString(x, radixMV).
    cx.bigint_to_string(x, radix_mv as u32).map(|s| s.value())
}

/// BigInt.prototype.toLocaleString ( [ locales [ , options ] ] ) (ECMA-402 §20.3.1; INTL
/// W2 NF-CORE): the Intl.NumberFormat of `locales` and `options` formats the BigInt.
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let x be ? thisBigIntValue(this value).
    let x = cx.this_bigint_value(this)?;
    // 2. Let numberFormat be ? Construct(%Intl.NumberFormat%, « locales, options »).
    let r = ep9_intl::number_format::locale_string_format(cx, args.get(0), args.get(1))?;
    // 3. Return FormatNumeric(numberFormat, ℝ(x)).
    let mv = ep9_intl::number_format::decimal::to_intl_mv(cx, x)?;
    let s = ep9_intl::number_format::format_numeric(&r, &mv);
    Ok(cx.new_string(&s)?.into())
}

/// BigInt.prototype.valueOf ( ) (§21.2.3.4).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisBigIntValue(this value).
    cx.this_bigint_value(this)
}
