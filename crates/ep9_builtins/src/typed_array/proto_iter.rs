#![allow(dead_code)] // m8-stub
//! The iteration and search methods of `%TypedArray.prototype%`
//! (`sec-properties-of-the-%typedarray%-prototype-object`). Owner: TypedArray TA-4 R3 PROTO-A
//! (this file, and the `%ArrayIteratorPrototype%.next` view arm in `ep9_vm` natives.rs). Until
//! R3 lands, each method but two is a stub that throws a TypeError (m8-stub, TypedArray TA-4
//! R1). R1 wrote the full steps of `toLocaleString` and `get @@toStringTag`: with a stub, two
//! test262 files that pass before TA-4 would fail (`toLocaleString/get-length-uses-internal-
//! arraylength.js` calls the method and wants no throw; `Symbol.toStringTag/invoked-as-
//! accessor.js` reads the getter on `%TypedArray.prototype%` and wants undefined).

use ep9_vm::{Args, Cx, JsResult, Local};

use super::VIEWS;
use super::from_of::validate_typed_array;

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-4)");
    Err(cx.throw_type_error(&msg))
}

/// `%TypedArray%.prototype.entries ( )` (`sec-%typedarray%.prototype.entries`).
pub(super) fn entries<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.entries")
}

/// `%TypedArray%.prototype.keys ( )` (`sec-%typedarray%.prototype.keys`).
pub(super) fn keys<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.keys")
}

/// `%TypedArray%.prototype.values ( )` (`sec-%typedarray%.prototype.values`).
pub(super) fn values<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.values")
}

/// `%TypedArray%.prototype.at ( index )` (`sec-%typedarray%.prototype.at`).
pub(super) fn at<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.at")
}

/// `%TypedArray%.prototype.every ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.every`).
pub(super) fn every<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.every")
}

/// `%TypedArray%.prototype.some ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.some`).
pub(super) fn some<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.some")
}

/// `%TypedArray%.prototype.find ( predicate [ , thisArg ] )` (`sec-%typedarray%.prototype.find`).
pub(super) fn find<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.find")
}

/// `%TypedArray%.prototype.findIndex ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findindex`).
pub(super) fn find_index<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.findIndex")
}

/// `%TypedArray%.prototype.findLast ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findlast`).
pub(super) fn find_last<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.findLast")
}

/// `%TypedArray%.prototype.findLastIndex ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findlastindex`).
pub(super) fn find_last_index<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.findLastIndex")
}

/// `%TypedArray%.prototype.forEach ( callback [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.foreach`).
pub(super) fn for_each<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.forEach")
}

/// `%TypedArray%.prototype.includes ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.includes`).
pub(super) fn includes<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.includes")
}

/// `%TypedArray%.prototype.indexOf ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.indexof`).
pub(super) fn index_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.indexOf")
}

/// `%TypedArray%.prototype.lastIndexOf ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.lastindexof`).
pub(super) fn last_index_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.lastIndexOf")
}

/// `%TypedArray%.prototype.join ( separator )` (`sec-%typedarray%.prototype.join`).
pub(super) fn join<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.join")
}

/// `%TypedArray%.prototype.reduce ( callback [ , initialValue ] )`
/// (`sec-%typedarray%.prototype.reduce`).
pub(super) fn reduce<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.reduce")
}

/// `%TypedArray%.prototype.reduceRight ( callback [ , initialValue ] )`
/// (`sec-%typedarray%.prototype.reduceright`).
pub(super) fn reduce_right<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.reduceRight")
}

/// `%TypedArray%.prototype.toString ( )` (`sec-%typedarray%.prototype.tostring`).
pub(super) fn to_string<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "TypedArray.prototype.toString")
}

/// `%TypedArray%.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )`
/// (`sec-%typedarray%.prototype.tolocalestring`; the ECMA-262 form without ECMA-402): the
/// steps of `Array.prototype.toLocaleString` after ValidateTypedArray, with the length of the
/// TypedArray (no `length` property read).
pub(super) fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let array be the this value.
    // 2. Let taRecord be ? ValidateTypedArray(array, seq-cst).
    // 3. Let len be TypedArrayLength(taRecord).
    let len = validate_typed_array(cx, this)?;
    let array = this.as_object().expect("a TypedArray is an object");
    // 4. Let separator be the implementation-defined list-separator String appropriate for
    //    the host environment's current locale (such as ", "). ("," as V8.)
    let separator = u16::from(b',');
    // 5. Let R be the empty String. (Code units; the string is made once, at the end.)
    let mut r: Vec<u16> = Vec::new();
    // 6. Let k be 0.
    // 7. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. If k > 0, set R to the string-concatenation of R and separator.
        if k > 0 {
            r.push(separator);
        }
        // b. Let nextElement be ! Get(array, ! ToString(𝔽(k))). (Undefined once a user
        //    `toLocaleString` shrank or detached the buffer.)
        let element = cx.get_index(array, k)?;
        // c. If nextElement is neither undefined nor null, then
        if !element.is_undefined_or_null() {
            // i. Let S be ? ToString(? Invoke(nextElement, "toLocaleString")).
            let f = cx.get_v(element, "toLocaleString")?;
            if !cx.is_callable(f) {
                return Err(cx.throw_type_error("toLocaleString is not a function"));
            }
            let s = cx.call(f, element, &[])?;
            let s = cx.to_string(s)?;
            let s = cx.flatten(s)?;
            // ii. Set R to the string-concatenation of R and S.
            let mut i = 0;
            while let Some(u) = cx.code_unit_at(s, i) {
                r.push(u);
                i += 1;
            }
        }
        // The string length limit: `new_string_utf16` throws the RangeError.
        if r.len() > 1 << 30 {
            cx.new_string_utf16(&r)?;
        }
        // d. Set k to k + 1.
    }
    // 8. Return R.
    Ok(cx.new_string_utf16(&r)?.value())
}

/// `get %TypedArray%.prototype [ %Symbol.toStringTag% ]`
/// (`sec-get-%typedarray%.prototype-%symbol.tostringtag%`): the `[[TypedArrayName]]` of the
/// this value, or undefined (no throw).
pub(super) fn to_string_tag<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, return undefined.
    // 3. If O does not have a [[TypedArrayName]] internal slot, return undefined.
    let Some(row) = cx.typed_array_element(this) else {
        return Ok(Local::undefined());
    };
    // 4. Let name be O.[[TypedArrayName]].
    // 5. Assert: name is a String.
    // 6. Return name.
    Ok(cx.new_string(VIEWS[usize::from(row)].2)?.value())
}
