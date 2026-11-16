//! `%TypedArray%.from` (§23.2.2.1, `sec-%typedarray%.from`) and `%TypedArray%.of` (§23.2.2.2,
//! `sec-%typedarray%.of`), with the abstract operations they call: ValidateTypedArray
//! (`sec-validatetypedarray`) and TypedArrayCreateFromConstructor
//! (`sec-typedarraycreatefromconstructor`). Owner: TypedArray TA-3 TA-CTORS R3.
//!
//! ```text
//! from(source, mapper, thisArg)
//!   this not a constructor            TypeError
//!   mapper not undefined, not callable TypeError
//!   GetMethod(source, @@iterator)
//!     a function                      IteratorToList (a ScratchList) BEFORE the create, then
//!                                     TypedArrayCreateFromConstructor(C, «len»), map, Set
//!     undefined or null               ToObject, LengthOfArrayLike, create, then Get, map, Set
//!     else                            TypeError
//! of(...items)                        create with «len», then Set per item
//! ```
//!
//! Every `Set` is the element `[[Set]]` of the new TypedArray (ToNumber or ToBigInt first: a
//! `valueOf` or the mapper can detach, shrink or grow its buffer; an index out of bounds is
//! ignored). No buffer byte is read or written here except through that `[[Set]]`.

use ep9_vm::abi::JsObject;
use ep9_vm::{Args, Cx, JsResult, Local};

/// The TypeError text of a `this` value that is not a constructor.
const NOT_A_CONSTRUCTOR: &str = "this is not a constructor";

/// ValidateTypedArray ( O, order ) (§23.2.4.4): the TypedArrayLength of the record of `o`. A
/// TypeError if `o` is no TypedArray, or if it is out of bounds (a detached buffer included).
pub(crate) fn validate_typed_array<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<u64> {
    // 1. Perform ? RequireInternalSlot(O, [[TypedArrayName]]).
    if cx.typed_array_element(o).is_none() {
        return Err(cx.throw_type_error("this is not a typed array."));
    }
    // 3. Let taRecord be MakeTypedArrayWithBufferWitnessRecord(O, order).
    // 4. If IsTypedArrayOutOfBounds(taRecord) is true, throw a TypeError exception.
    match cx.typed_array_length(o)? {
        // 5. Return taRecord (here its TypedArrayLength).
        Some(length) => Ok(length),
        None => Err(cx.throw_type_error(
            "Cannot perform this operation on a detached or out-of-bounds TypedArray",
        )),
    }
}

/// TypedArrayCreateFromConstructor ( constructor, argumentList ) (§23.2.4.2) with the one
/// argument `length` (the form that `from` and `of` use).
pub(crate) fn typed_array_create_from_constructor<'s>(
    cx: &mut Cx<'s>,
    constructor: Local<'s>,
    length: u64,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let newTypedArray be ? Construct(constructor, argumentList).
    let new = cx.construct(constructor, &[Local::number(length as f64)], constructor)?;
    // 2. Let taRecord be ? ValidateTypedArray(newTypedArray, seq-cst).
    let new_length = validate_typed_array(cx, new.value())?;
    // 3. If argumentList is a List of a single Number, then
    //    a. If IsTypedArrayOutOfBounds(taRecord) is true, throw a TypeError (step 2 did).
    //    b. Let length be TypedArrayLength(taRecord).
    //    c. If length < ℝ(argumentList[0]), throw a TypeError exception.
    if new_length < length {
        return Err(cx.throw_type_error(
            "The constructor made a TypedArray shorter than the length it was given",
        ));
    }
    // 4. Return newTypedArray.
    Ok(new)
}

/// `%TypedArray%.from ( source [ , mapper [ , thisArg ] ] )` (§23.2.2.1).
pub(super) fn from<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (source, mapper, this_arg) = (args.get(0), args.get(1), args.get(2));
    // 1. Let C be the this value.
    let c = this;
    // 2. If IsConstructor(C) is false, throw a TypeError exception.
    if !cx.is_constructor(c) {
        return Err(cx.throw_type_error(NOT_A_CONSTRUCTOR));
    }
    // 3. If mapper is undefined, let mapping be false.
    // 4. Else: a. if IsCallable(mapper) is false, throw a TypeError exception; b. mapping is
    //    true.
    let mapping = !mapper.is_undefined();
    if mapping && !cx.is_callable(mapper) {
        return Err(cx.throw_type_error("TypedArray.from: the mapper is not a function"));
    }
    // 5. Let usingIterator be ? GetMethod(source, %Symbol.iterator%). GetV throws for
    //    undefined and null (ToObject); a value that is not undefined, null or callable is a
    //    TypeError (GetMethod step 3).
    if source.is_undefined_or_null() {
        return Err(cx.throw_type_error("TypedArray.from: the source is undefined or null"));
    }
    let key = crate::string::well_known(cx, "iterator")?;
    let using_iterator = cx.get_v(source, key)?;
    // 6. If usingIterator is not undefined, then
    if !using_iterator.is_undefined_or_null() {
        if !cx.is_callable(using_iterator) {
            return Err(
                cx.throw_type_error("TypedArray.from: source[Symbol.iterator] is not a function")
            );
        }
        // a. Let values be ? IteratorToList(? GetIteratorFromMethod(source, usingIterator)):
        //    the whole list is read before the create. The values live in a ScratchList
        //    (precise roots).
        let mut record = cx.get_iterator_from_method(source, using_iterator)?;
        let values = cx.scratch_list();
        while let Some(next) = cx.iterator_step_value(&mut record)? {
            values.push(next);
        }
        // b. Let len be the number of elements in values.
        let len = values.len();
        // c. Let targetObj be ? TypedArrayCreateFromConstructor(C, « 𝔽(len) »).
        let target = typed_array_create_from_constructor(cx, c, len as u64)?;
        // d-e. For each k < len: kValue is values[k]; mappedValue is ? Call(mapper, thisArg,
        //      « kValue, 𝔽(k) ») if mapping, else kValue; Perform ? Set(targetObj, Pk,
        //      mappedValue, true). (The spec removes the first element of values each time;
        //      reading index k is the same.) A Number runs no JS, so the loop polls.
        for k in 0..len {
            cx.poll_interrupt()?;
            let k_value = values.get(k);
            let mapped_value = if mapping {
                cx.call(mapper, this_arg, &[k_value, Local::number(k as f64)])?
            } else {
                k_value
            };
            cx.set_index(target, k as u64, mapped_value, true)?;
        }
        // f-g. Return targetObj.
        return Ok(target.value());
    }
    // 7. NOTE: source is not an Iterable, so assume it is already an array-like object.
    // 8. Let arrayLike be ! ToObject(source) (step 5 threw for undefined and null).
    let array_like = cx.to_object(source)?;
    // 9. Let len be ? LengthOfArrayLike(arrayLike).
    let len = cx.get(array_like, "length")?;
    let len = cx.to_length(len)?;
    // 10. Let targetObj be ? TypedArrayCreateFromConstructor(C, « 𝔽(len) »).
    let target = typed_array_create_from_constructor(cx, c, len)?;
    // 11-12. For each k < len: Let kValue be ? Get(arrayLike, Pk); mappedValue is ?
    //        Call(mapper, thisArg, « kValue, 𝔽(k) ») if mapping, else kValue; Perform ?
    //        Set(targetObj, Pk, mappedValue, true). A hole and a plain number run no JS, so
    //        the loop polls.
    for k in 0..len {
        cx.poll_interrupt()?;
        let k_value = cx.get_index(array_like, k)?;
        let mapped_value = if mapping {
            cx.call(mapper, this_arg, &[k_value, Local::number(k as f64)])?
        } else {
            k_value
        };
        cx.set_index(target, k, mapped_value, true)?;
    }
    // 13. Return targetObj.
    Ok(target.value())
}

/// `%TypedArray%.of ( ...items )` (§23.2.2.2).
pub(super) fn of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let len be the number of elements in items.
    let len = args.len();
    // 2. Let C be the this value.
    let c = this;
    // 3. If IsConstructor(C) is false, throw a TypeError exception.
    if !cx.is_constructor(c) {
        return Err(cx.throw_type_error(NOT_A_CONSTRUCTOR));
    }
    // 4. Let newObj be ? TypedArrayCreateFromConstructor(C, « 𝔽(len) »).
    let new_obj = typed_array_create_from_constructor(cx, c, len as u64)?;
    // 5-6. For each k < len: Let kValue be items[k]; Perform ? Set(newObj, Pk, kValue, true).
    //      A Number runs no JS, so the loop polls.
    for k in 0..len {
        cx.poll_interrupt()?;
        cx.set_index(new_obj, k as u64, args.get(k), true)?;
    }
    // 7. Return newObj.
    Ok(new_obj.value())
}
