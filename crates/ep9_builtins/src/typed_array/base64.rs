#![allow(dead_code)] // m8-stub
//! The base64 and hex methods of `Uint8Array` (ES2026: `sec-uint8array.frombase64`,
//! `sec-uint8array.fromhex`, `sec-uint8array.prototype.tobase64`,
//! `sec-uint8array.prototype.setfrombase64`, `sec-uint8array.prototype.tohex`,
//! `sec-uint8array.prototype.setfromhex`): module `base64`, block 37. Owner of the tables:
//! TypedArray TA-4 R1; owner of the function bodies: TypedArray TA-4 R5 B64. Until R5 lands,
//! each function is a stub that throws a TypeError (m8-stub, TypedArray TA-4 R1). The module
//! joins the lazy group of `array_buffer` (TA-D14): each table entry targets `%Uint8Array%` or
//! `%Uint8Array.prototype%`.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-base64.txt`. The ids follow node v26's key order. A missing definition is
//! appended at the end (37.6 and on), never inserted.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicId, JsResult, Local, ObjectProps, PropDef,
    PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 37;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("fromBase64", 1, from_base64), // 0
    BuiltinDef::method("fromHex", 1, from_hex),       // 1
    BuiltinDef::method("toBase64", 0, to_base64),     // 2
    BuiltinDef::method("setFromBase64", 1, set_from_base64), // 3
    BuiltinDef::method("toHex", 0, to_hex),           // 4
    BuiltinDef::method("setFromHex", 1, set_from_hex), // 5
];

/// `key` = the function of definition `d` ({w, c}).
const fn method(key: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), d)
}

/// The properties this module adds, in node v26's key order: on `%Uint8Array%` after
/// `BYTES_PER_ELEMENT` (module `typed_array` installs first), and on `%Uint8Array.prototype%`
/// after `constructor` and `BYTES_PER_ELEMENT`. All {w, c}.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Uint8Array,
        props: &[method("fromBase64", 0), method("fromHex", 1)],
    },
    ObjectProps {
        target: IntrinsicId::Uint8ArrayPrototype,
        props: &[
            method("toBase64", 2),
            method("setFromBase64", 3),
            method("toHex", 4),
            method("setFromHex", 5),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it after `typed_array_proto`).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "base64",
    block: BLOCK,
    defs: DEFS,
    intrinsics: &[],
    props: PROPS,
};

/// The TypeError of a function that is not built yet.
fn stub<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s>> {
    let msg = format!("{name}: not built yet (m8-stub, TypedArray TA-4)");
    Err(cx.throw_type_error(&msg))
}

/// `Uint8Array.fromBase64 ( string [ , options ] )` (`sec-uint8array.frombase64`).
fn from_base64<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.fromBase64")
}

/// `Uint8Array.fromHex ( string )` (`sec-uint8array.fromhex`).
fn from_hex<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.fromHex")
}

/// `Uint8Array.prototype.toBase64 ( [ options ] )` (`sec-uint8array.prototype.tobase64`).
fn to_base64<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.prototype.toBase64")
}

/// `Uint8Array.prototype.setFromBase64 ( string [ , options ] )`
/// (`sec-uint8array.prototype.setfrombase64`).
fn set_from_base64<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.prototype.setFromBase64")
}

/// `Uint8Array.prototype.toHex ( )` (`sec-uint8array.prototype.tohex`).
fn to_hex<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.prototype.toHex")
}

/// `Uint8Array.prototype.setFromHex ( string )` (`sec-uint8array.prototype.setfromhex`).
fn set_from_hex<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    stub(cx, "Uint8Array.prototype.setFromHex")
}
