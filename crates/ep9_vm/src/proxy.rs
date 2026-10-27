//! Proxy exotic objects (§10.5; owner M4-P4 PROXY1, LB-6, V9-D19): the kinds PROXY (27) and
//! PROXY_CALLABLE (28) of OBJECT_LAYOUT 4 (`kinds.rs`), their layout, and their internal
//! methods, routed here from `exotic.rs` and `property.rs`.
//!
//! ```text
//!  PROXY / PROXY_CALLABLE (JS object kinds; object.rs `fixed::PROXY`)
//!  +0  word0 (shape of the cell: prototype null; only private names are stored here)
//!  +8  SLOTS (out-of-line fields) | PROPS | undefined
//!  +16 undefined (no elements)
//!  +24 fixed 0  [[ProxyTarget]]   an object; null once revoked
//!  +32 fixed 1  [[ProxyHandler]]  an object; null once revoked
//! ```
//!
//! The kind is PROXY_CALLABLE when the target is callable at creation (§10.5.14 step 7), so
//! IsCallable stays a kind test (`kinds::is_callable`). Both fields live in the heap cell
//! (plan rule 8) and are traced as Values (object.rs registers both kinds).
//!
//! Routing (MC-w15-12, H31 fast-path row 3): both kinds are `exotic::is_routed` kinds, so
//! every hook of `exotic.rs` and `property.rs` that can throw sends them here, and the keyless
//! index paths (`property::own_index`) answer `Unknown` for them (the key path asks the
//! proxy). The hooks that cannot throw (`exotic::own_attrs`, `exotic::own_property_keys`)
//! assert in debug builds that they never see a proxy; their fallible forms
//! (`exotic::try_own_attrs`, `property::try_own_property_keys`) route it here.
//!
//! ```text
//!  internal method      here (month 4)                       PROXY2 (month 5)
//!  any                  revoked (handler null): TypeError    the same
//!                       native stack low: RangeError
//!  [[GetPrototypeOf]] .. [[OwnPropertyKeys]]
//!                       the target's internal method         GetMethod(handler, trap); the
//!                       (§10.5.x, the "trap is undefined"    call and the invariant checks
//!                       step)                                when the trap is not undefined
//!  [[Call]], [[Construct]]  not here: call.rs (PROXY2)
//! ```
//!
//! Stub traps (plan v9 §6.3): no JS can make a Proxy before PROXY2 brings the `Proxy`
//! constructor (month 5), so each internal method takes the step of the spec for an
//! undefined trap and forwards to the target. That is the ordinary behaviour of a proxy whose
//! handler is `{}`. Each method checks the native stack first: a chain of proxies (or a
//! target whose prototype chain comes back through its proxy) recurses in Rust.

use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::{stack_overflow, throw_error};
use crate::frame::NULL;
use crate::interp::Cx;
use crate::kinds::{self as kind, is_callable, is_object};
use crate::object::{self, fixed};
use crate::property;
use crate::runtime::{ErrorKind, Runtime};

/// Fixed-field indices (`object::fixed_get` / `fixed_set`).
mod field {
    pub const TARGET: usize = 0;
    pub const HANDLER: usize = 1;
}

/// True for the proxy kinds PROXY and PROXY_CALLABLE (adjacent ids 27 and 28).
#[inline(always)]
pub(crate) fn is_proxy_kind(k: u8) -> bool {
    const _: () = assert!(kind::PROXY.get() + 1 == kind::PROXY_CALLABLE.get());
    k.wrapping_sub(kind::PROXY.get()) <= 1
}

/// True if `v` is a Proxy exotic object (either kind). Any value.
#[inline]
pub(crate) fn is_proxy(v: u64) -> bool {
    is_object(v) && is_proxy_kind(kind::cell_kind(v))
}

/// `[[ProxyTarget]]` of the proxy `p`: an object, or null once revoked.
#[inline]
pub(crate) fn target(p: u64) -> u64 {
    debug_assert!(is_proxy(p));
    object::fixed_get(p, field::TARGET)
}

/// `[[ProxyHandler]]` of the proxy `p`: an object, or null once revoked.
#[inline]
pub(crate) fn handler(p: u64) -> u64 {
    debug_assert!(is_proxy(p));
    object::fixed_get(p, field::HANDLER)
}

/// ProxyCreate(target, handler) (§10.5.14): a new proxy, PROXY_CALLABLE if `target` is
/// callable. TypeError if either is not an object. `target` and `handler` are rooted by the
/// caller (Rust locals). GC point. The `Proxy` constructor and `Proxy.revocable` (PROXY2,
/// month 5) call it; before them only tests do.
#[allow(dead_code)] // first JS caller: PROXY2 (month 5); tests::proxy
pub(crate) fn proxy_create(cx: &mut Cx, target: u64, handler: u64) -> Res<u64> {
    // 1-2. If target or handler is not an Object, throw a TypeError exception.
    if !is_object(target) || !is_object(handler) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot create proxy with a non-object as target or handler",
        ));
    }
    // 3-6. MakeBasicObject; the internal methods are the routed hooks (`exotic.rs`). The
    //      cell's own prototype is null: no walk reads it (a proxy answers [[GetPrototypeOf]]
    //      here).
    // 7. The [[Call]] / [[Construct]] question is the kind (IsCallable is a kind test).
    let k = if is_callable(target) {
        kind::PROXY_CALLABLE
    } else {
        kind::PROXY
    };
    let rt = cx.rt();
    // GC point: `target` and `handler` live in Rust locals (CSS).
    let p = object::new_object_with_fields(rt, k, NULL, fixed::PROXY);
    // 8-9. Set P.[[ProxyTarget]] and P.[[ProxyHandler]].
    object::fixed_set(rt, p, field::TARGET, target);
    object::fixed_set(rt, p, field::HANDLER, handler);
    Ok(p)
}

/// The revoke step of a revocable proxy (§28.2.2.1.1 steps 3-5): both fields become null.
/// Not a GC point.
#[allow(dead_code)] // first JS caller: PROXY2 (`Proxy.revocable`, month 5); tests::proxy
pub(crate) fn revoke(rt: &Runtime, p: u64) {
    object::fixed_set(rt, p, field::TARGET, NULL);
    object::fixed_set(rt, p, field::HANDLER, NULL);
}

/// The common start of every internal method of the proxy `p`: the native stack check, then
/// ValidateNonRevokedProxy (§10.5.15; TypeError with V8's text naming `trap`). Returns the
/// target. Month 4: no trap is looked up (PROXY2 adds `GetMethod(handler, trap)` here), so
/// the caller takes the step for an undefined trap.
#[inline]
fn start(cx: &mut Cx, p: u64, trap: &str) -> Res<u64> {
    if !cx.rt().native_stack_ok() {
        return stack_overflow(cx);
    }
    if handler(p) == NULL {
        let msg = format!("Cannot perform '{trap}' on a proxy that has been revoked");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    Ok(target(p))
}

/// `p.[[GetPrototypeOf]]()` (§10.5.1). GC point.
pub(crate) fn get_prototype_of(cx: &mut Cx, p: u64) -> Res<u64> {
    let t = start(cx, p, "getPrototypeOf")?;
    // 7. If trap is undefined, return ? target.[[GetPrototypeOf]]().
    property::try_get_prototype_of(cx, t)
}

/// `p.[[SetPrototypeOf]](v)` (§10.5.2); `v` is an object or null. GC point.
pub(crate) fn set_prototype_of(cx: &mut Cx, p: u64, v: u64) -> Res<bool> {
    let t = start(cx, p, "setPrototypeOf")?;
    // 7. If trap is undefined, return ? target.[[SetPrototypeOf]](V).
    property::try_set_prototype_of(cx, t, v)
}

/// `p.[[IsExtensible]]()` (§10.5.3). GC point.
pub(crate) fn is_extensible(cx: &mut Cx, p: u64) -> Res<bool> {
    let t = start(cx, p, "isExtensible")?;
    // 6. If trap is undefined, return ? IsExtensible(target).
    property::try_is_extensible(cx, t)
}

/// `p.[[PreventExtensions]]()` (§10.5.4). GC point.
pub(crate) fn prevent_extensions(cx: &mut Cx, p: u64) -> Res<bool> {
    let t = start(cx, p, "preventExtensions")?;
    // 6. If trap is undefined, return ? target.[[PreventExtensions]]().
    property::try_prevent_extensions(cx, t)
}

/// `p.[[GetOwnProperty]](key)` (§10.5.5) as (value, attribute bits) in the stored form of
/// `property.rs` (an accessor's value is its ACCESSOR_PAIR), or `None`. GC point.
pub(crate) fn get_own_property(cx: &mut Cx, p: u64, key: u64) -> Res<Option<(u64, i32)>> {
    let t = start(cx, p, "getOwnPropertyDescriptor")?;
    // 7. If trap is undefined, return ? target.[[GetOwnProperty]](P).
    property::own_property_stored(cx, t, key)
}

/// `p.[[DefineOwnProperty]](key, desc)` (§10.5.6). GC point.
pub(crate) fn define_own_property(cx: &mut Cx, p: u64, key: u64, desc: Desc) -> Res<bool> {
    let t = start(cx, p, "defineProperty")?;
    // 7. If trap is undefined, return ? target.[[DefineOwnProperty]](P, Desc).
    property::define_own_property(cx, t, key, desc)
}

/// `p.[[HasProperty]](key)` (§10.5.7). GC point.
pub(crate) fn has_property(cx: &mut Cx, p: u64, key: u64) -> Res<bool> {
    let t = start(cx, p, "has")?;
    // 7. If trap is undefined, return ? target.[[HasProperty]](P).
    property::try_has_property(cx, t, key)
}

/// `p.[[Get]](key, receiver)` (§10.5.8). GC point.
pub(crate) fn get(cx: &mut Cx, p: u64, key: u64, receiver: u64) -> Res<u64> {
    let t = start(cx, p, "get")?;
    // 7. If trap is undefined, return ? target.[[Get]](P, Receiver).
    property::get_with_receiver(cx, t, key, receiver)
}

/// `p.[[Set]](key, v, receiver)` (§10.5.9). GC point.
pub(crate) fn set(cx: &mut Cx, p: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    let t = start(cx, p, "set")?;
    // 7. If trap is undefined, return ? target.[[Set]](P, V, Receiver).
    property::set_with_receiver(cx, t, key, v, receiver)
}

/// `p.[[Delete]](key)` (§10.5.10). GC point.
pub(crate) fn delete(cx: &mut Cx, p: u64, key: u64) -> Res<bool> {
    let t = start(cx, p, "deleteProperty")?;
    // 7. If trap is undefined, return ? target.[[Delete]](P).
    property::internal_delete(cx, t, key)
}

/// `p.[[OwnPropertyKeys]]()` (§10.5.11) as a new array. GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, p: u64) -> Res<u64> {
    let t = start(cx, p, "ownKeys")?;
    // 7. If trap is undefined, return ? target.[[OwnPropertyKeys]]().
    property::try_own_property_keys(cx, t)
}

/// IsArray(v) (§7.2.2) with the Proxy step: an Array exotic object is an array; a proxy
/// answers for its target, down a chain of proxies in a loop (no Rust recursion); a revoked
/// proxy throws a TypeError (step 3.a). Any value. Not a GC point before the throw.
pub(crate) fn is_array(cx: &mut Cx, v: u64) -> Res<bool> {
    let mut v = v;
    while is_proxy(v) {
        if handler(v) == NULL {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot perform 'IsArray' on a proxy that has been revoked",
            ));
        }
        v = target(v);
    }
    Ok(kind::is_array(v))
}
