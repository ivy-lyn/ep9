//! Proxy (§10.5, §28.2; M5-P3 PROXY2) from JS: the constructor and `Proxy.revocable`, the 13
//! traps and their invariants, the `IsArray` callers, a proxy cycle (verifier B V-B3) and
//! for-in at a proxy (V10-D61).
//!
//! The expected lines are node v24.22.1's output for the same script (`print` = the
//! arguments' ToString joined by spaces; `t(f)` gives `f()` as text, or the error as
//! "Name: message"). `common::run_both` runs each script with and without GC stress (a
//! collection before every allocation, then the heap verifier). Texts that differ from node
//! are left out here and listed in the PROXY2 report: call-site names ("(intermediate
//! value)"), a few falsish texts made by the callers (`Object.defineProperty`, a strict
//! store or delete), and the operation name in a revoked proxy's `Object.prototype.toString`.

mod common;

/// The test prelude: `t(f)`.
macro_rules! js {
    ($src:literal) => {
        concat!(
            "function t(f) { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } }\n",
            $src
        )
    };
}

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `Proxy` and `Proxy.revocable` (§28.2): the constructor's keys (no `prototype`), the
/// TypeErrors of ProxyCreate, `typeof` by target, the revoke function, and a revoked proxy.
#[test]
fn the_constructor_and_revocable() {
    assert_eq!(
        common::run_both(js!(r#"
print(typeof Proxy, Proxy.length, Proxy.name, Object.getOwnPropertyNames(Proxy).join(), 'prototype' in Proxy);
print(t(() => Proxy({}, {})), t(() => new Proxy(1, {})), t(() => new Proxy({}, null)));
var d = Object.getOwnPropertyDescriptor(globalThis, 'Proxy');
print(d.writable, d.enumerable, d.configurable, typeof Proxy.revocable, Proxy.revocable.length, Proxy.revocable.name);
print(typeof new Proxy({}, {}), typeof new Proxy(function () {}, {}), typeof new Proxy(class {}, {}), typeof new Proxy(() => 1, {}));
print(t(() => { new (new Proxy(() => 1, {}))(); }).split(":")[0], t(() => { (new Proxy({}, {}))(); }).split(":")[0]);
var r = Proxy.revocable({ a: 1 }, {});
print(Object.keys(r).join(), r.proxy.a, typeof r.revoke, r.revoke.length, JSON.stringify(r.revoke.name), Object.getPrototypeOf(r) === Object.prototype);
print(r.revoke(), r.revoke(), t(() => r.proxy.a), t(() => 'a' in r.proxy), t(() => Object.keys(r.proxy)));
print(t(() => new Proxy(r.proxy, {})), t(() => typeof r.proxy), t(() => Array.isArray(r.proxy)));
"#)),
        lines(&[
            "function 2 Proxy length,name,revocable false",
            "TypeError: Constructor Proxy requires 'new' TypeError: Cannot create proxy with a non-object as target or handler TypeError: Cannot create proxy with a non-object as target or handler",
            "true false true function 2 revocable",
            "object function function function",
            "TypeError TypeError",
            "proxy,revoke 1 function 0 \"\" true",
            "undefined undefined TypeError: Cannot perform 'get' on a proxy that has been revoked TypeError: Cannot perform 'has' on a proxy that has been revoked TypeError: Cannot perform 'ownKeys' on a proxy that has been revoked",
            "TypeError: Cannot perform 'get' on a proxy that has been revoked object TypeError: Cannot perform 'IsArray' on a proxy that has been revoked",
        ])
    );
}

/// The 13 traps (§10.5.1-13): each internal method calls its trap with the target, the
/// receiver of `get` and `set` on a prototype chain, `{...p}` through `ownKeys`, `[[Call]]` and
/// `[[Construct]]` (also through `super`, `bind` and `Reflect.apply`).
#[test]
fn every_trap_gets_its_target_and_its_result_counts() {
    assert_eq!(
        common::run_both(js!(r#"
var log = [];
var target = { a: 1 };
var h = {};
['getPrototypeOf', 'setPrototypeOf', 'isExtensible', 'preventExtensions', 'getOwnPropertyDescriptor', 'defineProperty', 'has', 'get', 'set', 'deleteProperty', 'ownKeys'].forEach(function (n) {
  h[n] = function (t) { log.push(n + (typeof arguments[1] === 'string' ? ':' + arguments[1] : '') + (t === target ? '' : '!')); return Reflect[n].apply(null, arguments); };
});
var p = new Proxy(target, h);
p.b = 2; var x = p.a; 'a' in p; delete p.b; Object.keys(p); Object.getPrototypeOf(p); Object.isExtensible(p);
Object.defineProperty(p, 'c', { value: 3, configurable: true }); Object.getOwnPropertyDescriptor(p, 'c');
Object.setPrototypeOf(p, null); Object.preventExtensions(p);
print(log.join(' '));
print(JSON.stringify(target), Object.isExtensible(target), Object.getPrototypeOf(target));
var recv = [];
var q = new Proxy({}, { get: function (t, k, r) { recv.push(r === q, r === child); return k; }, set: function (t, k, v, r) { recv.push(r === child); return true; } });
var child = Object.create(q);
print(q.foo, child.bar, (child.baz = 1), recv.join());
var spread = { ...new Proxy({ m: 1, n: 2 }, { ownKeys: function () { return ['n', 'm']; } }) };
print(JSON.stringify(spread), Object.keys(new Proxy([1, 2], {})).join(), Object.assign({}, new Proxy({ z: 9 }, {})).z);
var calls = [];
var f = new Proxy(function (a, b) { return a + b; }, {
  apply: function (t, th, args) { calls.push('apply', th, args.length); return t.apply(th, args) * 10; },
  construct: function (t, args, nt) { calls.push('construct', args.join('+'), nt === f); return { made: args.length }; }
});
print(f(1, 2), f.call('T', 3, 4), new f(5, 6, 7).made, Reflect.apply(f, null, [1, 1]), calls.join());
class C { constructor(v) { this.v = v; } }
var PC = new Proxy(C, {});
var o = new PC(4);
print(o.v, o instanceof C, o instanceof PC, Object.getPrototypeOf(o) === C.prototype);
class D extends PC { constructor() { super(8); } }
print(new D().v, new D() instanceof C);
var bound = f.bind(null, 2);
print(bound(3), new bound(1).made);
print(Function.prototype.toString.call(new Proxy(function g() {}, {})).indexOf('native code') >= 0);
"#)),
        lines(&[
            "set:b getOwnPropertyDescriptor:b defineProperty:b get:a has:a deleteProperty:b ownKeys getOwnPropertyDescriptor:a getPrototypeOf isExtensible defineProperty:c getOwnPropertyDescriptor:c setPrototypeOf preventExtensions",
            "{\"a\":1} false null",
            "foo bar 1 true,false,false,true,true",
            "{\"n\":2,\"m\":1} 0,1 9",
            "30 70 3 20 apply,,2,apply,T,2,construct,5+6+7,true,apply,,2",
            "4 true true true",
            "8 true",
            "50 2",
            "true",
        ])
    );
}

/// The invariant checks of the 13 methods: each throws the TypeError text of node, and a
/// falsish trap result reaches `Reflect` as false.
#[test]
fn the_invariants_throw_the_v8_texts() {
    assert_eq!(
        common::run_both(js!(r#"
var nc = {}; Object.defineProperty(nc, 'x', { value: 1, writable: false, configurable: false });
var ne = Object.preventExtensions({});
var neX = Object.preventExtensions({ x: 1 });
print(t(() => new Proxy(nc, { get: () => 2 }).x));
print(t(() => new Proxy(nc, { get: () => 1 }).x));
print(t(() => 'x' in new Proxy(nc, { has: () => false })), t(() => 'y' in new Proxy(neX, { has: () => false })), t(() => 'x' in new Proxy(neX, { has: () => false })));
print(t(() => Object.keys(new Proxy(nc, { ownKeys: () => [] }))));
print(t(() => Object.keys(new Proxy(neX, { ownKeys: () => ['x', 'y'] }))));
print(t(() => Object.keys(new Proxy({}, { ownKeys: () => ['a', 'a'] }))));
print(t(() => Object.keys(new Proxy({}, { ownKeys: () => [1] }))));
print(t(() => Object.keys(new Proxy({}, { ownKeys: () => 1 }))));
print(t(() => Object.getPrototypeOf(new Proxy({}, { getPrototypeOf: () => 1 }))));
print(t(() => Object.getPrototypeOf(new Proxy(ne, { getPrototypeOf: () => Array.prototype }))));
print(t(() => Object.isExtensible(new Proxy({}, { isExtensible: () => false }))));
print(t(() => Object.preventExtensions(new Proxy({}, { preventExtensions: () => true }))));
print(t(() => Object.getOwnPropertyDescriptor(new Proxy(nc, { getOwnPropertyDescriptor: () => undefined }), 'x')));
print(t(() => Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: () => 1 }), 'x')));
print(t(() => Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: () => ({ value: 1, configurable: false }) }), 'x')));
print(t(() => Object.defineProperty(new Proxy(ne, { defineProperty: () => true }), 'y', { value: 1 })));
print(t(() => Object.defineProperty(new Proxy({}, { defineProperty: () => true }), 'y', { value: 1, configurable: false })));
print(t(() => delete new Proxy(nc, { deleteProperty: () => true }).x));
print(t(() => { 'use strict'; new Proxy(nc, { set: () => true }).x = 5; }));
print(t(() => Reflect.set(new Proxy({}, { set: () => false }), 'x', 1)));
print(t(() => Reflect.defineProperty(new Proxy({}, { defineProperty: () => false }), 'x', {})));
print(t(() => Reflect.deleteProperty(new Proxy({}, { deleteProperty: () => 0 }), 'x')));
print(t(() => Reflect.setPrototypeOf(new Proxy({}, { setPrototypeOf: () => false }), null)));
print(t(() => Reflect.preventExtensions(new Proxy({}, { preventExtensions: () => false }))));
print(t(() => new Proxy({}, { get: 1 }).x));
print(t(() => new (new Proxy(function () {}, { construct: () => 1 }))()));
print(t(() => new Proxy({ x: 1 }, { get: null }).x), t(() => new Proxy({ x: 1 }, { get: undefined }).x));
print(t(() => { var p = new Proxy({}, { defineProperty: () => false }); p.x = 1; return Object.keys(p).length; }));
print(t(() => { var p = new Proxy(ne, {}); p[0] = 1; return 'no throw'; }));
"#)),
        lines(&[
            "TypeError: 'get' on proxy: property 'x' is a read-only and non-configurable data property on the proxy target but the proxy did not return its actual value (expected '1' but got '2')",
            "1",
            "TypeError: 'has' on proxy: trap returned falsish for property 'x' which exists in the proxy target as non-configurable false TypeError: 'has' on proxy: trap returned falsish for property 'x' but the proxy target is not extensible",
            "TypeError: 'ownKeys' on proxy: trap result did not include 'x'",
            "TypeError: 'ownKeys' on proxy: trap returned extra keys but proxy target is non-extensible",
            "TypeError: 'ownKeys' on proxy: trap returned duplicate entries",
            "TypeError: 1 is not a valid property name",
            "TypeError: CreateListFromArrayLike called on non-object",
            "TypeError: 'getPrototypeOf' on proxy: trap returned neither object nor null",
            "TypeError: 'getPrototypeOf' on proxy: proxy target is non-extensible but the trap did not return its actual prototype",
            "TypeError: 'isExtensible' on proxy: trap result does not reflect extensibility of proxy target (which is 'true')",
            "TypeError: 'preventExtensions' on proxy: trap returned truish but the proxy target is extensible",
            "TypeError: 'getOwnPropertyDescriptor' on proxy: trap returned undefined for property 'x' which is non-configurable in the proxy target",
            "TypeError: 'getOwnPropertyDescriptor' on proxy: trap returned neither object nor undefined for property 'x'",
            "TypeError: 'getOwnPropertyDescriptor' on proxy: trap reported non-configurability for property 'x' which is either non-existent or configurable in the proxy target",
            "TypeError: 'defineProperty' on proxy: trap returned truish for adding property 'y'  to the non-extensible proxy target",
            "TypeError: 'defineProperty' on proxy: trap returned truish for defining non-configurable property 'y' which is either non-existent or configurable in the proxy target",
            "TypeError: 'deleteProperty' on proxy: trap returned truish for property 'x' which is non-configurable in the proxy target",
            "TypeError: 'set' on proxy: trap returned truish for property 'x' which exists in the proxy target as a non-configurable and non-writable data property with a different value",
            "false",
            "false",
            "false",
            "false",
            "false",
            "TypeError: '1' returned for property 'get' of object '#<Object>' is not a function",
            "TypeError: 'construct' on proxy: trap returned non-object ('1')",
            "1 1",
            "0",
            "no throw",
        ])
    );
}

/// The `IsArray` callers that H31 lists (PROXY2 list): `Array.isArray`, JSON.stringify's
/// value and replacer, IsConcatSpreadable, `Object.prototype.toString`, `flat`; a revoked
/// proxy throws.
#[test]
fn is_array_callers_follow_the_proxy() {
    assert_eq!(
        common::run_both(js!(r#"
var pa = new Proxy([1, 2], {});
var r = Proxy.revocable([], {}); r.revoke();
print(Array.isArray(pa), Array.isArray(new Proxy(pa, {})), Array.isArray(new Proxy({}, {})), t(() => Array.isArray(r.proxy)));
print(JSON.stringify(pa), JSON.stringify({ a: new Proxy([3], {}) }), JSON.stringify([1, [2]], new Proxy(['0'], {})));
print([0].concat(pa).join(), [].concat(new Proxy({ length: 1, 0: 'z', [Symbol.isConcatSpreadable]: true }, {})).join());
print(t(() => [].concat(r.proxy)));
print(Object.prototype.toString.call(pa), Object.prototype.toString.call(new Proxy(function () {}, {})), Object.prototype.toString.call(new Proxy({}, {})));
print(t(() => Object.prototype.toString.call(r.proxy)).split(':')[0]);
print(JSON.parse('[1,[2]]', function (k, v) { return v; }).length, [new Proxy([1, [2]], {})].flat(2).join());
"#)),
        lines(&[
            "true true false TypeError: Cannot perform 'IsArray' on a proxy that has been revoked",
            "[1,2] {\"a\":[3]} [1,[2]]",
            "0,1,2 z",
            "TypeError: Cannot perform 'get' on a proxy that has been revoked",
            "[object Array] [object Function] [object Object]",
            "TypeError",
            "2 1,2",
        ])
    );
}

/// Verifier B V-B3 (M4-P4; H31 PROXY2 list): `a` whose prototype is `new Proxy(a, {})`.
/// `isPrototypeOf`, `__lookupGetter__`, `__lookupSetter__`, `instanceof`, `in` and `[[Get]]`
/// on it throw the RangeError, as in node; for-in over `a` lists `x` (V8 stops after the
/// first proxy, V10-D61), and for-in over the proxy throws. Without GC stress: each walk
/// passes `MAX_PROXY_HOPS` (100 x 1024) proxies.
#[test]
fn v_b3_a_proxy_cycle_throws_the_range_error() {
    assert_eq!(
        common::on_big_stack(|| common::run(
            js!(r#"
var a = { x: 1 };
var p = new Proxy(a, {});
Object.setPrototypeOf(a, p);
var keys = []; for (var k in a) keys.push(k);
print(keys.join());
print(t(() => { for (var k in p); }));
print(t(() => a.isPrototypeOf(a)), t(() => Object.prototype.isPrototypeOf.call({}, Object.create(a))), t(() => Object.prototype.isPrototypeOf.call(p, {})));
print(t(() => a.__lookupGetter__('zz')), t(() => a.__lookupSetter__('zz')));
print(t(() => a instanceof Object), t(() => 'zz' in a), t(() => a.zz));
"#),
            false
        )),
        lines(&[
            "x",
            "RangeError: Maximum call stack size exceeded",
            "RangeError: Maximum call stack size exceeded RangeError: Maximum call stack size exceeded false",
            "RangeError: Maximum call stack size exceeded RangeError: Maximum call stack size exceeded",
            "RangeError: Maximum call stack size exceeded RangeError: Maximum call stack size exceeded RangeError: Maximum call stack size exceeded",
        ])
    );
}

/// for-in and a proxy on the chain (V10-D61, H31 the for-in row): a receiver that is not
/// a proxy stops after the first proxy (its `ownKeys` and one `getPrototypeOf`); a proxy
/// receiver walks on. Key order, symbols and a key deleted during the loop.
#[test]
fn for_in_follows_v8_at_a_proxy() {
    assert_eq!(
        common::run_both(js!(r#"
var log = [];
var h = { ownKeys: function (x) { log.push('ownKeys'); return Reflect.ownKeys(x); }, getPrototypeOf: function (x) { log.push('gpo'); return Reflect.getPrototypeOf(x); } };
var g = new Proxy(Object.create({ top: 1 }, { g: { value: 1, enumerable: true }, hid: { value: 1 } }), h);
var o = Object.create(g); o.own = 1;
var r = []; for (var k in o) r.push(k);
print(r.join(), log.join(' '));
log = []; r = []; for (var k in g) r.push(k);
print(r.join(), log.join(' ').indexOf('ownKeys gpo'));
var c = { c: 1 }; Object.setPrototypeOf(c, { top: 1 });
var d = Object.create(new Proxy(c, {})); d.d = 1; var e = Object.create(d); e.e = 1;
r = []; for (var k in e) r.push(k); print(r.join());
r = []; for (var k in Object.create(new Proxy(c, {}))) r.push(k); print(r.join());
var pk = new Proxy({ b: 1, a: 2, 1: 3, [Symbol('s')]: 4 }, {}); r = []; for (var k in pk) r.push(k); print(r.join());
var del = new Proxy({ p: 1, q: 2 }, {}); r = []; for (var k in del) { r.push(k); delete del.q; } print(r.join());
"#)),
        lines(&["own,g ownKeys gpo", "g,top 0", "e,d,c", "c", "1,b,a", "p",])
    );
}

/// for-in (§14.7.5.9): a key seen before is skipped later, enumerable or not, even when a
/// proxy trap on the chain deletes that property and a collection runs (Codex r1 proxy2
/// [high]: the seen non-enumerable keys of an ordinary receiver were not rooted, and the
/// atom table is weak, so a key the trap gives again read as new). Node v24.22.1 prints `1 b`.
#[test]
fn for_in_skips_a_seen_key_after_a_trap_deletes_it() {
    assert_eq!(
        common::run_both(js!(r#"
function mk(i) { return ['hidden', 'Key', String(i)].join('_'); }
var o = { a: 1, b: 2 };
delete o.a;
for (var i = 0; i < 8; i++) Object.defineProperty(o, mk(i), { value: i, enumerable: false, configurable: true });
var p = new Proxy({}, {
  ownKeys: function () { for (var i = 0; i < 8; i++) delete o[mk(i)]; var r = []; for (var i = 0; i < 8; i++) r.push(mk(i)); return r; },
  has: function () { return true; },
  getOwnPropertyDescriptor: function () { return { value: 1, enumerable: true, configurable: true }; }
});
Object.setPrototypeOf(o, p);
var out = [];
for (var k in o) out.push(k);
print(out.length, out.join());
"#)),
        lines(&["1 b"])
    );
}

/// IsConstructor of a revoked proxy (§10.5.14 step 7.b; Codex r1 proxy2 [medium]): a proxy
/// keeps the `[[Construct]]` it got at creation, so `Array.of` constructs through it (and
/// throws the revoked TypeError), and a live outer proxy whose inner target was revoked
/// still runs its `construct` trap. Node v24.22.1 prints these lines.
#[test]
fn a_revoked_proxy_keeps_its_construct() {
    assert_eq!(
        common::run_both(js!(r#"
var r = Proxy.revocable(function () {}, {});
r.revoke();
print(t(() => Array.of.call(r.proxy, 1)));
var r2 = Proxy.revocable(function () {}, {});
var outer = new Proxy(r2.proxy, { construct: function () { return { ok: 1 }; } });
var bound = Function.prototype.bind.call(r2.proxy, null);
r2.revoke();
print(t(() => JSON.stringify(new outer())), t(() => new bound()));
var r3 = Proxy.revocable(() => 1, {});
r3.revoke();
print(t(() => Array.of.call(r3.proxy, 1).length), t(() => new r3.proxy()).split(':')[0]);
"#)),
        lines(&[
            "TypeError: Cannot perform 'construct' on a proxy that has been revoked",
            "{\"ok\":1} TypeError: Cannot perform 'construct' on a proxy that has been revoked",
            "1 TypeError",
        ])
    );
}

/// GetFunctionRealm (§7.3.22) step 4: a newTarget proxy revoked while its `prototype` is read
/// throws the TypeError in OrdinaryCreateFromConstructor, GetPrototypeFromConstructor and a
/// NativeError (Codex r1 proxy2 [medium]; test262 base-ctor-revoked-proxy.js). Node v24.22.1
/// throws a TypeError in each (its text names the operation 'undefined').
#[test]
fn get_function_realm_throws_for_a_revoked_proxy() {
    assert_eq!(
        common::run_both(js!(r#"
function nt() { var r = Proxy.revocable(function () {}, { get: function (t, k) { if (k === 'prototype') { r.revoke(); return undefined; } return t[k]; } }); return r.proxy; }
print(t(() => Reflect.construct(function () {}, [], nt())).split(':')[0], t(() => Reflect.construct(Array, [], nt())).split(':')[0], t(() => Reflect.construct(TypeError, [], nt())).split(':')[0]);
var live = new Proxy(function () {}, { get: function () { return undefined; } });
print(Object.getPrototypeOf(Reflect.construct(function () {}, [], live)) === Object.prototype, Array.isArray(Reflect.construct(Array, [], live)));
"#)),
        lines(&["TypeError TypeError TypeError", "true true"])
    );
}

/// JSON.stringify asks a proxy only the traps of SerializeJSONProperty (§25.5.2.2): the
/// wrapper test reads the internal slot and calls no trap (Codex r1 proxy2 [medium]: the
/// `length` probe of `primitive_data` called `getOwnPropertyDescriptor`). Node v24.22.1.
#[test]
fn json_stringify_asks_no_extra_trap() {
    assert_eq!(
        common::run_both(js!(r#"
print(t(() => JSON.stringify(new Proxy([], { getOwnPropertyDescriptor: function () { throw 123; } }))));
var log = [];
var h = {};
['get', 'getOwnPropertyDescriptor', 'ownKeys', 'has'].forEach(function (n) { h[n] = function (t, k) { log.push(n + (k === undefined ? '' : ':' + String(k))); return Reflect[n].apply(null, arguments); }; });
print(JSON.stringify({ a: new Proxy({ x: 1 }, h) }), JSON.stringify(new Proxy([1], h)), JSON.stringify(1, null, new Proxy({}, h)), JSON.stringify({ a: 1 }, new Proxy({}, h)));
print(log.join(' '));
print(JSON.stringify([new Number(1), new String('s'), new Boolean(false)]));
"#)),
        lines(&[
            "[]",
            "{\"a\":{\"x\":1}} [1] 1 {\"a\":1}",
            "get:toJSON ownKeys getOwnPropertyDescriptor:x get:x get:toJSON get:length get:0",
            "[1,\"s\",false]",
        ])
    );
}

/// Function.prototype.toString (§20.2.3.5 step 4) of a callable proxy asks the proxy for
/// nothing: no trap runs, and the name part is empty (M5-P3 PROXY2 need; node v24.22.1
/// prints `function () { [native code] }` and an empty trap log). So a throwing
/// `getOwnPropertyDescriptor` trap and a revoked callable proxy give the same text (M5-P3
/// reviewer A F1: before eecc049 the first threw "boom" and the second a TypeError).
#[test]
fn function_to_string_of_a_proxy_runs_no_trap() {
    assert_eq!(
        common::run_both(js!(r#"
var log = [];
var h = {};
['get', 'getOwnPropertyDescriptor', 'has', 'ownKeys', 'getPrototypeOf'].forEach(function (n) { h[n] = function (t, k) { log.push(n + ':' + String(k)); return Reflect[n].apply(null, arguments); }; });
var p = new Proxy(function g(a) { return a; }, h);
print(Function.prototype.toString.call(p), '|', log.join(' '));
var boom = new Proxy(function h() {}, { getOwnPropertyDescriptor: function () { throw new Error('boom'); } });
print(t(function () { return Function.prototype.toString.call(boom); }));
var r = Proxy.revocable(function h() {}, {});
r.revoke();
print(t(function () { return Function.prototype.toString.call(r.proxy); }));
"#)),
        lines(&[
            "function () { [native code] } | ",
            "function () { [native code] }",
            "function () { [native code] }"
        ])
    );
}
