//! WeakMap (§24.3) and WeakSet (§24.4). A key must satisfy CanBeHeldWeakly (§9.13): an
//! object, or a symbol that `Symbol.for` did not make. The table holds its keys weakly
//! (`ordered_table.rs`): an entry whose key dies goes away at a GC.

use ep9_vm::abi::ObjectClass;
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local};

use super::{Adder, add_from_iterable, requires_new};

const WEAK_MAP: ObjectClass = ObjectClass::WeakMap;
const WEAK_SET: ObjectClass = ObjectClass::WeakSet;

/// RequireInternalSlot(o, …) (§10.1.15) for a weak collection of `class`, before a step that
/// does not read the table: a TypeError for any other value.
fn require<'s>(cx: &mut Cx<'s>, o: Local<'s>, class: ObjectClass) -> JsResult<()> {
    if cx.object_class(o) == Some(class) {
        return Ok(());
    }
    // `collection_has` gives the RequireInternalSlot TypeError.
    cx.collection_has(o, class, Local::undefined()).map(|_| ())
}

/// WeakMap ( \[ iterable \] ) (§24.3.1.1).
pub(super) fn weak_map_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "WeakMap"));
    };
    // 2. Let map be ? OrdinaryCreateFromConstructor(NewTarget, "%WeakMap.prototype%",
    //    « [[WeakMapData]] »). 3. Set map.[[WeakMapData]] to a new empty List.
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::WeakMapPrototype)?;
    let map = cx.new_collection(proto, WEAK_MAP);
    // 4-7. As Map: adder = ? Get(map, "set"); AddEntriesFromIterable.
    add_from_iterable(cx, map, args.get(0), "set", Adder::Entries)?;
    Ok(map.value())
}

/// WeakMap.prototype.delete ( key ) (§24.3.3.2).
pub(super) fn weak_map_delete<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot. 3. If CanBeHeldWeakly(key) is false, return false (such a
    //      key is never in the table). 4-5. Remove the entry.
    Ok(Local::boolean(cx.collection_delete(
        this,
        WEAK_MAP,
        args.get(0),
    )?))
}

/// WeakMap.prototype.get ( key ) (§24.3.3.3).
pub(super) fn weak_map_get<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx
        .collection_get(this, WEAK_MAP, args.get(0))?
        .unwrap_or_else(Local::undefined))
}

/// WeakMap.prototype.has ( key ) (§24.3.3.4).
pub(super) fn weak_map_has<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_has(
        this,
        WEAK_MAP,
        args.get(0),
    )?))
}

/// WeakMap.prototype.set ( key, value ) (§24.3.3.5).
pub(super) fn weak_map_set<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let key = args.get(0);
    // 1-2. RequireInternalSlot(M, [[WeakMapData]]).
    require(cx, this, WEAK_MAP)?;
    // 3. If CanBeHeldWeakly(key) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(key) {
        return Err(cx.throw_type_error("Invalid value used as weak map key"));
    }
    // 4-5. Set the value of the entry, or append one.
    cx.collection_set(this, WEAK_MAP, key, args.get(1))?;
    // 6. Return M.
    Ok(this)
}

/// WeakMap.prototype.getOrInsert ( key, value ) (proposal-upsert).
pub(super) fn weak_map_get_or_insert<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (key, value) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(M, [[WeakMapData]]).
    require(cx, this, WEAK_MAP)?;
    // 3. If CanBeHeldWeakly(key) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(key) {
        return Err(cx.throw_type_error("Invalid value used as weak map key"));
    }
    // 4. An existing entry's value.
    if let Some(v) = cx.collection_get(this, WEAK_MAP, key)? {
        return Ok(v);
    }
    // 5-6. Append { key, value }; return value.
    cx.collection_set(this, WEAK_MAP, key, value)?;
    Ok(value)
}

/// WeakMap.prototype.getOrInsertComputed ( key, callbackfn ) (proposal-upsert).
pub(super) fn weak_map_get_or_insert_computed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (key, callback) = (args.get(0), args.get(1));
    // 1-2. RequireInternalSlot(M, [[WeakMapData]]).
    require(cx, this, WEAK_MAP)?;
    // 3. If CanBeHeldWeakly(key) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(key) {
        return Err(cx.throw_type_error("Invalid value used as weak map key"));
    }
    // 4. If IsCallable(callbackfn) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error(
            "WeakMap.prototype.getOrInsertComputed: the callback is not a function",
        ));
    }
    // 5. An existing entry's value.
    if let Some(v) = cx.collection_get(this, WEAK_MAP, key)? {
        return Ok(v);
    }
    // 6. Let value be ? Call(callbackfn, undefined, « key »).
    let value = cx.call(callback, Local::undefined(), &[key])?;
    // 7-8. Set the entry of key (the callback may have added it), or append it.
    cx.collection_set(this, WEAK_MAP, key, value)?;
    // 9. Return value.
    Ok(value)
}

/// WeakSet ( \[ iterable \] ) (§24.4.1.1).
pub(super) fn weak_set_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "WeakSet"));
    };
    // 2-3. OrdinaryCreateFromConstructor(NewTarget, "%WeakSet.prototype%",
    //      « [[WeakSetData]] »), with an empty List.
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::WeakSetPrototype)?;
    let set = cx.new_collection(proto, WEAK_SET);
    // 4-8. As Set: adder = ? Get(set, "add"); each value goes to it.
    add_from_iterable(cx, set, args.get(0), "add", Adder::Values)?;
    Ok(set.value())
}

/// WeakSet.prototype.add ( value ) (§24.4.3.1).
pub(super) fn weak_set_add<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let value = args.get(0);
    // 1-2. RequireInternalSlot(S, [[WeakSetData]]).
    require(cx, this, WEAK_SET)?;
    // 3. If CanBeHeldWeakly(value) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(value) {
        return Err(cx.throw_type_error("Invalid value used in weak set"));
    }
    // 4-5. Append value if it is not there.
    cx.collection_set(this, WEAK_SET, value, Local::undefined())?;
    // 6. Return S.
    Ok(this)
}

/// WeakSet.prototype.delete ( value ) (§24.4.3.3).
pub(super) fn weak_set_delete<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_delete(
        this,
        WEAK_SET,
        args.get(0),
    )?))
}

/// WeakSet.prototype.has ( value ) (§24.4.3.4).
pub(super) fn weak_set_has<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::boolean(cx.collection_has(
        this,
        WEAK_SET,
        args.get(0),
    )?))
}
