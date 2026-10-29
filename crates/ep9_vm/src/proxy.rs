//! Proxy exotic objects (§10.5; M4-P4 PROXY1 made the kinds and the routing, LB-6, V9-D19;
//! M5-P3 PROXY2 the traps, `Proxy` and `Proxy.revocable`): the kinds PROXY (27) and
//! PROXY_CALLABLE (28) of OBJECT_LAYOUT 4 (`kinds.rs`), their layout, and their 13 internal
//! methods, routed here from `exotic.rs`, `property.rs` and `call.rs`.
//!
//! ```text
//!  PROXY / PROXY_CALLABLE (JS object kinds; object.rs `fixed::PROXY`)
//!  +0  word0 (shape of the cell: prototype null; only private names are stored here)
//!  +8  SLOTS (out-of-line fields) | PROPS | undefined
//!  +16 undefined (no elements)
//!  +24 fixed 0  [[ProxyTarget]]   an object; once revoked null, or undefined if it has [[Construct]]
//!  +32 fixed 1  [[ProxyHandler]]  an object; null once revoked
//! ```
//!
//! The kind is PROXY_CALLABLE when the target is callable at creation (§10.5.14 step 7), so
//! IsCallable stays a kind test (`kinds::is_callable`). Both fields live in the heap cell
//! (plan rule 8) and are traced as Values (object.rs registers both kinds). IsConstructor of
//! a callable proxy is its target's ([`is_constructor`]); a revoked one keeps its answer
//! (§10.5.14 step 7.b): its target field holds null, or `undefined` if it has `[[Construct]]`.
//!
//! Routing (MC-w15-12, H31 fast-path row 3): both kinds are `exotic::is_routed` kinds, so
//! every hook of `exotic.rs` and `property.rs` that can throw sends them here, and the keyless
//! index paths (`property::own_index`) answer `Unknown` for them (the key path asks the
//! proxy). The routed-kind test stays one compare on the hot paths: every routed kind goes
//! through `exotic.rs`. The hooks that cannot throw (`exotic::own_attrs`,
//! `exotic::own_property_keys`) assert in debug builds that they never see a proxy; their
//! fallible forms (`exotic::try_own_attrs`, `property::try_own_property_keys`) route it here.
//! `[[Call]]` and `[[Construct]]` come from `call::call_with` and `call::construct_with`
//! (and so from `slow::call` for bytecode callers).
//!
//! ```text
//!  every internal method      start: native stack and forwarding depth (RangeError), then
//!                             ValidateNonRevokedProxy (TypeError naming the trap)
//!                             trap: GetMethod(handler, name) (TypeError if not callable)
//!  trap undefined             the target's internal method (§10.5.x "If trap is undefined")
//!  trap defined               Call(trap, handler, «target, ...»), then the invariant checks
//!                             of §10.5.x against the target (TypeError, V8's texts)
//! ```
//!
//! | Method | Trap | Invariants checked against the target (§10.5) |
//! |---|---|---|
//! | `[[GetPrototypeOf]]` | getPrototypeOf | an object or null; a non-extensible target's own prototype |
//! | `[[SetPrototypeOf]]` | setPrototypeOf | true on a non-extensible target only for its own prototype |
//! | `[[IsExtensible]]` | isExtensible | the target's answer |
//! | `[[PreventExtensions]]` | preventExtensions | true only if the target is non-extensible |
//! | `[[GetOwnProperty]]` | getOwnPropertyDescriptor | an object or undefined; IsCompatiblePropertyDescriptor; non-configurable and non-writable only as the target's |
//! | `[[DefineOwnProperty]]` | defineProperty | no new property on a non-extensible target; compatible; non-configurable and non-writable only as the target's |
//! | `[[HasProperty]]` | has | false never hides a non-configurable property or one of a non-extensible target |
//! | `[[Get]]` | get | a non-configurable, non-writable data property's value; undefined for a getter-less non-configurable accessor |
//! | `[[Set]]` | set | true never changes such a data property, nor a setter-less non-configurable accessor |
//! | `[[Delete]]` | deleteProperty | true never removes a non-configurable property or one of a non-extensible target |
//! | `[[OwnPropertyKeys]]` | ownKeys | strings and symbols, no duplicates; every non-configurable key; exactly the target's keys if it is non-extensible |
//! | `[[Call]]` | apply | none |
//! | `[[Construct]]` | construct | an object |
//!
//! GC: every Value of a method lives in a Rust local (the conservative stack scan; the heap
//! does not move cells) or in a JS array that a Rust local holds (the `ownKeys` result, the
//! argument array of `apply` and `construct`). The `[[Call]]` and `[[Construct]]` argument
//! lists are on the VM stack's scratch area (`call.rs`).
//!
//! Forwarding depth (M4-P4 fixer B; Codex PROXY1 P-1, R-GC-P4-4). The native stack check
//! alone does not end a recursion through proxies: an optimized build may turn the forward of
//! `[[HasProperty]]` (proxy -> target -> the target's prototype, the proxy again) into a loop
//! at one stack depth, so the stack never runs low. So every internal method also counts its
//! depth: [`MAX_FORWARD_DEPTH`] nested methods on one thread give the RangeError "Maximum
//! call stack size exceeded" (V8's text for the same JS). The count goes up before the trap
//! or the forward and down after it returns (a guard's `Drop`), so the forward is never a
//! tail call, and the bound holds whatever the optimizer does.
//!
//! ```text
//!  a.[[HasProperty]](k)            Object.setPrototypeOf(a, p), p = new Proxy(a, {})
//!   └ p.[[HasProperty]](k)         depth 1
//!      └ a.[[HasProperty]](k)
//!         └ p.[[HasProperty]](k)   depth 2 ... depth MAX_FORWARD_DEPTH + 1: RangeError
//! ```

use core::cell::Cell;

use ep9_value::Throw;

use crate::call;
use crate::conv::{Res, to_boolean, to_number};
use crate::descriptor::Desc;
use crate::error::{Part::Key, Part::Text, bounded_message, stack_overflow, throw_error};
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::{intern_key, is_property_key};
use crate::kinds::{self as kind, is_callable, is_object, is_symbol};
use crate::object::{self, attr, fixed, is_string};
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
/// caller (Rust locals). GC point. The `Proxy` constructor and `Proxy.revocable`
/// (`Cx::proxy_create`, ep9_builtins `proxy.rs`) call it.
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
    // 7. The [[Call]] / [[Construct]] question is the kind (IsCallable is a kind test) and,
    //    for [[Construct]], the target ([`is_constructor`]).
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

/// The target field of a revoked proxy that has `[[Construct]]` (§10.5.14 step 7.b: a
/// revoked proxy keeps the internal methods it got at creation). Not an object, so nothing
/// reads it as a target; a revoked proxy without `[[Construct]]` holds null.
const REVOKED_CONSTRUCTOR: u64 = UNDEFINED;

/// The revoke step of a revocable proxy (§28.2.2.1.1 steps 3-5): the handler becomes null,
/// and the target null, or [`REVOKED_CONSTRUCTOR`] when the proxy has `[[Construct]]` (so
/// IsConstructor keeps its creation-time answer, as V8's map bit does). Revoking a revoked
/// proxy changes nothing. Not a GC point.
pub(crate) fn revoke(rt: &Runtime, p: u64) {
    if handler(p) == NULL {
        return;
    }
    let t = if is_constructor(p) {
        REVOKED_CONSTRUCTOR
    } else {
        NULL
    };
    object::fixed_set(rt, p, field::TARGET, t);
    object::fixed_set(rt, p, field::HANDLER, NULL);
}

/// IsConstructor (§7.2.4) of `p` when it is not a FUNCTION (`object::is_constructor` asks
/// this through `bound::is_constructor`): a bound function's or a proxy's is its target's,
/// down a chain of proxies and bound functions in a loop (no Rust recursion). A revoked
/// proxy keeps the answer it had at creation (§10.5.14 step 7.b; [`revoke`] stores it in the
/// target field), so revoking a proxy down the chain changes no outer answer. Not a GC point.
pub(crate) fn is_constructor(p: u64) -> bool {
    let mut f = p;
    loop {
        let k = kind::cell_kind(f);
        if k == kind::PROXY_CALLABLE.get() {
            f = target(f);
            if !is_object(f) {
                return f == REVOKED_CONSTRUCTOR;
            }
        } else if k == kind::BOUND_FUNCTION.get() {
            f = crate::bound::innermost(f);
        } else {
            return k == kind::FUNCTION.get() && object::function_info(f).is_constructor();
        }
    }
}

/// The most proxy internal methods that may run at once on one thread (nested, each
/// waiting for its trap or its target's method): past it a method throws the RangeError
/// "Maximum call stack size exceeded". Above the depth node v24.22.1 reaches in a chain of
/// plain proxies (`'x' in p` passes 4,000 nested proxies and throws at 8,000), so the native
/// stack check usually ends a deep chain first; this bound ends the loop that the stack check
/// cannot see (module doc).
pub(crate) const MAX_FORWARD_DEPTH: u32 = 10_000;

thread_local! {
    /// Proxy internal methods now running on this thread ([`Forward`]). Per thread, not per
    /// isolate: the nesting is on the native stack, which one thread's isolates share.
    static FORWARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// One level of forwarding depth, held while a proxy internal method runs its trap or its
/// target's method; its `Drop` gives the level back (also on an error return).
struct Forward;

impl Forward {
    /// Takes one level, or `None` at [`MAX_FORWARD_DEPTH`].
    #[inline]
    fn enter() -> Option<Forward> {
        FORWARD_DEPTH.with(|d| {
            let n = d.get();
            if n >= MAX_FORWARD_DEPTH {
                return None;
            }
            d.set(n + 1);
            Some(Forward)
        })
    }
}

impl Drop for Forward {
    #[inline]
    fn drop(&mut self) {
        FORWARD_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// The forwarding depth now in use on this thread (tests: 0 outside any proxy method).
#[cfg(test)]
pub(crate) fn forward_depth() -> u32 {
    FORWARD_DEPTH.with(Cell::get)
}

/// A proxy internal method after its common start ([`trap`]): the target, the handler, the
/// trap (undefined when the handler has none), and the depth level, held until the method
/// returns.
struct Trap {
    target: u64,
    handler: u64,
    trap: u64,
    _level: Forward,
}

impl Trap {
    /// True if the handler has no trap: the method takes the target's ("If trap is
    /// undefined").
    #[inline]
    fn none(&self) -> bool {
        self.trap == UNDEFINED
    }
}

/// The common start of every internal method of the proxy `p`: the native stack check and
/// the forwarding depth check (both the RangeError "Maximum call stack size exceeded"), then
/// ValidateNonRevokedProxy (§10.5.15; TypeError with V8's text naming `name`), then
/// GetMethod(handler, name) (§7.3.11): undefined or null is no trap, a value that is not
/// callable is a TypeError. GC point (the trap name's atom, a getter on the handler).
fn trap(cx: &mut Cx, p: u64, name: &'static str) -> Res<Trap> {
    if !cx.rt().native_stack_ok() {
        return stack_overflow(cx);
    }
    let Some(level) = Forward::enter() else {
        return stack_overflow(cx);
    };
    let h = handler(p);
    if h == NULL {
        let msg = format!("Cannot perform '{name}' on a proxy that has been revoked");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let t = target(p);
    // GC point: `p`, `t` and `h` live in Rust locals. A getter on the handler may revoke
    // `p`; the steps go on with the target and handler read above (§10.5.x steps 2-4).
    let key = crate::atoms::intern_str(cx.rt(), name);
    let m = property::get_from(cx, h, key)?;
    let m = if m == UNDEFINED || m == NULL {
        UNDEFINED
    } else if is_callable(m) {
        m
    } else {
        // V8 names an object `#<Object>` here (its no-side-effect ToString).
        let what = if is_object(m) {
            Text("#<Object>")
        } else {
            Key(m)
        };
        let msg = bounded_message(&[
            Text("'"),
            what,
            Text("' returned for property '"),
            Text(name),
            Text("' of object '#<Object>' is not a function"),
        ]);
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    };
    Ok(Trap {
        target: t,
        handler: h,
        trap: m,
        _level: level,
    })
}

/// A TypeError with the text `'<trap>' on proxy: <rest>` (V8's form).
fn invariant(cx: &mut Cx, parts: &[crate::error::Part<'_>]) -> Throw {
    let msg = bounded_message(parts);
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// Call(trap, handler, args) of a [`Trap`] with a defined trap. GC point, runs JS.
#[inline]
fn call_trap(cx: &mut Cx, t: &Trap, args: &[u64]) -> Res<u64> {
    call::call_function(cx, t.trap, t.handler, args)
}

/// IsExtensible(target) (§7.2.5) for any object. GC point.
#[inline]
fn target_extensible(cx: &mut Cx, target: u64) -> Res<bool> {
    property::try_is_extensible(cx, target)
}

/// `target.[[GetOwnProperty]](key)` as a complete descriptor, or `None`. GC point.
#[inline]
fn target_desc(cx: &mut Cx, target: u64, key: u64) -> Res<Option<Desc>> {
    property::get_own_property(cx, target, key)
}

/// IsCompatiblePropertyDescriptor(extensible, desc, current) (§10.1.6.2): the check of
/// ValidateAndApplyPropertyDescriptor with no object.
fn is_compatible(extensible: bool, desc: &Desc, current: Option<&Desc>) -> bool {
    match current {
        None => extensible,
        Some(c) => property::validate(desc, c),
    }
}

/// ToPropertyDescriptor(obj) (§6.2.6.5) of the object `o`, in the spec's field order
/// (enumerable, configurable, value, writable, get, set; HasProperty then Get each). GC
/// point, runs JS (getters, a proxy's traps).
fn to_property_descriptor(cx: &mut Cx, o: u64) -> Res<Desc> {
    debug_assert!(is_object(o));
    let mut d = Desc::default();
    for name in [
        "enumerable",
        "configurable",
        "value",
        "writable",
        "get",
        "set",
    ] {
        // GC point: `o` and the values in `d` live in Rust locals.
        let key = crate::atoms::intern_str(cx.rt(), name);
        if !property::try_has_property(cx, o, key)? {
            continue;
        }
        let v = property::get_from(cx, o, key)?;
        match name {
            "enumerable" => d.enumerable = Some(to_boolean(v)),
            "configurable" => d.configurable = Some(to_boolean(v)),
            "value" => d.value = Some(v),
            "writable" => d.writable = Some(to_boolean(v)),
            _ => {
                if v != UNDEFINED && !is_callable(v) {
                    let which = if name == "get" { "Getter" } else { "Setter" };
                    let msg =
                        bounded_message(&[Text(which), Text(" must be a function: "), Key(v)]);
                    return Err(throw_error(cx, ErrorKind::TypeError, &msg));
                }
                if name == "get" {
                    d.get = Some(v);
                } else {
                    d.set = Some(v);
                }
            }
        }
    }
    if d.is_accessor() && d.is_data() {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Invalid property descriptor. Cannot both specify accessors and a value or \
             writable attribute",
        ));
    }
    Ok(d)
}

/// FromPropertyDescriptor(desc) (§6.2.6.4): a new object (%Object.prototype%) with the
/// present fields in the order value, writable, get, set, enumerable, configurable. GC point.
fn from_property_descriptor(cx: &mut Cx, d: &Desc) -> Res<u64> {
    let o = object::new_plain_object(cx.rt());
    let b = crate::conv::boolean;
    let fields = [
        ("value", d.value),
        ("writable", d.writable.map(b)),
        ("get", d.get),
        ("set", d.set),
        ("enumerable", d.enumerable.map(b)),
        ("configurable", d.configurable.map(b)),
    ];
    for (name, v) in fields {
        if let Some(v) = v {
            // GC point: `o` and `d`'s values live in Rust locals.
            let key = crate::atoms::intern_str(cx.rt(), name);
            property::define(cx, o, key, v)?;
        }
    }
    Ok(o)
}

/// CompletePropertyDescriptor(desc) (§6.2.6.6): a generic or data descriptor gets value
/// undefined and writable false; an accessor gets get and set undefined; both get enumerable
/// and configurable false.
fn complete(d: &mut Desc) {
    if d.is_accessor() {
        d.get.get_or_insert(UNDEFINED);
        d.set.get_or_insert(UNDEFINED);
    } else {
        d.value.get_or_insert(UNDEFINED);
        d.writable.get_or_insert(false);
    }
    d.enumerable.get_or_insert(false);
    d.configurable.get_or_insert(false);
}

/// `p.[[GetPrototypeOf]]()` (§10.5.1). GC point.
pub(crate) fn get_prototype_of(cx: &mut Cx, p: u64) -> Res<u64> {
    let t = trap(cx, p, "getPrototypeOf")?;
    // 7. If trap is undefined, return ? target.[[GetPrototypeOf]]().
    if t.none() {
        return property::try_get_prototype_of(cx, t.target);
    }
    // 8. Let handlerProto be ? Call(trap, handler, « target »).
    let handler_proto = call_trap(cx, &t, &[t.target])?;
    // 9. If handlerProto is not an Object and handlerProto is not null, throw a TypeError.
    if handler_proto != NULL && !is_object(handler_proto) {
        return Err(invariant(
            cx,
            &[Text(
                "'getPrototypeOf' on proxy: trap returned neither object nor null",
            )],
        ));
    }
    // 10-11. If ? IsExtensible(target) is true, return handlerProto.
    if target_extensible(cx, t.target)? {
        return Ok(handler_proto);
    }
    // 12-13. If SameValue(handlerProto, ? target.[[GetPrototypeOf]]()) is false, throw.
    let target_proto = property::try_get_prototype_of(cx, t.target)?;
    if handler_proto != target_proto {
        return Err(invariant(
            cx,
            &[Text(
                "'getPrototypeOf' on proxy: proxy target is non-extensible but the trap did \
                 not return its actual prototype",
            )],
        ));
    }
    // 14. Return handlerProto.
    Ok(handler_proto)
}

/// `p.[[SetPrototypeOf]](v)` (§10.5.2); `v` is an object or null. GC point.
pub(crate) fn set_prototype_of(cx: &mut Cx, p: u64, v: u64) -> Res<bool> {
    let t = trap(cx, p, "setPrototypeOf")?;
    // 7. If trap is undefined, return ? target.[[SetPrototypeOf]](V).
    if t.none() {
        return property::try_set_prototype_of(cx, t.target, v);
    }
    // 8. Let booleanTrapResult be ToBoolean(? Call(trap, handler, « target, V »)).
    // 9. If booleanTrapResult is false, return false.
    if !to_boolean(call_trap(cx, &t, &[t.target, v])?) {
        return Ok(false);
    }
    // 10-11. If ? IsExtensible(target) is true, return true.
    if target_extensible(cx, t.target)? {
        return Ok(true);
    }
    // 12-13. If SameValue(V, ? target.[[GetPrototypeOf]]()) is false, throw a TypeError.
    if property::try_get_prototype_of(cx, t.target)? != v {
        return Err(invariant(
            cx,
            &[Text(
                "'setPrototypeOf' on proxy: trap returned truish for setting a new prototype \
                 on the non-extensible proxy target",
            )],
        ));
    }
    // 14. Return true.
    Ok(true)
}

/// `p.[[IsExtensible]]()` (§10.5.3). GC point.
pub(crate) fn is_extensible(cx: &mut Cx, p: u64) -> Res<bool> {
    let t = trap(cx, p, "isExtensible")?;
    // 6. If trap is undefined, return ? IsExtensible(target).
    if t.none() {
        return target_extensible(cx, t.target);
    }
    // 7. Let booleanTrapResult be ToBoolean(? Call(trap, handler, « target »)).
    let result = to_boolean(call_trap(cx, &t, &[t.target])?);
    // 8-9. If booleanTrapResult is not ? IsExtensible(target), throw a TypeError.
    let target_result = target_extensible(cx, t.target)?;
    if result != target_result {
        let which = if target_result { "true" } else { "false" };
        return Err(invariant(
            cx,
            &[
                Text(
                    "'isExtensible' on proxy: trap result does not reflect extensibility of \
                     proxy target (which is '",
                ),
                Text(which),
                Text("')"),
            ],
        ));
    }
    // 10. Return booleanTrapResult.
    Ok(result)
}

/// `p.[[PreventExtensions]]()` (§10.5.4). GC point.
pub(crate) fn prevent_extensions(cx: &mut Cx, p: u64) -> Res<bool> {
    let t = trap(cx, p, "preventExtensions")?;
    // 6. If trap is undefined, return ? target.[[PreventExtensions]]().
    if t.none() {
        return property::try_prevent_extensions(cx, t.target);
    }
    // 7. Let booleanTrapResult be ToBoolean(? Call(trap, handler, « target »)).
    let result = to_boolean(call_trap(cx, &t, &[t.target])?);
    // 8. If booleanTrapResult is true, then: if ? IsExtensible(target) is true, throw.
    if result && target_extensible(cx, t.target)? {
        return Err(invariant(
            cx,
            &[Text(
                "'preventExtensions' on proxy: trap returned truish but the proxy target is \
                 extensible",
            )],
        ));
    }
    // 9. Return booleanTrapResult.
    Ok(result)
}

/// `p.[[GetOwnProperty]](key)` (§10.5.5) as (value, attribute bits) in the stored form of
/// `property.rs` (an accessor's value is a new ACCESSOR_PAIR), or `None`. GC point.
pub(crate) fn get_own_property(cx: &mut Cx, p: u64, key: u64) -> Res<Option<(u64, i32)>> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "getOwnPropertyDescriptor")?;
    // 7. If trap is undefined, return ? target.[[GetOwnProperty]](P).
    if t.none() {
        return property::own_property_stored(cx, t.target, key);
    }
    // 8. Let trapResultObj be ? Call(trap, handler, « target, P »).
    let result = call_trap(cx, &t, &[t.target, key])?;
    // 9. If trapResultObj is not an Object and not undefined, throw a TypeError.
    if result != UNDEFINED && !is_object(result) {
        return Err(invariant(
            cx,
            &[
                Text(
                    "'getOwnPropertyDescriptor' on proxy: trap returned neither object nor \
                     undefined for property '",
                ),
                Key(key),
                Text("'"),
            ],
        ));
    }
    // 10. Let targetDesc be ? target.[[GetOwnProperty]](P).
    let target_desc = target_desc(cx, t.target, key)?;
    // 11. If trapResultObj is undefined, then
    if result == UNDEFINED {
        let Some(td) = target_desc else {
            // a. If targetDesc is undefined, return undefined.
            return Ok(None);
        };
        // b. If targetDesc.[[Configurable]] is false, throw a TypeError exception.
        if td.configurable == Some(false) {
            return Err(invariant(
                cx,
                &[
                    Text(
                        "'getOwnPropertyDescriptor' on proxy: trap returned undefined for property '",
                    ),
                    Key(key),
                    Text("' which is non-configurable in the proxy target"),
                ],
            ));
        }
        // c-d. If ? IsExtensible(target) is false, throw a TypeError exception.
        if !target_extensible(cx, t.target)? {
            return Err(invariant(
                cx,
                &[
                    Text(
                        "'getOwnPropertyDescriptor' on proxy: trap returned undefined for property '",
                    ),
                    Key(key),
                    Text("' which exists in the non-extensible proxy target"),
                ],
            ));
        }
        // e. Return undefined.
        return Ok(None);
    }
    // 12. Let extensibleTarget be ? IsExtensible(target).
    let extensible = target_extensible(cx, t.target)?;
    // 13-14. Let resultDesc be ? ToPropertyDescriptor(trapResultObj); complete it.
    let mut rd = to_property_descriptor(cx, result)?;
    complete(&mut rd);
    // 15-16. IsCompatiblePropertyDescriptor(extensibleTarget, resultDesc, targetDesc).
    if !is_compatible(extensible, &rd, target_desc.as_ref()) {
        return Err(invariant(
            cx,
            &[
                Text(
                    "'getOwnPropertyDescriptor' on proxy: trap returned descriptor for property '",
                ),
                Key(key),
                Text("' that is incompatible with the existing property in the proxy target"),
            ],
        ));
    }
    // 17. If resultDesc.[[Configurable]] is false, then
    if rd.configurable == Some(false) {
        // a. If targetDesc is undefined or targetDesc.[[Configurable]] is true, throw.
        match target_desc {
            None
            | Some(Desc {
                configurable: Some(true),
                ..
            }) => {
                return Err(invariant(
                    cx,
                    &[
                        Text(
                            "'getOwnPropertyDescriptor' on proxy: trap reported non-configurability for property '",
                        ),
                        Key(key),
                        Text("' which is either non-existent or configurable in the proxy target"),
                    ],
                ));
            }
            Some(td) => {
                // b. If resultDesc has a [[Writable]] field and it is false, then: if
                //    targetDesc.[[Writable]] is true, throw a TypeError exception.
                if rd.writable == Some(false) && td.writable == Some(true) {
                    return Err(invariant(
                        cx,
                        &[
                            Text(
                                "'getOwnPropertyDescriptor' on proxy: trap reported non-configurable and non-writable for property '",
                            ),
                            Key(key),
                            Text("' which is non-configurable, writable in the proxy target"),
                        ],
                    ));
                }
            }
        }
    }
    // 18. Return resultDesc, in the stored form.
    let ec = |d: &Desc| {
        (if d.enumerable == Some(true) {
            attr::ENUMERABLE
        } else {
            0
        }) | (if d.configurable == Some(true) {
            attr::CONFIGURABLE
        } else {
            0
        })
    };
    if rd.is_accessor() {
        let get = rd.get.unwrap_or(UNDEFINED);
        let set = rd.set.unwrap_or(UNDEFINED);
        // GC point: `get` and `set` live in Rust locals.
        let pair = crate::accessor::new_pair(cx.rt(), get, set);
        return Ok(Some((pair, attr::ACCESSOR | ec(&rd))));
    }
    let w = if rd.writable == Some(true) {
        attr::WRITABLE
    } else {
        0
    };
    Ok(Some((rd.value.unwrap_or(UNDEFINED), w | ec(&rd))))
}

/// `p.[[DefineOwnProperty]](key, desc)` (§10.5.6). GC point.
pub(crate) fn define_own_property(cx: &mut Cx, p: u64, key: u64, desc: Desc) -> Res<bool> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "defineProperty")?;
    // 7. If trap is undefined, return ? target.[[DefineOwnProperty]](P, Desc).
    if t.none() {
        return property::define_own_property(cx, t.target, key, desc);
    }
    define_with_trap(cx, &t, key, desc)
}

/// CreateDataPropertyOrThrow on the proxy `p` (`property::define`), as V8 words its
/// rejection: the first proxy down the target chain whose handler has a `defineProperty`
/// trap answers (`Ok(Ok(result))`, and a false result gets the trap text); if none has one,
/// the first target that is not a proxy (`Ok(Err(target))`) takes the define, with its own
/// text. Each proxy on the way takes its start (`trap`) once, as the forwards of
/// `define_own_property` would. GC point.
pub(crate) fn define_by_trap(cx: &mut Cx, p: u64, key: u64, desc: Desc) -> Res<Result<bool, u64>> {
    debug_assert!(is_property_key(key));
    let mut p = p;
    loop {
        let t = trap(cx, p, "defineProperty")?;
        if !t.none() {
            return define_with_trap(cx, &t, key, desc).map(Ok);
        }
        // 7. Trap undefined: target.[[DefineOwnProperty]](P, Desc).
        if !is_proxy(t.target) {
            return Ok(Err(t.target));
        }
        p = t.target;
    }
}

/// §10.5.6 steps 8-17: the `defineProperty` trap of `t` and the invariant checks. GC point.
fn define_with_trap(cx: &mut Cx, t: &Trap, key: u64, desc: Desc) -> Res<bool> {
    // 8. Let descObj be FromPropertyDescriptor(Desc).
    let desc_obj = from_property_descriptor(cx, &desc)?;
    // 9-10. ToBoolean(? Call(trap, handler, « target, P, descObj »)); false: return false.
    if !to_boolean(call_trap(cx, t, &[t.target, key, desc_obj])?) {
        return Ok(false);
    }
    // 11. Let targetDesc be ? target.[[GetOwnProperty]](P).
    let target_desc = target_desc(cx, t.target, key)?;
    // 12. Let extensibleTarget be ? IsExtensible(target).
    let extensible = target_extensible(cx, t.target)?;
    // 13-14. settingConfigFalse: Desc has [[Configurable]] false.
    let setting_config_false = desc.configurable == Some(false);
    match target_desc {
        // 15. If targetDesc is undefined, then
        None => {
            // a. If extensibleTarget is false, throw a TypeError exception.
            if !extensible {
                return Err(invariant(
                    cx,
                    &[
                        Text(
                            "'defineProperty' on proxy: trap returned truish for adding property '",
                        ),
                        Key(key),
                        Text("'  to the non-extensible proxy target"),
                    ],
                ));
            }
            // b. If settingConfigFalse is true, throw a TypeError exception.
            if setting_config_false {
                return Err(non_configurable_define(cx, key));
            }
        }
        // 16. Else,
        Some(td) => {
            // a. If IsCompatiblePropertyDescriptor(extensibleTarget, Desc, targetDesc) is
            //    false, throw a TypeError exception.
            if !is_compatible(extensible, &desc, Some(&td)) {
                return Err(invariant(
                    cx,
                    &[
                        Text(
                            "'defineProperty' on proxy: trap returned truish for adding property '",
                        ),
                        Key(key),
                        Text(
                            "'  that is incompatible with the existing property in the proxy target",
                        ),
                    ],
                ));
            }
            // b. If settingConfigFalse is true and targetDesc.[[Configurable]] is true, throw.
            if setting_config_false && td.configurable == Some(true) {
                return Err(non_configurable_define(cx, key));
            }
            // c. If IsDataDescriptor(targetDesc) is true, targetDesc.[[Configurable]] is
            //    false, and targetDesc.[[Writable]] is true, then: if Desc has a [[Writable]]
            //    field and Desc.[[Writable]] is false, throw a TypeError exception.
            if td.is_data()
                && td.configurable == Some(false)
                && td.writable == Some(true)
                && desc.writable == Some(false)
            {
                return Err(invariant(
                    cx,
                    &[
                        Text(
                            "'defineProperty' on proxy: trap returned truish for defining non-configurable property '",
                        ),
                        Key(key),
                        Text(
                            "' which cannot be non-writable, unless there exists a corresponding non-configurable, non-writable own property of the target object.",
                        ),
                    ],
                ));
            }
        }
    }
    // 17. Return true.
    Ok(true)
}

/// The TypeError of CreateDataPropertyOrThrow on a proxy whose `defineProperty` trap returned
/// false (`property::define`; V8's text).
pub(crate) fn define_falsish(cx: &mut Cx, key: u64) -> Throw {
    invariant(
        cx,
        &[
            Text("'defineProperty' on proxy: trap returned falsish for property '"),
            Key(key),
            Text("'"),
        ],
    )
}

/// The TypeError of §10.5.6 steps 15.b and 16.b.
fn non_configurable_define(cx: &mut Cx, key: u64) -> Throw {
    invariant(
        cx,
        &[
            Text(
                "'defineProperty' on proxy: trap returned truish for defining non-configurable property '",
            ),
            Key(key),
            Text("' which is either non-existent or configurable in the proxy target"),
        ],
    )
}

/// `p.[[HasProperty]](key)` (§10.5.7). GC point.
pub(crate) fn has_property(cx: &mut Cx, p: u64, key: u64) -> Res<bool> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "has")?;
    // 7. If trap is undefined, return ? target.[[HasProperty]](P).
    if t.none() {
        return property::try_has_property(cx, t.target, key);
    }
    // 8. Let booleanTrapResult be ToBoolean(? Call(trap, handler, « target, P »)).
    let result = to_boolean(call_trap(cx, &t, &[t.target, key])?);
    // 9. If booleanTrapResult is false, then
    if !result && let Some(td) = target_desc(cx, t.target, key)? {
        // b.i. If targetDesc.[[Configurable]] is false, throw a TypeError exception.
        if td.configurable == Some(false) {
            return Err(invariant(
                cx,
                &[
                    Text("'has' on proxy: trap returned falsish for property '"),
                    Key(key),
                    Text("' which exists in the proxy target as non-configurable"),
                ],
            ));
        }
        // b.ii-iii. If ? IsExtensible(target) is false, throw a TypeError exception.
        if !target_extensible(cx, t.target)? {
            return Err(invariant(
                cx,
                &[
                    Text("'has' on proxy: trap returned falsish for property '"),
                    Key(key),
                    Text("' but the proxy target is not extensible"),
                ],
            ));
        }
    }
    // 10. Return booleanTrapResult.
    Ok(result)
}

/// `p.[[Get]](key, receiver)` (§10.5.8). GC point.
pub(crate) fn get(cx: &mut Cx, p: u64, key: u64, receiver: u64) -> Res<u64> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "get")?;
    // 7. If trap is undefined, return ? target.[[Get]](P, Receiver).
    if t.none() {
        return property::get_with_receiver(cx, t.target, key, receiver);
    }
    // 8. Let trapResult be ? Call(trap, handler, « target, P, Receiver »).
    let result = call_trap(cx, &t, &[t.target, key, receiver])?;
    // 9. Let targetDesc be ? target.[[GetOwnProperty]](P).
    // 10. If targetDesc is not undefined and targetDesc.[[Configurable]] is false, then
    if let Some(td) = target_desc(cx, t.target, key)?
        && td.configurable == Some(false)
    {
        // a. A non-writable data property: SameValue(trapResult, targetDesc.[[Value]]).
        if td.is_data() && td.writable == Some(false) {
            let v = td.value.unwrap_or(UNDEFINED);
            if !property::same_value(result, v) {
                return Err(invariant(
                    cx,
                    &[
                        Text("'get' on proxy: property '"),
                        Key(key),
                        Text(
                            "' is a read-only and non-configurable data property on the proxy target but the proxy did not return its actual value (expected '",
                        ),
                        Key(v),
                        Text("' but got '"),
                        Key(result),
                        Text("')"),
                    ],
                ));
            }
        }
        // b. An accessor whose [[Get]] is undefined: trapResult must be undefined.
        if td.is_accessor() && td.get.unwrap_or(UNDEFINED) == UNDEFINED && result != UNDEFINED {
            return Err(invariant(
                cx,
                &[
                    Text("'get' on proxy: property '"),
                    Key(key),
                    Text(
                        "' is a non-configurable accessor property on the proxy target and does not have a getter function, but the trap did not return 'undefined' (got '",
                    ),
                    Key(result),
                    Text("')"),
                ],
            ));
        }
    }
    // 11. Return trapResult.
    Ok(result)
}

/// `p.[[Set]](key, v, receiver)` (§10.5.9). GC point.
pub(crate) fn set(cx: &mut Cx, p: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "set")?;
    // 7. If trap is undefined, return ? target.[[Set]](P, V, Receiver).
    if t.none() {
        return property::set_with_receiver(cx, t.target, key, v, receiver);
    }
    // 8-9. ToBoolean(? Call(trap, handler, « target, P, V, Receiver »)); false: return false.
    if !to_boolean(call_trap(cx, &t, &[t.target, key, v, receiver])?) {
        return Ok(false);
    }
    // 10. Let targetDesc be ? target.[[GetOwnProperty]](P).
    // 11. If targetDesc is not undefined and targetDesc.[[Configurable]] is false, then
    if let Some(td) = target_desc(cx, t.target, key)?
        && td.configurable == Some(false)
    {
        // a. A non-writable data property: SameValue(V, targetDesc.[[Value]]).
        if td.is_data()
            && td.writable == Some(false)
            && !property::same_value(v, td.value.unwrap_or(UNDEFINED))
        {
            return Err(invariant(
                cx,
                &[
                    Text("'set' on proxy: trap returned truish for property '"),
                    Key(key),
                    Text(
                        "' which exists in the proxy target as a non-configurable and non-writable data property with a different value",
                    ),
                ],
            ));
        }
        // b. An accessor whose [[Set]] is undefined.
        if td.is_accessor() && td.set.unwrap_or(UNDEFINED) == UNDEFINED {
            return Err(invariant(
                cx,
                &[
                    Text("'set' on proxy: trap returned truish for property '"),
                    Key(key),
                    Text(
                        "' which exists in the proxy target as a non-configurable and non-writable accessor property without a setter",
                    ),
                ],
            ));
        }
    }
    // 12. Return true.
    Ok(true)
}

/// `p.[[Delete]](key)` (§10.5.10). GC point.
pub(crate) fn delete(cx: &mut Cx, p: u64, key: u64) -> Res<bool> {
    debug_assert!(is_property_key(key));
    let t = trap(cx, p, "deleteProperty")?;
    // 7. If trap is undefined, return ? target.[[Delete]](P).
    if t.none() {
        return property::internal_delete(cx, t.target, key);
    }
    // 8-9. ToBoolean(? Call(trap, handler, « target, P »)); false: return false.
    if !to_boolean(call_trap(cx, &t, &[t.target, key])?) {
        return Ok(false);
    }
    // 10-11. Let targetDesc be ? target.[[GetOwnProperty]](P); undefined: return true.
    let Some(td) = target_desc(cx, t.target, key)? else {
        return Ok(true);
    };
    // 12. If targetDesc.[[Configurable]] is false, throw a TypeError exception.
    if td.configurable == Some(false) {
        return Err(invariant(
            cx,
            &[
                Text("'deleteProperty' on proxy: trap returned truish for property '"),
                Key(key),
                Text("' which is non-configurable in the proxy target"),
            ],
        ));
    }
    // 13-14. If ? IsExtensible(target) is false, throw a TypeError exception.
    if !target_extensible(cx, t.target)? {
        return Err(invariant(
            cx,
            &[
                Text("'deleteProperty' on proxy: trap returned truish for property '"),
                Key(key),
                Text("' but the proxy target is non-extensible"),
            ],
        ));
    }
    // 15. Return true.
    Ok(true)
}

/// One interrupt budget unit for a loop over a list of any length (a Rust loop has no back
/// edge that polls), and a poll when it runs out. `Err` on termination.
#[inline]
fn poll(cx: &mut Cx) -> Res<()> {
    cx.budget -= 1;
    if cx.budget <= 0 && crate::interp::slow::interrupt(cx, core::ptr::null_mut()) == 0 {
        return Err(Throw);
    }
    Ok(())
}

/// LengthOfArrayLike(o) (§7.3.20): ToLength(? Get(o, "length")). GC point.
fn length_of_array_like(cx: &mut Cx, o: u64) -> Res<f64> {
    let key = cx.rt().atom(crate::runtime::Atom::Length);
    let v = property::get_from(cx, o, key)?;
    let n = to_number(cx, v)?;
    // ToLength (§7.1.20): ToIntegerOrInfinity, clamped to [0, 2^53 - 1].
    let n = if n.is_nan() { 0.0 } else { n.trunc() };
    Ok(n.clamp(0.0, 9_007_199_254_740_991.0))
}

/// `p.[[OwnPropertyKeys]]()` (§10.5.11) as a new array of keys (strings interned). GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, p: u64) -> Res<u64> {
    let t = trap(cx, p, "ownKeys")?;
    // 7. If trap is undefined, return ? target.[[OwnPropertyKeys]]().
    if t.none() {
        return property::try_own_property_keys(cx, t.target);
    }
    // 8. Let trapResultArray be ? Call(trap, handler, « target »).
    let array = call_trap(cx, &t, &[t.target])?;
    // 9. Let trapResult be ? CreateListFromArrayLike(trapResultArray, property-key).
    //    The list is a new array (`keys`, a Rust local: a root); `seen` holds the bits of
    //    its keys (interned strings and symbols compare by bits).
    if !is_object(array) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "CreateListFromArrayLike called on non-object",
        ));
    }
    let len = length_of_array_like(cx, array)?;
    if len > f64::from(object::arr::MAX_LENGTH) {
        return Err(throw_error(
            cx,
            ErrorKind::RangeError,
            "Invalid array length",
        ));
    }
    let len = len as u32;
    let keys = object::new_array(cx.rt(), len as usize);
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for i in 0..len {
        poll(cx)?;
        // GC point: `array`, `keys` and the target live in Rust locals.
        let v = property::get_index(cx, array, u64::from(i))?;
        if !is_string(v) && !is_symbol(v) {
            let msg = bounded_message(&[Key(v), Text(" is not a valid property name")]);
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        // Not a GC point.
        let k = intern_key(cx.rt(), v);
        // 10. If trapResult contains any duplicate entries, throw a TypeError exception.
        if !seen.insert(k) {
            return Err(invariant(
                cx,
                &[Text("'ownKeys' on proxy: trap returned duplicate entries")],
            ));
        }
        object::array_set(cx.rt(), keys, i, k);
    }
    // 11. Let extensibleTarget be ? IsExtensible(target).
    let extensible = target_extensible(cx, t.target)?;
    // 12. Let targetKeys be ? target.[[OwnPropertyKeys]]().
    let target_keys = property::try_own_property_keys(cx, t.target)?;
    // 14-17. targetConfigurableKeys and targetNonconfigurableKeys (indices into targetKeys).
    //       Each key is interned in place first (not a GC point), so it compares by bits
    //       with the trap result's keys; `target_keys` (a Rust local) holds them all.
    let n = object::array_len(target_keys);
    let mut configurable: Vec<u32> = Vec::new();
    let mut nonconfigurable: Vec<u32> = Vec::new();
    for j in 0..n {
        poll(cx)?;
        let k = intern_key(
            cx.rt(),
            object::array_get(target_keys, j).unwrap_or(UNDEFINED),
        );
        object::array_set(cx.rt(), target_keys, j, k);
        // GC point: `target_keys` holds `k`.
        match target_desc(cx, t.target, k)? {
            Some(d) if d.configurable == Some(false) => nonconfigurable.push(j),
            _ => configurable.push(j),
        }
    }
    // 18. If extensibleTarget is true and targetNonconfigurableKeys is empty, return.
    if extensible && nonconfigurable.is_empty() {
        return Ok(keys);
    }
    // 19-21. Every non-configurable key must be in the trap result (removed as it is found).
    let mut unchecked = seen;
    let target_key = |j: u32| object::array_get(target_keys, j).unwrap_or(UNDEFINED);
    for &j in &nonconfigurable {
        let k = target_key(j);
        if !unchecked.remove(&k) {
            return Err(invariant(
                cx,
                &[
                    Text("'ownKeys' on proxy: trap result did not include '"),
                    Key(k),
                    Text("'"),
                ],
            ));
        }
    }
    // 22. If extensibleTarget is true, return trapResult.
    if extensible {
        return Ok(keys);
    }
    // 23. Every configurable key must be in the trap result too.
    for &j in &configurable {
        let k = target_key(j);
        if !unchecked.remove(&k) {
            return Err(invariant(
                cx,
                &[
                    Text("'ownKeys' on proxy: trap result did not include '"),
                    Key(k),
                    Text("'"),
                ],
            ));
        }
    }
    // 24. If uncheckedResultKeys is not empty, throw a TypeError exception.
    if !unchecked.is_empty() {
        return Err(invariant(
            cx,
            &[Text(
                "'ownKeys' on proxy: trap returned extra keys but proxy target is non-extensible",
            )],
        ));
    }
    // 25. Return trapResult.
    std::hint::black_box(target_keys);
    Ok(keys)
}

/// CreateArrayFromList(args) (§7.3.18): a new array of `args` (rooted by the caller: the VM
/// stack's scratch area). RangeError past the dense limit. GC point.
fn array_from_list(cx: &mut Cx, args: &[u64]) -> Res<u64> {
    if args.len() > object::arr::MAX_LENGTH as usize {
        return Err(throw_error(
            cx,
            ErrorKind::RangeError,
            "Invalid array length",
        ));
    }
    let rt = cx.rt();
    let a = object::new_array(rt, args.len());
    for (i, &v) in args.iter().enumerate() {
        // The capacity is `args.len()`: no growth, so no GC point inside the loop.
        object::array_set(rt, a, i as u32, v);
    }
    Ok(a)
}

/// `p.[[Call]](this, args)` (§10.5.12) of a callable proxy. `args` are rooted by the caller
/// (the VM stack's scratch area, `call::call_with`). GC point, runs JS.
pub(crate) fn call(cx: &mut Cx, p: u64, this: u64, args: &[u64]) -> Res<u64> {
    debug_assert!(kind::cell_kind(p) == kind::PROXY_CALLABLE.get());
    let t = trap(cx, p, "apply")?;
    // 7. If trap is undefined, return ? Call(target, thisArgument, argumentsList).
    if t.none() {
        return call::call_function(cx, t.target, this, args);
    }
    // 8. Let argArray be CreateArrayFromList(argumentsList).
    let arg_array = array_from_list(cx, args)?;
    // 9. Return ? Call(trap, handler, « target, thisArgument, argArray »).
    call_trap(cx, &t, &[t.target, this, arg_array])
}

/// `p.[[Construct]](args, newTarget)` (§10.5.13) of a callable proxy whose target is a
/// constructor (`call::construct_with` checked IsConstructor). `args` are rooted by the
/// caller. GC point, runs JS.
pub(crate) fn construct(cx: &mut Cx, p: u64, args: &[u64], new_target: u64) -> Res<u64> {
    debug_assert!(kind::cell_kind(p) == kind::PROXY_CALLABLE.get());
    let t = trap(cx, p, "construct")?;
    // 7. If trap is undefined, return ? Construct(target, argumentsList, newTarget).
    if t.none() {
        let argc = u32::try_from(args.len()).unwrap_or(u32::MAX);
        return call::construct_with(cx, t.target, argc, |i| args[i as usize], new_target);
    }
    // 8. Let argArray be CreateArrayFromList(argumentsList).
    let arg_array = array_from_list(cx, args)?;
    // 9. Let newObj be ? Call(trap, handler, « target, argArray, newTarget »).
    let new_obj = call_trap(cx, &t, &[t.target, arg_array, new_target])?;
    // 10. If newObj is not an Object, throw a TypeError exception.
    if !is_object(new_obj) {
        return Err(invariant(
            cx,
            &[
                Text("'construct' on proxy: trap returned non-object ('"),
                Key(new_obj),
                Text("')"),
            ],
        ));
    }
    // 11. Return newObj.
    Ok(new_obj)
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
