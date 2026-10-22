//! Reflect (§28.1): module `reflect`, block 11. Makes `%Reflect%` (`IntrinsicId::Reflect`), an
//! ordinary object whose prototype is `%Object.prototype%`, with its 13 functions and
//! `@@toStringTag` "Reflect", and the global binding `Reflect` {w, c}.
//!
//! Each function maps onto one `Cx` object operation, after the same first step: a target that
//! is not an object is a TypeError (apply and construct: not callable, not a constructor).
//!
//! | Function | Cx operation |
//! |---|---|
//! | apply, construct | `create_list_from_array_like` (an `ArgsBuilder` on the VM stack), then `call_built` / `construct_built` |
//! | defineProperty | `to_property_descriptor`, then `define_own_property` (a boolean, never a throw) |
//! | deleteProperty, has, getOwnPropertyDescriptor | `delete`, `has_property`, `get_own_property` + `from_property_descriptor` |
//! | get, set | `get_with_receiver`, `set_with_receiver`: the receiver is the target unless passed (M2-P5-PLAN-D2) |
//! | getPrototypeOf, setPrototypeOf, isExtensible, preventExtensions | the internal methods of the same name |
//! | ownKeys | `own_property_keys`: already a new array, and it never lists a private name (§6.2.12) |
//!
//! None of them loops over a list in Rust: CreateListFromArrayLike is the VM's (bounded by the
//! VM stack), and `[[OwnPropertyKeys]]` makes its array in one VM call. So none polls or checks
//! the native stack (`Cx::poll_interrupt`, `Cx::check_native_stack`).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-reflect.txt`. The ids follow the spec's order (§28.1.1-13); the keys of
//! `%Reflect%` follow V8's (node 24 `Reflect.ownKeys(Reflect)`).

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsObject, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 11;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const APPLY: u16 = 0;
    pub const CONSTRUCT: u16 = 1;
    pub const DEFINE_PROPERTY: u16 = 2;
    pub const DELETE_PROPERTY: u16 = 3;
    pub const GET: u16 = 4;
    pub const GET_OWN_PROPERTY_DESCRIPTOR: u16 = 5;
    pub const GET_PROTOTYPE_OF: u16 = 6;
    pub const HAS: u16 = 7;
    pub const IS_EXTENSIBLE: u16 = 8;
    pub const OWN_KEYS: u16 = 9;
    pub const PREVENT_EXTENSIONS: u16 = 10;
    pub const SET: u16 = 11;
    pub const SET_PROTOTYPE_OF: u16 = 12;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("apply", 3, apply),         // 11.0
    BuiltinDef::method("construct", 2, construct), // 11.1
    BuiltinDef::method("defineProperty", 3, define_property), // 11.2
    BuiltinDef::method("deleteProperty", 2, delete_property), // 11.3
    BuiltinDef::method("get", 2, get),             // 11.4
    BuiltinDef::method("getOwnPropertyDescriptor", 2, get_own_property_descriptor), // 11.5
    BuiltinDef::method("getPrototypeOf", 1, get_prototype_of), // 11.6
    BuiltinDef::method("has", 2, has),             // 11.7
    BuiltinDef::method("isExtensible", 1, is_extensible), // 11.8
    BuiltinDef::method("ownKeys", 1, own_keys),    // 11.9
    BuiltinDef::method("preventExtensions", 1, prevent_extensions), // 11.10
    BuiltinDef::method("set", 3, set),             // 11.11
    BuiltinDef::method("setPrototypeOf", 2, set_prototype_of), // 11.12
];

/// The intrinsics this module makes: `%Reflect%` (§28.1: an ordinary object, not a function).
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Reflect,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Reflect,
        // §28.1.1-13 ({w, c}), in V8's order, then §28.1.14 Reflect [ %Symbol.toStringTag% ]
        // ({c}).
        props: &[
            PropDef::method(PropKey::Str("defineProperty"), def::DEFINE_PROPERTY),
            PropDef::method(PropKey::Str("deleteProperty"), def::DELETE_PROPERTY),
            PropDef::method(PropKey::Str("apply"), def::APPLY),
            PropDef::method(PropKey::Str("construct"), def::CONSTRUCT),
            PropDef::method(PropKey::Str("get"), def::GET),
            PropDef::method(
                PropKey::Str("getOwnPropertyDescriptor"),
                def::GET_OWN_PROPERTY_DESCRIPTOR,
            ),
            PropDef::method(PropKey::Str("getPrototypeOf"), def::GET_PROTOTYPE_OF),
            PropDef::method(PropKey::Str("has"), def::HAS),
            PropDef::method(PropKey::Str("isExtensible"), def::IS_EXTENSIBLE),
            PropDef::method(PropKey::Str("ownKeys"), def::OWN_KEYS),
            PropDef::method(PropKey::Str("preventExtensions"), def::PREVENT_EXTENSIONS),
            PropDef::method(PropKey::Str("set"), def::SET),
            PropDef::method(PropKey::Str("setPrototypeOf"), def::SET_PROTOTYPE_OF),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Reflect"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.4.4 Reflect ({w, c}).
        props: &[PropDef::intrinsic(
            PropKey::Str("Reflect"),
            IntrinsicId::Reflect,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "reflect",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Step 1 of most Reflect functions: "If target is not an Object, throw a TypeError
/// exception."
fn target<'s>(cx: &mut Cx<'s>, v: Local<'s>, name: &str) -> JsResult<Local<'s, JsObject>> {
    match v.as_object() {
        Some(o) => Ok(o),
        None => Err(cx.throw_type_error(&format!("Reflect.{name} called on non-object"))),
    }
}

/// Reflect.apply ( target, thisArgument, argumentsList ) (§28.1.1).
fn apply<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let target = args.get(0);
    // 1. If IsCallable(target) is false, throw a TypeError exception.
    if !target.is_callable() {
        let what = cx.type_of(target);
        let msg = format!("Reflect.apply called on a value that is not a function ({what})");
        return Err(cx.throw_type_error(&msg));
    }
    // 2. Let args be ? CreateListFromArrayLike(argumentsList).
    let list = cx.create_list_from_array_like(args.get(2))?;
    // 3. Perform PrepareForTailCall() (v0: no tail call; H20-Q3).
    // 4. Return ? Call(target, thisArgument, args).
    cx.call_built(target, args.get(1), list)
}

/// Reflect.construct ( target, argumentsList [ , newTarget ] ) (§28.1.2).
fn construct<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let target = args.get(0);
    // 1. If IsConstructor(target) is false, throw a TypeError exception.
    if !cx.is_constructor(target) {
        return Err(cx.throw_type_error("Reflect.construct target is not a constructor"));
    }
    // 2. If newTarget is not present, set newTarget to target.
    // 3. Else if IsConstructor(newTarget) is false, throw a TypeError exception.
    let new_target = if args.len() < 3 {
        target
    } else {
        let nt = args.get(2);
        if !cx.is_constructor(nt) {
            return Err(cx.throw_type_error("Reflect.construct newTarget is not a constructor"));
        }
        nt
    };
    // 4. Let args be ? CreateListFromArrayLike(argumentsList).
    let list = cx.create_list_from_array_like(args.get(1))?;
    // 5. Return ? Construct(target, args, newTarget).
    Ok(cx.construct_built(target, list, new_target)?.value())
}

/// Reflect.defineProperty ( target, propertyKey, attributes ) (§28.1.3).
fn define_property<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "defineProperty")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? ToPropertyDescriptor(attributes). (User getters run here; `o` and
    //    `key` stay live in Rust locals, R1.)
    let desc = cx.to_property_descriptor(args.get(2))?;
    // 4. Return ? target.[[DefineOwnProperty]](key, desc).
    Ok(Local::boolean(cx.define_own_property(o, key, desc)?))
}

/// Reflect.deleteProperty ( target, propertyKey ) (§28.1.4).
fn delete_property<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "deleteProperty")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Return ? target.[[Delete]](key).
    Ok(Local::boolean(cx.delete(o, key)?))
}

/// Reflect.get ( target, propertyKey [ , receiver ] ) (§28.1.5).
fn get<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "get")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. If receiver is not present, then
    //    a. Set receiver to target.
    let receiver = if args.len() < 3 {
        o.value()
    } else {
        args.get(2)
    };
    // 4. Return ? target.[[Get]](key, receiver).
    cx.get_with_receiver(o, key, receiver)
}

/// Reflect.getOwnPropertyDescriptor ( target, propertyKey ) (§28.1.6).
fn get_own_property_descriptor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "getOwnPropertyDescriptor")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Let desc be ? target.[[GetOwnProperty]](key).
    let desc = cx.get_own_property(o, key)?;
    // 4. Return FromPropertyDescriptor(desc).
    cx.from_property_descriptor(desc)
}

/// Reflect.getPrototypeOf ( target ) (§28.1.7).
fn get_prototype_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "getPrototypeOf")?;
    // 2. Return ? target.[[GetPrototypeOf]]().
    cx.get_prototype_of(o)
}

/// Reflect.has ( target, propertyKey ) (§28.1.8).
fn has<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "has")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. Return ? target.[[HasProperty]](key).
    Ok(Local::boolean(cx.has_property(o, key)?))
}

/// Reflect.isExtensible ( target ) (§28.1.9).
fn is_extensible<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "isExtensible")?;
    // 2. Return ? target.[[IsExtensible]]().
    Ok(Local::boolean(cx.is_extensible(o)?))
}

/// Reflect.ownKeys ( target ) (§28.1.10).
fn own_keys<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "ownKeys")?;
    // 2. Let keys be ? target.[[OwnPropertyKeys]]().
    // 3. Return CreateArrayFromList(keys). (`own_property_keys` returns a new array: no
    //    private name is in it, M2-P4-REFLECT-PRIV.)
    Ok(cx.own_property_keys(o)?.value())
}

/// Reflect.preventExtensions ( target ) (§28.1.11).
fn prevent_extensions<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "preventExtensions")?;
    // 2. Return ? target.[[PreventExtensions]]().
    Ok(Local::boolean(cx.prevent_extensions(o)?))
}

/// Reflect.set ( target, propertyKey, V [ , receiver ] ) (§28.1.12).
fn set<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "set")?;
    // 2. Let key be ? ToPropertyKey(propertyKey).
    let key = cx.to_property_key(args.get(1))?;
    // 3. If receiver is not present, then
    //    a. Set receiver to target.
    let receiver = if args.len() < 4 {
        o.value()
    } else {
        args.get(3)
    };
    // 4. Return ? target.[[Set]](key, V, receiver).
    Ok(Local::boolean(cx.set_with_receiver(
        o,
        key,
        args.get(2),
        receiver,
    )?))
}

/// Reflect.setPrototypeOf ( target, proto ) (§28.1.13).
fn set_prototype_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If target is not an Object, throw a TypeError exception.
    let o = target(cx, args.get(0), "setPrototypeOf")?;
    // 2. If proto is not an Object and proto is not null, throw a TypeError exception.
    let proto = args.get(1);
    if !proto.is_object() && !proto.is_null() {
        return Err(cx.throw_type_error("Object prototype may only be an Object or null"));
    }
    // 3. Return ? target.[[SetPrototypeOf]](proto).
    Ok(Local::boolean(cx.set_prototype_of(o, proto)?))
}
