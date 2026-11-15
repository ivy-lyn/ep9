//! The abstract operations of the `%TypedArray.prototype%` methods that make a new TypedArray:
//! TypedArraySpeciesCreate (`sec-typedarray-species-create`), TypedArrayCreateSameType
//! (`sec-typedarray-create-same-type`) and SpeciesConstructor (`sec-speciesconstructor`, a
//! private copy, the crate form). Owner: TypedArray TA-4 R4 PROTO-B.
//!
//! ```text
//! species_create(O, len)            filter map slice     SpeciesConstructor(O, %T%), then
//!                                                        TypedArrayCreateFromConstructor(C,
//!                                                        «len») (from_of.rs: the length check),
//!                                                        then the [[ContentType]] check
//! species_create_view(O, buf, ...)  subarray             the same, with «buffer, byteOffset
//!                                                        [, length]» (no length check)
//! create_same_type(O, len)          toReversed toSorted  AllocateTypedArray of O's row on a new
//!                                   with                 %ArrayBuffer% (no user code)
//! ```

use ep9_vm::abi::JsObject;
use ep9_vm::{Cx, IntrinsicId, JsResult, Local};

use super::VIEWS;
use super::ctor::{allocate, is_bigint_row};
use super::from_of::{typed_array_create_from_constructor, validate_typed_array};

/// The row of spec Table 71 of the TypedArray `o` (a builtin bug otherwise).
fn row<'s>(cx: &Cx<'s>, o: impl Into<Local<'s>>) -> u8 {
    cx.typed_array_element(o).expect("a TypedArray")
}

/// SpeciesConstructor ( O, defaultConstructor ) (`sec-speciesconstructor`).
fn species_constructor<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    default_constructor: IntrinsicId,
) -> JsResult<Local<'s>> {
    // 1. Let C be ? Get(O, "constructor").
    let c = cx.get(o, "constructor")?;
    // 2. If C is undefined, return defaultConstructor.
    if c.is_undefined() {
        return Ok(cx.intrinsic(default_constructor).value());
    }
    // 3. If C is not an Object, throw a TypeError exception.
    let Some(c) = c.as_object() else {
        return Err(cx.throw_type_error("The .constructor property is not an object"));
    };
    // 4. Let S be ? Get(C, %Symbol.species%).
    let species = crate::string::well_known(cx, "species")?;
    let s = cx.get(c, species)?;
    // 5. If S is either undefined or null, return defaultConstructor.
    if s.is_undefined_or_null() {
        return Ok(cx.intrinsic(default_constructor).value());
    }
    // 6. If IsConstructor(S) is true, return S.
    if cx.is_constructor(s) {
        return Ok(s);
    }
    // 7. Throw a TypeError exception.
    Err(cx.throw_type_error("object.constructor[Symbol.species] is not a constructor"))
}

/// Steps 1-2 of TypedArraySpeciesCreate ( exemplar, argumentList ): the constructor.
fn species_of<'s>(cx: &mut Cx<'s>, exemplar: Local<'s, JsObject>) -> JsResult<Local<'s>> {
    // 1. Let defaultConstructor be the intrinsic object associated with the constructor name
    //    exemplar.[[TypedArrayName]] in Table 71.
    let default_constructor = VIEWS[usize::from(row(cx, exemplar))].0;
    // 2. Let constructor be ? SpeciesConstructor(exemplar, defaultConstructor).
    species_constructor(cx, exemplar, default_constructor)
}

/// Step 4 of TypedArraySpeciesCreate: `result` if its `[[ContentType]]` is the one of
/// `exemplar`, else a TypeError.
fn same_content_type<'s>(
    cx: &mut Cx<'s>,
    exemplar: Local<'s, JsObject>,
    result: Local<'s, JsObject>,
) -> JsResult<Local<'s, JsObject>> {
    // 4. If result.[[ContentType]] is not exemplar.[[ContentType]], throw a TypeError
    //    exception.
    if is_bigint_row(row(cx, result)) != is_bigint_row(row(cx, exemplar)) {
        return Err(
            cx.throw_type_error("Content type of the species result is not the content type")
        );
    }
    // 5. Return result.
    Ok(result)
}

/// TypedArraySpeciesCreate ( exemplar, « 𝔽(length) » ) (`sec-typedarray-species-create`), the
/// form of `filter`, `map` and `slice`.
pub(super) fn typed_array_species_create<'s>(
    cx: &mut Cx<'s>,
    exemplar: Local<'s, JsObject>,
    length: u64,
) -> JsResult<Local<'s, JsObject>> {
    // 1-2. defaultConstructor, constructor.
    let constructor = species_of(cx, exemplar)?;
    // 3. Let result be ? TypedArrayCreateFromConstructor(constructor, argumentList): Construct,
    //    ValidateTypedArray, and (one Number argument) a TypeError when the result is shorter.
    let result = typed_array_create_from_constructor(cx, constructor, length)?;
    // 4-5. The [[ContentType]] check.
    same_content_type(cx, exemplar, result)
}

/// TypedArraySpeciesCreate ( exemplar, « buffer, 𝔽(byteOffset) [ , 𝔽(length) ] » )
/// (`sec-typedarray-species-create`), the form of `subarray`: `length` `None` gives the
/// two-argument list.
pub(super) fn typed_array_species_create_view<'s>(
    cx: &mut Cx<'s>,
    exemplar: Local<'s, JsObject>,
    buffer: Local<'s, JsObject>,
    byte_offset: u64,
    length: Option<u64>,
) -> JsResult<Local<'s, JsObject>> {
    // 1-2. defaultConstructor, constructor.
    let constructor = species_of(cx, exemplar)?;
    // 3. Let result be ? TypedArrayCreateFromConstructor(constructor, argumentList):
    //    a. Let newTypedArray be ? Construct(constructor, argumentList).
    let offset = Local::number(byte_offset as f64);
    let result = match length {
        Some(n) => cx.construct(
            constructor,
            &[buffer.value(), offset, Local::number(n as f64)],
            constructor,
        )?,
        None => cx.construct(constructor, &[buffer.value(), offset], constructor)?,
    };
    //    b. Let taRecord be ? ValidateTypedArray(newTypedArray, seq-cst).
    //    c. (The list is not one Number: no length check.)
    validate_typed_array(cx, result.value())?;
    // 4-5. The [[ContentType]] check.
    same_content_type(cx, exemplar, result)
}

/// TypedArrayCreateSameType ( exemplar, length ) (`sec-typedarray-create-same-type`): a new
/// TypedArray of the row of `exemplar` with `length` elements on a new `%ArrayBuffer%`.
/// TypedArrayCreateFromConstructor on the intrinsic constructor runs no user code (its
/// `prototype` is a non-writable, non-configurable data property), so this is
/// AllocateTypedArray with the intrinsic prototype of the current realm.
pub(super) fn typed_array_create_same_type<'s>(
    cx: &mut Cx<'s>,
    exemplar: Local<'s, JsObject>,
    length: u64,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let constructor be the intrinsic object associated with the constructor name
    //    exemplar.[[TypedArrayName]] in Table 71.
    let r = row(cx, exemplar);
    // 2. Let result be ? TypedArrayCreateFromConstructor(constructor, « 𝔽(length) »).
    // 3. Assert: result has [[TypedArrayName]] and [[ContentType]] internal slots.
    // 4. Assert: result.[[ContentType]] is exemplar.[[ContentType]].
    // 5. Return result.
    let proto = cx.intrinsic(VIEWS[usize::from(r)].1);
    allocate(cx, proto, r, length)
}
