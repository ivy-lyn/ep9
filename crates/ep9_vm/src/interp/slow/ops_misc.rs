//! Operators, conversions and other value-only entries.

use ep9_bytecode::Constant;

use crate::bigint;
use crate::conv::{self, BinOp, CompareOp, Numeric, Res};
use crate::interp::Cx;
use crate::property;

use super::{enter, info, ret};

// ---- operators -----------------------------------------------------------------------------

#[cold]
#[inline(never)]
pub(crate) fn binary(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: BinOp, l: u64, r: u64) -> u64 {
    enter(cx, fp, pc);
    ret(conv::binary(cx, op, l, r))
}

/// Unary operators and conversions on the accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnOp {
    Inc,
    Dec,
    Negate,
    BitNot,
    ToNumber,
    ToNumeric,
    ToName,
    ToString,
    ToObject,
}

#[cold]
#[inline(never)]
pub(crate) fn unary(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: UnOp, v: u64) -> u64 {
    enter(cx, fp, pc);
    let r = (|| -> Res<u64> {
        // The numeric operators: ToNumeric, then the Number or the BigInt operation.
        let numeric = |cx: &mut Cx, n: fn(f64) -> f64, b: bigint::UnaryOp| -> Res<u64> {
            match conv::to_numeric(cx, v)? {
                Numeric::Number(d) => Ok(conv::num(n(d))),
                Numeric::BigInt(x) => bigint::unary(cx, b, x),
            }
        };
        Ok(match op {
            UnOp::Inc => numeric(cx, |d| d + 1.0, bigint::UnaryOp::Inc)?,
            UnOp::Dec => numeric(cx, |d| d - 1.0, bigint::UnaryOp::Dec)?,
            UnOp::Negate => numeric(cx, |d| -d, bigint::UnaryOp::Negate)?,
            UnOp::BitNot => numeric(
                cx,
                |d| f64::from(!ep9_value::to_int32(d)),
                bigint::UnaryOp::BitNot,
            )?,
            UnOp::ToNumber => conv::num(conv::to_number(cx, v)?),
            // ToNumeric keeps a BigInt (`x++` is ToNumeric, then Inc).
            UnOp::ToNumeric => conv::to_numeric(cx, v)?.bits(),
            UnOp::ToName => conv::to_property_key(cx, v)?,
            UnOp::ToString => conv::to_string(cx, v)?,
            // A wrapper object for a primitive (§7.1.18); TypeError for undefined and null.
            UnOp::ToObject => conv::to_object(cx, v)?,
        })
    })();
    ret(r)
}

/// Comparison operators: `l OP r`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Lt,
    Gt,
    Le,
    Ge,
    InstanceOf,
    In,
}

#[cold]
#[inline(never)]
pub(crate) fn compare(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: CmpOp, l: u64, r: u64) -> u64 {
    enter(cx, fp, pc);
    let b = match op {
        CmpOp::Eq => conv::loose_equals(cx, l, r),
        CmpOp::Lt => conv::compare(cx, CompareOp::Lt, l, r),
        CmpOp::Gt => conv::compare(cx, CompareOp::Gt, l, r),
        CmpOp::Le => conv::compare(cx, CompareOp::Le, l, r),
        CmpOp::Ge => conv::compare(cx, CompareOp::Ge, l, r),
        CmpOp::InstanceOf => property::instance_of(cx, l, r),
        // `#x in o` (a private name, symbol.rs) goes to the private path (C7).
        CmpOp::In if crate::symbol::is_private_key(l) => super::private::keyed_has(cx, r, l),
        CmpOp::In => property::has(cx, r, l),
    };
    ret(b.map(conv::boolean))
}

// ---- misc ----------------------------------------------------------------------------------

/// `SwitchOnSmi(table, base)` with an int32 accumulator: the jump target, or the next
/// instruction if the value is outside the table.
#[cold]
#[inline(never)]
pub(crate) fn switch_target(
    fp: *mut u64,
    pc: *const u8,
    next: *const u8,
    k: u32,
    base: i32,
    v: i32,
) -> *const u8 {
    let fi = info(fp);
    let Constant::JumpTable(t) = fi.constant_entry(k) else {
        return next;
    };
    let i = i64::from(v) - i64::from(base);
    if i < 0 || i >= t.len() as i64 {
        return next;
    }
    pc.wrapping_add(t[i as usize] as usize)
}

/// `TypeOf`: the type string of `v`.
#[cold]
#[inline(never)]
pub(crate) fn type_of(cx: &Cx, v: u64) -> u64 {
    cx.rt().atom(conv::type_of(v))
}

/// ToBoolean for values that the fast path does not handle.
#[cold]
#[inline(never)]
pub(crate) fn to_boolean(v: u64) -> bool {
    conv::to_boolean(v)
}

/// `TestEqualStrict` for values that the fast path does not handle.
#[cold]
#[inline(never)]
pub(crate) fn strict_equals(l: u64, r: u64) -> bool {
    conv::strict_equals(l, r)
}
