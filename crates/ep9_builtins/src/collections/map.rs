//! Map (§24.1): the constructor, `Map.groupBy`, `Map.prototype`, `%MapIteratorPrototype%.next`
//! and the upsert methods (`getOrInsert`, `getOrInsertComputed`; proposal-upsert).

use ep9_vm::abi::{JsObject, ObjectClass};
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local};

use super::{Adder, add_from_iterable, iter_result, number, requires_new};

const MAP: ObjectClass = ObjectClass::Map;

/// The largest index GroupBy takes (2^53 - 1).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Map ( [ iterable ] ) (§24.1.1.1).
pub(super) fn map_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "Map"));
    };
    // 2. Let map be ? OrdinaryCreateFromConstructor(NewTarget, "%Map.prototype%",
    //    « [[MapData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::MapPrototype)?;
    // 3. Set map.[[MapData]] to a new empty List.
    let map = cx.new_collection(proto, MAP);
    // 4-7. If iterable is undefined or null, return map; else let adder be ? Get(map, "set");
    //      it must be callable; return ? AddEntriesFromIterable(map, iterable, adder).
    add_from_iterable(cx, map, args.get(0), "set", Adder::Entries)?;
    Ok(map.value())
}

/// Map.groupBy ( items, callback ) (§24.1.2.1), with GroupBy (§7.3.35, zero keys) and
/// AddValueToKeyedGroup (§7.3.34). The groups are the result Map itself: each key gets its
/// array when its first value comes, so the entries are in the order of the groups list.
/// Neither the Map nor the arrays are visible to `callback`.
pub(super) fn group_by<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (items, callback) = (args.get(0), args.get(1));
    // GroupBy 1. Perform ? RequireObjectCoercible(items).
    if items.is_undefined_or_null() {
        return Err(cx.throw_type_error("Map.groupBy: items is undefined or null"));
    }
    // GroupBy 2. If IsCallable(callback) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error("Map.groupBy: the callback is not a function"));
    }
    // 2. Let map be ! Construct(%Map%). (Made first: it holds the groups.)
    let proto = cx.intrinsic(IntrinsicId::MapPrototype);
    let map = cx.new_collection(proto, MAP);
    // GroupBy 4. Let iteratorRecord be ? GetIterator(items, sync).
    let mut record = cx.get_iterator(items)?;
    // GroupBy 5. Let k be 0.
    let mut k: u64 = 0;
    // GroupBy 6. Repeat,
    loop {
        // a. If k ≥ 2^53 - 1, then let error be ThrowCompletion(a TypeError); return
        //    ? IteratorClose(iteratorRecord, error).
        if k >= MAX_SAFE_INTEGER {
            let error = Err(cx.throw_type_error("Map.groupBy: too many items"));
            return cx.iterator_close(&record, error);
        }
        // b-c. Let next be ? IteratorStepValue(iteratorRecord); if done, return groups.
        let Some(value) = cx.iterator_step_value(&mut record)? else {
            break;
        };
        // e-i. The key (Completion(Call(callback, undefined, « value, 𝔽(k) »)), closing the
        //      iterator on an abrupt one; -0 becomes +0), then AddValueToKeyedGroup.
        if let Err(e) = group_value(cx, map, callback, value, k) {
            return cx.iterator_close(&record, Err(e));
        }
        // j. Set k to k + 1.
        k += 1;
    }
    // 3. For each Record { [[Key]], [[Elements]] } g of groups: elements =
    //    CreateArrayFromList(g.[[Elements]]); append { g.[[Key]], elements } to
    //    map.[[MapData]]. (Done as the groups grew.)
    // 4. Return map.
    Ok(map.value())
}

/// GroupBy steps 6.e-6.i for one value: the key, then AddValueToKeyedGroup on `map`.
fn group_value<'s>(
    cx: &mut Cx<'s>,
    map: Local<'s, JsObject>,
    callback: Local<'s>,
    value: Local<'s>,
    k: u64,
) -> JsResult<()> {
    let key = cx.call(
        callback,
        Local::undefined(),
        &[value, Local::number(k as f64)],
    )?;
    // AddValueToKeyedGroup 1. For each Record g of groups: if SameValue(g.[[Key]], key),
    //    append value to g.[[Elements]] and return. (-0 was +0: SameValueZero here.)
    if let Some(group) = cx.collection_get(map, MAP, key)?
        && let Some(group) = group.as_object()
    {
        let len = cx.get(group, "length")?;
        let len = cx.to_length(len)?;
        let index = cx.to_property_key(Local::number(len as f64))?;
        return cx.create_data_property_or_throw(group, index, value);
    }
    // 2-3. A new group « value » for key, appended to groups.
    let group = cx.create_array_from_list(&[value])?;
    cx.collection_set(map, MAP, key, group)
}

/// Map.prototype.get ( key ) (§24.1.3.6).
pub(super) fn get<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(M, [[MapData]]). 3-5. The value of the entry of key, or
    //      undefined.
    Ok(cx
        .collection_get(this, MAP, args.get(0))?
        .unwrap_or_else(Local::undefined))
}

/// Map.prototype.set ( key, value ) (§24.1.3.9).
pub(super) fn set<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot. 3. CanonicalizeKeyedCollectionKey(key). 4-5. Set the value
    //      of the entry, or append one.
    cx.collection_set(this, MAP, args.get(0), args.get(1))?;
    // 6. Return M.
    Ok(this)
}

/// Map.prototype.has ( key ) (§24.1.3.7).
pub(super) fn has<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_has(this, MAP, args.get(0))?))
}

/// Map.prototype.delete ( key ) (§24.1.3.3).
pub(super) fn delete<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_delete(
        this,
        MAP,
        args.get(0),
    )?))
}

/// Map.prototype.clear ( ) (§24.1.3.1).
pub(super) fn clear<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.collection_clear(this, MAP)?;
    Ok(Local::undefined())
}

/// Map.prototype.entries ( ) (§24.1.3.4): CreateMapIterator(M, key+value).
pub(super) fn entries<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_collection_iterator(this, MAP, true, true)?.value())
}

/// Map.prototype.keys ( ) (§24.1.3.8): CreateMapIterator(M, key).
pub(super) fn keys<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_collection_iterator(this, MAP, true, false)?.value())
}

/// Map.prototype.values ( ) (§24.1.3.11): CreateMapIterator(M, value).
pub(super) fn values<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_collection_iterator(this, MAP, false, true)?.value())
}

/// Map.prototype.forEach ( callbackfn [ , thisArg ] ) (§24.1.3.5).
pub(super) fn for_each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (callback, this_arg) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(M, [[MapData]]).
    let mut cursor = cx.collection_cursor(this, MAP)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error("Map.prototype.forEach: the callback is not a function"));
    }
    // 4-7. For each entry, in order, with the entries added during the walk: Perform
    //      ? Call(callbackfn, thisArg, « e.[[Value]], e.[[Key]], M »).
    while let Some((key, value)) = cx.collection_next(&mut cursor) {
        cx.call(callback, this_arg, &[value, key, this])?;
        cx.poll_interrupt()?;
    }
    // 8. Return undefined.
    Ok(Local::undefined())
}

/// get Map.prototype.size (§24.1.3.10).
pub(super) fn size<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(number(cx.collection_size(this, MAP)?))
}

/// %MapIteratorPrototype%.next ( ) (§24.1.5.2.1): GeneratorResume(this value, empty,
/// "%MapIteratorPrototype%").
pub(super) fn iterator_next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    match cx.collection_iterator_next(this, ObjectClass::MapIterator)? {
        Some(v) => iter_result(cx, v, false),
        None => iter_result(cx, Local::undefined(), true),
    }
}

/// Map.prototype.getOrInsert ( key, value ) (proposal-upsert).
pub(super) fn get_or_insert<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (key, value) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(M, [[MapData]]). 3. CanonicalizeKeyedCollectionKey(key).
    // 4. For each Record p of M.[[MapData]]: if SameValue(p.[[Key]], key), return
    //    p.[[Value]].
    if let Some(v) = cx.collection_get(this, MAP, key)? {
        return Ok(v);
    }
    // 5. Append { key, value }. 6. Return value.
    cx.collection_set(this, MAP, key, value)?;
    Ok(value)
}

/// Map.prototype.getOrInsertComputed ( key, callbackfn ) (proposal-upsert).
pub(super) fn get_or_insert_computed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (key, callback) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(M, [[MapData]]).
    cx.collection_size(this, MAP)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error(
            "Map.prototype.getOrInsertComputed: the callback is not a function",
        ));
    }
    // 4. Set key to CanonicalizeKeyedCollectionKey(key). 5. An existing entry's value.
    if let Some(v) = cx.collection_get(this, MAP, key)? {
        return Ok(v);
    }
    let key = canonical(key);
    // 6. Let value be ? Call(callbackfn, undefined, « key »).
    let value = cx.call(callback, Local::undefined(), &[key])?;
    // 7-8. Set the entry of key (the callback may have added it), or append it.
    cx.collection_set(this, MAP, key, value)?;
    // 9. Return value.
    Ok(value)
}

/// CanonicalizeKeyedCollectionKey ( key ) (§24.5.1): -0 becomes +0.
pub(super) fn canonical(key: Local<'_>) -> Local<'_> {
    if key.as_number().is_some_and(|n| n == 0.0) {
        Local::number(0.0)
    } else {
        key
    }
}
