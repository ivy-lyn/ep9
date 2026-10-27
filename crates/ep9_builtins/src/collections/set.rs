//! Set (§24.2): the constructor, `Set.prototype`, `%SetIteratorPrototype%.next` and the Set
//! methods (union, intersection, difference, symmetricDifference, isSubsetOf, isSupersetOf,
//! isDisjointFrom) over GetSetRecord (§24.2.1.2).
//!
//! The spec walks `O.[[SetData]]` by index and re-reads its length after each call into JS
//! (`thisSize`), so it sees elements added during the walk and skips deleted ones: that is
//! the collection cursor (`Cx::collection_cursor`). A result Set is a new Set with
//! `%Set.prototype%` (no @@species), and the walk over a copy (`difference`) runs over the
//! result itself, which no JS code can reach.

use ep9_vm::abi::{JsObject, ObjectClass};
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local};

use super::{Adder, add_from_iterable, close_with, iter_result, number, requires_new};

const SET: ObjectClass = ObjectClass::Set;

/// Set ( \[ iterable \] ) (§24.2.2.1).
pub(super) fn set_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "Set"));
    };
    // 2. Let set be ? OrdinaryCreateFromConstructor(NewTarget, "%Set.prototype%",
    //    « [[SetData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::SetPrototype)?;
    // 3. Set set.[[SetData]] to a new empty List.
    let set = cx.new_collection(proto, SET);
    // 4-8. If iterable is undefined or null, return set; else adder = ? Get(set, "add"),
    //      callable; each value of the iterable: ? Call(adder, set, « next »), closing the
    //      iterator on an abrupt completion.
    add_from_iterable(cx, set, args.get(0), "add", Adder::Values)?;
    Ok(set.value())
}

/// Set.prototype.add ( value ) (§24.2.4.1).
pub(super) fn add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot. 3. CanonicalizeKeyedCollectionKey(value). 4-5. Append it if
    //      it is not there.
    cx.collection_set(this, SET, args.get(0), Local::undefined())?;
    // 6. Return S.
    Ok(this)
}

/// Set.prototype.has ( value ) (§24.2.4.9).
pub(super) fn has<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_has(this, SET, args.get(0))?))
}

/// Set.prototype.delete ( value ) (§24.2.4.4).
pub(super) fn delete<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_delete(
        this,
        SET,
        args.get(0),
    )?))
}

/// Set.prototype.clear ( ) (§24.2.4.2).
pub(super) fn clear<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.collection_clear(this, SET)?;
    Ok(Local::undefined())
}

/// Set.prototype.entries ( ) (§24.2.4.6): CreateSetIterator(S, key+value).
pub(super) fn entries<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_collection_iterator(this, SET, true, true)?.value())
}

/// Set.prototype.values ( ) (§24.2.4.17), also `keys` and `@@iterator`:
/// CreateSetIterator(S, value).
pub(super) fn values<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_collection_iterator(this, SET, false, true)?.value())
}

/// Set.prototype.forEach ( callbackfn [ , thisArg ] ) (§24.2.4.8).
pub(super) fn for_each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (callback, this_arg) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(S, [[SetData]]).
    let mut cursor = cx.collection_cursor(this, SET)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error("Set.prototype.forEach: the callback is not a function"));
    }
    // 4-7. For each element e, in order, with the elements added during the walk: Perform
    //      ? Call(callbackfn, thisArg, « e, e, S »).
    while let Some((e, _)) = cx.collection_next(&mut cursor) {
        cx.call(callback, this_arg, &[e, e, this])?;
        cx.poll_interrupt()?;
    }
    // 8. Return undefined.
    Ok(Local::undefined())
}

/// get Set.prototype.size (§24.2.4.14).
pub(super) fn size<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(number(cx.collection_size(this, SET)?))
}

/// %SetIteratorPrototype%.next ( ) (§24.2.6.2.1): GeneratorResume(this value, empty,
/// "%SetIteratorPrototype%").
pub(super) fn iterator_next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    match cx.collection_iterator_next(this, ObjectClass::SetIterator)? {
        Some(v) => iter_result(cx, v, false),
        None => iter_result(cx, Local::undefined(), true),
    }
}

// ---- the Set methods -------------------------------------------------------------------------

/// A Set Record (§24.2.1.1): `[[SetObject]]`, `[[Size]]` (an integer or +∞), `[[Has]]`,
/// `[[Keys]]`. Stack-only (holds Locals).
struct SetRecord<'s> {
    object: Local<'s, JsObject>,
    size: f64,
    has: Local<'s>,
    keys: Local<'s>,
}

/// GetSetRecord ( obj ) (§24.2.1.2).
fn get_set_record<'s>(cx: &mut Cx<'s>, obj: Local<'s>) -> JsResult<SetRecord<'s>> {
    // 1. If obj is not an Object, throw a TypeError exception.
    let Some(object) = obj.as_object() else {
        return Err(cx.throw_type_error("The other argument of a Set method must be an object"));
    };
    // 2. Let rawSize be ? Get(obj, "size").
    let raw_size = cx.get(object, "size")?;
    // 3. Let numSize be ? ToNumber(rawSize). (undefined gives NaN.)
    let num_size = cx.to_number(raw_size)?;
    // 4. If numSize is NaN, throw a TypeError exception.
    if num_size.is_nan() {
        return Err(cx.throw_type_error("The 'size' of a set-like object is not a number"));
    }
    // 5. Let intSize be ! ToIntegerOrInfinity(numSize).
    let size = if num_size.is_infinite() {
        num_size
    } else {
        num_size.trunc() + 0.0
    };
    // 6. If intSize < 0, throw a RangeError exception.
    if size < 0.0 {
        return Err(cx.throw_range_error("The 'size' of a set-like object is negative"));
    }
    // 7. Let has be ? Get(obj, "has").
    let has = cx.get(object, "has")?;
    // 8. If IsCallable(has) is false, throw a TypeError exception.
    if !cx.is_callable(has) {
        return Err(cx.throw_type_error("The 'has' of a set-like object is not a function"));
    }
    // 9. Let keys be ? Get(obj, "keys").
    let keys = cx.get(object, "keys")?;
    // 10. If IsCallable(keys) is false, throw a TypeError exception.
    if !cx.is_callable(keys) {
        return Err(cx.throw_type_error("The 'keys' of a set-like object is not a function"));
    }
    // 11. Return a new Set Record.
    Ok(SetRecord {
        object,
        size,
        has,
        keys,
    })
}

/// ToBoolean(? Call(otherRec.\[\[Has\]\], otherRec.\[\[SetObject\]\], « e »)).
fn other_has<'s>(cx: &mut Cx<'s>, rec: &SetRecord<'s>, e: Local<'s>) -> JsResult<bool> {
    let r = cx.call(rec.has, rec.object, &[e])?;
    Ok(cx.to_boolean(r))
}

/// A new empty Set with `%Set.prototype%` (OrdinaryObjectCreate(%Set.prototype%,
/// « \[\[SetData\]\] »)).
fn new_set<'s>(cx: &mut Cx<'s>) -> Local<'s, JsObject> {
    let proto = cx.intrinsic(IntrinsicId::SetPrototype);
    cx.new_collection(proto, SET)
}

/// A new Set with `%Set.prototype%` and a copy of the elements of the Set `o`, in order.
fn copy_set<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<Local<'s, JsObject>> {
    let result = new_set(cx);
    let mut cursor = cx.collection_cursor(o, SET)?;
    while let Some((e, _)) = cx.collection_next(&mut cursor) {
        cx.collection_set(result, SET, e, Local::undefined())?;
    }
    Ok(result)
}

/// Set.prototype.union ( other ) (§24.2.4.18).
pub(super) fn union<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]], otherRec.[[Keys]]).
    let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
    // 5. Let resultSetData be a copy of O.[[SetData]].
    let result = copy_set(cx, this)?;
    // 6-7. For each value next of keysIter: CanonicalizeKeyedCollectionKey(next); append it
    //      if it is not in resultSetData.
    while let Some(next) = cx.iterator_step_value(&mut keys)? {
        if let Err(e) = cx.collection_set(result, SET, next, Local::undefined()) {
            return cx.iterator_close(&keys, Err(e));
        }
    }
    // 8-10. Return result.
    Ok(result.value())
}

/// Set.prototype.intersection ( other ) (§24.2.4.9).
pub(super) fn intersection<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. Let resultSetData be a new empty List. (The result Set holds it.)
    let result = new_set(cx);
    // 5. If SetDataSize(O.[[SetData]]) ≤ otherRec.[[Size]], then
    if cx.collection_size(this, SET)? as f64 <= rec.size {
        // a-c. For each element e of O.[[SetData]], in order, with the elements added
        //      during the walk:
        let mut cursor = cx.collection_cursor(this, SET)?;
        while let Some((e, _)) = cx.collection_next(&mut cursor) {
            // 1. Let inOther be ToBoolean(? Call(otherRec.[[Has]], otherRec.[[SetObject]],
            //    « e »)).
            // 2. If inOther is true, append e to resultSetData if it is not there.
            if other_has(cx, &rec, e)? {
                cx.collection_set(result, SET, e, Local::undefined())?;
            }
            cx.poll_interrupt()?;
        }
    } else {
        // 6. Else, a. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]],
        //    otherRec.[[Keys]]).
        let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
        // b-c. For each value next: canonicalize it; if SetDataHas(O.[[SetData]], next),
        //      append it to resultSetData if it is not there.
        while let Some(next) = cx.iterator_step_value(&mut keys)? {
            let step = match cx.collection_has(this, SET, next) {
                Ok(true) => cx.collection_set(result, SET, next, Local::undefined()),
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            };
            if let Err(e) = step {
                return cx.iterator_close(&keys, Err(e));
            }
        }
    }
    // 7-9. Return result.
    Ok(result.value())
}

/// Set.prototype.difference ( other ) (§24.2.4.5).
pub(super) fn difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. Let resultSetData be a copy of O.[[SetData]].
    let result = copy_set(cx, this)?;
    // 5. If SetDataSize(O.[[SetData]]) ≤ otherRec.[[Size]], then
    if cx.collection_size(this, SET)? as f64 <= rec.size {
        // a-c. For each element e of resultSetData (a copy no JS code can reach): if
        //      ToBoolean(? Call(otherRec.[[Has]], otherRec.[[SetObject]], « e »)), set it to
        //      empty.
        let mut cursor = cx.collection_cursor(result, SET)?;
        while let Some((e, _)) = cx.collection_next(&mut cursor) {
            if other_has(cx, &rec, e)? {
                cx.collection_delete(result, SET, e)?;
            }
            cx.poll_interrupt()?;
        }
    } else {
        // 6. Else, a. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]],
        //    otherRec.[[Keys]]).
        let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
        // b-c. For each value next: canonicalize it; remove it from resultSetData.
        while let Some(next) = cx.iterator_step_value(&mut keys)? {
            if let Err(e) = cx.collection_delete(result, SET, next) {
                return cx.iterator_close(&keys, Err(e));
            }
        }
    }
    // 7-9. Return result.
    Ok(result.value())
}

/// Set.prototype.symmetricDifference ( other ) (§24.2.4.16).
pub(super) fn symmetric_difference<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]], otherRec.[[Keys]]).
    let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
    // 5. Let resultSetData be a copy of O.[[SetData]].
    let result = copy_set(cx, this)?;
    // 6-7. For each value next of keysIter:
    while let Some(next) = cx.iterator_step_value(&mut keys)? {
        if let Err(e) = symmetric_step(cx, this, result, next) {
            return cx.iterator_close(&keys, Err(e));
        }
    }
    // 8-10. Return result.
    Ok(result.value())
}

/// symmetricDifference step 7.b for one value `next` of the other set.
fn symmetric_step<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    result: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<()> {
    // i. Set next to CanonicalizeKeyedCollectionKey(next).
    // ii-iii. alreadyInResult: SetDataIndex(resultSetData, next) is not not-found.
    let already_in_result = cx.collection_has(result, SET, next)?;
    // iv. If SetDataHas(O.[[SetData]], next) is true, then
    if cx.collection_has(this, SET, next)? {
        // 1. If alreadyInResult is true, set resultSetData[resultIndex] to empty.
        if already_in_result {
            cx.collection_delete(result, SET, next)?;
        }
    } else if !already_in_result {
        // v. Else, if alreadyInResult is false, append next to resultSetData.
        cx.collection_set(result, SET, next, Local::undefined())?;
    }
    Ok(())
}

/// Set.prototype.isSubsetOf ( other ) (§24.2.4.11).
pub(super) fn is_subset_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. If SetDataSize(O.[[SetData]]) > otherRec.[[Size]], return false.
    if cx.collection_size(this, SET)? as f64 > rec.size {
        return Ok(Local::boolean(false));
    }
    // 5-7. For each element e of O.[[SetData]], with the elements added during the walk: if
    //      ToBoolean(? Call(otherRec.[[Has]], otherRec.[[SetObject]], « e »)) is false,
    //      return false.
    let mut cursor = cx.collection_cursor(this, SET)?;
    while let Some((e, _)) = cx.collection_next(&mut cursor) {
        if !other_has(cx, &rec, e)? {
            return Ok(Local::boolean(false));
        }
        cx.poll_interrupt()?;
    }
    // 8. Return true.
    Ok(Local::boolean(true))
}

/// Set.prototype.isSupersetOf ( other ) (§24.2.4.12).
pub(super) fn is_superset_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. If SetDataSize(O.[[SetData]]) < otherRec.[[Size]], return false.
    if (cx.collection_size(this, SET)? as f64) < rec.size {
        return Ok(Local::boolean(false));
    }
    // 5. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]], otherRec.[[Keys]]).
    let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
    // 6. Repeat: next = ? IteratorStepValue(keysIter); done: return true; if
    //    SetDataHas(O.[[SetData]], next) is false: ? IteratorClose(keysIter,
    //    NormalCompletion(unused)) and return false.
    while let Some(next) = cx.iterator_step_value(&mut keys)? {
        match cx.collection_has(this, SET, next) {
            Ok(true) => {}
            Ok(false) => return close_with(cx, &keys, Local::boolean(false)),
            Err(e) => return cx.iterator_close(&keys, Err(e)),
        }
    }
    Ok(Local::boolean(true))
}

/// Set.prototype.isDisjointFrom ( other ) (§24.2.4.10).
pub(super) fn is_disjoint_from<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(O, [[SetData]]).
    cx.collection_size(this, SET)?;
    // 3. Let otherRec be ? GetSetRecord(other).
    let rec = get_set_record(cx, args.get(0))?;
    // 4. If SetDataSize(O.[[SetData]]) ≤ otherRec.[[Size]], then
    if cx.collection_size(this, SET)? as f64 <= rec.size {
        // a-c. For each element e of O.[[SetData]], with the elements added during the walk:
        //      if ToBoolean(? Call(otherRec.[[Has]], otherRec.[[SetObject]], « e »)), return
        //      false.
        let mut cursor = cx.collection_cursor(this, SET)?;
        while let Some((e, _)) = cx.collection_next(&mut cursor) {
            if other_has(cx, &rec, e)? {
                return Ok(Local::boolean(false));
            }
            cx.poll_interrupt()?;
        }
    } else {
        // 5. Else, a. Let keysIter be ? GetIteratorFromMethod(otherRec.[[SetObject]],
        //    otherRec.[[Keys]]).
        let mut keys = cx.get_iterator_from_method(rec.object, rec.keys)?;
        // b-c. For each value next: if SetDataHas(O.[[SetData]], next), then
        //      ? IteratorClose(keysIter, NormalCompletion(unused)) and return false.
        while let Some(next) = cx.iterator_step_value(&mut keys)? {
            match cx.collection_has(this, SET, next) {
                Ok(false) => {}
                Ok(true) => return close_with(cx, &keys, Local::boolean(false)),
                Err(e) => return cx.iterator_close(&keys, Err(e)),
            }
        }
    }
    // 6. Return true.
    Ok(Local::boolean(true))
}
