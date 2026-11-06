//! BigInt values on the builtin ABI (spec-temporal B4; H20 part 2, append-only): the `Cx`
//! methods that `ep9_builtins` bigint.rs, Number() and later `ep9_temporal` call. A builtin
//! never sees the digits: it holds a BigInt as a [`Local`] and converts through these.
//!
//! | Method | Spec | Errors |
//! |---|---|---|
//! | `is_bigint(v)` | Type(v) is BigInt | none |
//! | `to_numeric(v)` | ToNumeric (§7.1.3): a Number or a BigInt | as ToPrimitive, ToNumber |
//! | `to_bigint(v)` | ToBigInt (§7.1.13) | TypeError (undefined, null, Number, Symbol), SyntaxError (a String that is no StringIntegerLiteral), RangeError (size) |
//! | `number_to_bigint(n)` | NumberToBigInt (§21.2.1.1.1) | RangeError if `n` is not integral |
//! | `bigint_to_number(v)` | 𝔽(ℝ(v)), correctly rounded | none |
//! | `bigint_to_string(v, radix)` | BigInt::toString (§6.1.6.2.23) | RangeError "Invalid string length" |
//! | `bigint_as_int_n(bits, v)`, `bigint_as_uint_n(bits, v)` | BigInt.asIntN, BigInt.asUintN after their ToIndex and ToBigInt steps (§21.2.2.1-2) | RangeError (size) |
//! | `this_bigint_value(v)` | thisBigIntValue (§21.2.3.4.1) | TypeError |
//! | `bigint_from_i128(n)`, `bigint_to_i128(v)` | - (Temporal epoch nanoseconds) | none |
//!
//! A method that takes a BigInt panics for any other value (a builtin bug, as `Local` casts).

use ep9_value::JsResult;

use super::{Cx, JsString, Local};
use crate::bigint;
use crate::conv::{self, Numeric};
use crate::kinds;
use crate::wrapper;

impl<'s> Cx<'s> {
    /// Is `v` a BigInt (Type(v) is BigInt)?
    pub fn is_bigint(&self, v: impl Into<Local<'s>>) -> bool {
        kinds::is_bigint(v.into().bits())
    }

    /// ToNumeric (§7.1.3): ToPrimitive (hint number), then the BigInt itself or ToNumber of
    /// the primitive. The result is a Number or a BigInt.
    pub fn to_numeric(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        conv::to_numeric(self.cx, v.into().bits()).map(|n| match n {
            Numeric::Number(d) => Local::number(d),
            Numeric::BigInt(b) => Self::local(b),
        })
    }

    /// ToBigInt (§7.1.13).
    pub fn to_bigint(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        bigint::to_bigint(self.cx, v.into().bits()).map(Self::local)
    }

    /// NumberToBigInt (§21.2.1.1.1): RangeError if `n` is not an integral Number.
    pub fn number_to_bigint(&mut self, n: f64) -> JsResult<Local<'s>> {
        bigint::number_to_bigint(self.cx, n).map(Self::local)
    }

    /// 𝔽(ℝ(v)) of the BigInt `v`: the nearest Number (round half to even; ±Infinity past the
    /// largest finite Number).
    pub fn bigint_to_number(&self, v: impl Into<Local<'s>>) -> f64 {
        bigint::to_f64(Self::expect_bigint(v.into()))
    }

    /// BigInt::toString(v, radix) (§6.1.6.2.23) of the BigInt `v`; `radix` is 2..=36.
    /// RangeError "Invalid string length" when the text could pass the string length limit.
    pub fn bigint_to_string(
        &mut self,
        v: impl Into<Local<'s>>,
        radix: u32,
    ) -> JsResult<Local<'s, JsString>> {
        assert!((2..=36).contains(&radix), "radix {radix}");
        let b = Self::expect_bigint(v.into());
        bigint::to_string(self.cx, b, radix).map(Self::local)
    }

    /// BigInt.asIntN (§21.2.2.1) steps 3-4: the BigInt `v` modulo 2^bits as a signed
    /// `bits`-bit integer (`bits` is the ToIndex result).
    pub fn bigint_as_int_n(&mut self, bits: u64, v: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        let b = Self::expect_bigint(v.into());
        bigint::as_n(self.cx, b, bits, true).map(Self::local)
    }

    /// BigInt.asUintN (§21.2.2.2) step 3: the BigInt `v` modulo 2^bits (`bits` is the
    /// ToIndex result). RangeError when the result would pass the size limit.
    pub fn bigint_as_uint_n(&mut self, bits: u64, v: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        let b = Self::expect_bigint(v.into());
        bigint::as_n(self.cx, b, bits, false).map(Self::local)
    }

    /// thisBigIntValue (§21.2.3.4.1): `v` if it is a BigInt, the BigInt inside `v` if it is a
    /// BigInt wrapper (`[[BigIntData]]`), else a TypeError.
    pub fn this_bigint_value(&mut self, v: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        let v = v.into().bits();
        if kinds::is_bigint(v) {
            return Ok(Self::local(v));
        }
        if kinds::is_object(v)
            && let Some(p) = wrapper::wrapper_value(v)
            && kinds::is_bigint(p)
        {
            return Ok(Self::local(p));
        }
        let msg = format!(
            "BigInt.prototype method called on an incompatible receiver {}",
            conv::describe(v)
        );
        Err(self.throw_type_error(&msg))
    }

    /// The BigInt of `n`.
    pub fn bigint_from_i128(&mut self, n: i128) -> JsResult<Local<'s>> {
        Ok(Self::local(bigint::from_i128(self.rt(), n)))
    }

    /// The BigInt `v` as an i128, or `None` if it does not fit.
    pub fn bigint_to_i128(&self, v: impl Into<Local<'s>>) -> Option<i128> {
        bigint::to_i128(Self::expect_bigint(v.into()))
    }

    /// The bits of the BigInt `v`; panics for another value (a builtin bug).
    fn expect_bigint(v: Local<'s>) -> u64 {
        let b = v.bits();
        assert!(kinds::is_bigint(b), "not a BigInt: {}", conv::describe(b));
        b
    }
}
