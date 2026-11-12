//! The iteration and search methods of `%TypedArray.prototype%`
//! (`sec-properties-of-the-%typedarray%-prototype-object`). Owner: TypedArray TA-4 R3 PROTO-A
//! (this file, and the `%ArrayIteratorPrototype%.next` view arm in `ep9_vm` natives.rs).
//!
//! ```text
//! every method          ValidateTypedArray(O, seq-cst) -> len (TypedArrayLength; no "length"
//!                       read), then the steps below with that len
//! entries keys values   CreateArrayIterator (`Cx::create_array_iterator`); @@iterator = values
//! at                    ToIntegerOrInfinity(index), then Get
//! every some forEach    callback(Get(O, k), k, O) per index; a callback that detaches or
//! find findIndex        shrinks the buffer makes later Gets give undefined (no throw)
//! findLast findLastIndex
//! reduce reduceRight
//! includes              SameValueZero with Get(O, k) for k < len (undefined past a shrink)
//! indexOf lastIndexOf   IsStrictlyEqual, only when HasProperty(O, k) (false past a shrink)
//! join                  ToString(separator) once, then ToString per element (undefined: "")
//! toLocaleString        Invoke(element, "toLocaleString") per element (undefined, null: "")
//! toString              the steps of Array.prototype.toString (route TA-TOSTRING-IDENTITY)
//! get @@toStringTag     [[TypedArrayName]], else undefined (no throw)
//! ```
//!
//! Elements are read only through `Cx::get_index` and `Cx::has_index` (the TypedArray
//! `[[Get]]` and `[[HasProperty]]`): no buffer byte is read here, so a SharedArrayBuffer view
//! takes the `ep9_vm` shared-memory accessors. Each loop polls interrupts once per index.

use ep9_vm::abi::JsObject;
use ep9_vm::{Args, ArrayIterKind, Cx, JsResult, JsString, Local};

use super::VIEWS;
use super::from_of::validate_typed_array;
use crate::string::MAX_LENGTH;

/// The this value of a method after ValidateTypedArray(O, seq-cst) (`sec-validatetypedarray`),
/// and its TypedArrayLength: steps 1-3 of each method in this file.
fn validated<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> JsResult<(Local<'s, JsObject>, u64)> {
    let len = validate_typed_array(cx, this)?;
    let o = this.as_object().expect("a TypedArray is an object");
    Ok((o, len))
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

/// SameValueZero ( x, y ) (`sec-samevaluezero`).
fn same_value_zero<'s>(cx: &Cx<'s>, x: Local<'s>, y: Local<'s>) -> bool {
    if let (Some(a), Some(b)) = (x.as_number(), y.as_number()) {
        return a == b || (a.is_nan() && b.is_nan());
    }
    cx.same_value(x, y)
}

/// Appends the code units of `s` to `r`; a RangeError once `r` passes the string length limit
/// (`Cx::new_string_utf16` throws it), before `r` grows further.
fn push_string<'s>(cx: &mut Cx<'s>, r: &mut Vec<u16>, s: Local<'s, JsString>) -> JsResult<()> {
    if r.len() + cx.string_len(s) > MAX_LENGTH {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    let s = cx.flatten(s)?;
    let mut i = 0;
    while let Some(u) = cx.code_unit_at(s, i) {
        r.push(u);
        i += 1;
    }
    Ok(())
}

/// Appends the separator `sep` to `r` (the length limit as in [`push_string`]).
fn push_units<'s>(cx: &mut Cx<'s>, r: &mut Vec<u16>, sep: &[u16]) -> JsResult<()> {
    if r.len() + sep.len() > MAX_LENGTH {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    r.extend_from_slice(sep);
    Ok(())
}

/// The steps of `entries`, `keys` and `values`: ValidateTypedArray, then
/// CreateArrayIterator(O, kind) (`sec-createarrayiterator`).
fn array_iterator<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    kind: ArrayIterKind,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. Perform ? ValidateTypedArray(O, seq-cst).
    let (o, _) = validated(cx, this)?;
    // 3. Return CreateArrayIterator(O, kind).
    Ok(cx.create_array_iterator(o, kind).value())
}

/// `%TypedArray%.prototype.entries ( )` (`sec-%typedarray%.prototype.entries`).
pub(super) fn entries<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    array_iterator(cx, this, ArrayIterKind::Entries)
}

/// `%TypedArray%.prototype.keys ( )` (`sec-%typedarray%.prototype.keys`).
pub(super) fn keys<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    array_iterator(cx, this, ArrayIterKind::Keys)
}

/// `%TypedArray%.prototype.values ( )` (`sec-%typedarray%.prototype.values`); also
/// `%TypedArray.prototype%[@@iterator]` (the same function object).
pub(super) fn values<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    array_iterator(cx, this, ArrayIterKind::Values)
}

/// `%TypedArray%.prototype.at ( index )` (`sec-%typedarray%.prototype.at`).
pub(super) fn at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. Let relativeIndex be ? ToIntegerOrInfinity(index).
    let relative = cx.to_integer_or_infinity(args.get(0))?;
    // 5. If relativeIndex ≥ 0, let k be relativeIndex.
    // 6. Else, let k be len + relativeIndex.
    let k = if relative >= 0.0 {
        relative
    } else {
        len as f64 + relative
    };
    // 7. If k < 0 or k ≥ len, return undefined.
    if k < 0.0 || k >= len as f64 {
        return Ok(Local::undefined());
    }
    // 8. Return ! Get(O, ! ToString(𝔽(k))). (Undefined if a `valueOf` of the index shrank or
    //    detached the buffer.)
    cx.get_index(o, k as u64)
}

/// The steps of `every` (`want` false) and `some` (`want` true): the first index whose
/// callback result has ToBoolean `want` ends the loop with `want`; else `!want`.
fn every_or_some<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    want: bool,
    method: &str,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), method)?;
    let this_arg = args.get(1);
    // 5. Let k be 0.
    // 6. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kValue be ! Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // c. Let testResult be ToBoolean(? Call(callbackfn, thisArg, « kValue, 𝔽(k), O »)).
        let r = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
        // d. (every) If testResult is false, return false.
        //    (some) If testResult is true, return true.
        if cx.to_boolean(r) == want {
            return Ok(Local::boolean(want));
        }
        // e. Set k to k + 1.
    }
    // 7. (every) Return true. (some) Return false.
    Ok(Local::boolean(!want))
}

/// `%TypedArray%.prototype.every ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.every`).
pub(super) fn every<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    every_or_some(cx, this, args, false, "every")
}

/// `%TypedArray%.prototype.some ( callback [ , thisArg ] )` (`sec-%typedarray%.prototype.some`).
pub(super) fn some<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    every_or_some(cx, this, args, true, "some")
}

/// FindViaPredicate ( O, len, direction, predicate, thisArg ) (`sec-findviapredicate`) after
/// steps 1-3 of `find`, `findIndex`, `findLast` and `findLastIndex`: the index and the value of
/// the first element (in the direction) whose predicate result is true, else (-1, undefined).
fn find_via_predicate<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    ascending: bool,
    method: &str,
) -> JsResult<(f64, Local<'s>)> {
    // (find) 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 1. If IsCallable(predicate) is false, throw a TypeError exception.
    let predicate = callback(cx, args.get(0), method)?;
    let this_arg = args.get(1);
    // 2-3. The indices from 0 to len - 1 (ascending), or from len - 1 to 0 (descending).
    for i in 0..len {
        cx.poll_interrupt()?;
        let k = if ascending { i } else { len - 1 - i };
        // 4.a. Let Pk be ! ToString(𝔽(k)).
        // 4.b. NOTE: If O is a TypedArray, the following invocation of Get will return a
        //      normal completion.
        // 4.c. Let kValue be ? Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // 4.d. Let testResult be ? Call(predicate, thisArg, « kValue, 𝔽(k), O »).
        let r = cx.call(predicate, this_arg, &[k_value, number(k), o.value()])?;
        // 4.e. If ToBoolean(testResult) is true, return the Record { [[Index]]: 𝔽(k),
        //      [[Value]]: kValue }.
        if cx.to_boolean(r) {
            return Ok((k as f64, k_value));
        }
    }
    // 5. Return the Record { [[Index]]: -1𝔽, [[Value]]: undefined }.
    Ok((-1.0, Local::undefined()))
}

/// `%TypedArray%.prototype.find ( predicate [ , thisArg ] )` (`sec-%typedarray%.prototype.find`).
pub(super) fn find<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Let findRec be ? FindViaPredicate(O, len, ascending, predicate, thisArg).
    // 5. Return findRec.[[Value]].
    Ok(find_via_predicate(cx, this, args, true, "find")?.1)
}

/// `%TypedArray%.prototype.findIndex ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findindex`).
pub(super) fn find_index<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Let findRec be ? FindViaPredicate(O, len, ascending, predicate, thisArg).
    // 5. Return findRec.[[Index]].
    Ok(Local::number(
        find_via_predicate(cx, this, args, true, "findIndex")?.0,
    ))
}

/// `%TypedArray%.prototype.findLast ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findlast`).
pub(super) fn find_last<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Let findRec be ? FindViaPredicate(O, len, descending, predicate, thisArg).
    // 5. Return findRec.[[Value]].
    Ok(find_via_predicate(cx, this, args, false, "findLast")?.1)
}

/// `%TypedArray%.prototype.findLastIndex ( predicate [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.findlastindex`).
pub(super) fn find_last_index<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 4. Let findRec be ? FindViaPredicate(O, len, descending, predicate, thisArg).
    // 5. Return findRec.[[Index]].
    Ok(Local::number(
        find_via_predicate(cx, this, args, false, "findLastIndex")?.0,
    ))
}

/// `%TypedArray%.prototype.forEach ( callback [ , thisArg ] )`
/// (`sec-%typedarray%.prototype.foreach`).
pub(super) fn for_each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), "forEach")?;
    let this_arg = args.get(1);
    // 5. Let k be 0.
    // 6. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kValue be ! Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // c. Perform ? Call(callbackfn, thisArg, « kValue, 𝔽(k), O »).
        cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
        // d. Set k to k + 1.
    }
    // 7. Return undefined.
    Ok(Local::undefined())
}

/// `%TypedArray%.prototype.includes ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.includes`).
pub(super) fn includes<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If len = 0, return false.
    if len == 0 {
        return Ok(Local::boolean(false));
    }
    // 5. Let n be ? ToIntegerOrInfinity(fromIndex).
    // 6. Assert: If fromIndex is undefined, then n is 0.
    let n = cx.to_integer_or_infinity(args.get(1))?;
    // 7. If n = +∞, return false.
    if n == f64::INFINITY {
        return Ok(Local::boolean(false));
    }
    // 8. Else if n = -∞, set n to 0.
    // 9. If n ≥ 0, let k be n.
    // 10. Else, let k be len + n; if k < 0, set k to 0.
    let k = if n >= 0.0 {
        n
    } else {
        (len as f64 + n).max(0.0)
    };
    // 11. Repeat, while k < len, (the len of step 3: no re-read after a `valueOf` of fromIndex)
    let mut k = k.min(len as f64) as u64;
    while k < len {
        cx.poll_interrupt()?;
        // a. Let elementK be ! Get(O, ! ToString(𝔽(k))). (Undefined past a shrink.)
        let element_k = cx.get_index(o, k)?;
        // b. If SameValueZero(searchElement, elementK) is true, return true.
        if same_value_zero(cx, search_element, element_k) {
            return Ok(Local::boolean(true));
        }
        // c. Set k to k + 1.
        k += 1;
    }
    // 12. Return false.
    Ok(Local::boolean(false))
}

/// `%TypedArray%.prototype.indexOf ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.indexof`).
pub(super) fn index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If len = 0, return -1𝔽.
    if len == 0 {
        return Ok(Local::int32(-1));
    }
    // 5. Let n be ? ToIntegerOrInfinity(fromIndex).
    // 6. Assert: If fromIndex is undefined, then n is 0.
    let n = cx.to_integer_or_infinity(args.get(1))?;
    // 7. If n = +∞, return -1𝔽.
    if n == f64::INFINITY {
        return Ok(Local::int32(-1));
    }
    // 8. Else if n = -∞, set n to 0.
    // 9. If n ≥ 0, let k be n.
    // 10. Else, let k be len + n; if k < 0, set k to 0.
    let k = if n >= 0.0 {
        n
    } else {
        (len as f64 + n).max(0.0)
    };
    // 11. Repeat, while k < len,
    let mut k = k.min(len as f64) as u64;
    while k < len {
        cx.poll_interrupt()?;
        // a. Let kPresent be ! HasProperty(O, ! ToString(𝔽(k))). (False past a shrink.)
        // b. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let elementK be ! Get(O, ! ToString(𝔽(k))).
            let element_k = cx.get_index(o, k)?;
            // ii. If IsStrictlyEqual(searchElement, elementK) is true, return 𝔽(k).
            if cx.is_strictly_equal(search_element, element_k) {
                return Ok(number(k));
            }
        }
        // c. Set k to k + 1.
        k += 1;
    }
    // 12. Return -1𝔽.
    Ok(Local::int32(-1))
}

/// `%TypedArray%.prototype.lastIndexOf ( searchElement [ , fromIndex ] )`
/// (`sec-%typedarray%.prototype.lastindexof`).
pub(super) fn last_index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If len = 0, return -1𝔽.
    if len == 0 {
        return Ok(Local::int32(-1));
    }
    // 5. If fromIndex is present, let n be ? ToIntegerOrInfinity(fromIndex); else let n be
    //    len - 1.
    let n = if args.len() > 1 {
        cx.to_integer_or_infinity(args.get(1))?
    } else {
        (len - 1) as f64
    };
    // 6. If n = -∞, return -1𝔽.
    if n == f64::NEG_INFINITY {
        return Ok(Local::int32(-1));
    }
    // 7. If n ≥ 0, then let k be min(n, len - 1).
    // 8. Else, let k be len + n.
    let k = if n >= 0.0 {
        n.min((len - 1) as f64)
    } else {
        len as f64 + n
    };
    // 9. Repeat, while k ≥ 0,
    if k < 0.0 {
        return Ok(Local::int32(-1));
    }
    let mut k = k as u64;
    loop {
        cx.poll_interrupt()?;
        // a. Let kPresent be ! HasProperty(O, ! ToString(𝔽(k))). (False past a shrink.)
        // b. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let elementK be ! Get(O, ! ToString(𝔽(k))).
            let element_k = cx.get_index(o, k)?;
            // ii. If IsStrictlyEqual(searchElement, elementK) is true, return 𝔽(k).
            if cx.is_strictly_equal(search_element, element_k) {
                return Ok(number(k));
            }
        }
        // c. Set k to k - 1.
        if k == 0 {
            break;
        }
        k -= 1;
    }
    // 10. Return -1𝔽.
    Ok(Local::int32(-1))
}

/// `%TypedArray%.prototype.join ( separator )` (`sec-%typedarray%.prototype.join`).
pub(super) fn join<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If separator is undefined, let sep be ",".
    // 5. Else, let sep be ? ToString(separator).
    let separator = args.get(0);
    let sep: Vec<u16> = if separator.is_undefined() {
        vec![u16::from(b',')]
    } else {
        let s = cx.to_string(separator)?;
        let mut units = Vec::new();
        push_string(cx, &mut units, s)?;
        units
    };
    // 6. Let R be the empty String. (Code units; the string is made once, at the end.)
    let mut r: Vec<u16> = Vec::new();
    // 7. Let k be 0.
    // 8. Repeat, while k < len, (the len of step 3: a shrink in ToString(separator) makes
    //    the later elements undefined)
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. If k > 0, set R to the string-concatenation of R and sep.
        if k > 0 {
            push_units(cx, &mut r, &sep)?;
        }
        // b. Let element be ! Get(O, ! ToString(𝔽(k))).
        let element = cx.get_index(o, k)?;
        // c. If element is undefined, let next be the empty String; otherwise, let next be
        //    ! ToString(element). (A Number or a BigInt: no user code runs.)
        if !element.is_undefined() {
            let next = cx.to_string(element)?;
            // d. Set R to the string-concatenation of R and next.
            push_string(cx, &mut r, next)?;
        }
        // e. Set k to k + 1.
    }
    // 9. Return R.
    Ok(cx.new_string_utf16(&r)?.value())
}

/// The steps of `reduce` (`ascending` true) and `reduceRight`.
fn reduce_in<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    ascending: bool,
    method: &str,
) -> JsResult<Local<'s>> {
    // 1-3. O, ValidateTypedArray, len.
    let (o, len) = validated(cx, this)?;
    // 4. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), method)?;
    // 5. If len = 0 and initialValue is not present, throw a TypeError exception.
    let has_initial = args.len() > 1;
    if len == 0 && !has_initial {
        let msg = format!("TypedArray.prototype.{method} of an empty array with no initial value");
        return Err(cx.throw_type_error(&msg));
    }
    // 6. Let k be 0 (reduce) or len - 1 (reduceRight); here `i` counts the steps.
    // 7. Let accumulator be undefined.
    // 8. If initialValue is present, set accumulator to initialValue.
    // 9. Else, set accumulator to ! Get(O, Pk) of the first index, and step k once.
    let mut i = 0;
    let index = |i: u64| if ascending { i } else { len - 1 - i };
    let mut accumulator = if has_initial {
        args.get(1)
    } else {
        i = 1;
        cx.get_index(o, index(0))?
    };
    // 10. Repeat, while k < len (reduce) or k ≥ 0 (reduceRight),
    while i < len {
        cx.poll_interrupt()?;
        let k = index(i);
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kValue be ! Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // c. Set accumulator to ? Call(callbackfn, undefined, « accumulator, kValue, 𝔽(k),
        //    O »).
        accumulator = cx.call(
            f,
            Local::undefined(),
            &[accumulator, k_value, number(k), o.value()],
        )?;
        // d. Set k to k + 1 (reduce) or k - 1 (reduceRight).
        i += 1;
    }
    // 11. Return accumulator.
    Ok(accumulator)
}

/// `%TypedArray%.prototype.reduce ( callback [ , initialValue ] )`
/// (`sec-%typedarray%.prototype.reduce`).
pub(super) fn reduce<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    reduce_in(cx, this, args, true, "reduce")
}

/// `%TypedArray%.prototype.reduceRight ( callback [ , initialValue ] )`
/// (`sec-%typedarray%.prototype.reduceright`).
pub(super) fn reduce_right<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    reduce_in(cx, this, args, false, "reduceRight")
}

/// `%TypedArray%.prototype.toString ( )` (`sec-%typedarray%.prototype.tostring`). The spec
/// makes it the same function object as `Array.prototype.toString`; here it is a function of
/// its own (route TA-TOSTRING-IDENTITY) that runs the same steps (`sec-array.prototype.tostring`)
/// through `Cx::array_prototype_to_string`: a `join` that is not callable gives the intrinsic
/// `%Object.prototype.toString%`, not the live property.
pub(super) fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.array_prototype_to_string(this, args, new_target)
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
    let (array, len) = validated(cx, this)?;
    // 4. Let separator be the implementation-defined list-separator String appropriate for
    //    the host environment's current locale (such as ", "). ("," as V8.)
    let separator = [u16::from(b',')];
    // 5. Let R be the empty String. (Code units; the string is made once, at the end.)
    let mut r: Vec<u16> = Vec::new();
    // 6. Let k be 0.
    // 7. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. If k > 0, set R to the string-concatenation of R and separator.
        if k > 0 {
            push_units(cx, &mut r, &separator)?;
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
            // ii. Set R to the string-concatenation of R and S.
            push_string(cx, &mut r, s)?;
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
