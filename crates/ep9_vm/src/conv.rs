//! Type conversions and operators (ECMA-262 §7.1, §7.2, §13). v0: no BigInt values, so
//! ToNumeric is ToNumber and `typeof` never says "bigint".
//!
//! Functions that can run JS (ToPrimitive calls `@@toPrimitive`, `valueOf`/`toString`) take
//! `&mut Cx` and return `Res`: `Err(Throw)` means the exception is pending in the runtime.
//!
//! Symbols (H19 "Symbols"):
//!
//! | Operation | On a symbol |
//! |---|---|
//! | `typeof` | "symbol" |
//! | ToBoolean | true |
//! | ToNumber, ToNumeric (`+s`, `s - 1`, `s < 1`) | TypeError "Cannot convert a Symbol value to a number" |
//! | ToString (`${s}`, `s + ""`) | TypeError "Cannot convert a Symbol value to a string" |
//! | ToPropertyKey | the symbol itself (`key.rs`) |
//! | ToObject | a Symbol wrapper (`wrapper.rs`) |
//! | `describe`, `key::key_text` | `Symbol(desc)` (SymbolDescriptiveString, `symbol::descriptive_text`) |

use ep9_value::{Throw, Value, to_int32, to_uint32};

use crate::error::{not_implemented, throw_error};
use crate::frame::{FALSE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::{is_callable, is_symbol};
use crate::numconv::{exponentiate, fmod, number_to_string, string_to_number};
use crate::object::{self, is_function, is_object, is_string, str_view};
use crate::runtime::{Atom, ErrorKind, Root, val};
use crate::symbol::{self, WellKnownSymbol};
use crate::{atoms, property, wrapper};

pub(crate) use crate::key::to_property_key;

pub(crate) type Res<T = u64> = Result<T, Throw>;

/// Value bits of the number `d` (int32 form when it fits, H6).
#[inline]
pub(crate) fn num(d: f64) -> u64 {
    Value::number(d).to_bits()
}

#[inline]
pub(crate) fn boolean(b: bool) -> u64 {
    if b { TRUE } else { FALSE }
}

/// The number in `b`, if it is a number.
#[inline]
pub(crate) fn as_number(b: u64) -> Option<f64> {
    val(b).as_number()
}

/// ToBoolean (§7.1.2).
pub(crate) fn to_boolean(b: u64) -> bool {
    match b {
        TRUE => true,
        FALSE | UNDEFINED | NULL => false,
        _ => {
            let v = val(b);
            if let Some(i) = v.as_int32() {
                i != 0
            } else if let Some(d) = v.as_double() {
                !(d == 0.0 || d.is_nan())
            } else if is_string(b) {
                object::str_len(b) != 0
            } else {
                // Symbols and objects (v0: no document.all).
                true
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hint {
    Default,
    Number,
    String,
}

/// ToPrimitive (§7.1.1): a primitive as it is; for an object, its `@@toPrimitive` method
/// called with the hint "default", "number" or "string", else OrdinaryToPrimitive.
pub(crate) fn to_primitive(cx: &mut Cx, b: u64, hint: Hint) -> Res<u64> {
    if !is_object(b) {
        return Ok(b);
    }
    let rt = cx.rt();
    // Step 1.a: GetMethod(input, @@toPrimitive) (§7.3.10). The symbol is undefined only
    // while the realm is being built (`symbol::install`), when no conversion runs.
    let key = symbol::well_known(rt, WellKnownSymbol::ToPrimitive);
    if key != UNDEFINED {
        let exotic = property::get(cx, b, key)?;
        if exotic != UNDEFINED && exotic != NULL {
            if !is_callable(exotic) {
                let msg = format!("{} is not a function", describe(exotic));
                return Err(throw_error(cx, ErrorKind::TypeError, &msg));
            }
            // Steps 1.b.i-v. GC point (the "default" atom): `b` and `exotic` are Rust locals.
            let hint_string = match hint {
                Hint::Default => atoms::intern_str(rt, "default"),
                Hint::Number => rt.atom(Atom::Number),
                Hint::String => rt.atom(Atom::String),
            };
            let r = crate::call::call_function(cx, exotic, b, &[hint_string])?;
            if !is_object(r) {
                return Ok(r);
            }
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot convert object to primitive value",
            ));
        }
    }
    ordinary_to_primitive(cx, b, hint)
}

/// OrdinaryToPrimitive (§7.1.1.1): `toString` then `valueOf` for the hint "string",
/// `valueOf` then `toString` otherwise.
fn ordinary_to_primitive(cx: &mut Cx, b: u64, hint: Hint) -> Res<u64> {
    let rt = cx.rt();
    let order = if hint == Hint::String {
        [Atom::ToStringKey, Atom::ValueOf]
    } else {
        [Atom::ValueOf, Atom::ToStringKey]
    };
    for name in order {
        let f = property::get(cx, b, rt.atom(name))?;
        if is_callable(f) {
            let r = crate::call::call_function(cx, f, b, &[])?;
            if !is_object(r) {
                return Ok(r);
            }
        }
    }
    Err(throw_error(
        cx,
        ErrorKind::TypeError,
        "Cannot convert object to primitive value",
    ))
}

/// ToNumber (§7.1.4).
pub(crate) fn to_number(cx: &mut Cx, b: u64) -> Res<f64> {
    if let Some(d) = as_number(b) {
        return Ok(d);
    }
    Ok(match b {
        UNDEFINED => f64::NAN,
        NULL | FALSE => 0.0,
        TRUE => 1.0,
        _ if is_string(b) => string_to_number(&str_view(b).to_utf16()),
        _ if is_object(b) => {
            let p = to_primitive(cx, b, Hint::Number)?;
            return to_number(cx, p);
        }
        _ if is_symbol(b) => {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot convert a Symbol value to a number",
            ));
        }
        _ => return Err(not_implemented(cx, "ToNumber of this value")),
    })
}

/// ToNumeric (§7.1.3). v0: no BigInt.
pub(crate) fn to_numeric(cx: &mut Cx, b: u64) -> Res<f64> {
    to_number(cx, b)
}

pub(crate) fn to_int32_of(cx: &mut Cx, b: u64) -> Res<i32> {
    if let Some(i) = val(b).as_int32() {
        return Ok(i);
    }
    Ok(to_int32(to_number(cx, b)?))
}

pub(crate) fn to_uint32_of(cx: &mut Cx, b: u64) -> Res<u32> {
    Ok(to_uint32(to_number(cx, b)?))
}

/// A heap string for the number `d`.
pub(crate) fn number_string(cx: &mut Cx, d: f64) -> u64 {
    object::new_string(cx.rt(), &number_to_string(d))
}

/// ToString (§7.1.17), as a heap string.
pub(crate) fn to_string(cx: &mut Cx, b: u64) -> Res<u64> {
    if is_string(b) {
        return Ok(b);
    }
    let rt = cx.rt();
    if let Some(d) = as_number(b) {
        return Ok(number_string(cx, d));
    }
    Ok(match b {
        UNDEFINED => rt.atom(Atom::Undefined),
        NULL => rt.atom(Atom::Null),
        TRUE => rt.atom(Atom::True),
        FALSE => rt.atom(Atom::False),
        _ if is_object(b) => {
            let p = to_primitive(cx, b, Hint::String)?;
            return to_string(cx, p);
        }
        _ if is_symbol(b) => {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot convert a Symbol value to a string",
            ));
        }
        _ => return Err(not_implemented(cx, "ToString of this value")),
    })
}

/// ToString as Rust text (lone surrogates become U+FFFD).
pub(crate) fn to_rust_string(cx: &mut Cx, b: u64) -> Res<String> {
    let s = to_string(cx, b)?;
    Ok(str_view(s).to_string_lossy())
}

/// `typeof` (§13.5.3).
pub(crate) fn type_of(b: u64) -> Atom {
    match b {
        UNDEFINED => Atom::Undefined,
        NULL => Atom::Object,
        TRUE | FALSE => Atom::Boolean,
        _ if val(b).is_number() => Atom::Number,
        _ if is_string(b) => Atom::String,
        _ if is_symbol(b) => Atom::Symbol,
        _ if is_callable(b) => Atom::Function,
        _ => Atom::Object,
    }
}

/// ToObject (§7.1.18): an object as it is; a wrapper object for a string, number, boolean or
/// symbol (its prototype is the realm's `%String.prototype%`, `%Number.prototype%`,
/// `%Boolean.prototype%` or `%Symbol.prototype%`); TypeError for undefined and null. GC point.
pub(crate) fn to_object(cx: &mut Cx, b: u64) -> Res<u64> {
    if is_object(b) {
        return Ok(b);
    }
    let rt = cx.rt();
    let proto = match b {
        UNDEFINED | NULL => {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot convert undefined or null to object",
            ));
        }
        TRUE | FALSE => rt.root(Root::BooleanPrototype),
        _ if val(b).is_number() => rt.root(Root::NumberPrototype),
        _ if is_string(b) => rt.root(Root::StringPrototype),
        _ if is_symbol(b) => symbol::prototype(rt),
        _ => return Err(not_implemented(cx, "ToObject of this value")),
    };
    Ok(wrapper::new_wrapper(rt, proto, b))
}

/// IsStrictlyEqual (§7.2.16).
pub(crate) fn strict_equals(a: u64, b: u64) -> bool {
    if a == b {
        // Same bits: equal unless NaN (NaN is boxed as one pattern).
        return !as_number(a).is_some_and(f64::is_nan);
    }
    match (as_number(a), as_number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => is_string(a) && is_string(b) && str_view(a).eq(&str_view(b)),
    }
}

/// IsLooselyEqual (§7.2.15).
pub(crate) fn loose_equals(cx: &mut Cx, a: u64, b: u64) -> Res<bool> {
    let (na, nb) = (as_number(a).is_some(), as_number(b).is_some());
    let (sa, sb) = (is_string(a), is_string(b));
    let (oa, ob) = (is_object(a), is_object(b));
    let (ya, yb) = (is_symbol(a), is_symbol(b));
    let nullish = |x: u64| x == UNDEFINED || x == NULL;
    let boolish = |x: u64| x == TRUE || x == FALSE;
    if (na && nb) || (sa && sb) || (oa && ob) || a == b {
        return Ok(strict_equals(a, b));
    }
    if nullish(a) || nullish(b) {
        return Ok(nullish(a) && nullish(b));
    }
    if na && sb {
        let y = to_number(cx, b)?;
        return Ok(as_number(a) == Some(y));
    }
    if sa && nb {
        let x = to_number(cx, a)?;
        return Ok(Some(x) == as_number(b));
    }
    if boolish(a) {
        let x = num(f64::from(u8::from(a == TRUE)));
        return loose_equals(cx, x, b);
    }
    if boolish(b) {
        let y = num(f64::from(u8::from(b == TRUE)));
        return loose_equals(cx, a, y);
    }
    // Steps 11-12 (BigInt: not in v0).
    if (na || sa || ya) && ob {
        let p = to_primitive(cx, b, Hint::Default)?;
        return loose_equals(cx, a, p);
    }
    if oa && (nb || sb || yb) {
        let p = to_primitive(cx, a, Hint::Default)?;
        return loose_equals(cx, p, b);
    }
    Ok(false)
}

/// IsLessThan (§7.2.13): `Some(a < b)`, or `None` for undefined (a NaN operand).
pub(crate) fn less_than(cx: &mut Cx, a: u64, b: u64, left_first: bool) -> Res<Option<bool>> {
    let (pa, pb) = if left_first {
        let pa = to_primitive(cx, a, Hint::Number)?;
        let pb = to_primitive(cx, b, Hint::Number)?;
        (pa, pb)
    } else {
        let pb = to_primitive(cx, b, Hint::Number)?;
        let pa = to_primitive(cx, a, Hint::Number)?;
        (pa, pb)
    };
    if is_string(pa) && is_string(pb) {
        return Ok(Some(
            str_view(pa).cmp(&str_view(pb)) == core::cmp::Ordering::Less,
        ));
    }
    let x = to_number(cx, pa)?;
    let y = to_number(cx, pb)?;
    if x.is_nan() || y.is_nan() {
        return Ok(None);
    }
    Ok(Some(x < y))
}

/// The relational operators: `lhs OP rhs` as a boolean (§13.10.1).
pub(crate) fn compare(cx: &mut Cx, op: CompareOp, lhs: u64, rhs: u64) -> Res<bool> {
    Ok(match op {
        CompareOp::Lt => less_than(cx, lhs, rhs, true)? == Some(true),
        CompareOp::Gt => less_than(cx, rhs, lhs, false)? == Some(true),
        CompareOp::Le => less_than(cx, rhs, lhs, false)? == Some(false),
        CompareOp::Ge => less_than(cx, lhs, rhs, true)? == Some(false),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompareOp {
    Lt,
    Gt,
    Le,
    Ge,
}

/// Binary numeric operators (§6.1.6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Sar,
    Shr,
}

/// `lhs OP rhs` on two numbers. No number operation throws since `**` has its correctly
/// rounded core (M4-P1); the `Cx` and `Res` stay for the callers.
pub(crate) fn number_op(_cx: &mut Cx, op: BinOp, x: f64, y: f64) -> Res<u64> {
    Ok(match op {
        BinOp::Add => num(x + y),
        BinOp::Sub => num(x - y),
        BinOp::Mul => num(x * y),
        BinOp::Div => num(x / y),
        BinOp::Mod => num(fmod(x, y)),
        BinOp::Exp => num(exponentiate(x, y)),
        BinOp::BitOr => num(f64::from(to_int32(x) | to_int32(y))),
        BinOp::BitXor => num(f64::from(to_int32(x) ^ to_int32(y))),
        BinOp::BitAnd => num(f64::from(to_int32(x) & to_int32(y))),
        BinOp::Shl => num(f64::from(to_int32(x).wrapping_shl(to_uint32(y) & 31))),
        BinOp::Sar => num(f64::from(to_int32(x) >> (to_uint32(y) & 31))),
        BinOp::Shr => num(f64::from(to_uint32(x) >> (to_uint32(y) & 31))),
    })
}

/// ApplyStringOrNumericBinaryOperator (§13.15.3) for `lhs OP rhs`.
pub(crate) fn binary(cx: &mut Cx, op: BinOp, lhs: u64, rhs: u64) -> Res<u64> {
    if op == BinOp::Add {
        let lp = to_primitive(cx, lhs, Hint::Default)?;
        let rp = to_primitive(cx, rhs, Hint::Default)?;
        if is_string(lp) || is_string(rp) {
            let ls = to_string(cx, lp)?;
            let rs = to_string(cx, rp)?;
            return match object::concat(cx.rt(), ls, rs) {
                Some(s) => Ok(s),
                None => Err(throw_error(
                    cx,
                    ErrorKind::RangeError,
                    "Invalid string length",
                )),
            };
        }
        let x = to_numeric(cx, lp)?;
        let y = to_numeric(cx, rp)?;
        return number_op(cx, op, x, y);
    }
    let x = to_numeric(cx, lhs)?;
    let y = to_numeric(cx, rhs)?;
    number_op(cx, op, x, y)
}

/// Text of a value for error messages (never runs JS).
pub(crate) fn describe(b: u64) -> String {
    if is_string(b) {
        return format!("\"{}\"", str_view(b).to_string_lossy());
    }
    if let Some(d) = as_number(b) {
        return number_to_string(d);
    }
    match b {
        UNDEFINED => "undefined".into(),
        NULL => "null".into(),
        TRUE => "true".into(),
        FALSE => "false".into(),
        _ if is_function(b) => {
            let n = &object::function_info(b).name;
            if n.is_empty() {
                "function".into()
            } else {
                format!("function {n}")
            }
        }
        _ if is_symbol(b) => symbol::descriptive_text(b),
        _ if is_callable(b) => "function".into(),
        _ if object::is_array(b) => "array".into(),
        _ => "object".into(),
    }
}
