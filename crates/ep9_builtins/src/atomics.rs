//! The Atomics object (`sec-atomics-object`): module `atomics`, block 44. Makes `%Atomics%`
//! (`IntrinsicId::Atomics`), an ordinary object whose prototype is `%Object.prototype%`, with
//! its 14 functions and `@@toStringTag` "Atomics", and the global key `Atomics` {w, c}. Owner
//! of the tables: TypedArray TA-4 R1; owner of the function bodies: TypedArray TA-4 R2
//! ATOMICS. The key `Atomics` is a key of the lazy group of `array_buffer` (TA-D14; `ep9_vm`
//! builtins.rs `LAZY_GROUPS`): realm init makes no `%Atomics%`.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-atomics.txt`. The ids follow node v26's key order. A missing definition
//! is appended at the end (44.14 and on), never inserted.
//!
//! ```text
//! load, store, add .. exchange, compareExchange
//!   ValidateIntegerTypedArray(ta, false)    a TypedArray in bounds, an integer type (not
//!                                           Uint8Clamped, no float)       TypeError
//!   ValidateAtomicAccess(index)             ToIndex; index < length       RangeError
//!   the value(s): ToBigInt (BigInt64, BigUint64) or ToIntegerOrInfinity
//!   RevalidateAtomicAccess                  a valueOf that detached or shrank the buffer:
//!                                           out of bounds TypeError; past its byte length
//!                                           RangeError
//!   Cx::atomic_* (seq-cst; it checks the range again for memory safety)
//! wait, waitAsync (DoWait)                  ValidateIntegerTypedArray(ta, true): Int32Array or
//!                                           BigInt64Array; a SharedArrayBuffer (TypeError);
//!                                           ValidateAtomicAccess; ToInt32 or ToBigInt64;
//!                                           ToNumber(timeout); Cx::atomics_wait (AgentCanSuspend)
//!                                           or Cx::atomics_wait_async
//! notify                                    ValidateIntegerTypedArray(ta, true);
//!                                           ValidateAtomicAccess; the count; 0 if not shared
//! isLockFree(n)                             true for 1, 2, 4 and 8 (aarch64 and x86-64)
//! pause(N)                                  N undefined or an integral Number, else TypeError
//! ```
//!
//! No byte of a buffer is read or written here: every access is a `Cx` method (the shared-byte
//! accessors of `ep9_platform::shared_mem` sit behind them).

use ep9_vm::abi::JsObject;
use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

use crate::array_buffer::to_index;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 44;

/// The builtin functions, in id order (append-only). Lengths: the spec's (node v26 gives the
/// same).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("load", 2, load),                        // 0
    BuiltinDef::method("store", 3, store),                      // 1
    BuiltinDef::method("add", 3, add),                          // 2
    BuiltinDef::method("sub", 3, sub),                          // 3
    BuiltinDef::method("and", 3, and),                          // 4
    BuiltinDef::method("or", 3, or),                            // 5
    BuiltinDef::method("xor", 3, xor),                          // 6
    BuiltinDef::method("exchange", 3, exchange),                // 7
    BuiltinDef::method("compareExchange", 4, compare_exchange), // 8
    BuiltinDef::method("isLockFree", 1, is_lock_free),          // 9
    BuiltinDef::method("wait", 4, wait),                        // 10
    BuiltinDef::method("waitAsync", 4, wait_async),             // 11
    BuiltinDef::method("notify", 3, notify),                    // 12
    BuiltinDef::method("pause", 0, pause),                      // 13
];

/// The intrinsics this module makes: `%Atomics%` (`sec-atomics-object`: an ordinary object,
/// not a function).
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Atomics,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// `key` = the function of definition `d` ({w, c}).
const fn method(key: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), d)
}

/// The properties this module adds, per object, in node v26's key order.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Atomics,
        // The 14 functions ({w, c}), then `Atomics [ %Symbol.toStringTag% ]` ({c};
        // `sec-atomics-%symbol.tostringtag%`).
        props: &[
            method("load", 0),
            method("store", 1),
            method("add", 2),
            method("sub", 3),
            method("and", 4),
            method("or", 5),
            method("xor", 6),
            method("exchange", 7),
            method("compareExchange", 8),
            method("isLockFree", 9),
            method("wait", 10),
            method("waitAsync", 11),
            method("notify", 12),
            method("pause", 13),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Atomics"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // `sec-atomics` ({w, c}): a key of the lazy group.
        props: &[PropDef::intrinsic(
            PropKey::Str("Atomics"),
            IntrinsicId::Atomics,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it after `base64`).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "atomics",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the abstract operations (`sec-abstract-operations-for-atomics`) -------------------------

/// The Table 71 rows of Int32 and BigInt64 (the waitable types).
const INT32: u8 = 5;
const BIG_INT64: u8 = 7;

/// The element size of the integer Table 71 row `row` (0-8 except 2).
fn element_size(row: u8) -> u64 {
    match row {
        0 | 1 => 1,
        3 | 4 => 2,
        5 | 6 => 4,
        _ => 8,
    }
}

/// True for the BigInt rows (7 BigInt64, 8 BigUint64).
fn is_bigint_row(row: u8) -> bool {
    row == 7 || row == 8
}

/// A TypedArray that passed ValidateIntegerTypedArray: the view, its row and the
/// TypedArrayLength of its record.
#[derive(Clone, Copy)]
struct Ta<'s> {
    o: Local<'s>,
    row: u8,
    length: u64,
}

/// ValidateIntegerTypedArray ( typedArray, waitable ) (`sec-validateintegertypedarray`).
fn validate_integer_typed_array<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    waitable: bool,
) -> JsResult<Ta<'s>> {
    // 1. Let taRecord be ? ValidateTypedArray(typedArray, unordered).
    let Some(row) = cx.typed_array_element(o) else {
        return Err(cx.throw_type_error("Argument is not a typed array"));
    };
    let Some(length) = cx.typed_array_length(o)? else {
        return Err(cx.throw_type_error(
            "Cannot perform Atomics operation on a detached or out-of-bounds TypedArray",
        ));
    };
    if waitable {
        // 3.a. Int32Array or BigInt64Array only.
        if row != INT32 && row != BIG_INT64 {
            return Err(cx.throw_type_error(
                "Atomics.wait and Atomics.notify need an Int32Array or a BigInt64Array",
            ));
        }
    } else if row == 2 || row > 8 {
        // 4.b. IsUnclampedIntegerElementType and IsBigIntElementType are both false.
        return Err(cx.throw_type_error(
            "Atomics operations need an integer TypedArray (not Uint8Clamped or float)",
        ));
    }
    Ok(Ta { o, row, length })
}

/// ValidateAtomicAccess ( taRecord, requestIndex ) (`sec-validateatomicaccess`): the byte
/// index in the buffer.
fn validate_atomic_access<'s>(cx: &mut Cx<'s>, ta: Ta<'s>, index: Local<'s>) -> JsResult<u64> {
    // 2. Let accessIndex be ? ToIndex(requestIndex).
    let i = to_index(cx, index, "Invalid atomic access index")?;
    // 4. If accessIndex ≥ length, throw a RangeError exception.
    if i >= ta.length {
        return Err(cx.throw_range_error("Invalid atomic access index"));
    }
    // 6-8. Return (accessIndex × elementSize) + offset.
    let offset = cx.typed_array_byte_offset(ta.o)?;
    Ok(i * element_size(ta.row) + offset)
}

/// RevalidateAtomicAccess ( typedArray, byteIndexInBuffer ) (`sec-revalidateatomicaccess`),
/// after a conversion that may have detached or shrunk the buffer.
fn revalidate_atomic_access<'s>(cx: &mut Cx<'s>, ta: Ta<'s>, byte_index: u64) -> JsResult<()> {
    // 3. If IsTypedArrayOutOfBounds(taRecord) is true, throw a TypeError exception.
    if cx.typed_array_length(ta.o)?.is_none() {
        return Err(cx.throw_type_error(
            "Cannot perform Atomics operation on a detached or out-of-bounds TypedArray",
        ));
    }
    // 5. If byteIndexInBuffer ≥ taRecord.[[CachedBufferByteLength]], throw a RangeError.
    let buffer = cx.typed_array_buffer(ta.o)?;
    if byte_index >= cx.array_buffer_byte_length(buffer)? {
        return Err(cx.throw_range_error("Invalid atomic access index"));
    }
    Ok(())
}

/// The value conversion of the Atomics functions: ToBigInt for a BigInt row, else
/// 𝔽(ToIntegerOrInfinity(value)) (+0 for -0).
fn to_element_value<'s>(cx: &mut Cx<'s>, ta: Ta<'s>, v: Local<'s>) -> JsResult<Local<'s>> {
    if is_bigint_row(ta.row) {
        return cx.to_bigint(v);
    }
    let n = cx.to_integer_or_infinity(v)?;
    Ok(Local::number(n + 0.0))
}

/// The buffer of `ta` (a TypedArray: never throws here).
fn buffer_of<'s>(cx: &mut Cx<'s>, ta: Ta<'s>) -> JsResult<Local<'s, JsObject>> {
    cx.typed_array_buffer(ta.o)
}

/// AtomicReadModifyWrite ( typedArray, index, value, op ) (`sec-atomicreadmodifywrite`) with
/// the op code of `Cx::atomic_read_modify_write` (0 add, 1 and, 2 exchange, 3 or, 4 sub,
/// 5 xor).
fn read_modify_write<'s>(cx: &mut Cx<'s>, args: Args<'s>, op: u8) -> JsResult<Local<'s>> {
    // 1. Let byteIndexInBuffer be ? ValidateAtomicAccessOnIntegerTypedArray(typedArray, index).
    let ta = validate_integer_typed_array(cx, args.get(0), false)?;
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 2-3. The value.
    let v = to_element_value(cx, ta, args.get(2))?;
    // 4. Perform ? RevalidateAtomicAccess(typedArray, byteIndexInBuffer).
    revalidate_atomic_access(cx, ta, byte_index)?;
    // 5-7. GetModifySetValueInBuffer(buffer, byteIndexInBuffer, elementType, v, op).
    let buffer = buffer_of(cx, ta)?;
    cx.atomic_read_modify_write(buffer, byte_index, ta.row, op, v)
}

// ---- the functions -----------------------------------------------------------------------------

/// `Atomics.load ( typedArray, index )` (`sec-atomics.load`).
fn load<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let byteIndexInBuffer be ? ValidateAtomicAccessOnIntegerTypedArray(typedArray, index).
    let ta = validate_integer_typed_array(cx, args.get(0), false)?;
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 2. Perform ? RevalidateAtomicAccess(typedArray, byteIndexInBuffer).
    revalidate_atomic_access(cx, ta, byte_index)?;
    // 3-5. Return GetValueFromBuffer(buffer, byteIndexInBuffer, elementType, true, seq-cst).
    let buffer = buffer_of(cx, ta)?;
    cx.atomic_load(buffer, byte_index, ta.row)
}

/// `Atomics.store ( typedArray, index, value )` (`sec-atomics.store`): returns the converted
/// value (not the stored bytes).
fn store<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let ta = validate_integer_typed_array(cx, args.get(0), false)?;
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 2-3. v: ToBigInt, or 𝔽(ToIntegerOrInfinity(value)).
    let v = to_element_value(cx, ta, args.get(2))?;
    // 4. Perform ? RevalidateAtomicAccess(typedArray, byteIndexInBuffer).
    revalidate_atomic_access(cx, ta, byte_index)?;
    // 5. Perform SetValueInBuffer(buffer, byteIndexInBuffer, elementType, v, true, seq-cst).
    let buffer = buffer_of(cx, ta)?;
    cx.atomic_store(buffer, byte_index, ta.row, v)?;
    // 6. Return v.
    Ok(v)
}

/// `Atomics.add ( typedArray, index, value )` (`sec-atomics.add`).
fn add<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 0)
}

/// `Atomics.sub ( typedArray, index, value )` (`sec-atomics.sub`).
fn sub<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 4)
}

/// `Atomics.and ( typedArray, index, value )` (`sec-atomics.and`).
fn and<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 1)
}

/// `Atomics.or ( typedArray, index, value )` (`sec-atomics.or`).
fn or<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 3)
}

/// `Atomics.xor ( typedArray, index, value )` (`sec-atomics.xor`).
fn xor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 5)
}

/// `Atomics.exchange ( typedArray, index, value )` (`sec-atomics.exchange`).
fn exchange<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    read_modify_write(cx, args, 2)
}

/// `Atomics.compareExchange ( typedArray, index, expectedValue, replacementValue )`
/// (`sec-atomics.compareexchange`).
fn compare_exchange<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let byteIndexInBuffer be ? ValidateAtomicAccessOnIntegerTypedArray(typedArray, index).
    let ta = validate_integer_typed_array(cx, args.get(0), false)?;
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 4-5. expected, then replacement.
    let expected = to_element_value(cx, ta, args.get(2))?;
    let replacement = to_element_value(cx, ta, args.get(3))?;
    // 6. Perform ? RevalidateAtomicAccess(typedArray, byteIndexInBuffer).
    revalidate_atomic_access(cx, ta, byte_index)?;
    // 7-13. The compare and the store in one seq-cst step; the old element.
    let buffer = buffer_of(cx, ta)?;
    cx.atomic_compare_exchange(buffer, byte_index, ta.row, expected, replacement)
}

/// `Atomics.isLockFree ( size )` (`sec-atomics.islockfree`): true for 1, 2, 4 and 8 (every
/// width is one instruction or one LL/SC loop on aarch64 and x86-64).
fn is_lock_free<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let n be ? ToIntegerOrInfinity(size).
    let n = cx.to_integer_or_infinity(args.get(0))?;
    // 3-7. [[IsLockFree1]], [[IsLockFree2]], true, [[IsLockFree8]]; else false.
    Ok(Local::boolean(n == 1.0 || n == 2.0 || n == 4.0 || n == 8.0))
}

/// DoWait ( mode, typedArray, index, value, timeout ) (`sec-dowait`), steps 1-9 (the
/// conversions); the rest is `Cx::atomics_wait` (sync) or `Cx::atomics_wait_async`.
fn do_wait<'s>(cx: &mut Cx<'s>, args: Args<'s>, sync: bool) -> JsResult<Local<'s>> {
    // 1. Let taRecord be ? ValidateIntegerTypedArray(typedArray, true).
    let ta = validate_integer_typed_array(cx, args.get(0), true)?;
    // 2-3. If IsSharedArrayBuffer(buffer) is false, throw a TypeError exception.
    let buffer = buffer_of(cx, ta)?;
    if !cx.is_shared_array_buffer(buffer) {
        return Err(cx.throw_type_error(
            "Atomics.wait and Atomics.waitAsync need a TypedArray on a SharedArrayBuffer",
        ));
    }
    // 4. Let i be ? ValidateAtomicAccess(taRecord, index).
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 5-7. v: ToBigInt64 (the ABI keeps the low 64 bits) or ToInt32.
    let v = if ta.row == BIG_INT64 {
        cx.to_bigint(args.get(2))?
    } else {
        Local::number(f64::from(cx.to_int32(args.get(2))?))
    };
    // 8. Let q be ? ToNumber(timeout).
    let q = cx.to_number(args.get(3))?;
    // 9. NaN or +∞: +∞; -∞: 0; else max(q, 0) (`Cx` takes NaN as +∞).
    let t = if q.is_nan() {
        f64::INFINITY
    } else {
        q.max(0.0)
    };
    // 10-. AgentCanSuspend (sync) and the waiter list: the ABI.
    if sync {
        cx.atomics_wait(buffer, byte_index, ta.row, v, t)
    } else {
        cx.atomics_wait_async(buffer, byte_index, ta.row, v, t)
            .map(Into::into)
    }
}

/// `Atomics.wait ( typedArray, index, value, timeout )` (`sec-atomics.wait`).
fn wait<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    do_wait(cx, args, true)
}

/// `Atomics.waitAsync ( typedArray, index, value, timeout )` (`sec-atomics.waitasync`).
fn wait_async<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    do_wait(cx, args, false)
}

/// `Atomics.notify ( typedArray, index, count )` (`sec-atomics.notify`).
fn notify<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let taRecord be ? ValidateIntegerTypedArray(typedArray, true).
    let ta = validate_integer_typed_array(cx, args.get(0), true)?;
    // 2. Let byteIndexInBuffer be ? ValidateAtomicAccess(taRecord, index).
    let byte_index = validate_atomic_access(cx, ta, args.get(1))?;
    // 3-4. c: +∞ for undefined, else max(ToIntegerOrInfinity(count), 0).
    let count = args.get(2);
    let c = if count.is_undefined() {
        u64::MAX
    } else {
        let n = cx.to_integer_or_infinity(count)?;
        if n >= u64::MAX as f64 {
            u64::MAX
        } else {
            n.max(0.0) as u64
        }
    };
    // 5-7. A buffer that is not shared gives +0 (the ABI); 8-. wake up to c waiters.
    let buffer = buffer_of(cx, ta)?;
    let n = cx.atomics_notify(buffer, byte_index, c)?;
    Ok(Local::number(n as f64))
}

/// `Atomics.pause ( [ N ] )` (`sec-atomics.pause`): a spin-wait hint.
fn pause<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let n = args.get(0);
    // 1. If N is neither undefined nor an integral Number, throw a TypeError exception.
    if !n.is_undefined()
        && !n
            .as_number()
            .is_some_and(|x| x.is_finite() && x.trunc() == x)
    {
        return Err(cx.throw_type_error("Atomics.pause needs an integral Number or undefined"));
    }
    // 2. The spin-wait hint (one per call; N is only a hint of the wait's length).
    core::hint::spin_loop();
    // 3. Return undefined.
    Ok(Local::undefined())
}
