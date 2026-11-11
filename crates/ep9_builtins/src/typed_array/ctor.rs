//! `%TypedArray%` (§23.2.1, `sec-%typedarray%`), the 12 TypedArray constructors (§23.2.5,
//! `sec-typedarray`), `get %TypedArray%[@@species]` (§23.2.2.4) and the getters `buffer`,
//! `byteLength`, `byteOffset` and `length` of `%TypedArray.prototype%` (§23.2.3; decision O3).
//! Owner: TypedArray TA-3 TA-CTORS R2.
//!
//! ```text
//! <T>Array(...args)                       (sec-typedarray)
//!   no NewTarget                          TypeError
//!   no argument                           AllocateTypedArray(len 0)
//!   first argument not an Object          ToIndex, then AllocateTypedArray(len)
//!   first argument an Object              AllocateTypedArray (GetPrototypeFromConstructor), then
//!     a TypedArray                        InitializeTypedArrayFromTypedArray
//!     an ArrayBuffer / SharedArrayBuffer  InitializeTypedArrayFromArrayBuffer
//!     @@iterator is a function            IteratorToList (a ScratchList), InitializeTypedArrayFromList
//!     else                                InitializeTypedArrayFromArrayLike
//! ```
//!
//! The VM makes a view only on a buffer (`Cx::new_typed_array`), so the object of
//! AllocateTypedArray comes into being after its buffer: GetPrototypeFromConstructor runs at
//! the spec's place, and the view is made when the spec gives it its buffer. No user code can
//! see the object before that. Every buffer byte is read and written through `Cx` methods
//! (`copy_buffer_bytes`, `get_value_from_buffer`, `set_value_in_buffer`, the element
//! `[[Set]]`), which check the range again and use atomic access on shared bytes.

use ep9_vm::abi::JsObject;
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local};

use super::VIEWS;
use crate::array_buffer::{incompatible, requires_new, to_index};

/// The `[[LittleEndian]]` value of the agent: a copy reads and writes with the same order.
const LITTLE_ENDIAN: bool = cfg!(target_endian = "little");

/// The RangeError text of a length that is no index or does not fit.
const LENGTH: &str = "Invalid typed array length";

/// The RangeError text of a byte offset that is no index.
const OFFSET: &str = "Start offset is outside the bounds of the buffer";

/// The TypeError text of a detached or out-of-bounds source.
const DETACHED: &str = "Cannot perform Construct on a detached ArrayBuffer";

/// Is row `r` of [`VIEWS`] of `[[ContentType]]` bigint (BigInt64, BigUint64)?
fn is_bigint_row(r: u8) -> bool {
    r == 7 || r == 8
}

/// The element size of row `r` of [`VIEWS`].
fn element_size(r: u8) -> u64 {
    u64::from(VIEWS[usize::from(r)].3)
}

// ---- %TypedArray% ----------------------------------------------------------------------------

/// `%TypedArray%` ( ) (§23.2.1.1): always a TypeError, with and without `new`.
pub(super) fn typed_array<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Throw a TypeError exception.
    Err(cx.throw_type_error("Abstract class TypedArray not directly constructable"))
}

/// `get %TypedArray%[@@species]` (§23.2.2.4): the this value.
pub(super) fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return the this value.
    Ok(this)
}

// ---- the getters of %TypedArray.prototype% ---------------------------------------------------

/// `this` if it is a TypedArray (`RequireInternalSlot(O, [[TypedArrayName]])`), else a
/// TypeError naming `what`.
fn this_typed_array<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s>> {
    if cx.typed_array_element(this).is_some() {
        Ok(this)
    } else {
        Err(incompatible(cx, what))
    }
}

/// `get %TypedArray%.prototype.buffer` (§23.2.3.2).
pub(super) fn buffer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let O be the this value; ? RequireInternalSlot(O, [[TypedArrayName]]).
    let o = this_typed_array(cx, this, "get %TypedArray%.prototype.buffer")?;
    // 4. Let buffer be O.[[ViewedArrayBuffer]] (also when it is detached). 5. Return buffer.
    Ok(cx.typed_array_buffer(o)?.value())
}

/// `get %TypedArray%.prototype.byteLength` (§23.2.3.3).
pub(super) fn byte_length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let O be the this value; ? RequireInternalSlot(O, [[TypedArrayName]]).
    let o = this_typed_array(cx, this, "get %TypedArray%.prototype.byteLength")?;
    let row = cx.typed_array_element(o).expect("a TypedArray");
    // 4. Let taRecord be MakeTypedArrayWithBufferWitnessRecord(O, seq-cst).
    // 5. Let size be TypedArrayByteLength(taRecord): 0 when IsTypedArrayOutOfBounds, else
    //    TypedArrayLength(taRecord) × TypedArrayElementSize(O) (a fixed [[ByteLength]] is the
    //    same product). 6. Return 𝔽(size).
    let size = cx
        .typed_array_length(o)?
        .map_or(0, |n| n * element_size(row));
    Ok(Local::number(size as f64))
}

/// `get %TypedArray%.prototype.byteOffset` (§23.2.3.4).
pub(super) fn byte_offset<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let O be the this value; ? RequireInternalSlot(O, [[TypedArrayName]]).
    let o = this_typed_array(cx, this, "get %TypedArray%.prototype.byteOffset")?;
    // 4. Let taRecord be MakeTypedArrayWithBufferWitnessRecord(O, seq-cst).
    // 5. If IsTypedArrayOutOfBounds(taRecord) is true, return +0𝔽.
    if cx.typed_array_length(o)?.is_none() {
        return Ok(Local::number(0.0));
    }
    // 6. Let offset be O.[[ByteOffset]]. 7. Return 𝔽(offset).
    let offset = cx.typed_array_byte_offset(o)?;
    Ok(Local::number(offset as f64))
}

/// `get %TypedArray%.prototype.length` (§23.2.3.21).
pub(super) fn length<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. Let O be the this value; ? RequireInternalSlot(O, [[TypedArrayName]]).
    let o = this_typed_array(cx, this, "get %TypedArray%.prototype.length")?;
    // 4. Let taRecord be MakeTypedArrayWithBufferWitnessRecord(O, seq-cst).
    // 5. If IsTypedArrayOutOfBounds(taRecord) is true, return +0𝔽.
    // 6. Let length be TypedArrayLength(taRecord). 7. Return 𝔽(length).
    let n = cx.typed_array_length(o)?.unwrap_or(0);
    Ok(Local::number(n as f64))
}

// ---- the 12 constructors -------------------------------------------------------------------

/// AllocateTypedArrayBuffer (§23.2.5.1.6) for a TypedArray of row `row` and prototype `proto`
/// (AllocateTypedArray step 1 ran), then the object (TypedArrayCreate): a new
/// `%ArrayBuffer%` of the current realm with `length` × the element size zero bytes (a
/// RangeError if it cannot be allocated), and a view of `length` elements on it.
fn allocate<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    row: u8,
    length: u64,
) -> JsResult<Local<'s, JsObject>> {
    // 3. Let byteLength be elementSize × length (at most 8 × (2^53 - 1): no overflow).
    let byte_length = length * element_size(row);
    // 4. Let data be ? AllocateArrayBuffer(%ArrayBuffer%, byteLength): %ArrayBuffer%.prototype
    //    is non-writable and non-configurable, so the prototype is the intrinsic.
    let ab_proto = cx.intrinsic(IntrinsicId::ArrayBufferPrototype);
    let data = cx.allocate_array_buffer(ab_proto, byte_length, None)?;
    // 5-8. [[ViewedArrayBuffer]] data, [[ByteLength]] byteLength, [[ByteOffset]] 0,
    //      [[ArrayLength]] length.
    cx.new_typed_array(proto, row, data, 0, Some(length))
}

/// TypedArray ( ...args ) (§23.2.5.1) of row `row` of [`VIEWS`].
fn construct<'s>(
    cx: &mut Cx<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
    row: u8,
) -> JsResult<Local<'s>> {
    let (_, default_proto, name, _) = VIEWS[usize::from(row)];
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, name));
    };
    // 2-4. constructorName, proto "%TypedArray.prototype%", numberOfArgs.
    // 5. If numberOfArgs = 0, return ? AllocateTypedArray(constructorName, NewTarget, proto, 0).
    if args.is_empty() {
        let proto = cx.get_prototype_from_constructor(nt, default_proto)?;
        return Ok(allocate(cx, proto, row, 0)?.value());
    }
    // 6.a. Let firstArgument be args[0].
    let first = args.get(0);
    // 6.c. firstArgument is not an Object: ToIndex runs before AllocateTypedArray.
    let Some(first_obj) = first.as_object() else {
        // ii. Let elementLength be ? ToIndex(firstArgument).
        let length = to_index(cx, first, LENGTH)?;
        // iii. Return ? AllocateTypedArray(constructorName, NewTarget, proto, elementLength).
        let proto = cx.get_prototype_from_constructor(nt, default_proto)?;
        return Ok(allocate(cx, proto, row, length)?.value());
    };
    // 6.b.i. Let O be ? AllocateTypedArray(constructorName, NewTarget, proto): its step 1,
    //        GetPrototypeFromConstructor (a `prototype` getter runs here, first).
    let proto = cx.get_prototype_from_constructor(nt, default_proto)?;
    let o = if cx.typed_array_element(first).is_some() {
        // ii. ? InitializeTypedArrayFromTypedArray(O, firstArgument).
        from_typed_array(cx, proto, row, first)?
    } else if cx.is_array_buffer(first) || cx.is_shared_array_buffer(first) {
        // iii. byteOffset args[1], length args[2] (undefined when absent);
        //      ? InitializeTypedArrayFromArrayBuffer(O, firstArgument, byteOffset, length).
        from_array_buffer(cx, proto, row, first, args.get(1), args.get(2))?
    } else {
        // iv.2. Let usingIterator be ? GetMethod(firstArgument, %Symbol.iterator%).
        let key = crate::string::well_known(cx, "iterator")?;
        let using_iterator = cx.get(first_obj, key)?;
        if using_iterator.is_undefined_or_null() {
            // iv.4. ? InitializeTypedArrayFromArrayLike(O, firstArgument).
            from_array_like(cx, proto, row, first_obj)?
        } else if cx.is_callable(using_iterator) {
            // iv.3. Let values be ? IteratorToList(? GetIteratorFromMethod(firstArgument,
            //       usingIterator)); ? InitializeTypedArrayFromList(O, values).
            from_iterable(cx, proto, row, first, using_iterator)?
        } else {
            // GetMethod step 3: a value that is not callable is a TypeError.
            return Err(cx.throw_type_error("object[Symbol.iterator] is not a function"));
        }
    };
    // 6.b.v. Return O.
    Ok(o.value())
}

/// InitializeTypedArrayFromTypedArray ( O, srcArray ) (§23.2.5.1.2) for a TypedArray of row
/// `row` and prototype `proto`.
fn from_typed_array<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    row: u8,
    src: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let srcData be srcArray.[[ViewedArrayBuffer]].
    let src_data = cx.typed_array_buffer(src)?;
    // 2-5. elementType, elementSize, srcType, srcElementSize.
    let src_row = cx.typed_array_element(src).expect("a TypedArray");
    let (size, src_size) = (element_size(row), element_size(src_row));
    // 6. Let srcByteOffset be srcArray.[[ByteOffset]].
    let src_offset = cx.typed_array_byte_offset(src)?;
    // 7. Let srcRecord be MakeTypedArrayWithBufferWitnessRecord(srcArray, seq-cst).
    // 8. If IsTypedArrayOutOfBounds(srcRecord) is true, throw a TypeError exception.
    // 9. Let elementLength be TypedArrayLength(srcRecord).
    let Some(length) = cx.typed_array_length(src)? else {
        return Err(cx.throw_type_error(
            "Cannot perform Construct on a detached or out-of-bounds ArrayBuffer",
        ));
    };
    // 10. Let byteLength be elementSize × elementLength.
    let byte_length = size * length;
    let ab_proto = cx.intrinsic(IntrinsicId::ArrayBufferPrototype);
    let data = if src_row == row {
        // 11.a. Let data be ? CloneArrayBuffer(srcData, srcByteOffset, byteLength): a new
        //       %ArrayBuffer%, then CopyDataBlockBytes (byte copies on shared bytes).
        let data = cx.allocate_array_buffer(ab_proto, byte_length, None)?;
        cx.copy_buffer_bytes(data, 0, src_data, src_offset, byte_length)?;
        data
    } else {
        // 12.a. Let data be ? AllocateArrayBuffer(%ArrayBuffer%, byteLength).
        let data = cx.allocate_array_buffer(ab_proto, byte_length, None)?;
        // 12.b. If srcArray.[[ContentType]] is not O.[[ContentType]], throw a TypeError.
        if is_bigint_row(src_row) != is_bigint_row(row) {
            return Err(
                cx.throw_type_error("Cannot mix BigInt and other types, use explicit conversions")
            );
        }
        // 12.c-g. For each element: GetValueFromBuffer(srcData, srcByteIndex, srcType, true,
        //         unordered), then SetValueInBuffer(data, targetByteIndex, elementType, value,
        //         true, unordered). No JS runs in the loop.
        for k in 0..length {
            cx.poll_interrupt()?;
            let value = cx.get_value_from_buffer(
                src_data,
                src_offset + k * src_size,
                src_row,
                LITTLE_ENDIAN,
            )?;
            cx.set_value_in_buffer(data, k * size, row, value, LITTLE_ENDIAN)?;
        }
        data
    };
    // 13-17. [[ViewedArrayBuffer]] data, [[ByteLength]] byteLength, [[ByteOffset]] 0,
    //        [[ArrayLength]] elementLength.
    cx.new_typed_array(proto, row, data, 0, Some(length))
}

/// InitializeTypedArrayFromArrayBuffer ( O, buffer, byteOffset, length ) (§23.2.5.1.3) for a
/// TypedArray of row `row` and prototype `proto`.
fn from_array_buffer<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    row: u8,
    buffer: Local<'s>,
    byte_offset: Local<'s>,
    length: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    let name = VIEWS[usize::from(row)].2;
    // 1. Let elementSize be TypedArrayElementSize(O).
    let size = element_size(row);
    // 2. Let offset be ? ToIndex(byteOffset).
    let offset = to_index(cx, byte_offset, OFFSET)?;
    // 3. If offset modulo elementSize ≠ 0, throw a RangeError exception.
    if !offset.is_multiple_of(size) {
        let msg = format!("start offset of {name} should be a multiple of {size}");
        return Err(cx.throw_range_error(&msg));
    }
    // 4. Let bufferIsFixedLength be IsFixedLengthArrayBuffer(buffer).
    let fixed = cx.array_buffer_max_byte_length(buffer)?.is_none();
    // 5. If length is not undefined, let newLength be ? ToIndex(length).
    let new_length = if length.is_undefined() {
        None
    } else {
        Some(to_index(cx, length, LENGTH)?)
    };
    // 6. If IsDetachedBuffer(buffer) is true, throw a TypeError exception.
    if cx.is_detached_buffer(buffer) {
        return Err(cx.throw_type_error(DETACHED));
    }
    // 7. Let bufferByteLength be ArrayBufferByteLength(buffer, seq-cst).
    let buffer_byte_length = cx.array_buffer_byte_length(buffer)?;
    match new_length {
        // 8. If length is undefined and bufferIsFixedLength is false: a length-tracking view.
        //    a. If offset > bufferByteLength, throw a RangeError exception.
        None if !fixed => {
            if offset > buffer_byte_length {
                let msg = format!("Start offset {offset} is outside the bounds of the buffer");
                return Err(cx.throw_range_error(&msg));
            }
        }
        // 9.a. If length is undefined:
        None => {
            // i. If bufferByteLength modulo elementSize ≠ 0, throw a RangeError exception.
            if !buffer_byte_length.is_multiple_of(size) {
                let msg = format!("byte length of {name} should be a multiple of {size}");
                return Err(cx.throw_range_error(&msg));
            }
            // ii-iii. Let newByteLength be bufferByteLength - offset; if it is < 0, throw a
            //         RangeError exception.
            if offset > buffer_byte_length {
                let msg = format!("Start offset {offset} is outside the bounds of the buffer");
                return Err(cx.throw_range_error(&msg));
            }
        }
        // 9.b. Else: i. Let newByteLength be newLength × elementSize; ii. if offset +
        //      newByteLength > bufferByteLength, throw a RangeError exception.
        Some(n) => {
            if offset + n * size > buffer_byte_length {
                let msg = format!("{LENGTH}: {n}");
                return Err(cx.throw_range_error(&msg));
            }
        }
    }
    // 8.b-c, 9.c-d, 10-11. The slots: the VM makes the view (auto for `None` on a resizable or
    // growable buffer, else (bufferByteLength - offset) / elementSize elements for `None`), and
    // checks the range again.
    cx.new_typed_array(proto, row, buffer, offset, new_length)
}

/// IteratorToList(GetIteratorFromMethod(`items`, `method`)) (§7.4.13), then
/// InitializeTypedArrayFromList ( O, values ) (§23.2.5.1.4) for a TypedArray of row `row` and
/// prototype `proto`. The values live in a [`ep9_vm::ScratchList`] (precise roots).
fn from_iterable<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    row: u8,
    items: Local<'s>,
    method: Local<'s>,
) -> JsResult<Local<'s, JsObject>> {
    // IteratorToList: 1. Let values be a new empty List. 2. Repeat: a. Let next be ?
    // IteratorStepValue(iteratorRecord); b. if next is done, return values; c. append next.
    let mut record = cx.get_iterator_from_method(items, method)?;
    let values = cx.scratch_list();
    while let Some(next) = cx.iterator_step_value(&mut record)? {
        values.push(next);
    }
    // 1. Let len be the number of elements in values.
    let len = values.len() as u64;
    // 2. Perform ? AllocateTypedArrayBuffer(O, len).
    let o = allocate(cx, proto, row, len)?;
    // 3-4. For each k < len: Perform ? Set(O, ! ToString(𝔽(k)), values[k], true) (the
    //      element [[Set]]: ToNumber or ToBigInt, which can detach or shrink the buffer).
    for k in 0..values.len() {
        cx.set_index(o, k as u64, values.get(k), true)?;
    }
    Ok(o)
}

/// InitializeTypedArrayFromArrayLike ( O, arrayLike ) (§23.2.5.1.5) for a TypedArray of row
/// `row` and prototype `proto`.
fn from_array_like<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    row: u8,
    array_like: Local<'s, JsObject>,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let len be ? LengthOfArrayLike(arrayLike).
    let len = cx.get(array_like, "length")?;
    let len = cx.to_length(len)?;
    // 2. Perform ? AllocateTypedArrayBuffer(O, len).
    let o = allocate(cx, proto, row, len)?;
    // 3-4. For each k < len: Let kValue be ? Get(arrayLike, Pk); Perform ? Set(O, Pk, kValue,
    //      true). A hole runs no JS, so the loop polls (a walk to a long length must stop).
    for k in 0..len {
        cx.poll_interrupt()?;
        let value = cx.get_index(array_like, k)?;
        cx.set_index(o, k, value, true)?;
    }
    Ok(o)
}

/// The constructor of row `r` of [`VIEWS`].
macro_rules! views {
    ($($f:ident $r:literal),*) => {
        $(
            #[doc = concat!("TypedArray ( ...args ) (§23.2.5.1) of row ", $r, " of [`VIEWS`].")]
            pub(super) fn $f<'s>(
                cx: &mut Cx<'s>,
                _this: Local<'s>,
                args: Args<'s>,
                new_target: Local<'s>,
            ) -> JsResult<Local<'s>> {
                construct(cx, args, new_target, $r)
            }
        )*
    };
}

views!(
    int8_array 0,
    uint8_array 1,
    uint8_clamped_array 2,
    int16_array 3,
    uint16_array 4,
    int32_array 5,
    uint32_array 6,
    big_int64_array 7,
    big_uint64_array 8,
    float16_array 9,
    float32_array 10,
    float64_array 11
);
