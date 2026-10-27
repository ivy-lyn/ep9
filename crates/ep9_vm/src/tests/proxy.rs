//! Tests of Proxy exotic objects (`proxy.rs`, kinds 27 and 28; M4-P4 PROXY1, plan v9 §6.3):
//! the routing (MC-w15-12: proxies are routed kinds, so the keyless index paths ask them), the
//! month-4 internal methods (the step for an undefined trap: the target's method), revocation,
//! `Cx::is_array_or_throw`, the fallible forms of `property.rs` and their callers, and the
//! debug asserts of the ordinary forms. No JS can make a Proxy before PROXY2 (month 5), so
//! the proxies here come from `proxy::proxy_create`.

use super::{config, isolate};
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
        // `for-in` lists the receiver's keys, then the proxy's (its target's).
        let keys = property::for_in_keys(cx.inner(), o.bits()).unwrap();
        let keys: Local<'_, JsObject> = Local::from_bits(keys);
        assert_eq!(key_texts(cx, keys), ["data"]);
        cx.set(t, "more", Local::int32(1), true).unwrap();
        let keys = property::for_in_keys(cx.inner(), o.bits()).unwrap();
        let keys: Local<'_, JsObject> = Local::from_bits(keys);
        assert_eq!(key_texts(cx, keys), ["data", "more"]);
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
/// into `a` again; the native stack check of each proxy method ends it in a RangeError.
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
