#![allow(dead_code)] // m8-stub
//! The Atomics object (`sec-atomics-object`): module `atomics`, block 38. Makes `%Atomics%`
//! (`IntrinsicId::Atomics`), an ordinary object whose prototype is `%Object.prototype%`, with
//! its 14 functions and `@@toStringTag` "Atomics", and the global key `Atomics` {w, c}. Owner
//! of the tables: TypedArray TA-4 R1; owner of the function bodies: TypedArray TA-4 R2 ATOMICS.
//! Until R2 lands, each function is a stub that throws a TypeError (m8-stub, TypedArray TA-4
//! R1). The key `Atomics` is a key of the lazy group of `array_buffer` (TA-D14; `ep9_vm`
//! builtins.rs `LAZY_GROUPS`): realm init makes no `%Atomics%`.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-atomics.txt`. The ids follow node v26's key order. A missing definition
//! is appended at the end (38.14 and on), never inserted.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue,
    JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 38;

/// The builtin functions, in id order (append-only). Lengths: the spec's (node v26 gives the
/// same).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("load", 2, load),                        // 0
    BuiltinDef::method("store", 3, store),                      // 1
    BuiltinDef::method("add", 3, add),                          // 2
    BuiltinDef::method("sub", 3, sub),                          // 3
    BuiltinDef::method("and", 3, and),                          // 4
    BuiltinDef::method("or", 3, or),                            // 5
    BuiltinDef::method("xor", 3, xor),                          // 6
    BuiltinDef::method("exchange", 3, exchange),                // 7
    BuiltinDef::method("compareExchange", 4, compare_exchange), // 8
    BuiltinDef::method("isLockFree", 1, is_lock_free),          // 9
    BuiltinDef::method("wait", 4, wait),                        // 10
    BuiltinDef::method("waitAsync", 4, wait_async),             // 11
    BuiltinDef::method("notify", 3, notify),                    // 12
    BuiltinDef::method("pause", 0, pause),                      // 13
];

/// The intrinsics this module makes: `%Atomics%` (`sec-atomics-object`: an ordinary object,
/// not a function).
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Atomics,
    value: IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    },
}];

/// `key` = the function of definition `d` ({w, c}).
const fn method(key: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), d)
}

/// The properties this module adds, per object, in node v26's key order.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Atomics,
        // The 14 functions ({w, c}), then `Atomics [ %Symbol.toStringTag% ]` ({c};
        // `sec-atomics-%symbol.tostringtag%`).
        props: &[
            method("load", 0),
            method("store", 1),
            method("add", 2),
            method("sub", 3),
            method("and", 4),
            method("or", 5),
            method("xor", 6),
            method("exchange", 7),
            method("compareExchange", 8),
            method("isLockFree", 9),
            method("wait", 10),
            method("waitAsync", 11),
            method("notify", 12),
            method("pause", 13),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Atomics"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // `sec-atomics` ({w, c}): a key of the lazy group.
        props: &[PropDef::intrinsic(
            PropKey::Str("Atomics"),
            IntrinsicId::Atomics,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it after `base64`).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "atomics",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-4)");
    Err(cx.throw_type_error(&msg))
}

/// `Atomics.load ( typedArray, index )` (`sec-atomics.load`).
fn load<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.load")
}

/// `Atomics.store ( typedArray, index, value )` (`sec-atomics.store`).
fn store<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.store")
}

/// `Atomics.add ( typedArray, index, value )` (`sec-atomics.add`).
fn add<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.add")
}

/// `Atomics.sub ( typedArray, index, value )` (`sec-atomics.sub`).
fn sub<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.sub")
}

/// `Atomics.and ( typedArray, index, value )` (`sec-atomics.and`).
fn and<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.and")
}

/// `Atomics.or ( typedArray, index, value )` (`sec-atomics.or`).
fn or<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.or")
}

/// `Atomics.xor ( typedArray, index, value )` (`sec-atomics.xor`).
fn xor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.xor")
}

/// `Atomics.exchange ( typedArray, index, value )` (`sec-atomics.exchange`).
fn exchange<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.exchange")
}

/// `Atomics.compareExchange ( typedArray, index, expectedValue, replacementValue )`
/// (`sec-atomics.compareexchange`).
fn compare_exchange<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.compareExchange")
}

/// `Atomics.isLockFree ( size )` (`sec-atomics.islockfree`).
fn is_lock_free<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.isLockFree")
}

/// `Atomics.wait ( typedArray, index, value, timeout )` (`sec-atomics.wait`).
fn wait<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.wait")
}

/// `Atomics.waitAsync ( typedArray, index, value, timeout )` (`sec-atomics.waitasync`).
fn wait_async<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.waitAsync")
}

/// `Atomics.notify ( typedArray, index, count )` (`sec-atomics.notify`).
fn notify<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.notify")
}

/// `Atomics.pause ( [ N ] )` (`sec-atomics.pause`).
fn pause<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Atomics.pause")
}
