//! Object builtins (§20.1): module `object`, block 1. Targets `%Object%` and `%Object.prototype%`
//! (`IntrinsicId::Object`, `IntrinsicId::ObjectPrototype`).
//!
//! The `Object` constructor and `Object.prototype.toString`/`valueOf` are VM builtins (block 0).
//! Week 4 (P3-BLTA): `Object.prototype.hasOwnProperty`, `propertyIsEnumerable`,
//! `Object.defineProperty`, `getOwnPropertyDescriptor`, `getOwnPropertyNames`, `keys`.
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `hasOwnProperty` | 2 | ToPropertyKey before ToObject(this), then `[[GetOwnProperty]]` |
//! | `defineProperty` | 3 | ToPropertyDescriptor runs user getters, so every step is a GC point; DefinePropertyOrThrow |
//! | `getOwnPropertyDescriptor` | 4 | FromPropertyDescriptor: a new object, filled |
//! | `getOwnPropertyNames` | 5 | a list of values in a `ScratchList`, returned with CreateArrayFromList |
//! | `propertyIsEnumerable`, `keys` | companions | as `hasOwnProperty`; as `getOwnPropertyNames` |
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-object.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsArray, JsObject, JsResult,
    Local, ObjectProps, PropDef, PropKey, ScratchList,
};

use crate::array::{index_key, length_of_array_like};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 1;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const HAS_OWN_PROPERTY: u16 = 0;
    pub const DEFINE_PROPERTY: u16 = 1;
    pub const GET_OWN_PROPERTY_DESCRIPTOR: u16 = 2;
    pub const GET_OWN_PROPERTY_NAMES: u16 = 3;
    pub const PROPERTY_IS_ENUMERABLE: u16 = 4;
    pub const KEYS: u16 = 5;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("hasOwnProperty", 1, has_own_property), // 1.0
    BuiltinDef::method("defineProperty", 3, define_property),  // 1.1
    BuiltinDef::method("getOwnPropertyDescriptor", 2, get_own_property_descriptor), // 1.2
    BuiltinDef::method("getOwnPropertyNames", 1, get_own_property_names), // 1.3
    BuiltinDef::method("propertyIsEnumerable", 1, property_is_enumerable), // 1.4
    BuiltinDef::method("keys", 1, keys),                       // 1.5
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Object,
        props: &[
            PropDef::method(
                PropKey::Str("getOwnPropertyDescriptor"),
                def::GET_OWN_PROPERTY_DESCRIPTOR,
            ),
            PropDef::method(
                PropKey::Str("getOwnPropertyNames"),
                def::GET_OWN_PROPERTY_NAMES,
            ),
            PropDef::method(PropKey::Str("defineProperty"), def::DEFINE_PROPERTY),
            PropDef::method(PropKey::Str("keys"), def::KEYS),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ObjectPrototype,
        props: &[
            PropDef::method(PropKey::Str("hasOwnProperty"), def::HAS_OWN_PROPERTY),
            PropDef::method(
                PropKey::Str("propertyIsEnumerable"),
                def::PROPERTY_IS_ENUMERABLE,
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "object",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Template 2. Object.prototype.hasOwnProperty ( V ) (§20.1.3.2).
fn has_own_property<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let P be ? ToPropertyKey(V). (Before ToObject: a throwing `toString` of V wins over
    //    a TypeError for an undefined `this`.)
    let p = cx.to_property_key(args.get(0))?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Return ? HasOwnProperty(O, P).
    Ok(Local::boolean(cx.has_own_property(o, p)?))
}

/// Object.prototype.propertyIsEnumerable ( V ) (§20.1.3.4).
fn property_is_enumerable<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let P be ? ToPropertyKey(V).
    let p = cx.to_property_key(args.get(0))?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Let desc be ? O.[[GetOwnProperty]](P).
    let desc = cx.get_own_property(o, p)?;
    // 4. If desc is undefined, return false.
    // 5. Return desc.[[Enumerable]].
    Ok(Local::boolean(
        desc.is_some_and(|d| d.enumerable == Some(true)),
    ))
}

/// Template 3. Object.defineProperty ( O, P, Attributes ) (§20.1.2.4).
fn define_property<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If O is not an Object, throw a TypeError exception.
    let Some(o) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("Object.defineProperty called on non-object"));
    };
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? ToPropertyDescriptor(Attributes). It reads the fields with Get, so
    //    user getters run here: `o` and `key` stay live in Rust locals (R1).
    let desc = cx.to_property_descriptor(args.get(2))?;
    // 4. Perform ? DefinePropertyOrThrow(O, key, desc).
    cx.define_property_or_throw(o, key, desc)?;
    // 5. Return O.
    Ok(o.value())
}

/// Template 4. Object.getOwnPropertyDescriptor ( O, P ) (§20.1.2.8).
fn get_own_property_descriptor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Let key be ? ToPropertyKey(P).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? obj.[[GetOwnProperty]](key).
    let desc = cx.get_own_property(obj, key)?;
    // 4. Return FromPropertyDescriptor(desc).
    cx.from_property_descriptor(desc)
}

/// Template 5. Object.getOwnPropertyNames ( O ) (§20.1.2.10).
fn get_own_property_names<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return CreateArrayFromList(? GetOwnPropertyKeys(O, string)).
    let names = cx.scratch_list();
    get_own_property_keys(cx, args.get(0), &names)?;
    create_array_from_scratch(cx, &names).map(|a| a.value())
}

/// GetOwnPropertyKeys ( O, type ) (§20.1.2.11.1) for type string: the string keys go to
/// `name_list`. A list of values is a `ScratchList` (precise roots), never a `Vec<Local>`.
fn get_own_property_keys<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    name_list: &ScratchList<'s>,
) -> JsResult<()> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(o)?;
    // 2. Let keys be ? obj.[[OwnPropertyKeys]]() (a new array).
    let keys = cx.own_property_keys(obj)?;
    let len = length_of_array_like(cx, keys.into())?;
    // 3. Let nameList be a new empty List.
    // 4. For each element nextKey of keys, do
    for i in 0..len {
        let ik = index_key(cx, i);
        let next_key = cx.get(keys, ik)?;
        // a. If nextKey is a Symbol and type is symbol, or if nextKey is a String and type is
        //    string, then
        if next_key.is_string() {
            // i. Append nextKey to nameList.
            name_list.push(next_key);
        }
    }
    // 5. Return nameList.
    Ok(())
}

/// Object.keys ( O ) (§20.1.2.18).
fn keys<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let obj be ? ToObject(O).
    let obj = cx.to_object(args.get(0))?;
    // 2. Let keyList be ? EnumerableOwnProperties(obj, key).
    let key_list = cx.scratch_list();
    enumerable_own_keys(cx, obj, &key_list)?;
    // 3. Return CreateArrayFromList(keyList).
    create_array_from_scratch(cx, &key_list).map(|a| a.value())
}

/// EnumerableOwnProperties ( O, kind ) (§7.3.23) for kind key: the keys go to `results`.
fn enumerable_own_keys<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    results: &ScratchList<'s>,
) -> JsResult<()> {
    // 1. Let ownKeys be ? O.[[OwnPropertyKeys]]().
    let own_keys = cx.own_property_keys(o)?;
    let len = length_of_array_like(cx, own_keys.into())?;
    // 2. Let results be a new empty List.
    // 3. For each element key of ownKeys, do
    for i in 0..len {
        let ik = index_key(cx, i);
        let key = cx.get(own_keys, ik)?;
        // a. If key is a String, then
        if key.is_string() {
            // i. Let desc be ? O.[[GetOwnProperty]](key).
            let k = cx.to_property_key(key)?;
            let desc = cx.get_own_property(o, k)?;
            // ii. If desc is not undefined and desc.[[Enumerable]] is true, then
            if desc.is_some_and(|d| d.enumerable == Some(true)) {
                // 1. If kind is key, then
                //    a. Append key to results.
                results.push(key);
            }
        }
    }
    // 4. Return results.
    Ok(())
}

/// CreateArrayFromList ( elements ) (§7.3.17) for a `ScratchList`.
fn create_array_from_scratch<'s>(
    cx: &mut Cx<'s>,
    elements: &ScratchList<'s>,
) -> JsResult<Local<'s, JsArray>> {
    // 1. Let array be ! ArrayCreate(0).
    let array = cx.new_array();
    // 2. Let n be 0.
    // 3. For each element e of elements, do
    for n in 0..elements.len() {
        // a. Perform ! CreateDataPropertyOrThrow(array, ! ToString(𝔽(n)), e).
        // b. Set n to n + 1.
        let key = index_key(cx, n as u64);
        cx.create_data_property_or_throw(array, key, elements.get(n))?;
    }
    // 4. Return array.
    Ok(array)
}
