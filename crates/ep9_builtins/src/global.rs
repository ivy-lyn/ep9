//! Global functions (§19.2): module `global`, block 9. Target: the global object
//! (`IntrinsicId::GlobalObject`).
//!
//! ```text
//! isFinite(v)   ToNumber(v) is finite
//! isNaN(v)      ToNumber(v) is NaN
//! ```
//!
//! The global value properties (`globalThis`, `Infinity`, `NaN`, `undefined`) come from the VM
//! table (natives.rs); each constructor module adds its own global binding. `eval` is a VM
//! builtin (`%eval%`). `parseFloat` and `parseInt` are definitions of the `number` module
//! (number.rs), which installs each one function object as both `Number.parseX` and the global
//! `parseX` (§21.1.2.12-13; M2-P5-PLAN-D4): here, `Number.parseInt` would land after
//! `Number.EPSILON`, out of V8's key order. Not built yet: the URI functions.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-global.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsResult, Local, ObjectProps,
    PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 9;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("isFinite", 1, is_finite), // 0
    BuiltinDef::method("isNaN", 1, is_nan),       // 1
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[ObjectProps {
    target: IntrinsicId::GlobalObject,
    // §19.2.2, §19.2.3 ({w, c}).
    props: &[
        PropDef::method(PropKey::Str("isFinite"), 0),
        PropDef::method(PropKey::Str("isNaN"), 1),
    ],
}];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "global",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// isFinite ( number ) (§19.2.2).
fn is_finite<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let num be ? ToNumber(number).
    let num = cx.to_number(args.get(0))?;
    // 2. If num is not finite, return false.
    // 3. Otherwise, return true.
    Ok(Local::boolean(num.is_finite()))
}

/// isNaN ( number ) (§19.2.3).
fn is_nan<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let num be ? ToNumber(number).
    let num = cx.to_number(args.get(0))?;
    // 2. If num is NaN, return true.
    // 3. Otherwise, return false.
    Ok(Local::boolean(num.is_nan()))
}
