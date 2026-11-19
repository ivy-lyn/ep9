//! Tests of Proxy exotic objects (`proxy.rs`, kinds 27 and 28; M4-P4 PROXY1, plan v9 §6.3):
//! the routing (MC-w15-12: proxies are routed kinds, so the keyless index paths ask them), the
//! month-4 internal methods (the step for an undefined trap: the target's method), revocation,
//! `Cx::is_array_or_throw`, the fallible forms of `property.rs` and their callers, and the
//! debug asserts of the ordinary forms. The proxies here come from `proxy::proxy_create`
//! and the ABI (`Cx::proxy_create`, `Cx::proxy_revoke`); the traps from JS are tested in
//! `ep9_builtins` `tests/proxy.rs` (M5-P3 PROXY2), and the trap tests below call the methods
//! of `proxy.rs` from Rust, with and without GC stress.

use ep9_bytecode::VerifiedUnit;

use super::{Asm, config, isolate, on_big_stack, take_output, unit};
use crate::frame::NULL;
use crate::runtime::Runtime;
use crate::{
    Args, Cx, Dispatch, HostFn, Isolate, JsObject, JsResult, Local, PropertyDescriptor, exotic,
    kinds, object, property, proxy, shape,
};

/// Runs `f` with a builtin-ABI context of a fresh isolate. `stress`: a collection before
/// every allocation, then the heap verifier.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let out = {
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

/// A new proxy for `target` with an empty handler (`new Proxy(target, {})`).
fn new_proxy<'s>(cx: &mut Cx<'s>, target: impl Into<Local<'s>>) -> Local<'s, JsObject> {
    let target = target.into().bits();
    let handler = cx.new_object().bits();
    let p = proxy::proxy_create(cx.inner(), target, handler).expect("proxy_create");
    Local::from_bits(p)
}

/// The key atom of `name`.
fn atom(rt: &Runtime, name: &str) -> u64 {
    crate::atoms::intern_str(rt, name)
}

/// Takes the pending exception: "Name: message".
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// A host function object `name` (not on the global).
fn host_fn<'s>(rt: &Runtime, f: HostFn, name: &str) -> Local<'s> {
    Local::from_bits(crate::realm::new_host_function(rt, f, name, 0, false))
}

/// `getter`: returns its this value.
fn return_this<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

/// The keys of the array `keys` as Rust text.
fn key_texts<'s>(cx: &mut Cx<'s>, keys: Local<'s, JsObject>) -> Vec<String> {
    let n = object::array_len(keys.bits());
    (0..u64::from(n))
        .map(|i| {
            let k = cx.get_index(keys, i).unwrap();
            cx.to_rust_string(k).unwrap()
        })
        .collect()
}

/// The keys of the for-in key array `keys` that ForInNext visits for the receiver `o`.
fn visited<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, keys: Local<'s, JsObject>) -> Vec<String> {
    let n = object::array_len(keys.bits());
    (0..u64::from(n))
        .filter_map(|i| {
            let k = cx.get_index(keys, i).unwrap();
            let seen = property::for_in_has(cx.inner(), o.bits(), k.bits()).unwrap();
            seen.then(|| cx.to_rust_string(k).unwrap())
        })
        .collect()
}

// ---- routing ----------------------------------------------------------------------------------

/// MC-w15-12 (H31 fast-path row 3): the routed kinds are exactly ARGUMENTS, MODULE_NAMESPACE,
/// PROXY and PROXY_CALLABLE, so `property::own_index` refuses every kind that answers an
/// index through hooks; the proxy kinds are 27 and 28.
#[test]
fn routed_kinds_are_25_to_28_and_the_proxy_kinds_27_and_28() {
    for k in 0..=u8::MAX {
        assert_eq!(exotic::is_routed(k), (25..=28).contains(&k), "kind {k}");
        assert_eq!(proxy::is_proxy_kind(k), (27..=28).contains(&k), "kind {k}");
    }
    assert_eq!(
        (
            kinds::ARGUMENTS.get(),
            kinds::MODULE_NAMESPACE.get(),
            kinds::PROXY.get(),
            kinds::PROXY_CALLABLE.get()
        ),
        (25, 26, 27, 28)
    );
}

/// ProxyCreate: PROXY for an object target, PROXY_CALLABLE for a callable one (IsCallable is
/// a kind test), a TypeError for a target or handler that is not an object; the fields hold
/// the target and the handler across collections.
#[test]
fn proxy_create_picks_the_kind_and_keeps_its_fields() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let t = cx.new_object();
            let f = host_fn(rt, return_this, "f");
            let p = new_proxy(cx, t);
            let pc = new_proxy(cx, f);
            assert_eq!(kinds::cell_kind(p.bits()), kinds::PROXY.get());
            assert_eq!(kinds::cell_kind(pc.bits()), kinds::PROXY_CALLABLE.get());
            assert!(kinds::is_object(p.bits()) && !kinds::is_callable(p.bits()));
            assert!(cx.is_callable(pc) && !cx.is_callable(p));
            assert!(proxy::is_proxy(p.bits()) && proxy::is_proxy(pc.bits()));
            assert!(!proxy::is_proxy(t.bits()) && !proxy::is_proxy(Local::int32(1).bits()));
            // A proxy's own cell has a null prototype and no property.
            assert_eq!(object::proto_of(p.bits()), NULL);
            assert_eq!(object::own_count(p.bits()), 0);
            rt.heap.collect();
            assert_eq!(proxy::target(p.bits()), t.bits());
            assert_eq!(proxy::target(pc.bits()), f.bits());
            assert!(kinds::is_object(proxy::handler(p.bits())));
            for (target, handler) in [(Local::int32(1), t.into()), (t.into(), Local::null())] {
                let r = proxy::proxy_create(cx.inner(), target.bits(), handler.bits());
                assert!(r.is_err());
                assert_eq!(
                    take_error(cx, rt),
                    "TypeError: Cannot create proxy with a non-object as target or handler"
                );
            }
        });
    }
}

/// With no trap (month 4), every internal method of a proxy is the target's: the ABI
/// ObjectOps on the proxy read and change the target, and the proxy cell stays empty.
#[test]
fn a_proxy_forwards_every_internal_method_to_its_target() {
    for stress in [false, true] {
        with_cx(stress, |cx, _rt| {
            let proto = cx.new_object();
            cx.set(proto, "inherited", Local::int32(1), true).unwrap();
            let t = cx.new_object_with_proto(proto);
            let p = new_proxy(cx, t);
            // [[Set]] and [[Get]].
            assert!(cx.set(p, "x", Local::int32(7), true).is_ok());
            assert_eq!(cx.get(t, "x").unwrap().as_int32(), Some(7));
            assert_eq!(cx.get(p, "x").unwrap().as_int32(), Some(7));
            assert_eq!(cx.get(p, "inherited").unwrap().as_int32(), Some(1));
            // [[HasProperty]], [[GetOwnProperty]].
            assert!(cx.has_property(p, "x").unwrap());
            assert!(cx.has_property(p, "inherited").unwrap());
            assert!(!cx.has_property(p, "nothing").unwrap());
            assert!(cx.has_own_property(p, "x").unwrap());
            assert!(!cx.has_own_property(p, "inherited").unwrap());
            let d = cx.get_own_property(p, "x").unwrap().expect("x");
            assert_eq!(d.value.and_then(|v| v.as_int32()), Some(7));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(true), Some(true), Some(true))
            );
            // [[DefineOwnProperty]]: a read-only property on the target.
            let ro = PropertyDescriptor::data(Local::int32(3), false, false, true);
            assert!(cx.define_own_property(p, "ro", ro).unwrap());
            let d = cx
                .get_own_property(t, "ro")
                .unwrap()
                .expect("ro on the target");
            assert_eq!(d.writable, Some(false));
            // [[OwnPropertyKeys]]: the target's keys.
            let keys = cx.own_property_keys(p).unwrap();
            assert_eq!(key_texts(cx, keys.into()), ["x", "ro"]);
            // [[Delete]].
            assert!(cx.delete(p, "x").unwrap());
            assert!(!cx.has_own_property(t, "x").unwrap());
            // [[GetPrototypeOf]] and [[SetPrototypeOf]].
            assert!(cx.get_prototype_of(p).unwrap().is_same_bits(&proto));
            assert!(cx.set_prototype_of(p, Local::null()).unwrap());
            assert!(cx.get_prototype_of(t).unwrap().is_null());
            assert!(!cx.has_property(p, "inherited").unwrap());
            // [[IsExtensible]] and [[PreventExtensions]].
            assert!(cx.is_extensible(p).unwrap());
            assert!(cx.prevent_extensions(p).unwrap());
            assert!(!cx.is_extensible(t).unwrap() && !cx.is_extensible(p).unwrap());
            assert!(!cx.set_prototype_of(p, proto).unwrap(), "not extensible");
            // A failed define through the proxy: CreateDataPropertyOrThrow throws.
            let key = atom(cx.inner().rt(), "late");
            let v = Local::int32(1).bits();
            assert!(property::define(cx.inner(), p.bits(), key, v).is_err());
            let msg = take_error(cx, _rt);
            assert!(msg.starts_with("TypeError: "), "{msg}");
            // Nothing of it is stored in the proxy cell.
            assert_eq!(object::own_count(p.bits()), 0);
        });
    }
}

/// A proxy on the prototype chain: `[[Get]]` passes the original receiver to the target's
/// getter (§10.5.8 with Receiver), `[[Set]]` of a missing key adds it to the receiver, and
/// HasProperty asks the proxy for the rest of the chain.
#[test]
fn a_proxy_on_the_chain_sees_the_receiver() {
    with_cx(false, |cx, rt| {
        let t = cx.new_object();
        let getter = host_fn(rt, return_this, "getter");
        let desc = PropertyDescriptor {
            get: Some(getter),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        };
        cx.define_property_or_throw(t, "self", desc).unwrap();
        cx.set(t, "data", Local::int32(4), true).unwrap();
        let p = new_proxy(cx, t);
        let o = cx.new_object_with_proto(p);
        assert!(cx.get(o, "self").unwrap().is_same_bits(&o));
        assert!(cx.get(p, "self").unwrap().is_same_bits(&p));
        assert_eq!(cx.get(o, "data").unwrap().as_int32(), Some(4));
        assert!(cx.has_property(o, "data").unwrap());
        assert!(!cx.has_property(o, "nothing").unwrap());
        // [[Set]] walks into the proxy (step 1.b), whose target stores on the receiver.
        assert!(cx.set(o, "data", Local::int32(5), true).is_ok());
        assert_eq!(cx.get(t, "data").unwrap().as_int32(), Some(4));
        let own = cx.get_own_property(o, "data").unwrap().expect("own data");
        assert_eq!(own.value.and_then(|v| v.as_int32()), Some(5));
        // `for-in` lists the receiver's keys, then the proxy's (its target's) as candidates:
        // ForInNext's filter (`for_in_has`) asks the proxy's [[GetOwnProperty]] for each, so
        // the accessor `self` (not enumerable) is listed but not visited.
        let keys = property::for_in_keys(cx.inner(), o.bits()).unwrap();
        let keys: Local<'_, JsObject> = Local::from_bits(keys);
        assert_eq!(key_texts(cx, keys), ["data", "self"]);
        assert_eq!(visited(cx, o, keys), ["data"]);
        cx.set(t, "more", Local::int32(1), true).unwrap();
        let keys = property::for_in_keys(cx.inner(), o.bits()).unwrap();
        let keys: Local<'_, JsObject> = Local::from_bits(keys);
        assert_eq!(visited(cx, o, keys), ["data", "more"]);
    });
}

/// H31 fast-path row 3 (MC-w15-12): the keyless index paths reach `own_index`, which answers
/// `Unknown` for a proxy, so the key path asks it. A proxy that becomes a prototype sets the
/// sticky `proto_index` flag (a routed kind may answer an index), so an index read on its
/// child walks to it.
#[test]
fn the_keyless_index_paths_ask_a_proxy() {
    for stress in [false, true] {
        with_cx(stress, |cx, _rt| {
            let a = cx.new_array();
            for i in 0..3 {
                cx.set_index(a, i, Local::int32(10 + i as i32), true)
                    .unwrap();
            }
            let p = new_proxy(cx, a);
            assert_eq!(cx.get_index(p, 1).unwrap().as_int32(), Some(11));
            assert!(cx.get_index(p, 7).unwrap().as_int32().is_none());
            let (pb, ab) = (p.bits(), a.bits());
            assert!(property::try_has_index(cx.inner(), pb, 2).unwrap());
            assert!(!property::try_has_index(cx.inner(), pb, 3).unwrap());
            // `k in p` with an int32 key, and a keyed load.
            let two = Local::int32(2).bits();
            assert!(property::has(cx.inner(), pb, two).unwrap());
            let v = property::get_keyed(cx.inner(), pb, two).unwrap();
            assert_eq!(
                Local::<'_, crate::JsValue>::from_bits(v).as_int32(),
                Some(12)
            );
            // A store and a delete by index land on the target.
            assert!(cx.set_index(p, 3, Local::int32(13), true).unwrap());
            assert_eq!(
                object::array_get(ab, 3)
                    .map(|v| Local::<'_, crate::JsValue>::from_bits(v).as_int32()),
                Some(Some(13))
            );
            assert!(property::delete_index(cx.inner(), pb, 0, true).unwrap());
            assert!(object::array_get(ab, 0).is_none());
            // A child of the proxy: the walk reaches it.
            let o = cx.new_object_with_proto(p);
            assert!(
                shape::proto_index_keys(cx.inner().rt()),
                "a routed prototype"
            );
            assert_eq!(cx.get_index(o, 2).unwrap().as_int32(), Some(12));
            let ob = o.bits();
            assert!(property::try_has_index(cx.inner(), ob, 1).unwrap());
            assert!(
                !property::try_has_index(cx.inner(), ob, 0).unwrap(),
                "deleted"
            );
            assert!(property::has(cx.inner(), ob, two).unwrap());
        });
    }
}

/// A revoked proxy throws a TypeError in each internal method, with V8's text naming the trap.
#[test]
fn a_revoked_proxy_throws_in_every_internal_method() {
    with_cx(false, |cx, rt| {
        let t = cx.new_object();
        let p = new_proxy(cx, t);
        proxy::revoke(rt, p.bits());
        assert_eq!(proxy::target(p.bits()), NULL);
        let expect = |cx: &mut Cx<'_>, r: bool, trap: &str| {
            assert!(r, "{trap} must throw");
            assert_eq!(
                take_error(cx, rt),
                format!("TypeError: Cannot perform '{trap}' on a proxy that has been revoked")
            );
        };
        let r = cx.get(p, "x").is_err();
        expect(cx, r, "get");
        let r = cx.set(p, "x", Local::int32(1), false).is_err();
        expect(cx, r, "set");
        let r = cx.has_property(p, "x").is_err();
        expect(cx, r, "has");
        let r = cx.has_own_property(p, "x").is_err();
        expect(cx, r, "getOwnPropertyDescriptor");
        let r = cx.get_own_property(p, "x").is_err();
        expect(cx, r, "getOwnPropertyDescriptor");
        let d = PropertyDescriptor::data(Local::int32(1), true, true, true);
        let r = cx.define_own_property(p, "x", d).is_err();
        expect(cx, r, "defineProperty");
        let r = cx.delete(p, "x").is_err();
        expect(cx, r, "deleteProperty");
        let r = cx.own_property_keys(p).is_err();
        expect(cx, r, "ownKeys");
        let r = cx.get_prototype_of(p).is_err();
        expect(cx, r, "getPrototypeOf");
        let r = cx.set_prototype_of(p, Local::null()).is_err();
        expect(cx, r, "setPrototypeOf");
        let r = cx.is_extensible(p).is_err();
        expect(cx, r, "isExtensible");
        let r = cx.prevent_extensions(p).is_err();
        expect(cx, r, "preventExtensions");
        let r = cx.is_array_or_throw(p).is_err();
        expect(cx, r, "IsArray");
        // The frozen IsArray keeps its signature: false, no throw.
        assert!(!cx.is_array(p));
        // A revoked proxy on the chain: the walk throws there.
        let o = cx.new_object_with_proto(p);
        let r = cx.get(o, "x").is_err();
        expect(cx, r, "get");
        let r = cx.has_property(o, "x").is_err();
        expect(cx, r, "has");
    });
}

/// `Cx::is_array_or_throw` (V9-D12): an Array, and a proxy whose target (down a chain of
/// proxies) is an Array, are arrays; the frozen `Cx::is_array` says false for a proxy.
#[test]
fn is_array_or_throw_follows_the_proxy_target() {
    with_cx(false, |cx, _rt| {
        let a = cx.new_array();
        let o = cx.new_object();
        let pa = new_proxy(cx, a);
        let ppa = new_proxy(cx, pa);
        let po = new_proxy(cx, o);
        assert!(cx.is_array_or_throw(a).unwrap());
        assert!(cx.is_array_or_throw(pa).unwrap());
        assert!(cx.is_array_or_throw(ppa).unwrap());
        assert!(!cx.is_array_or_throw(po).unwrap());
        assert!(!cx.is_array_or_throw(o).unwrap());
        assert!(!cx.is_array_or_throw(Local::int32(1)).unwrap());
        assert!(cx.is_array(a) && !cx.is_array(pa));
    });
}

/// OrdinarySetPrototypeOf step 8.c.i: the cycle check stops at a proxy, so `a` may get a
/// proxy of itself as its prototype. A read of a missing key then recurses through the proxy
/// into `a` again; the native stack check or the forwarding depth bound of each proxy method
/// ends it in a RangeError (node v24.22.1: "Maximum call stack size exceeded"), in debug and
/// in release builds (R-GC-P4-4: this test never ended in release before the depth bound).
#[test]
fn the_cycle_check_stops_at_a_proxy_and_the_stack_check_ends_the_loop() {
    with_cx(false, |cx, rt| {
        let a = cx.new_object();
        let p = new_proxy(cx, a);
        assert!(cx.set_prototype_of(a, p).unwrap());
        assert!(cx.get_prototype_of(a).unwrap().is_same_bits(&p));
        assert!(cx.get(a, "missing").is_err());
        let msg = take_error(cx, rt);
        assert!(msg.starts_with("RangeError: "), "{msg}");
        assert!(cx.has_property(a, "missing").is_err());
        let msg = take_error(cx, rt);
        assert!(msg.starts_with("RangeError: "), "{msg}");
    });
}

/// OrdinaryHasInstance step 6 asks `[[GetPrototypeOf]]` of the value: a proxy whose target
/// inherits from `C.prototype` is an instance.
#[test]
fn ordinary_has_instance_asks_a_proxy_for_its_prototype() {
    with_cx(false, |cx, rt| {
        let c = host_fn(rt, return_this, "C");
        let c = c.as_object().unwrap();
        let cp = cx.new_object();
        cx.set(c, "prototype", cp, true).unwrap();
        let t = cx.new_object_with_proto(cp);
        let p = new_proxy(cx, t);
        let (cb, pb, tb) = (c.bits(), p.bits(), t.bits());
        assert!(property::ordinary_has_instance(cx.inner(), cb, tb).unwrap());
        assert!(property::ordinary_has_instance(cx.inner(), cb, pb).unwrap());
        let other = cx.new_object().bits();
        assert!(!property::ordinary_has_instance(cx.inner(), cb, other).unwrap());
    });
}

/// BoundFunctionCreate step 1: the prototype of a bound callable proxy is the proxy's
/// `[[GetPrototypeOf]]` (its target's).
#[test]
fn bind_takes_the_prototype_of_a_callable_proxy() {
    with_cx(false, |cx, rt| {
        let f = host_fn(rt, return_this, "f").as_object().unwrap();
        let fp = cx.new_object();
        assert!(cx.set_prototype_of(f, fp).unwrap());
        let p = new_proxy(cx, f);
        let b = crate::bound::new_bound_function(cx.inner(), p.bits(), NULL, &[]).unwrap();
        assert_eq!(object::proto_of(b), fp.bits());
    });
}

/// The fallible forms answer like the ordinary ones on objects that are not proxies.
#[test]
fn the_fallible_forms_agree_with_the_ordinary_ones() {
    with_cx(false, |cx, rt| {
        let proto = cx.new_object();
        cx.set(proto, "p", Local::int32(1), true).unwrap();
        let o = cx.new_object_with_proto(proto);
        cx.set(o, "x", Local::int32(1), true).unwrap();
        let (ob, pb) = (o.bits(), proto.bits());
        for name in ["x", "p", "z", "length"] {
            let key = atom(rt, name);
            let i = cx.inner();
            assert_eq!(
                property::try_has_property(i, ob, key).unwrap(),
                property::has_property(i, ob, key),
                "{name}"
            );
            assert_eq!(
                property::try_has_own_property(i, ob, key).unwrap(),
                property::has_own_property(i, ob, key),
                "{name}"
            );
            let how = crate::key::probe(key);
            assert_eq!(
                exotic::try_own_attrs(i, ob, key, how).unwrap(),
                exotic::own_attrs(rt, ob, key, how),
                "{name}"
            );
        }
        let i = cx.inner();
        assert_eq!(property::try_get_prototype_of(i, ob).unwrap(), pb);
        assert!(property::try_is_extensible(i, ob).unwrap());
        assert!(property::try_set_prototype_of(i, ob, NULL).unwrap());
        assert!(property::try_prevent_extensions(i, ob).unwrap());
        assert!(!property::is_extensible(ob));
        let keys = property::try_own_property_keys(i, ob).unwrap();
        assert_eq!(object::array_len(keys), 1);
    });
}

/// The ordinary (infallible) forms assert in debug builds that they never meet a proxy: a
/// caller that can meet one must take the fallible form (V9-A21).
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "use try_has_property")]
fn the_ordinary_has_property_asserts_not_a_proxy() {
    with_cx(false, |cx, rt| {
        let t = cx.new_object();
        let p = new_proxy(cx, t);
        let o = cx.new_object_with_proto(p);
        let key = atom(rt, "x");
        property::has_property(cx.inner(), o.bits(), key);
    });
}

/// See [`the_ordinary_has_property_asserts_not_a_proxy`].
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "try_own_property_keys")]
fn the_ordinary_own_property_keys_asserts_not_a_proxy() {
    with_cx(false, |cx, _rt| {
        let t = cx.new_object();
        let p = new_proxy(cx, t);
        property::own_property_keys(cx.inner(), p.bits());
    });
}

/// See [`the_ordinary_has_property_asserts_not_a_proxy`].
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "use try_is_extensible")]
fn the_ordinary_is_extensible_asserts_not_a_proxy() {
    with_cx(false, |cx, _rt| {
        let t = cx.new_object();
        let p = new_proxy(cx, t);
        property::is_extensible(p.bits());
    });
}

// ---- bounds on proxy cycles (M4-P4 fixer B; Codex PROXY1 P-1, P-2; R-GC-P4-4, R-VM-P4-2) ----

/// `a` whose prototype is `p = new Proxy(a, {})` (the cycle of the test above), with an own
/// property `a.a = 1` set before the cycle closes.
fn proxy_cycle<'s>(cx: &mut Cx<'s>) -> (Local<'s, JsObject>, Local<'s, JsObject>) {
    let a = cx.new_object();
    cx.set(a, "a", Local::int32(1), true).unwrap();
    let p = new_proxy(cx, a);
    assert!(cx.set_prototype_of(a, p).unwrap());
    (a, p)
}

/// The RangeError text node v24.22.1 gives for every operation on the cycle that does not
/// end (scratchpad m1m4/p4/fixB/js/cycle.js).
const STACK_TEXT: &str = "RangeError: Maximum call stack size exceeded";

/// A forward is never a tail call: each level holds its depth guard until the target's
/// method returns, so on the cycle `[[Get]]`, `[[HasProperty]]` and `[[Set]]` of a missing
/// key end in the RangeError on a 512 MiB stack too, in debug and release builds. Which stop
/// fires first is not pinned here: the depth bound (`proxy::MAX_FORWARD_DEPTH`) or the native
/// stack check (its limit is near the low end of the running thread's stack, `isolate.rs`), so
/// with the bound taken out this test still passes (M4-P4 verifier B, V-B2); the bound itself
/// is pinned by `a_chain_of_max_forward_depth_proxies_answers_and_one_more_throws`. With the
/// level given back at once (a tail call is possible again) this test runs past 60 s in
/// release (M4-P4 fixer C mutation run). Own keys answer before the walk reaches the proxy.
/// The depth count is back at 0 after each.
#[test]
fn a_proxy_cycle_ends_in_the_range_error_because_a_forward_is_never_a_tail_call() {
    on_big_stack(|| {
        with_cx(false, |cx, rt| {
            let (a, p) = proxy_cycle(cx);
            assert_eq!(cx.get(a, "a").unwrap().as_int32(), Some(1));
            assert!(cx.has_property(a, "a").unwrap());
            assert!(cx.get(a, "missing").is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert_eq!(proxy::forward_depth(), 0);
            assert!(cx.has_property(a, "missing").is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert!(cx.has_property(p, "missing").is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert!(cx.set(a, "missing", Local::int32(2), true).is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert_eq!(proxy::forward_depth(), 0);
        });
    });
}

/// The bound is exact: a chain of `MAX_FORWARD_DEPTH` proxies (each the target of the next)
/// answers `[[HasProperty]]` and `[[Get]]` from the object at its end; one more proxy throws
/// the RangeError. On a 512 MiB stack, so the native stack check does not fire first.
#[test]
fn a_chain_of_max_forward_depth_proxies_answers_and_one_more_throws() {
    on_big_stack(|| {
        with_cx(false, |cx, rt| {
            let t = cx.new_object();
            cx.set(t, "x", Local::int32(7), true).unwrap();
            let mut p = t;
            for _ in 0..proxy::MAX_FORWARD_DEPTH {
                p = new_proxy(cx, p);
            }
            assert!(cx.has_property(p, "x").unwrap());
            assert_eq!(cx.get(p, "x").unwrap().as_int32(), Some(7));
            let q = new_proxy(cx, p);
            assert!(cx.has_property(q, "x").is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert!(cx.get(q, "x").is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            assert_eq!(proxy::forward_depth(), 0);
        });
    });
}

/// The prototype walks of instanceof (OrdinaryHasInstance), for-in key collection and the
/// builtin step `Cx::prototype_walk_step` (`isPrototypeOf`, `__lookupGetter__`) end on the
/// cycle with the RangeError after `MAX_PROXY_HOPS` proxies, as V8's `PrototypeIterator`
/// does (node v24.22.1: `a instanceof C`, `Object.prototype.isPrototypeOf(a)` and
/// `a.__lookupGetter__('zz')` throw it). For-in over `a` (not a proxy) stops after the first
/// proxy and lists `a`, as node does (V10-D61, M5-P3 PROXY2); over `p` it walks on and throws.
/// No GC stress: for-in over `p` allocates one key array per proxy hop (102,400 of them), and
/// a collection before each would take minutes.
#[test]
fn the_prototype_walks_end_on_a_proxy_cycle() {
    {
        with_cx(false, |cx, rt| {
            let (a, p) = proxy_cycle(cx);
            let c = host_fn(rt, return_this, "C");
            let c = c.as_object().unwrap();
            let cp = cx.new_object();
            cx.set(c, "prototype", cp, true).unwrap();
            let (ab, pb, cb) = (a.bits(), p.bits(), c.bits());
            for o in [ab, pb] {
                assert!(property::ordinary_has_instance(cx.inner(), cb, o).is_err());
                assert_eq!(take_error(cx, rt), STACK_TEXT);
                assert!(property::instance_of(cx.inner(), o, cb).is_err());
                assert_eq!(take_error(cx, rt), STACK_TEXT);
            }
            let keys = property::for_in_keys(cx.inner(), ab).unwrap();
            assert_eq!(key_texts(cx, Local::from_bits(keys)), ["a"]);
            assert!(property::for_in_keys(cx.inner(), pb).is_err());
            assert_eq!(take_error(cx, rt), STACK_TEXT);
            // The builtin step: the hop count passes the bound, then the RangeError.
            let mut hops = 0;
            let mut o = a;
            let err = loop {
                match cx.prototype_walk_step(o, &mut hops) {
                    Ok(next) => o = next.as_object().expect("the cycle never reaches null"),
                    Err(_) => break take_error(cx, rt),
                }
            };
            assert_eq!(err, STACK_TEXT);
            assert_eq!(hops, property::MAX_PROXY_HOPS + 1);
        });
    }
}

/// The bound counts proxies only: an ordinary chain longer than `MAX_PROXY_HOPS` is walked to
/// its end by instanceof and the builtin step (no cycle can close without a proxy).
#[test]
fn a_long_ordinary_chain_is_not_cut_by_the_proxy_hop_bound() {
    with_cx(false, |cx, rt| {
        let c = host_fn(rt, return_this, "C");
        let c = c.as_object().unwrap();
        let cp = cx.new_object();
        cx.set(c, "prototype", cp, true).unwrap();
        let mut o = cx.new_object_with_proto(cp);
        for _ in 0..(property::MAX_PROXY_HOPS + 10) {
            o = cx.new_object_with_proto(o);
        }
        let (ob, cb) = (o.bits(), c.bits());
        assert!(property::ordinary_has_instance(cx.inner(), cb, ob).unwrap());
        let mut hops = 0;
        let mut steps = 0u32;
        let mut cur = o;
        while let Some(next) = cx.prototype_walk_step(cur, &mut hops).unwrap().as_object() {
            cur = next;
            steps += 1;
        }
        assert_eq!(hops, 0);
        // The chain: MAX + 10 objects, the first one, C.prototype, %Object.prototype%.
        assert_eq!(steps, property::MAX_PROXY_HOPS + 10 + 2);
    });
}

/// The walks poll for interrupts at each step: a termination requested before an instanceof
/// on the cycle stops it at the first poll (the interrupt budget, 10,000 steps), before the
/// proxy-hop bound, and nothing can catch it.
#[test]
fn a_termination_stops_a_prototype_walk_on_a_proxy_cycle() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let t = iso.terminate_handle();
    {
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        let (a, _p) = proxy_cycle(&mut cx);
        let c = host_fn(&iso.rt, return_this, "C").as_object().unwrap();
        let cp = cx.new_object();
        cx.set(c, "prototype", cp, true).unwrap();
        t.terminate();
        assert!(property::ordinary_has_instance(cx.inner(), c.bits(), a.bits()).is_err());
        assert!(
            iso.rt.terminating.get(),
            "a termination, not the RangeError"
        );
    }
}

// ---- Cx::has_index on proxies (M4-P4 fixer B; Codex PROXY1 P-3) -----------------------------

/// The index loop of `Array.prototype.forEach` (ep9_builtins `array.rs` `each`): length, then
/// HasProperty and Get of each index through the builtin ABI. The visited `index:value`.
fn for_each_like<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<Vec<String>> {
    let len = cx.get(o, "length")?;
    let len = cx.to_length(len)?;
    let mut seen = Vec::new();
    for k in 0..len {
        if cx.has_index(o, k)? {
            let v = cx.get_index(o, k)?;
            seen.push(format!("{k}:{}", cx.to_rust_string(v)?));
        }
    }
    Ok(seen)
}

/// `Cx::has_index` (the builtin ABI that forEach, filter, slice and the other Array builtins
/// call) asks a proxy, a proxy on the chain and a revoked proxy through `[[HasProperty]]`.
/// node v24.22.1 (scratchpad m1m4/p4/fixB/js/ic-proxy.js): `forEach` over
/// `new Proxy([1, , 3], {})` and over `Object.create` of it visits `0:1` and `2:3`; `1 in p`
/// of a revoked proxy throws "Cannot perform 'has' on a proxy that has been revoked".
#[test]
fn has_index_asks_a_live_an_inherited_and_a_revoked_proxy() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let arr = cx.new_array();
            cx.set_index(arr, 0, Local::int32(1), true).unwrap();
            cx.set_index(arr, 2, Local::int32(3), true).unwrap();
            let p = new_proxy(cx, arr);
            assert!(cx.has_index(p, 0).unwrap());
            assert!(!cx.has_index(p, 1).unwrap());
            assert!(cx.has_index(p, 2).unwrap());
            assert!(!cx.has_index(p, 3).unwrap());
            assert_eq!(for_each_like(cx, p).unwrap(), ["0:1", "2:3"]);
            let child = cx.new_object_with_proto(p);
            assert!(cx.has_index(child, 2).unwrap());
            assert!(!cx.has_index(child, 1).unwrap());
            assert_eq!(for_each_like(cx, child).unwrap(), ["0:1", "2:3"]);
            proxy::revoke(rt, p.bits());
            assert!(cx.has_index(p, 0).is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: Cannot perform 'has' on a proxy that has been revoked"
            );
            assert!(cx.has_index(child, 0).is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: Cannot perform 'has' on a proxy that has been revoked"
            );
            // forEach reads `length` first: node's text names 'get'.
            assert!(for_each_like(cx, p).is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: Cannot perform 'get' on a proxy that has been revoked"
            );
        });
    }
}

// ---- the named ICs and proxies (M4-P4 fixer B; R-VM-P4-1) -----------------------------------

/// Runs `setup` on a fresh isolate with dispatch `d`, then the script from `make`; the printed
/// lines, and "THROW ..." if it threw.
fn run_with_proxy(
    d: Dispatch,
    setup: impl for<'s> FnOnce(&mut Cx<'s>),
    make: fn() -> VerifiedUnit,
) -> Vec<String> {
    let mut iso = isolate(config(d, false));
    take_output();
    iso.with_cx_checkpoint(|cx| {
        setup(cx);
        Ok(())
    })
    .unwrap();
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(make(), "t.js").map(|v| cx.display(v)));
    let mut out = take_output();
    if let Err(e) = r {
        out.push(format!("THROW {e}"));
    }
    out
}

/// Globals `T = {x: 1}`, `P = new Proxy(T, {})`, `O = Object.create(P)`.
fn proxy_globals(cx: &mut Cx<'_>) {
    let t = cx.new_object();
    cx.set(t, "x", Local::int32(1), true).unwrap();
    let p = new_proxy(cx, t);
    let o = cx.new_object_with_proto(p);
    let g = cx.global();
    cx.set(g, "T", t, true).unwrap();
    cx.set(g, "P", p, true).unwrap();
    cx.set(g, "O", o, true).unwrap();
}

/// `for (i = 0; i < 4; i++) { if (i == 2) T.x = 2; print(<global>.x); }`, one load site.
fn load_x_four_times(global: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (o, t, i) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    s.lda_global(global);
    s.b.star(o);
    s.lda_global("T");
    s.b.star(t);
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    let skip = s.b.new_label();
    s.b.lda_smi(4).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.lda_smi(2)
        .test_equal_strict(i, s.cmp)
        .jump_if_false(skip);
    s.b.lda_smi(2);
    s.set_named(t, "x");
    s.b.bind(skip);
    s.get_named(o, "x");
    s.print_acc();
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// `function st(o, v) { o.y = v }`-like: one store site, run with `O1.y = 1` (an add the IC
/// may cache), then `arm()` gives `T` a setter `y` that prints, then `O2.y = 2` (the spec
/// runs the setter), then `print(O2.y)` (undefined: the accessor has no getter).
fn store_add_then_setter() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (o1, o2, o, v) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    s.lda_global("O1");
    s.b.star(o1);
    s.lda_global("O2");
    s.b.star(o2);
    s.b.ldar(o1).star(o);
    s.b.lda_smi(1).star(v);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    let second = s.b.new_label();
    s.b.lda_smi(3).test_less_than(v, s.cmp).jump_if_false(exit);
    s.b.ldar(v);
    s.set_named(o, "y");
    s.b.lda_smi(1)
        .test_equal_strict(v, s.cmp)
        .jump_if_false(second);
    s.lda_global("arm");
    s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    s.b.ldar(o2).star(o);
    s.b.bind(second);
    s.b.ldar(v).inc(s.bin).star(v).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
    s.get_named(o2, "y");
    s.print_acc();
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// `arm()`: defines `T.y` as an accessor whose setter is `print`.
fn arm<'s>(cx: &mut Cx<'s>, _this: Local<'s>, _a: Args<'s>, _nt: Local<'s>) -> JsResult<Local<'s>> {
    let g = cx.global();
    let t = cx.get(g, "T")?.as_object().unwrap();
    let pr = cx.get(g, "print")?;
    let desc = PropertyDescriptor {
        set: Some(pr),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    cx.define_property_or_throw(t, "y", desc)?;
    Ok(Local::undefined())
}

/// R-VM-P4-1: a named load from a proxy, or through a proxy on the chain, takes the generic
/// path at every run of the site (the IC never caches the proxy's empty cell as "missing"),
/// and a store does not cache an add past a proxy on the chain. node v24.22.1
/// (scratchpad m1m4/p4/fixB/js/ic-proxy.js): loads `[1, 1, 2, 2]`; the store runs the setter
/// (`set 2`), then `O2.y` is undefined. Every dispatch expansion.
#[test]
fn named_ics_take_the_generic_path_at_a_proxy() {
    for &d in Dispatch::ALL {
        let via_child = run_with_proxy(d, proxy_globals, || load_x_four_times("O"));
        assert_eq!(via_child, ["1", "1", "2", "2"], "{}", d.name());
        let on_proxy = run_with_proxy(d, proxy_globals, || load_x_four_times("P"));
        assert_eq!(on_proxy, ["1", "1", "2", "2"], "{}", d.name());
        let mut iso = isolate(config(d, false));
        iso.register_host_function("arm", 0, arm);
        take_output();
        iso.with_cx_checkpoint(|cx| {
            proxy_globals(cx);
            let g = cx.global();
            let p = cx.get(g, "P")?.as_object().unwrap();
            let o1 = cx.new_object_with_proto(p);
            let o2 = cx.new_object_with_proto(p);
            cx.set(g, "O1", o1, true)?;
            cx.set(g, "O2", o2, true)?;
            Ok(())
        })
        .unwrap();
        let r = iso.with_cx_checkpoint(|cx| {
            cx.run_script(store_add_then_setter(), "t.js")
                .map(|v| cx.display(v))
        });
        assert!(r.is_ok(), "{}: {r:?}", d.name());
        assert_eq!(take_output(), ["2", "undefined"], "{}", d.name());
    }
}

// ---- traps and the ABI (M5-P3 PROXY2) -------------------------------------------------------

/// A trap that returns its arguments as a new array (`[...arguments]`): it allocates, so with
/// GC stress a collection runs inside each trap call, while the method keeps the target, the
/// key and the receiver in Rust locals.
fn args_trap<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // A stack array (a Local never lives in Rust heap memory, H20 R2); the traps here get 3.
    let list = [args.get(0), args.get(1), args.get(2)];
    Ok(cx
        .create_array_from_list(&list[..args.len().min(3)])?
        .value())
}

/// An `ownKeys` trap: three new strings, "b", "a", "1" (each a new allocation).
fn own_keys_trap<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let b = cx.new_string("b")?.value();
    let a = cx.new_string("a")?.value();
    let one = cx.new_string("1")?.value();
    Ok(cx.create_array_from_list(&[b, a, one])?.value())
}

/// A handler whose traps `names` are `f`.
fn handler_of<'s>(cx: &mut Cx<'s>, rt: &Runtime, names: &[&str], f: HostFn) -> Local<'s, JsObject> {
    let h = cx.new_object();
    for &n in names {
        let t = host_fn(rt, f, n);
        cx.set(h, n, t, true).unwrap();
    }
    h
}

/// `[[Get]]`, `[[Call]]` and `[[Construct]]` call their traps with the spec's arguments
/// (§10.5.8: target, key, receiver; §10.5.12: target, this, an args array; §10.5.13: target,
/// an args array, newTarget), and `[[OwnPropertyKeys]]` takes the trap's list in its order
/// (§10.5.11; the target is extensible and has no non-configurable key). With and without GC
/// stress: each trap allocates.
#[test]
fn the_traps_get_the_spec_arguments_with_and_without_gc_stress() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let target = cx.new_object();
            let h = handler_of(cx, rt, &["get"], args_trap);
            let p = cx.proxy_create(target, h).unwrap();
            let r = cx.get(p, "k").unwrap().as_object().unwrap();
            assert_eq!(object::array_len(r.bits()), 3);
            let (t0, k1, r2) = (
                cx.get_index(r, 0).unwrap(),
                cx.get_index(r, 1).unwrap(),
                cx.get_index(r, 2).unwrap(),
            );
            assert!(cx.same_value(t0, target));
            assert_eq!(cx.to_rust_string(k1).unwrap(), "k");
            assert!(cx.same_value(r2, p));
            // The receiver of a get through the chain is the child (§10.1.8.1 step 3).
            let child = cx.new_object_with_proto(p);
            let r = cx.get(child, "k").unwrap().as_object().unwrap();
            let r2 = cx.get_index(r, 2).unwrap();
            assert!(cx.same_value(r2, child));

            let h = handler_of(cx, rt, &["ownKeys"], own_keys_trap);
            let p = cx.proxy_create(target, h).unwrap();
            let keys = cx.own_property_keys(p).unwrap();
            assert_eq!(key_texts(cx, keys.into()), ["b", "a", "1"]);

            let f = host_fn(rt, return_this, "f");
            let h = handler_of(cx, rt, &["apply"], args_trap);
            let pf = cx.proxy_create(f, h).unwrap();
            assert!(kinds::is_callable(pf.bits()));
            let this = cx.new_object();
            let r = cx
                .call(pf, this, &[Local::int32(1), Local::int32(2)])
                .unwrap();
            let r = r.as_object().unwrap();
            let (t0, this1, a2) = (
                cx.get_index(r, 0).unwrap(),
                cx.get_index(r, 1).unwrap(),
                cx.get_index(r, 2).unwrap(),
            );
            assert!(cx.same_value(t0, f));
            assert!(cx.same_value(this1, this));
            let a2 = a2.as_object().unwrap();
            assert_eq!(object::array_len(a2.bits()), 2);
            assert_eq!(cx.get_index(a2, 1).unwrap().as_int32(), Some(2));

            let c: Local<'_> = Local::from_bits(crate::realm::new_host_function(
                rt,
                return_this,
                "C",
                0,
                true,
            ));
            let h = handler_of(cx, rt, &["construct"], args_trap);
            let pc = cx.proxy_create(c, h).unwrap();
            assert!(cx.is_constructor(pc));
            let r = cx.construct(pc, &[Local::int32(5)], pc).unwrap();
            let (t0, a1, nt2) = (
                cx.get_index(r, 0).unwrap(),
                cx.get_index(r, 1).unwrap(),
                cx.get_index(r, 2).unwrap(),
            );
            assert!(cx.same_value(t0, c));
            let a1 = a1.as_object().unwrap();
            assert_eq!(cx.get_index(a1, 0).unwrap().as_int32(), Some(5));
            assert!(cx.same_value(nt2, pc));
        });
    }
}

/// `Cx::proxy_create` and `Cx::proxy_revoke` (H20 part 2, M5-P3 PROXY2): a TypeError for a
/// non-object target or handler; a proxy of a plain object is not callable; revoking twice,
/// or revoking an object that is not a proxy, changes nothing. A revoked callable proxy is
/// not a constructor here (its target is gone; the revoked-IsConstructor note in `proxy.rs`).
#[test]
fn the_abi_creates_and_revokes_a_proxy() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let t = cx.new_object();
            let h = cx.new_object();
            assert!(cx.proxy_create(Local::int32(1), h).is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: Cannot create proxy with a non-object as target or handler"
            );
            assert!(cx.proxy_create(t, Local::null()).is_err());
            take_error(cx, rt);
            let p = cx.proxy_create(t, h).unwrap();
            assert!(proxy::is_proxy(p.bits()));
            assert!(!kinds::is_callable(p.bits()));
            assert!(cx.set(p, "x", Local::int32(3), true).unwrap());
            assert_eq!(cx.get(t, "x").unwrap().as_int32(), Some(3));
            cx.proxy_revoke(p);
            cx.proxy_revoke(p);
            cx.proxy_revoke(t);
            assert!(cx.get(p, "x").is_err());
            assert_eq!(
                take_error(cx, rt),
                "TypeError: Cannot perform 'get' on a proxy that has been revoked"
            );
            assert_eq!(cx.get(t, "x").unwrap().as_int32(), Some(3));
            let c: Local<'_> = Local::from_bits(crate::realm::new_host_function(
                rt,
                return_this,
                "C",
                0,
                true,
            ));
            let pc = cx.proxy_create(c, h).unwrap();
            assert!(cx.is_constructor(pc));
            cx.proxy_revoke(pc);
            // §10.5.14 step 7.b: a revoked proxy keeps the [[Construct]] it got at creation.
            assert!(cx.is_constructor(pc));
        });
    }
}

// ---- the ownKeys duplicate check by content hash (M10-P0 HASH-SEED, HB-3) ------------------

/// The `ownKeys` duplicate error (§10.5.11 step 10).
const DUPLICATE: &str = "TypeError: 'ownKeys' on proxy: trap returned duplicate entries";

/// An `ownKeys` trap: `["a", "b", "a"]` (new strings, not atoms).
fn dup_strings_trap<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let a = cx.new_string("a")?.value();
    let b = cx.new_string("b")?.value();
    let a2 = cx.new_string("a")?.value();
    Ok(cx.create_array_from_list(&[a, b, a2])?.value())
}

/// An `ownKeys` trap: `[handler.sym, "x", handler.sym]`.
fn dup_symbol_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let sym = cx.get_v(this, "sym")?;
    let x = cx.new_string("x")?.value();
    Ok(cx.create_array_from_list(&[sym, x, sym])?.value())
}

/// An `ownKeys` trap: 40 new strings "k0" to "k39", then "k7" again if `handler.dup` is
/// true. Each string is a new allocation (a collection under GC stress).
fn many_keys_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let dup = cx.get_v(this, "dup")?;
    let dup = cx.to_boolean(dup);
    let list = cx.new_array();
    let n = if dup { 41 } else { 40 };
    for i in 0..n {
        let k = cx.new_string(&format!("k{}", if i == 40 { 7 } else { i }))?;
        cx.set(list, i.to_string().as_str(), k, true)?;
    }
    Ok(list.value())
}

std::thread_local! {
    /// The calls of the element-2 getters below.
    static GETTER_CALLS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// A getter: counts the call and returns 5 (not a property key).
fn count_then_5<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    GETTER_CALLS.with(|c| c.set(c.get() + 1));
    Ok(Local::int32(5))
}

/// A getter: counts the call and returns "b".
fn count_then_b<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    GETTER_CALLS.with(|c| c.set(c.get() + 1));
    Ok(cx.new_string("b")?.value())
}

/// An `ownKeys` trap: the array-like `{length: 3, 0: "a", 1: "a", get 2() {...}}`, whose
/// getter is `handler.getter`.
fn array_like_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let getter = cx.get_v(this, "getter")?;
    let o = cx.new_object();
    cx.set(o, "length", Local::int32(3), true)?;
    for i in ["0", "1"] {
        let a = cx.new_string("a")?;
        cx.set(o, i, a, true)?;
    }
    let desc = PropertyDescriptor {
        get: Some(getter),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    cx.define_property_or_throw(o, "2", desc)?;
    Ok(o.value())
}

/// HB-3: a trap result with a duplicate string (`["a", "b", "a"]`) or a duplicate symbol
/// throws the duplicate TypeError (§10.5.11 step 10), with and without GC stress.
#[test]
fn own_keys_duplicate_entries_still_throw() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let target = cx.new_object();
            let h = handler_of(cx, rt, &["ownKeys"], dup_strings_trap);
            let p = cx.proxy_create(target, h).unwrap();
            assert!(cx.own_property_keys(p).is_err());
            assert_eq!(take_error(cx, rt), DUPLICATE);

            let h = handler_of(cx, rt, &["ownKeys"], dup_symbol_trap);
            let sym = cx.new_symbol(Local::undefined()).unwrap();
            cx.set(h, "sym", sym, true).unwrap();
            let p = cx.proxy_create(target, h).unwrap();
            assert!(cx.own_property_keys(p).is_err());
            assert_eq!(take_error(cx, rt), DUPLICATE);
        });
    }
}

/// HB-3 with GC stress (a collection before every allocation): 40 keys, each a new string
/// that the check interns, give no false duplicate and keep their order; one more "k7"
/// throws the duplicate TypeError.
#[test]
fn own_keys_duplicate_check_under_gc_stress() {
    with_cx(true, |cx, rt| {
        let target = cx.new_object();
        let h = handler_of(cx, rt, &["ownKeys"], many_keys_trap);
        let p = cx.proxy_create(target, h).unwrap();
        let keys = cx.own_property_keys(p).unwrap();
        let want: Vec<String> = (0..40).map(|i| format!("k{i}")).collect();
        assert_eq!(key_texts(cx, keys.into()), want);
        cx.set(h, "dup", Local::boolean(true), true).unwrap();
        assert!(cx.own_property_keys(p).is_err());
        assert_eq!(take_error(cx, rt), DUPLICATE);
    });
}

/// HB-3, the spec order (§10.5.11 steps 9-10, §7.3.18 CreateListFromArrayLike): every element
/// is read and type-checked before the duplicate check. For `{length: 3, 0: "a", 1: "a",
/// get 2() {...}}` the getter runs once; if it returns 5 the TypeError is the one of element
/// 2, not the duplicate error; if it returns "b" the TypeError is the duplicate error (node
/// v26.13.1 gives both results).
#[test]
fn own_keys_reads_every_element_before_the_duplicate_check() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let target = cx.new_object();
            for (getter, want) in [
                (
                    count_then_5 as HostFn,
                    "TypeError: 5 is not a valid property name",
                ),
                (count_then_b as HostFn, DUPLICATE),
            ] {
                let h = handler_of(cx, rt, &["ownKeys"], array_like_trap);
                let g = host_fn(rt, getter, "get 2");
                cx.set(h, "getter", g, true).unwrap();
                let p = cx.proxy_create(target, h).unwrap();
                GETTER_CALLS.with(|c| c.set(0));
                assert!(cx.own_property_keys(p).is_err());
                assert_eq!(take_error(cx, rt), want);
                assert_eq!(GETTER_CALLS.with(core::cell::Cell::get), 1, "{want}");
            }
        });
    }
}

/// An `ownKeys` trap: returns `handler.list`.
fn list_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.get_v(this, "list")
}

/// HB-3, steps 19-24 (uncheckedResultKeys by (content hash, index), no address order): a
/// non-extensible target with "x" (non-configurable), "y" and a symbol. A trap result with the
/// same keys in a new order (new strings) passes and keeps its order; a result without "x",
/// without the symbol, or with one more key "z" throws the step 21, 23 or 24 TypeError. With
/// and without GC stress.
#[test]
fn own_keys_unchecked_keys_match_by_content_not_address() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let target = cx.new_object();
            let sym = cx.new_symbol(Local::undefined()).unwrap();
            let x = PropertyDescriptor::data(Local::int32(1), true, true, false);
            cx.define_property_or_throw(target, "x", x).unwrap();
            cx.set(target, "y", Local::int32(2), true).unwrap();
            cx.set(target, sym, Local::int32(3), true).unwrap();
            assert!(cx.prevent_extensions(target).unwrap());
            let missing = "TypeError: 'ownKeys' on proxy: trap result did not include '";
            let extra = "TypeError: 'ownKeys' on proxy: trap returned extra keys but proxy \
                         target is non-extensible";
            for (names, with_sym, want) in [
                (&["y", "x"][..], true, None),
                (&["y"][..], true, Some(format!("{missing}x'"))),
                (&["y", "x"][..], false, Some(missing.to_owned())),
                (&["y", "x", "z"][..], true, Some(extra.to_owned())),
            ] {
                let list = cx.new_array();
                let mut n = 0;
                for name in names {
                    let s = cx.new_string(name).unwrap();
                    cx.set(list, n.to_string().as_str(), s, true).unwrap();
                    n += 1;
                    if with_sym && n == 1 {
                        cx.set(list, "1", sym, true).unwrap();
                        n += 1;
                    }
                }
                let h = handler_of(cx, rt, &["ownKeys"], list_trap);
                cx.set(h, "list", list, true).unwrap();
                let p = cx.proxy_create(target, h).unwrap();
                match want {
                    None => {
                        let keys = cx.own_property_keys(p).unwrap();
                        assert_eq!(object::array_len(keys.bits()), 3);
                        let first = cx.get_index(keys, 0).unwrap();
                        assert_eq!(cx.to_rust_string(first).unwrap(), "y");
                        let second = cx.get_index(keys, 1).unwrap();
                        assert_eq!(second.bits(), sym.bits());
                        let third = cx.get_index(keys, 2).unwrap();
                        assert_eq!(cx.to_rust_string(third).unwrap(), "x");
                    }
                    Some(want) => {
                        assert!(cx.own_property_keys(p).is_err());
                        let got = take_error(cx, rt);
                        assert!(got.starts_with(&want), "{got} vs {want}");
                    }
                }
            }
        });
    }
}

// ---- the ownKeys duplicate pass polls (M10-P0 HASH-SEED, Codex r1 F1) ----------------------

/// A getter: counts the call, then requests a termination and sets the interrupt budget to 1,
/// so the next poll stops the isolate. Returns "c".
fn terminate_then_c<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    GETTER_CALLS.with(|c| c.set(c.get() + 1));
    let c = cx.new_string("c")?.value();
    let inner = cx.inner();
    inner.rt().terminate.fetch_or(
        crate::interp::interrupt::TERMINATE,
        std::sync::atomic::Ordering::Release,
    );
    inner.budget = 1;
    Ok(c)
}

/// An `ownKeys` trap: the array-like `{length: 3, 0: "a", 1: "b", get 2() {...}}`, whose
/// getter is `handler.getter`. No duplicate.
fn array_like_abc_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let getter = cx.get_v(this, "getter")?;
    let o = cx.new_object();
    cx.set(o, "length", Local::int32(3), true)?;
    for (i, k) in [("0", "a"), ("1", "b")] {
        let s = cx.new_string(k)?;
        cx.set(o, i, s, true)?;
    }
    let desc = PropertyDescriptor {
        get: Some(getter),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    cx.define_property_or_throw(o, "2", desc)?;
    Ok(o.value())
}

/// Codex r1 F1: the getter of the last trap-result element requests a termination (and the
/// budget runs out at the next poll). The pass after the read loop (step 10 and the
/// uncheckedResultKeys build) polls, so `[[OwnPropertyKeys]]` stops with the termination: for
/// an empty extensible target (no poll in steps 11-18) it does not return the keys, and for an
/// empty non-extensible target it does not throw the step 24 TypeError. With and without GC
/// stress.
#[test]
fn own_keys_stops_on_a_termination_from_the_last_getter() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            for extensible in [true, false] {
                let target = cx.new_object();
                if !extensible {
                    assert!(cx.prevent_extensions(target).unwrap());
                }
                let h = handler_of(cx, rt, &["ownKeys"], array_like_abc_trap);
                let g = host_fn(rt, terminate_then_c, "get 2");
                cx.set(h, "getter", g, true).unwrap();
                let p = cx.proxy_create(target, h).unwrap();
                GETTER_CALLS.with(|c| c.set(0));
                let failed = cx.own_property_keys(p).is_err();
                let terminating = rt.terminating.get();
                let pending = rt.take_pending();
                // Clear the request before the asserts, so a failure does not stop the next run.
                rt.terminating.set(false);
                rt.terminate.fetch_and(
                    !crate::interp::interrupt::TERMINATE,
                    std::sync::atomic::Ordering::AcqRel,
                );
                assert_eq!(GETTER_CALLS.with(core::cell::Cell::get), 1);
                assert!(failed, "extensible {extensible}: the keys came back");
                assert!(terminating, "extensible {extensible}: not a termination");
                assert_eq!(
                    pending,
                    crate::frame::UNDEFINED,
                    "extensible {extensible}: a TypeError, not the termination"
                );
            }
        });
    }
}

/// 2 different texts "h<i>" with the same 30-bit table hash (`string::hash_text`) under this
/// process's SipHash key: a birthday search (about 2^15 texts; any key works).
fn same_hash_texts() -> (String, String) {
    let mut by_hash = std::collections::HashMap::new();
    for i in 0..(1u32 << 24) {
        let s = format!("h{i}");
        let (_, h) = crate::string::hash_text(crate::string::JsStr::Latin1(s.as_bytes()));
        if let Some(t) = by_hash.insert(h, s.clone()) {
            return (t, s);
        }
    }
    panic!("no 30-bit hash collision in 2^24 texts");
}

/// Codex r1 next step: 2 different keys `a` and `b` with one content hash are one equal-hash
/// run of the step 10 pass and of uncheckedResultKeys (steps 19-24); the bits test keeps them
/// apart. A non-extensible target with `a` (non-configurable) and `b`: `[b, a]` passes in its
/// order; `[a, b, a]` throws the duplicate error; `[b]` misses `a` (step 21); `[a]` misses `b`
/// (step 23); `[b, a, "z"]` has an extra key (step 24). With and without GC stress.
#[test]
fn own_keys_keeps_apart_different_keys_with_one_hash() {
    let (ta, tb) = same_hash_texts();
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let (a, b) = (atom(rt, &ta), atom(rt, &tb));
            assert_ne!(a, b);
            assert_eq!(crate::string::hash(a), crate::string::hash(b));
            let target = cx.new_object();
            let x = PropertyDescriptor::data(Local::int32(1), true, true, false);
            cx.define_property_or_throw(target, ta.as_str(), x).unwrap();
            cx.set(target, tb.as_str(), Local::int32(2), true).unwrap();
            assert!(cx.prevent_extensions(target).unwrap());
            let missing = "TypeError: 'ownKeys' on proxy: trap result did not include '";
            let extra = "TypeError: 'ownKeys' on proxy: trap returned extra keys but proxy \
                         target is non-extensible";
            for (names, want) in [
                (vec![tb.as_str(), ta.as_str()], None),
                (
                    vec![ta.as_str(), tb.as_str(), ta.as_str()],
                    Some(DUPLICATE.to_owned()),
                ),
                (vec![tb.as_str()], Some(format!("{missing}{ta}'"))),
                (vec![ta.as_str()], Some(format!("{missing}{tb}'"))),
                (vec![tb.as_str(), ta.as_str(), "z"], Some(extra.to_owned())),
            ] {
                let list = cx.new_array();
                for (n, name) in names.iter().enumerate() {
                    // A new string, not the atom: the check interns it.
                    let s = cx.new_string(name).unwrap();
                    cx.set(list, n.to_string().as_str(), s, true).unwrap();
                }
                let h = handler_of(cx, rt, &["ownKeys"], list_trap);
                cx.set(h, "list", list, true).unwrap();
                let p = cx.proxy_create(target, h).unwrap();
                match want {
                    None => {
                        let keys = cx.own_property_keys(p).unwrap();
                        assert_eq!(key_texts(cx, keys.into()), [tb.clone(), ta.clone()]);
                    }
                    Some(want) => {
                        assert!(cx.own_property_keys(p).is_err());
                        assert_eq!(take_error(cx, rt), want);
                    }
                }
            }
        });
    }
}

// ---- the ownKeys removal and run scan (M10-P0 HASH-SEED, Codex r2 F1, F2) -------------------

/// Codex r2 F1: steps 19-23 can give `remove` a target key that is not a string or a symbol
/// (a target key list past the dense limit reads as UNDEFINED). `remove` returns false for it
/// (the caller then throws the step 21 or 23 TypeError, as the base `HashSet` lookup did), and
/// does not hash it (`string::hash` of a non-cell panics). The real keys are still found once.
#[test]
fn own_keys_unchecked_remove_of_a_value_that_is_not_a_key_is_false() {
    with_cx(false, |cx, rt| {
        let (a, b) = (atom(rt, "a"), atom(rt, "b"));
        let sym = cx.new_symbol(Local::undefined()).unwrap();
        let list = cx.new_array();
        for (n, k) in [a, b, sym.bits()].into_iter().enumerate() {
            cx.set(
                list,
                n.to_string().as_str(),
                Local::<'_, crate::JsValue>::from_bits(k),
                true,
            )
            .unwrap();
        }
        let keys = list.bits();
        for v in [
            crate::frame::UNDEFINED,
            NULL,
            Local::int32(5).bits(),
            Local::boolean(true).bits(),
        ] {
            assert_eq!(
                proxy::unchecked_keys_remove(cx.inner(), keys, v),
                Ok(false),
                "{v:#x}"
            );
        }
        for k in [a, b, sym.bits()] {
            assert_eq!(proxy::unchecked_keys_remove(cx.inner(), keys, k), Ok(true));
        }
        assert_eq!(
            proxy::unchecked_keys_remove(cx.inner(), keys, atom(rt, "c")),
            Ok(false)
        );
    });
}

/// The length of the trap result of [`one_key_run_trap`]: every element is "a".
const RUN_LEN: u32 = 256;

std::thread_local! {
    /// The interrupt budget that [`terminate_after_budget_a`] sets.
    static RUN_BUDGET: core::cell::Cell<i64> = const { core::cell::Cell::new(1) };
}

/// A getter: counts the call, requests a termination and sets the interrupt budget to
/// `RUN_BUDGET`, so poll number `RUN_BUDGET` after this call stops the isolate. Returns "a".
fn terminate_after_budget_a<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    GETTER_CALLS.with(|c| c.set(c.get() + 1));
    let a = cx.new_string("a")?.value();
    let inner = cx.inner();
    inner.rt().terminate.fetch_or(
        crate::interp::interrupt::TERMINATE,
        std::sync::atomic::Ordering::Release,
    );
    inner.budget = RUN_BUDGET.with(core::cell::Cell::get);
    Ok(a)
}

/// An `ownKeys` trap: the array-like `{length: RUN_LEN, 0: "a", ..., get [RUN_LEN - 1]() {...}}`
/// (one repeated key: one equal-hash run of every pair), whose last getter is
/// `handler.getter`.
fn one_key_run_trap<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let getter = cx.get_v(this, "getter")?;
    let o = cx.new_object();
    cx.set(o, "length", Local::int32(RUN_LEN as i32), true)?;
    for i in 0..RUN_LEN - 1 {
        let s = cx.new_string("a")?;
        cx.set(o, i.to_string().as_str(), s, true)?;
    }
    let desc = PropertyDescriptor {
        get: Some(getter),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    cx.define_property_or_throw(o, (RUN_LEN - 1).to_string().as_str(), desc)?;
    Ok(o.value())
}

/// Codex r2 F2: the termination request comes after the sort, inside the step 10 run scan.
/// After the last getter, the pass polls `RUN_LEN` times (the hashes), then `RUN_LEN / 2` and
/// `RUN_LEN - 1` times (the heapsort): `before` polls. The scan of the one run polls once for
/// the first pair and once for its first comparison, which finds the duplicate. So a budget of
/// `before + 1` or `before + 2` stops `[[OwnPropertyKeys]]` with the termination, and a budget
/// of `before + 3` gives the duplicate TypeError (this pins `before`, so the termination comes
/// from the scan, not from an earlier poll). With and without GC stress.
#[test]
fn own_keys_run_scan_stops_on_a_termination_after_the_sort() {
    let before = i64::from(RUN_LEN + RUN_LEN / 2 + (RUN_LEN - 1));
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            for (budget, stops) in [(before + 1, true), (before + 2, true), (before + 3, false)] {
                let target = cx.new_object();
                let h = handler_of(cx, rt, &["ownKeys"], one_key_run_trap);
                let g = host_fn(rt, terminate_after_budget_a, "get last");
                cx.set(h, "getter", g, true).unwrap();
                let p = cx.proxy_create(target, h).unwrap();
                GETTER_CALLS.with(|c| c.set(0));
                RUN_BUDGET.with(|c| c.set(budget));
                let failed = cx.own_property_keys(p).is_err();
                let terminating = rt.terminating.get();
                // Clear the request before the asserts, so a failure does not stop the next run.
                rt.terminating.set(false);
                rt.terminate.fetch_and(
                    !crate::interp::interrupt::TERMINATE,
                    std::sync::atomic::Ordering::AcqRel,
                );
                assert_eq!(GETTER_CALLS.with(core::cell::Cell::get), 1);
                assert!(failed, "budget {budget}: the keys came back");
                assert_eq!(terminating, stops, "budget {budget}: termination");
                if stops {
                    assert_eq!(
                        rt.take_pending(),
                        crate::frame::UNDEFINED,
                        "budget {budget}"
                    );
                } else {
                    assert_eq!(take_error(cx, rt), DUPLICATE, "budget {budget}");
                }
            }
        });
    }
}

// ---- the target key list past the dense limit (M10-P1 HASH-SEED-2, PROXY-KEYLIST-PANIC) ----

/// PROXY-KEYLIST-PANIC: `proxy::key_list_get` reads a key list that is DICTIONARY. A store at
/// `arr::MAX_LENGTH` moves the list to DICTIONARY. Its keys at a low index and at the high
/// index read back, where the dense-only `object::array_get` (the old read of steps 14-23)
/// gives `None`. With and without GC stress.
#[test]
fn the_key_list_read_takes_a_dictionary_list() {
    for stress in [false, true] {
        with_cx(stress, |_cx, rt| {
            let (a, b, c) = (atom(rt, "a"), atom(rt, "b"), atom(rt, "c"));
            let list = object::new_array(rt, 0);
            assert!(object::array_set(rt, list, 0, a));
            assert!(object::array_set(rt, list, 1, b));
            assert_eq!(proxy::key_list_get(list, 1), b, "a dense list");
            let high = object::arr::MAX_LENGTH;
            assert!(object::array_set(rt, list, high, c));
            assert!(
                crate::elements::dict_of(list).is_some(),
                "a DICTIONARY list"
            );
            assert_eq!(object::array_len(list), high + 1);
            for (j, k) in [(0, a), (1, b), (high, c)] {
                assert_eq!(
                    object::array_get(list, j),
                    None,
                    "the dense-only read at {j}"
                );
                assert_eq!(proxy::key_list_get(list, j), k, "index {j}");
            }
        });
    }
}

/// `[[OwnPropertyKeys]]` of a proxy for `target` whose `ownKeys` trap returns new strings
/// `names`: the keys as Rust text, or the error as "Name: message".
fn trap_keys<'s>(
    cx: &mut Cx<'s>,
    rt: &Runtime,
    target: Local<'s, JsObject>,
    names: &[&str],
) -> Result<Vec<String>, String> {
    let list = cx.new_array();
    for (i, name) in names.iter().enumerate() {
        let s = cx.new_string(name).unwrap();
        cx.set(list, i.to_string().as_str(), s, true).unwrap();
    }
    let h = handler_of(cx, rt, &["ownKeys"], list_trap);
    cx.set(h, "list", list, true).unwrap();
    let p = cx.proxy_create(target, h).unwrap();
    match cx.own_property_keys(p) {
        Ok(keys) => Ok(key_texts(cx, keys.into())),
        Err(_) => Err(take_error(cx, rt)),
    }
}

/// Steps 14-24 read the target keys through `key_list_get` and give the results and the
/// TypeError texts of before (node v26.13.1 is the oracle). The target has "x"
/// (non-configurable) and "y". Extensible: extra keys and a missing "y" pass, a missing "x"
/// throws (step 21). Non-extensible: only the same keys pass (steps 21, 23, 24). With and
/// without GC stress (and the check runs with and without `EP9_HASH_SEED`).
#[test]
fn own_keys_target_checks_keep_their_results_and_texts() {
    let missing = "TypeError: 'ownKeys' on proxy: trap result did not include ";
    let extra = "TypeError: 'ownKeys' on proxy: trap returned extra keys but proxy target is \
                 non-extensible";
    let ok = |v: &[&str]| Ok(v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    let cases = [
        (true, &["y", "x", "z"][..], ok(&["y", "x", "z"])),
        (true, &["y"][..], Err(format!("{missing}'x'"))),
        (true, &["x"][..], ok(&["x"])),
        (false, &["y", "x"][..], ok(&["y", "x"])),
        (false, &["y"][..], Err(format!("{missing}'x'"))),
        (false, &["x"][..], Err(format!("{missing}'y'"))),
        (false, &["x", "y", "z"][..], Err(extra.to_owned())),
    ];
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            for (extensible, names, want) in &cases {
                let target = cx.new_object();
                let x = PropertyDescriptor::data(Local::int32(1), true, true, false);
                cx.define_property_or_throw(target, "x", x).unwrap();
                cx.set(target, "y", Local::int32(2), true).unwrap();
                if !extensible {
                    assert!(cx.prevent_extensions(target).unwrap());
                }
                let got = trap_keys(cx, rt, target, names);
                assert_eq!(&got, want, "extensible {extensible}, {names:?}");
            }
        });
    }
}

/// Sets the 9 named properties "p0" .. "p8" of `o`.
fn nine_named<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) {
    for i in 0..9 {
        cx.set(o, format!("p{i}").as_str(), Local::int32(i), true)
            .unwrap();
    }
}

/// PROXY-KEYLIST-PANIC end to end: a target with more than `arr::MAX_LENGTH` = 2^26 keys gives
/// a DICTIONARY target key list. The targets are `new Uint8Array(2**26 + 1)` (configurable
/// keys, extensible: steps 14-18) and `new String("a".repeat(2**26))` (2^26 + 1 keys with
/// "length", non-configurable: steps 19-21), each with 9 named properties (more than 8 own
/// properties: an indexed table, so a lookup hashes the key, `props_index::find_indexed`).
/// The `ownKeys` trap returns []. Before the fix each key read as undefined: its lookup
/// hashed undefined and panicked (a release panic). Not in CI: a peak RSS of about 15 GiB
/// (M10-P1, aarch64 release, 51 s). Run: `cargo test -p ep9_vm --lib --release -- --ignored
/// own_keys_takes_a_target_key_list_past_the_dense_limit`.
#[test]
#[ignore = "a peak RSS of about 15 GiB; run with --ignored --release"]
fn own_keys_takes_a_target_key_list_past_the_dense_limit() {
    let n = u64::from(object::arr::MAX_LENGTH);
    with_cx(false, |cx, rt| {
        let null = Local::null();
        let buf = cx.allocate_array_buffer(null, n + 1, None).unwrap();
        let view = cx.new_typed_array(null, 1, buf, 0, None).unwrap();
        nine_named(cx, view);
        assert_eq!(trap_keys(cx, rt, view, &[]), Ok(Vec::new()));
        let text = cx.new_string(&"a".repeat(n as usize)).unwrap();
        let wrapper = cx.to_object(text).unwrap();
        nine_named(cx, wrapper);
        let want = "TypeError: 'ownKeys' on proxy: trap result did not include '0'";
        assert_eq!(trap_keys(cx, rt, wrapper, &[]), Err(want.to_owned()));
    });
}

/// PROXY-KEYLIST-PANIC: the production reads of steps 14-23 go through `key_list_get`. A
/// non-extensible target {x (non-configurable), y} with the trap result ["x", "y"] reads 4 target
/// keys (steps 14-17: 2, steps 19-21: 1, step 23: 1). The same target, extensible, with
/// ["y", "x"] reads 3 (steps 14-17: 2, steps 19-21: 1). If a read site goes back to the
/// dense-only `object::array_get`, the count drops and the test fails. With and without GC
/// stress (a collection at each `target_desc` GC point between the reads).
#[test]
fn own_keys_reads_each_target_key_through_the_key_list_read() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            for (extensible, names, reads) in [(false, ["x", "y"], 4), (true, ["y", "x"], 3)] {
                let target = cx.new_object();
                let x = PropertyDescriptor::data(Local::int32(1), true, true, false);
                cx.define_property_or_throw(target, "x", x).unwrap();
                cx.set(target, "y", Local::int32(2), true).unwrap();
                if !extensible {
                    assert!(cx.prevent_extensions(target).unwrap());
                }
                proxy::KEY_LIST_READS.with(|n| n.set(0));
                let got = trap_keys(cx, rt, target, &names);
                let want: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
                assert_eq!(got, Ok(want), "extensible {extensible}");
                assert_eq!(
                    proxy::KEY_LIST_READS.with(core::cell::Cell::get),
                    reads,
                    "the target key reads, extensible {extensible}"
                );
            }
        });
    }
}
