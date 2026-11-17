//! Tests of the `for-in` key list (`property::for_in_keys`, §14.7.5.9 EnumerateObjectProperties):
//! the key order (each object's indices ascending, then its strings in creation order, then
//! its prototype's), the seen rule (a key seen before, enumerable or not, is skipped later),
//! the seen test (FORIN-DEDUPE: atoms compare by bits, other strings by content; an ordinary
//! hop with nothing seen yet lists its unique keys with no test) and the rooting of the seen
//! keys at a proxy hop. Each run is with and without GC stress. The expected lists are node
//! v26.13.1's output for the JS in the comment above each case.

use super::{config, isolate};
use crate::runtime::Runtime;
use crate::{Args, Cx, Dispatch, HostFn, Isolate, JsObject, JsResult, Local, PropertyDescriptor};
use crate::{object, property, proxy, string};

/// Runs `f` with a builtin-ABI context of a fresh isolate, with and without GC stress (a
/// collection before every allocation, then the heap verifier).
fn each(f: impl Fn(&mut Cx<'_>, &Runtime)) {
    for stress in [false, true] {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        {
            let iso: &mut Isolate = &mut iso;
            let mut cx = Cx::new(&mut iso.cx);
            f(&mut cx, &iso.rt);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    }
}

/// The for-in keys of `o`, joined with commas.
fn keys<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> String {
    let a = property::for_in_keys(cx.inner(), o.bits()).unwrap();
    let a: Local<'_, JsObject> = Local::from_bits(a);
    let n = object::array_len(a.bits());
    (0..u64::from(n))
        .map(|i| {
            let k = cx.get_index(a, i).unwrap();
            cx.to_rust_string(k).unwrap()
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `Object.defineProperty(o, k, { value: 1, enumerable: false, writable: true,
/// configurable: true })`.
fn hide<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, k: &str) {
    let d = PropertyDescriptor::data(Local::int32(1), true, false, true);
    cx.define_property_or_throw(o, k, d).unwrap();
}

/// `o[k] = v` for each name.
fn put<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, names: &[&str]) {
    for (i, &k) in names.iter().enumerate() {
        cx.set(o, k, Local::int32(i as i32), true).unwrap();
    }
}

/// `var o = { b: 1, a: 2, toString: 3 }; Object.defineProperty(o, 'h', ...not enumerable)`:
/// `b,a,toString`. The own `toString` is listed once (`Object.prototype`'s is not
/// enumerable and comes later); the own hop has unique keys and nothing seen before.
#[test]
fn a_plain_object_lists_its_own_enumerable_keys_once_in_creation_order() {
    each(|cx, _| {
        let o = cx.new_object();
        put(cx, o, &["b", "a", "toString"]);
        hide(cx, o, "h");
        assert_eq!(keys(cx, o), "b,a,toString");
    });
}

/// `var top = { a: 1, t: 2 }; var mid = Object.create(top); (mid.a: not enumerable);
/// mid.m = 3; var o = Object.create(mid); o.x = 4; o.t = 5; o.m = 6`: `x,t,m`. The
/// non-enumerable `mid.a` hides the enumerable `top.a`; `t` and `m` are listed once.
#[test]
fn a_non_enumerable_key_in_the_middle_hides_an_enumerable_key_further_up() {
    each(|cx, _| {
        let top = cx.new_object();
        put(cx, top, &["a", "t"]);
        let mid = cx.new_object_with_proto(top);
        hide(cx, mid, "a");
        put(cx, mid, &["m"]);
        let o = cx.new_object_with_proto(mid);
        put(cx, o, &["x", "t", "m"]);
        assert_eq!(keys(cx, o), "x,t,m");
    });
}

/// `var p = { 1: 'x', 2: 'y', 3: 'z', s: 1 }; var o = Object.create(p); o[2] = 'b';
/// o[0] = 'a'; (o[3]: not enumerable); o.r = 1`: `0,2,r,1,s`. And `var arr = [10, 20, 30];
/// (arr[1]: not enumerable); Object.setPrototypeOf(arr, { 1: 'p', 5: 'q' })`: `0,2,5` (a
/// DICTIONARY array's entry keeps its bit and hides the inherited index).
#[test]
fn index_keys_come_first_per_object_and_a_seen_index_is_skipped() {
    each(|cx, _| {
        let p = cx.new_object();
        for i in 1..=3 {
            cx.set_index(p, i, Local::int32(i as i32), true).unwrap();
        }
        put(cx, p, &["s"]);
        let o = cx.new_object_with_proto(p);
        cx.set_index(o, 2, Local::int32(2), true).unwrap();
        cx.set_index(o, 0, Local::int32(0), true).unwrap();
        hide(cx, o, "3");
        put(cx, o, &["r"]);
        assert_eq!(keys(cx, o), "0,2,r,1,s");

        let arr = cx.new_array();
        for i in 0..3 {
            cx.set_index(arr, i, Local::int32(10 * (i as i32 + 1)), true)
                .unwrap();
        }
        hide(cx, arr.into(), "1");
        let q = cx.new_object();
        cx.set_index(q, 1, Local::int32(1), true).unwrap();
        cx.set_index(q, 5, Local::int32(5), true).unwrap();
        assert!(cx.set_prototype_of(arr, q).unwrap());
        assert_eq!(keys(cx, arr.into()), "0,2,5");
    });
}

/// `var w = Object.setPrototypeOf(new String('ab'), { length: 9, z: 1, 0: 'q' });
/// var o = Object.create(w); o.k = 1`: `k,0,1,z`. The wrapper's indices hide the inherited
/// `0`, and its `length` (not enumerable) hides the inherited `length` (W4-P2FIX-FORIN-WRAP).
#[test]
fn a_string_wrapper_on_the_chain_hides_an_inherited_length_and_index() {
    each(|cx, _| {
        let s = cx.new_string("ab").unwrap();
        let w = cx.to_object(s).unwrap();
        let p = cx.new_object();
        put(cx, p, &["length", "z"]);
        cx.set_index(p, 0, Local::int32(0), true).unwrap();
        assert!(cx.set_prototype_of(w, p).unwrap());
        let o = cx.new_object_with_proto(w);
        put(cx, o, &["k"]);
        assert_eq!(keys(cx, o), "k,0,1,z");
    });
}

/// The seen test: an atom compares by bits, a string that is not an atom by content
/// (`key::key_eq`), on both sides. No stored key is a non-atom string (stored keys and a
/// proxy's keys are interned), so the content side is checked here directly.
#[test]
fn the_seen_test_compares_atoms_by_bits_and_other_strings_by_content() {
    each(|cx, rt| {
        let a = crate::atoms::intern_str(rt, "alpha");
        let b = crate::atoms::intern_str(rt, "beta");
        let fresh = cx.new_string("alpha").unwrap().bits();
        assert!(string::is_atom(a) && string::is_atom(b));
        assert!(!string::is_atom(fresh), "a new string is not an atom");
        let seen = property::for_in_seen;
        // An atom against atoms: bits only.
        assert!(seen(&[b, a], &[], a, true));
        assert!(!seen(&[b], &[], a, true));
        assert!(!seen(&[], &[], a, true));
        // An atom against a seen non-atom string of the same text.
        assert!(seen(&[b], &[fresh], a, true));
        // A non-atom string against a seen atom of the same text, and against itself.
        assert!(seen(&[b, a], &[], fresh, false));
        assert!(seen(&[], &[fresh], fresh, false));
        assert!(!seen(&[b], &[], fresh, false));
    });
}

/// `var P = { a: 1, b: 2 }; (P.h: not enumerable); for (k in new Proxy(Object.create(P),
/// {}))`: `a,b` (the proxy hop lists no string key, so P is a hop with nothing seen yet).
/// With an own `c` and `b` on the target: `c,b,a`.
#[test]
fn an_ordinary_hop_after_a_proxy_hop_keeps_the_seen_rule() {
    each(|cx, _| {
        let p = cx.new_object();
        put(cx, p, &["a", "b"]);
        hide(cx, p, "h");
        let t = cx.new_object_with_proto(p);
        let px = new_proxy(cx, t);
        assert_eq!(keys(cx, px), "a,b");
        let t2 = cx.new_object_with_proto(p);
        put(cx, t2, &["c", "b"]);
        let px2 = new_proxy(cx, t2);
        assert_eq!(keys(cx, px2), "c,b,a");
    });
}

/// 40 own keys `k0..k39` (`k5` not enumerable) over a prototype with `k20..k59` and an
/// enumerable `k5` (`k45` not enumerable): `k0..k39` without `k5`, then `k40..k59` without
/// `k45` (node prints the same list).
#[test]
fn many_keys_on_a_two_object_chain_keep_the_order_and_the_seen_rule() {
    each(|cx, _| {
        let p = cx.new_object();
        let up: Vec<String> = (20..60).map(|i| format!("k{i}")).collect();
        let up: Vec<&str> = up.iter().map(String::as_str).collect();
        put(cx, p, &up);
        put(cx, p, &["k5"]);
        hide(cx, p, "k45");
        let o = cx.new_object_with_proto(p);
        let own: Vec<String> = (0..40).map(|i| format!("k{i}")).collect();
        let own: Vec<&str> = own.iter().map(String::as_str).collect();
        put(cx, o, &own);
        hide(cx, o, "k5");
        let want: Vec<String> = (0..60)
            .filter(|&i| i != 5 && i != 45)
            .map(|i| format!("k{i}"))
            .collect();
        assert_eq!(keys(cx, o), want.join(","));
    });
}

/// A new proxy for `target` with an empty handler (`new Proxy(target, {})`).
fn new_proxy<'s>(cx: &mut Cx<'s>, target: Local<'s, JsObject>) -> Local<'s, JsObject> {
    let handler = cx.new_object().bits();
    let p = proxy::proxy_create(cx.inner(), target.bits(), handler).expect("proxy_create");
    Local::from_bits(p)
}

/// The name of the hidden key `i` (built at run time, so no other code holds its atom).
fn hidden_name(i: usize) -> String {
    format!("hidden_Key_{i}")
}

/// An `ownKeys` trap: deletes `this.o[hidden_Key_i]` for i in 0..8 (the handler's `o`), then
/// returns new strings of the same names (each an allocation: with GC stress a collection
/// runs before each, so an unrooted atom of a deleted key is gone).
fn delete_then_list<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let h = this.as_object().expect("the handler");
    let o = cx.get(h, "o")?.as_object().expect("the receiver");
    for i in 0..8 {
        cx.delete(o, hidden_name(i).as_str())?;
    }
    let list = cx.new_array();
    for i in 0..8 {
        let s = cx.new_string(&hidden_name(i))?;
        cx.set_index(list, i as u64, s, true)?;
    }
    Ok(list.value())
}

/// `function mk(i) { ... }; var o = { a: 1, b: 2 }; delete o.a; (8 keys mk(i): not
/// enumerable); var p = new Proxy({}, { ownKeys() { delete o[mk(i)] ...; return [mk(i) ...] }
/// }); Object.setPrototypeOf(o, p); for (k in o)`: `b` (ep9_builtins
/// `for_in_skips_a_seen_key_after_a_trap_deletes_it`; node prints `1 b`). The seen keys of
/// the receiver (recorded with no test: not enumerable) are rooted in `kept` before the trap
/// runs, so the trap's names intern to the same atoms and stay hidden.
#[test]
fn a_seen_key_stays_hidden_after_a_trap_deletes_it_and_collects() {
    each(|cx, rt| {
        let o = cx.new_object();
        put(cx, o, &["a", "b"]);
        assert!(cx.delete(o, "a").unwrap());
        for i in 0..8 {
            hide(cx, o, &hidden_name(i));
        }
        let h = cx.new_object();
        let f: HostFn = delete_then_list;
        let trap: Local<'_> =
            Local::from_bits(crate::realm::new_host_function(rt, f, "ownKeys", 0, false));
        cx.set(h, "ownKeys", trap, true).unwrap();
        cx.set(h, "o", o, true).unwrap();
        let target = cx.new_object();
        let p = cx.proxy_create(target, h).unwrap();
        assert!(cx.set_prototype_of(o, p).unwrap());
        assert_eq!(keys(cx, o), "b");
    });
}
