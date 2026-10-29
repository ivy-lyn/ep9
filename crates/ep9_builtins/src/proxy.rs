//! Proxy (§28.2): module `proxy`, block 15. Owner: M5-P3 PROXY2 (M4-P4 PROXY1 made the
//! kinds and the routing in ep9_vm `proxy.rs`). Makes `%Proxy%` (`IntrinsicId::Proxy`, 64):
//! the constructor `Proxy` (length 2) with `Proxy.revocable` (length 2), and the global
//! binding `Proxy` {w, c}. `%Proxy%` has no `prototype` property (§28.2.2): its definition
//! is a constructor (so it has `[[Construct]]`), `check_tables` takes it with no `prototype`
//! key, and `exotic::is_lazy_prototype` makes no lazy one for it (the H20 table rule).
//!
//! | Function | Steps | Cx operation |
//! |---|---|---|
//! | `Proxy(target, handler)` | §28.2.1.1: TypeError without new.target, then ProxyCreate | `proxy_create` |
//! | `Proxy.revocable(target, handler)` | §28.2.2.1: ProxyCreate, a revoke closure, the result object `{proxy, revoke}` | `proxy_create`, `new_builtin_closure`, `create_data_property_or_throw` |
//! | revoke (anonymous, length 0) | §28.2.2.1.1: revoke the proxy once | `closure_data`, `proxy_revoke` |
//!
//! The revoke closure keeps its proxy as its closure data (immutable). §28.2.2.1.1 step 5
//! sets `[[RevocableProxy]]` to null; here a second call revokes the revoked proxy again,
//! which changes nothing (`Cx::proxy_revoke`), so the only difference is that the closure
//! keeps the empty proxy cell alive.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-proxy.txt`. The keys of `%Proxy%` follow V8's (node v24.22.1
//! `Reflect.ownKeys(Proxy)`: length, name, revocable).

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    Local, ObjectProps, PropAttrs, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 15;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const PROXY: u16 = 0;
    pub const REVOCABLE: u16 = 1;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Proxy", 2, proxy_ctor), // 15.0
    BuiltinDef::method("revocable", 2, revocable),   // 15.1
];

/// The intrinsics this module makes: `%Proxy%`.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Proxy,
    value: IntrinsicValue::Builtin(def::PROXY),
}];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Proxy,
        // §28.2.2.1 Proxy.revocable ({w, c}). No `prototype` (§28.2.2).
        props: &[PropDef::method(PropKey::Str("revocable"), def::REVOCABLE)],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Proxy ( . . . ) ({w, c}).
        props: &[PropDef::intrinsic(
            PropKey::Str("Proxy"),
            IntrinsicId::Proxy,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "proxy",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// Proxy ( target, handler ) (§28.2.1.1).
fn proxy_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    if new_target.is_undefined() {
        return Err(cx.throw_type_error("Constructor Proxy requires 'new'"));
    }
    // 2. Return ? ProxyCreate(target, handler).
    Ok(cx.proxy_create(args.get(0), args.get(1))?.value())
}

/// Proxy.revocable ( target, handler ) (§28.2.2.1).
fn revocable<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let proxy be ? ProxyCreate(target, handler).
    let proxy = cx.proxy_create(args.get(0), args.get(1))?;
    // 2-4. Let revoker be CreateBuiltinFunction(revokerClosure, 0, "", « [[RevocableProxy]] »),
    //      with [[RevocableProxy]] proxy (the closure data).
    let revoker = cx.new_builtin_closure(revoke, "", 0, proxy);
    // 5. Let result be OrdinaryObjectCreate(%Object.prototype%).
    let result = cx.new_object();
    // 6-7. CreateDataPropertyOrThrow(result, "proxy", proxy) and (result, "revoke", revoker).
    cx.create_data_property_or_throw(result, "proxy", proxy)?;
    cx.create_data_property_or_throw(result, "revoke", revoker)?;
    // 8. Return result.
    Ok(result.value())
}

/// A Proxy revocation function (§28.2.2.1.1).
fn revoke<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let p be F.[[RevocableProxy]]; if p is null, return undefined.
    let p = cx.closure_data(args.callee());
    // 3-6. Set F.[[RevocableProxy]] to null (module doc); revoke p; return undefined.
    cx.proxy_revoke(p);
    Ok(Local::undefined())
}
