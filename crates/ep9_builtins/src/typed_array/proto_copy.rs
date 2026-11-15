//! The copy, species and sort methods of `%TypedArray.prototype%`
//! (`sec-properties-of-the-%typedarray%-prototype-object`). Owner: TypedArray TA-4 R4 PROTO-B
//! (this file and `species.rs`).
//!
//! ```text
//! method       first steps                          result         the elements
//! copyWithin   Validate, len, 3 ToIntegerOrInf.     this           re-read len; one byte copy
//! fill         Validate, len, ToNumber/ToBigInt,    this           re-read len; [[Set]] per index
//!              2 ToIntegerOrInfinity
//! filter       Validate, len, callback              SpeciesCreate  callback per index, kept
//!                                                  («captured»)   values in a ScratchList
//! map          Validate, len, callback              SpeciesCreate  [[Set]] of each callback
//!                                                  («len»)        result
//! reverse      Validate, len                        this           Get and [[Set]] of 2 indices
//! set          this a TypedArray, ToIntegerOrInf.   undefined      a TypedArray source: one
//!                                                                  byte copy (same type) or
//!                                                                  Get/SetValueInBuffer per
//!                                                                  element; else Get, [[Set]]
//! slice        Validate, len, 2 ToIntegerOrInf.     SpeciesCreate  re-read len; one byte copy
//!                                                  («count»)      (same type), else Get, [[Set]]
//! sort         comparator, Validate, len            this           SortIndexedProperties, [[Set]]
//! subarray     this a TypedArray (no throw when     SpeciesCreate  none (a view on the same
//!              out of bounds), 2 ToIntegerOrInf.    («buffer,..»)  buffer)
//! toReversed   Validate, len                        CreateSameType Get, [[Set]]
//! toSorted     comparator, Validate, len            CreateSameType SortIndexedProperties, [[Set]]
//! with         Validate, len, ToIntegerOrInf.,      CreateSameType Get, [[Set]]
//!              ToNumber/ToBigInt, IsValidIntegerIndex (RangeError)
//! ```
//!
//! Elements are read and written only through `Cx` (`get_index`, `set_index`,
//! `get_value_from_buffer`, `set_value_in_buffer`, `copy_buffer_bytes`): no buffer byte is
//! touched here, so a SharedArrayBuffer view takes the `ep9_vm` shared-memory accessors
//! (`ep9_platform::shared_mem`). Each loop over elements polls interrupts once per step.

use ep9_vm::abi::JsObject;
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local, ScratchList};

use super::ctor::{element_size, is_bigint_row};
use super::from_of::validate_typed_array;
use super::species::{
    typed_array_create_same_type, typed_array_species_create, typed_array_species_create_view,
};

/// The `[[LittleEndian]]` value of the agent: an element copy reads and writes with the same
/// order.
const LITTLE_ENDIAN: bool = cfg!(target_endian = "little");

/// The TypeError text of a view that went out of bounds (or was detached) in a conversion.
const OUT_OF_BOUNDS: &str =
    "Cannot perform this operation on a detached or out-of-bounds TypedArray";

/// The this value of a method after ValidateTypedArray(O, seq-cst) (`sec-validatetypedarray`),
/// and its TypedArrayLength.
fn validated<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<(Local<'s, JsObject>, u64)> {
    let len = validate_typed_array(cx, this)?;
    let o = this.as_object().expect("a TypedArray is an object");
    Ok((o, len))
}

/// The row of spec Table 71 of the TypedArray `o`.
fn row<'s>(cx: &Cx<'s>, o: impl Into<Local<'s>>) -> u8 {
    cx.typed_array_element(o).expect("a TypedArray")
}

/// The callback argument `f` of `method`, if IsCallable(f), else a TypeError.
fn callback<'s>(cx: &mut Cx<'s>, f: Local<'s>, method: &str) -> JsResult<Local<'s>> {
    if cx.is_callable(f) {
        return Ok(f);
    }
    let msg = format!("TypedArray.prototype.{method}: the callback is not a function");
    Err(cx.throw_type_error(&msg))
}

/// 𝔽(k) for an index (below 2^53, so exact).
fn number<'s>(k: u64) -> Local<'s> {
    Local::number(k as f64)
}

/// The relative index steps of `copyWithin`, `fill`, `slice` and `subarray` (for example
/// `sec-%typedarray%.prototype.slice` steps 4-7): ToIntegerOrInfinity(`v`) taken from the end
/// when negative, clamped to [0, len]. `v` undefined gives `default` (no conversion runs).
fn relative_index<'s>(cx: &mut Cx<'s>, v: Local<'s>, len: u64, default: u64) -> JsResult<u64> {
    if v.is_undefined() {
        return Ok(default);
    }
    let rel = cx.to_integer_or_infinity(v)?;
    let len_f = len as f64;
    Ok(if rel == f64::NEG_INFINITY {
        0
    } else if rel < 0.0 {
        (len_f + rel).max(0.0) as u64
    } else {
        rel.min(len_f) as u64
    })
}

/// The value of `v` converted for an element of row `r`: ToBigInt for `[[ContentType]]`
/// bigint, else ToNumber (`fill` steps 4-5, `with` steps 7-8).
fn to_element_value<'s>(cx: &mut Cx<'s>, v: Local<'s>, r: u8) -> JsResult<Local<'s>> {
    if is_bigint_row(r) {
        cx.to_bigint(v)
    } else {
        Ok(Local::number(cx.to_number(v)?))
    }
}

/// MakeTypedArrayWithBufferWitnessRecord(O, seq-cst) and its TypedArrayLength, or a TypeError
/// when IsTypedArrayOutOfBounds (the length re-read of `copyWithin`, `fill` and `slice`).
fn length_again<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<u64> {
    match cx.typed_array_length(o)? {
        Some(len) => Ok(len),
        None => Err(cx.throw_type_error(OUT_OF_BOUNDS)),
    }
}

// ---- copyWithin, fill, reverse ---------------------------------------------------------------

/// `%TypedArray%.prototype.copyWithin ( target, start [ , end ] )`
/// (`sec-%typedarray%.prototype.copywithin`).
pub(super) fn copy_within<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, taRecord = ? ValidateTypedArray(O, seq-cst), len.
    let (o, len) = validated(cx, this)?;
    // 4-7. targetIndex (ToIntegerOrInfinity of undefined is 0).
    let target_index = relative_index(cx, args.get(0), len, 0)?;
    // 8-11. startIndex.
    let start_index = relative_index(cx, args.get(1), len, 0)?;
    // 12-15. endIndex (end undefined: len).
    let end_index = relative_index(cx, args.get(2), len, len)?;
    // 16. Let count be min(endIndex - startIndex, len - targetIndex).
    let count = end_index
        .saturating_sub(start_index)
        .min(len - target_index);
    // 17. If count > 0, then
    if count > 0 {
        // b. Let buffer be O.[[ViewedArrayBuffer]].
        let buffer = cx.typed_array_buffer(o)?;
        // c-d. Set taRecord to MakeTypedArrayWithBufferWitnessRecord(O, seq-cst); if
        //      IsTypedArrayOutOfBounds(taRecord), throw a TypeError exception.
        // e. Set len to TypedArrayLength(taRecord).
        let len = length_again(cx, o)?;
        // f-g. elementSize, byteOffset.
        let size = element_size(row(cx, o));
        let byte_offset = cx.typed_array_byte_offset(o)?;
        // h. Let bufferByteLimit be (len × elementSize) + byteOffset.
        let limit = len * size + byte_offset;
        // i-k. toByteIndex, fromByteIndex, countBytes.
        let to_byte = target_index * size + byte_offset;
        let from_byte = start_index * size + byte_offset;
        // l-n. The bytes are copied one by one (backwards when the target overlaps the end of
        //      the source) while both indices are below bufferByteLimit: a shrink in a
        //      conversion cuts the count to the bytes below the limit. That is one copy of
        //      min(countBytes, limit - fromByteIndex, limit - toByteIndex) bytes, as if
        //      through a temporary copy (CopyDataBlockBytes on one store keeps an overlap
        //      right; shared bytes go through the shared-memory accessors).
        let count_bytes = (count * size)
            .min(limit.saturating_sub(from_byte))
            .min(limit.saturating_sub(to_byte));
        if count_bytes > 0 {
            cx.copy_buffer_bytes(buffer, to_byte, buffer, from_byte, count_bytes)?;
        }
    }
    // 18. Return O.
    Ok(this)
}

/// `%TypedArray%.prototype.fill ( value [ , start [ , end ] ] )`
/// (`sec-%typedarray%.prototype.fill`).
pub(super) fn fill<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If O.[[ContentType]] is bigint, set value to ? ToBigInt(value).
    // 5. Otherwise, set value to ? ToNumber(value).
    let r = row(cx, o);
    let value = to_element_value(cx, args.get(0), r)?;
    // 6-9. startIndex.
    let start_index = relative_index(cx, args.get(1), len, 0)?;
    // 10-13. endIndex (end undefined: len).
    let end_index = relative_index(cx, args.get(2), len, len)?;
    // 14-15. Set taRecord to MakeTypedArrayWithBufferWitnessRecord(O, seq-cst); if
    //        IsTypedArrayOutOfBounds(taRecord), throw a TypeError exception.
    // 16. Set len to TypedArrayLength(taRecord).
    let len = length_again(cx, o)?;
    // 17. Set endIndex to min(endIndex, len).
    let end_index = end_index.min(len);
    // 18. Let k be startIndex.
    // 19. Repeat, while k < endIndex: Perform ! Set(O, ! ToString(𝔽(k)), value, true). (The
    //     value is a Number or a BigInt: no user code runs.)
    for k in start_index..end_index {
        cx.poll_interrupt()?;
        cx.set_index(o, k, value, true)?;
    }
    // 20. Return O.
    Ok(this)
}

/// `%TypedArray%.prototype.reverse ( )` (`sec-%typedarray%.prototype.reverse`).
pub(super) fn reverse<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. Let middle be floor(len / 2).
    let middle = len / 2;
    // 5. Let lower be 0.
    // 6. Repeat, while lower ≠ middle,
    for lower in 0..middle {
        cx.poll_interrupt()?;
        // a. Let upper be len - lower - 1.
        let upper = len - lower - 1;
        // b-e. upperValue = ! Get(O, upperP); lowerValue = ! Get(O, lowerP).
        let upper_value = cx.get_index(o, upper)?;
        let lower_value = cx.get_index(o, lower)?;
        // f-g. ! Set(O, lowerP, upperValue, true); ! Set(O, upperP, lowerValue, true).
        cx.set_index(o, lower, upper_value, true)?;
        cx.set_index(o, upper, lower_value, true)?;
        // h. Set lower to lower + 1.
    }
    // 7. Return O.
    Ok(this)
}

// ---- filter, map ----------------------------------------------------------------------------

/// `%TypedArray%.prototype.filter ( callback [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.filter`).
pub(super) fn filter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If IsCallable(callback) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), "filter")?;
    let this_arg = args.get(1);
    // 5. Let kept be a new empty List (precise roots).
    let kept = cx.scratch_list();
    // 6-8. k = 0, captured = 0; repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a-b. Let kValue be ! Get(O, ! ToString(𝔽(k))).
        let k_value = cx.get_index(o, k)?;
        // c. Let selected be ToBoolean(? Call(callback, thisArg, « kValue, 𝔽(k), O »)).
        let selected = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
        // d. If selected is true, append kValue to kept; captured + 1.
        if cx.to_boolean(selected) {
            kept.push(k_value);
        }
    }
    // 9. Let A be ? TypedArraySpeciesCreate(O, « 𝔽(captured) »).
    let a = typed_array_species_create(cx, o, kept.len() as u64)?;
    // 10-11. For each element e of kept: Perform ! Set(A, ! ToString(𝔽(n)), e, true).
    for n in 0..kept.len() {
        cx.poll_interrupt()?;
        cx.set_index(a, n as u64, kept.get(n), true)?;
    }
    // 12. Return A.
    Ok(a.value())
}

/// `%TypedArray%.prototype.map ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.map`).
pub(super) fn map<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If IsCallable(callback) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), "map")?;
    let this_arg = args.get(1);
    // 5. Let A be ? TypedArraySpeciesCreate(O, « 𝔽(len) »).
    let a = typed_array_species_create(cx, o, len)?;
    // 6-7. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a-b. Let kValue be ! Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // c. Let mappedValue be ? Call(callback, thisArg, « kValue, 𝔽(k), O »).
        let mapped = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
        // d. Perform ? Set(A, Pk, mappedValue, true).
        cx.set_index(a, k, mapped, true)?;
    }
    // 8. Return A.
    Ok(a.value())
}

// ---- set --------------------------------------------------------------------------------------

/// `%TypedArray%.prototype.set ( source [ , offset ] )` (`sec-%typedarray%.prototype.set`).
pub(super) fn set<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let source = args.get(0);
    // 1-3. Let target be the this value; ? RequireInternalSlot(target, [[TypedArrayName]]).
    if cx.typed_array_element(this).is_none() {
        return Err(cx.throw_type_error("this is not a typed array."));
    }
    let target = this.as_object().expect("a TypedArray is an object");
    // 4. Let targetOffset be ? ToIntegerOrInfinity(offset).
    let target_offset = cx.to_integer_or_infinity(args.get(1))?;
    // 5. If targetOffset < 0, throw a RangeError exception.
    if target_offset < 0.0 {
        return Err(cx.throw_range_error("offset is out of bounds"));
    }
    // 6. If source is an Object that has a [[TypedArrayName]] internal slot, then
    //    a. Perform ? SetTypedArrayFromTypedArray(target, targetOffset, source).
    // 7. Else, a. Perform ? SetTypedArrayFromArrayLike(target, targetOffset, source).
    if cx.typed_array_element(source).is_some() {
        let source = source.as_object().expect("a TypedArray is an object");
        set_from_typed_array(cx, target, target_offset, source)?;
    } else {
        set_from_array_like(cx, target, target_offset, source)?;
    }
    // 8. Return undefined.
    Ok(Local::undefined())
}

/// The RangeError of steps 15-16 of SetTypedArrayFromTypedArray (6-7 of
/// SetTypedArrayFromArrayLike): `targetOffset` is +∞, or `srcLength + targetOffset` is above
/// `targetLength`. Else the offset as an integer.
fn checked_offset<'s>(
    cx: &mut Cx<'s>,
    target_offset: f64,
    src_length: u64,
    target_length: u64,
) -> JsResult<u64> {
    if target_offset == f64::INFINITY || src_length as f64 + target_offset > target_length as f64 {
        return Err(cx.throw_range_error("offset is out of bounds"));
    }
    Ok(target_offset as u64)
}

/// SetTypedArrayFromTypedArray ( target, targetOffset, source )
/// (`sec-settypedarrayfromtypedarray`).
fn set_from_typed_array<'s>(
    cx: &mut Cx<'s>,
    target: Local<'s, JsObject>,
    target_offset: f64,
    source: Local<'s, JsObject>,
) -> JsResult<()> {
    // 1. Let targetBuffer be target.[[ViewedArrayBuffer]].
    let target_buffer = cx.typed_array_buffer(target)?;
    // 2-4. targetRecord (seq-cst); a TypeError if out of bounds; targetLength.
    let target_length = length_again(cx, target)?;
    // 5. Let srcBuffer be source.[[ViewedArrayBuffer]].
    let src_buffer = cx.typed_array_buffer(source)?;
    // 6-8. srcRecord (seq-cst); a TypeError if out of bounds; srcLength.
    let src_length = length_again(cx, source)?;
    // 9-14. targetType, targetElementSize, targetByteOffset, srcType, srcElementSize,
    //       srcByteOffset.
    let (target_type, src_type) = (row(cx, target), row(cx, source));
    let (target_size, src_size) = (element_size(target_type), element_size(src_type));
    let target_byte_offset = cx.typed_array_byte_offset(target)?;
    let src_byte_offset = cx.typed_array_byte_offset(source)?;
    // 15-16. The RangeErrors of the offset.
    let target_offset = checked_offset(cx, target_offset, src_length, target_length)?;
    // 17. If target.[[ContentType]] is not source.[[ContentType]], throw a TypeError exception.
    if is_bigint_row(target_type) != is_bigint_row(src_type) {
        return Err(
            cx.throw_type_error("Cannot mix BigInt and other types, use explicit conversions")
        );
    }
    // 21-22. targetByteIndex, limit.
    let mut target_byte_index = target_offset * target_size + target_byte_offset;
    // 18. sameSharedArrayBuffer: both buffers are SharedArrayBuffers on one data block.
    // 19. If SameValue(srcBuffer, targetBuffer) or sameSharedArrayBuffer is true, the source
    //     bytes are first cloned (CloneArrayBuffer), so a write never changes a later read.
    let same = cx.same_value(src_buffer, target_buffer)
        || cx.is_same_shared_block(src_buffer, target_buffer);
    // 23. If srcType is targetType, then
    if src_type == target_type {
        // a. NOTE: The transfer must be performed in a manner that preserves the bit-level
        //    encoding of the source data.
        // b. Copy srcLength × srcElementSize bytes. CopyDataBlockBytes on one store keeps an
        //    overlap right (as if through a temporary copy), which is the result of the
        //    clone of step 19: no clone is needed here.
        if src_length > 0 {
            cx.copy_buffer_bytes(
                target_buffer,
                target_byte_index,
                src_buffer,
                src_byte_offset,
                src_length * src_size,
            )?;
        }
        return Ok(());
    }
    // 19-20. The source buffer and its first byte index (a clone of the source bytes when the
    //        buffers share a data block).
    let (src_buffer, mut src_byte_index) = if same {
        // a. Let srcByteLength be TypedArrayByteLength(srcRecord).
        let src_byte_length = src_length * src_size;
        // b. Set srcBuffer to ? CloneArrayBuffer(srcBuffer, srcByteOffset, srcByteLength).
        let ab_proto = cx.intrinsic(IntrinsicId::ArrayBufferPrototype);
        let clone = cx.allocate_array_buffer(ab_proto, src_byte_length, None)?;
        cx.copy_buffer_bytes(clone, 0, src_buffer, src_byte_offset, src_byte_length)?;
        // c. Let srcByteIndex be 0.
        (clone, 0)
    } else {
        // 20. Else, let srcByteIndex be srcByteOffset.
        (src_buffer, src_byte_offset)
    };
    // 24. Else, repeat, while targetByteIndex < limit: GetValueFromBuffer(srcBuffer,
    //     srcByteIndex, srcType, true, unordered), then SetValueInBuffer(targetBuffer,
    //     targetByteIndex, targetType, value, true, unordered). No user code runs.
    for _ in 0..src_length {
        cx.poll_interrupt()?;
        let value =
            cx.get_value_from_buffer(src_buffer, src_byte_index, src_type, LITTLE_ENDIAN)?;
        cx.set_value_in_buffer(
            target_buffer,
            target_byte_index,
            target_type,
            value,
            LITTLE_ENDIAN,
        )?;
        src_byte_index += src_size;
        target_byte_index += target_size;
    }
    // 25. Return unused.
    Ok(())
}

/// SetTypedArrayFromArrayLike ( target, targetOffset, source )
/// (`sec-settypedarrayfromarraylike`).
fn set_from_array_like<'s>(
    cx: &mut Cx<'s>,
    target: Local<'s, JsObject>,
    target_offset: f64,
    source: Local<'s>,
) -> JsResult<()> {
    // 1-3. targetRecord (seq-cst); a TypeError if out of bounds; targetLength.
    let target_length = length_again(cx, target)?;
    // 4. Let src be ? ToObject(source).
    let src = cx.to_object(source)?;
    // 5. Let srcLength be ? LengthOfArrayLike(src).
    let src_length = cx.get(src, "length")?;
    let src_length = cx.to_length(src_length)?;
    // 6-7. The RangeErrors of the offset.
    let target_offset = checked_offset(cx, target_offset, src_length, target_length)?;
    // 8-9. Repeat, while k < srcLength,
    for k in 0..src_length {
        cx.poll_interrupt()?;
        // a-b. Let value be ? Get(src, ! ToString(𝔽(k))).
        let value = cx.get_index(src, k)?;
        // c-d. Perform ? TypedArraySetElement(target, 𝔽(targetOffset + k), value): the
        //      element [[Set]] (ToNumber or ToBigInt, then a write only if the index is still
        //      valid).
        cx.set_index(target, target_offset + k, value, true)?;
    }
    // 10. Return unused.
    Ok(())
}

// ---- slice, subarray -------------------------------------------------------------------------

/// `%TypedArray%.prototype.slice ( start, end )` (`sec-%typedarray%.prototype.slice`).
pub(super) fn slice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, srcArrayLength.
    let (o, src_array_length) = validated(cx, this)?;
    // 4-7. startIndex.
    let start_index = relative_index(cx, args.get(0), src_array_length, 0)?;
    // 8-11. endIndex (end undefined: srcArrayLength).
    let end_index = relative_index(cx, args.get(1), src_array_length, src_array_length)?;
    // 12. Let countBytes be max(endIndex - startIndex, 0).
    let count = end_index.saturating_sub(start_index);
    // 13. Let A be ? TypedArraySpeciesCreate(O, « 𝔽(countBytes) »).
    let a = typed_array_species_create(cx, o, count)?;
    // 14. If countBytes > 0, then
    if count > 0 {
        // a-b. Set taRecord to MakeTypedArrayWithBufferWitnessRecord(O, seq-cst); if
        //      IsTypedArrayOutOfBounds(taRecord), throw a TypeError exception.
        // c. Set endIndex to min(endIndex, TypedArrayLength(taRecord)).
        let end_index = end_index.min(length_again(cx, o)?);
        // d. Set countBytes to max(endIndex - startIndex, 0).
        let count = end_index.saturating_sub(start_index);
        // e-f. srcType, targetType.
        let (src_type, target_type) = (row(cx, o), row(cx, a));
        // g. If srcType is targetType, then
        if src_type == target_type {
            // i. NOTE: The transfer must be performed in a manner that preserves the
            //    bit-level encoding of the source data.
            // ii-iii. srcBuffer, targetBuffer.
            let src_buffer = cx.typed_array_buffer(o)?;
            let target_buffer = cx.typed_array_buffer(a)?;
            // iv. Let elementSize be TypedArrayElementSize(O).
            let size = element_size(src_type);
            // v-vi. srcByteOffset; srcByteIndex = (startIndex × elementSize) + srcByteOffset.
            let src_byte_index = start_index * size + cx.typed_array_byte_offset(o)?;
            // vii. Let targetByteIndex be A.[[ByteOffset]].
            let target_byte_index = cx.typed_array_byte_offset(a)?;
            // viii-ix. endByteIndex = targetByteIndex + (countBytes × elementSize); copy one
            //          byte at a time, ascending, while targetByteIndex < endByteIndex.
            let same = cx.same_value(src_buffer, target_buffer)
                || cx.is_same_shared_block(src_buffer, target_buffer);
            copy_ascending(
                cx,
                target_buffer,
                target_byte_index,
                src_buffer,
                src_byte_index,
                count * size,
                same,
            )?;
        } else {
            // h. Else: n = 0, k = startIndex; repeat, while k < endIndex: kValue = ! Get(O,
            //    Pk); ! Set(A, ! ToString(𝔽(n)), kValue, true).
            for (n, k) in (start_index..end_index).enumerate() {
                cx.poll_interrupt()?;
                let k_value = cx.get_index(o, k)?;
                cx.set_index(a, n as u64, k_value, true)?;
            }
        }
    }
    // 15. Return A.
    Ok(a.value())
}

/// `count` bytes from `from` at `from_index` to `to` at `to_index`, one byte at a time in
/// ascending order (the loop of `slice` step 14.g.ix). `same`: the two buffers share a data
/// block. When the target starts inside the source range of one block, an ascending byte loop
/// reads bytes it wrote before (a repeat of the first `to_index - from_index` bytes); a copy
/// of chunks of that size, in ascending order, gives the same bytes. Else one copy (no
/// overlap, or an overlap that an ascending loop copies right). A count of 0 copies nothing
/// (a shrink in a conversion can put both indices past the end).
fn copy_ascending<'s>(
    cx: &mut Cx<'s>,
    to: Local<'s, JsObject>,
    to_index: u64,
    from: Local<'s, JsObject>,
    from_index: u64,
    count: u64,
    same: bool,
) -> JsResult<()> {
    if same && from_index < to_index && to_index < from_index + count {
        let step = to_index - from_index;
        let mut done = 0;
        while done < count {
            cx.poll_interrupt()?;
            let n = step.min(count - done);
            cx.copy_buffer_bytes(to, to_index + done, from, from_index + done, n)?;
            done += n;
        }
        return Ok(());
    }
    if count > 0 {
        cx.copy_buffer_bytes(to, to_index, from, from_index, count)?;
    }
    Ok(())
}

/// `%TypedArray%.prototype.subarray ( start, end )` (`sec-%typedarray%.prototype.subarray`).
pub(super) fn subarray<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let O be the this value; ? RequireInternalSlot(O, [[TypedArrayName]]).
    if cx.typed_array_element(this).is_none() {
        return Err(cx.throw_type_error("this is not a typed array."));
    }
    let o = this.as_object().expect("a TypedArray is an object");
    // 4. Let buffer be O.[[ViewedArrayBuffer]].
    let buffer = cx.typed_array_buffer(o)?;
    // 5-7. srcRecord (seq-cst); srcLength is 0 when IsTypedArrayOutOfBounds, else
    //      TypedArrayLength(srcRecord).
    let src_length = cx.typed_array_length(o)?.unwrap_or(0);
    // 8-11. startIndex.
    let start_index = relative_index(cx, args.get(0), src_length, 0)?;
    // 12-13. elementSize, srcByteOffset.
    let size = element_size(row(cx, o));
    let src_byte_offset = cx.typed_array_byte_offset(o)?;
    // 14. Let beginByteOffset be srcByteOffset + (startIndex × elementSize).
    let begin_byte_offset = src_byte_offset + start_index * size;
    let end = args.get(1);
    // 15. If O.[[ArrayLength]] is auto and end is undefined, then
    //     a. Let argumentsList be « buffer, 𝔽(beginByteOffset) ».
    let new_length = if cx.typed_array_fixed_length(o)?.is_none() && end.is_undefined() {
        None
    } else {
        // 16. Else,
        //     a-d. endIndex (end undefined: srcLength).
        let end_index = relative_index(cx, end, src_length, src_length)?;
        //     e. Let newLength be max(endIndex - startIndex, 0).
        //     f. Let argumentsList be « buffer, 𝔽(beginByteOffset), 𝔽(newLength) ».
        Some(end_index.saturating_sub(start_index))
    };
    // 17. Return ? TypedArraySpeciesCreate(O, argumentsList).
    let r = typed_array_species_create_view(cx, o, buffer, begin_byte_offset, new_length)?;
    Ok(r.value())
}

// ---- sort, toSorted, toReversed, with ----------------------------------------------------------

/// Step 1 of `sort` and `toSorted`: the comparator, if it is undefined or callable, else a
/// TypeError.
fn comparator<'s>(cx: &mut Cx<'s>, f: Local<'s>, method: &str) -> JsResult<Local<'s>> {
    if f.is_undefined() || cx.is_callable(f) {
        return Ok(f);
    }
    let msg = format!(
        "TypedArray.prototype.{method}: the comparison function must be either a function or undefined"
    );
    Err(cx.throw_type_error(&msg))
}

/// SortIndexedProperties ( obj, len, SortCompare, read-through-holes )
/// (`sec-sortindexedproperties`) with the SortCompare of CompareTypedArrayElements
/// (`sec-comparetypedarrayelements`), then the writes: `Set(dest, j, sortedList[j], true)` for
/// each `j` (`dest` is `obj` for `sort`, the new TypedArray for `toSorted`).
///
/// Without a comparator no user code runs: the elements are read once, as plain f64 or i128
/// values (no GC value in Rust memory), and sorted in the numeric order (-0 before +0, NaN
/// last). With a comparator the values live in a `ScratchList`, and a stable merge sort of
/// their indices calls it; its throw (or the throw of ToNumber of its result) ends the sort
/// before any write. A write after a detach or a shrink is ignored (the element `[[Set]]`).
fn sort_into<'s>(
    cx: &mut Cx<'s>,
    obj: Local<'s, JsObject>,
    len: u64,
    comparefn: Local<'s>,
    dest: Local<'s, JsObject>,
) -> JsResult<()> {
    let bigint = is_bigint_row(row(cx, obj));
    if comparefn.is_undefined() {
        if bigint {
            // A BigInt64 or BigUint64 element fits an i128.
            let mut items = Vec::new();
            for k in 0..len {
                cx.poll_interrupt()?;
                let v = cx.get_index(obj, k)?;
                items.push(cx.bigint_to_i128(v).expect("a 64-bit BigInt element"));
            }
            items.sort_unstable();
            for (j, x) in items.into_iter().enumerate() {
                cx.poll_interrupt()?;
                let v = cx.bigint_from_i128(x)?;
                cx.set_index(dest, j as u64, v, true)?;
            }
        } else {
            let mut items = Vec::new();
            for k in 0..len {
                cx.poll_interrupt()?;
                let v = cx.get_index(obj, k)?;
                items.push(v.as_number().expect("a Number element"));
            }
            // CompareTypedArrayElements steps 3-10: NaN last, -0 before +0. Every NaN is one
            // value here (`total_cmp` would put a NaN with the sign bit first).
            items.sort_by(|x, y| match (x.is_nan(), y.is_nan()) {
                (true, true) => core::cmp::Ordering::Equal,
                (true, false) => core::cmp::Ordering::Greater,
                (false, true) => core::cmp::Ordering::Less,
                (false, false) => x.total_cmp(y),
            });
            for (j, x) in items.into_iter().enumerate() {
                cx.poll_interrupt()?;
                cx.set_index(dest, j as u64, Local::number(x), true)?;
            }
        }
        return Ok(());
    }
    // 1-3. items: ! Get(obj, Pk) for each k < len (read-through-holes).
    let items = cx.scratch_list();
    for k in 0..len {
        cx.poll_interrupt()?;
        let v = cx.get_index(obj, k)?;
        items.push(v);
    }
    // 4. Sort items using an implementation-defined sequence of calls to SortCompare. If any
    //    such call returns an abrupt completion, stop before performing any further calls to
    //    SortCompare and return that Completion Record.
    let order = merge_sort(cx, &items, comparefn)?;
    // (sort steps 8-9, toSorted steps 9-10) Perform ! Set(dest, ! ToString(𝔽(j)),
    // sortedList[j], true).
    for (j, &i) in order.iter().enumerate() {
        cx.poll_interrupt()?;
        cx.set_index(dest, j as u64, items.get(i), true)?;
    }
    Ok(())
}

/// A stable bottom-up merge sort of the indices of `items` with CompareTypedArrayElements
/// ( x, y, comparefn ) step 2: `v` = ? ToNumber(? Call(comparefn, undefined, « x, y »)), and
/// `x` comes after `y` only when `v` > 0 (NaN is +0). Only indices move: the values stay in
/// their `ScratchList`.
fn merge_sort<'s>(
    cx: &mut Cx<'s>,
    items: &ScratchList<'s>,
    comparefn: Local<'s>,
) -> JsResult<Vec<usize>> {
    let n = items.len();
    let mut order: Vec<usize> = (0..n).collect();
    let mut buf = vec![0usize; n];
    let mut width = 1;
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut l, mut r, mut out) = (lo, mid, lo);
            while l < mid && r < hi {
                cx.poll_interrupt()?;
                let v = cx.call(
                    comparefn,
                    Local::undefined(),
                    &[items.get(order[l]), items.get(order[r])],
                )?;
                if cx.to_number(v)? > 0.0 {
                    buf[out] = order[r];
                    r += 1;
                } else {
                    buf[out] = order[l];
                    l += 1;
                }
                out += 1;
            }
            buf[out..out + (mid - l)].copy_from_slice(&order[l..mid]);
            out += mid - l;
            buf[out..out + (hi - r)].copy_from_slice(&order[r..hi]);
            lo = hi;
        }
        core::mem::swap(&mut order, &mut buf);
        width *= 2;
    }
    Ok(order)
}

/// `%TypedArray%.prototype.sort ( comparator )` (`sec-%typedarray%.prototype.sort`).
pub(super) fn sort<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If comparator is not undefined and IsCallable(comparator) is false, throw a TypeError
    //    exception.
    let comparefn = comparator(cx, args.get(0), "sort")?;
    // 2-4. obj, ValidateTypedArray, len.
    let (obj, len) = validated(cx, this)?;
    // 5-9. SortIndexedProperties, then the writes into obj.
    sort_into(cx, obj, len, comparefn, obj)?;
    // 10. Return obj.
    Ok(this)
}

/// `%TypedArray%.prototype.toSorted ( comparator )` (`sec-%typedarray%.prototype.tosorted`).
pub(super) fn to_sorted<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If comparator is not undefined and IsCallable(comparator) is false, throw a TypeError
    //    exception.
    let comparefn = comparator(cx, args.get(0), "toSorted")?;
    // 2-4. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 5. Let A be ? TypedArrayCreateSameType(O, len).
    let a = typed_array_create_same_type(cx, o, len)?;
    // 6-10. SortIndexedProperties(O, len, SortCompare, read-through-holes), then the writes
    //       into A.
    sort_into(cx, o, len, comparefn, a)?;
    // 11. Return A.
    Ok(a.value())
}

/// `%TypedArray%.prototype.toReversed ( )` (`sec-%typedarray%.prototype.toreversed`).
pub(super) fn to_reversed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. Let A be ? TypedArrayCreateSameType(O, len).
    let a = typed_array_create_same_type(cx, o, len)?;
    // 5-6. Repeat, while k < len: fromValue = ! Get(O, ! ToString(𝔽(len - k - 1)));
    //      ! Set(A, Pk, fromValue, true).
    for k in 0..len {
        cx.poll_interrupt()?;
        let from_value = cx.get_index(o, len - k - 1)?;
        cx.set_index(a, k, from_value, true)?;
    }
    // 7. Return A.
    Ok(a.value())
}

/// `%TypedArray%.prototype.with ( index, value )` (`sec-%typedarray%.prototype.with`).
pub(super) fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. Let relativeIndex be ? ToIntegerOrInfinity(index).
    let relative_index = cx.to_integer_or_infinity(args.get(0))?;
    // 5-6. actualIndex: relativeIndex if ≥ 0, else len + relativeIndex.
    let actual_index = if relative_index >= 0.0 {
        relative_index
    } else {
        len as f64 + relative_index
    };
    // 7. If O.[[ContentType]] is bigint, let numericValue be ? ToBigInt(value).
    // 8. Else, let numericValue be ? ToNumber(value).
    let r = row(cx, o);
    let numeric_value = to_element_value(cx, args.get(1), r)?;
    // 9. If IsValidIntegerIndex(O, 𝔽(actualIndex)) is false, throw a RangeError exception:
    //    detached or out of bounds, or actualIndex outside [0, TypedArrayLength).
    let valid = match cx.typed_array_length(o)? {
        Some(now) => actual_index >= 0.0 && actual_index < now as f64,
        None => false,
    };
    if !valid {
        return Err(cx.throw_range_error("Invalid typed array index"));
    }
    let actual_index = actual_index as u64;
    // 10. Let A be ? TypedArrayCreateSameType(O, len).
    let a = typed_array_create_same_type(cx, o, len)?;
    // 11-12. Repeat, while k < len: fromValue is numericValue at actualIndex, else ! Get(O,
    //        Pk); Perform ! Set(A, Pk, fromValue, true).
    for k in 0..len {
        cx.poll_interrupt()?;
        let from_value = if k == actual_index {
            numeric_value
        } else {
            cx.get_index(o, k)?
        };
        cx.set_index(a, k, from_value, true)?;
    }
    // 13. Return A.
    Ok(a.value())
}
