//! Reflect (§28.1): the 13 functions, the receiver forms of get and set, construct with a
//! newTarget, the private-name check of ownKeys (M2-P4-REFLECT-PRIV), and `%Reflect%` itself.
//!
//! The expected lines are node v24.22.1's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Each function once on ordinary objects and arrays: the result, and the effect.
#[test]
fn every_function() {
    assert_eq!(
        common::run_both(
            r#"
            function f(a, b) { return [this === o, a, b, arguments.length].join(); }
            var o = {};
            print(Reflect.apply(f, o, [1, 2]), Reflect.apply(f, o, { length: 3, 0: 'x' }), Reflect.apply(Math.max, null, [3, 9, 4]));
            var d = {};
            print(Reflect.defineProperty(d, 'x', { value: 1 }), Reflect.defineProperty(d, 'x', { value: 2 }), d.x, Reflect.defineProperty(Object.freeze({}), 'y', { value: 1 }));
            print(Reflect.defineProperty(d, 'g', { get: function () { return 7; }, configurable: true }), d.g);
            var e = { a: 1 }; Object.defineProperty(e, 'b', { value: 2 });
            print(Reflect.deleteProperty(e, 'a'), 'a' in e, Reflect.deleteProperty(e, 'b'), e.b, Reflect.deleteProperty(e, 'zz'));
            print(Reflect.get({ p: 5 }, 'p'), Reflect.get([1, 2], 1), Reflect.get([1, 2], 'length'), Reflect.get(Object.create({ q: 3 }), 'q'));
            var desc = Reflect.getOwnPropertyDescriptor({ k: 4 }, 'k');
            print(desc.value, desc.writable, desc.enumerable, desc.configurable, Reflect.getOwnPropertyDescriptor({}, 'k'));
            var acc = Reflect.getOwnPropertyDescriptor(Object.prototype, '__proto__');
            print(typeof acc.get, typeof acc.set, acc.enumerable, acc.configurable, 'value' in acc);
            print(Reflect.getPrototypeOf([]) === Array.prototype, Reflect.getPrototypeOf(Object.create(null)));
            print(Reflect.has({ a: 1 }, 'a'), Reflect.has(Object.create({ b: 1 }), 'b'), Reflect.has({}, 'c'), Reflect.has([], 'length'), Reflect.has({ 1: 0 }, 1));
            var x = {};
            print(Reflect.isExtensible(x), Reflect.preventExtensions(x), Reflect.isExtensible(x), Reflect.preventExtensions(x));
            var s = Symbol('s'), k = { b: 1, a: 2, 1: 3, 0: 4 }; k[s] = 5;
            print(Reflect.ownKeys(k).map(String).join(), Reflect.ownKeys([7, 8]).join(), Reflect.ownKeys((a) => {}).join());
            var t = {};
            print(Reflect.set(t, 'v', 1), t.v, Reflect.set(Object.freeze({ w: 1 }), 'w', 2), Reflect.set([], 'length', 3));
            var p = {};
            print(Reflect.setPrototypeOf(p, Array.prototype), p instanceof Array, Reflect.setPrototypeOf(p, null), Reflect.getPrototypeOf(p));
            print(Reflect.setPrototypeOf(Object.preventExtensions({}), {}), Reflect.setPrototypeOf(Array.prototype, Object.create(Array.prototype)));
            var keyCalls = 0;
            var key = { toString: function () { keyCalls++; return 'kk'; } };
            print(Reflect.get({ kk: 'got' }, key), Reflect.has({ kk: 1 }, key), keyCalls);
"#
        ),
        lines(&[
            "true,1,2,2 true,x,,3 9",
            "true false 1 false",
            "true 7",
            "true false false 2 true",
            "5 2 2 3",
            "4 true true true undefined",
            "function function false true false",
            "true null",
            "true true false true true",
            "true true false true",
            "0,1,b,a,Symbol(s) 0,1,length length,name",
            "true 1 false true",
            "true true true null",
            "false false",
            "got true 2",
        ])
    );
}

/// Reflect.get and Reflect.set with a receiver (`Cx::get_with_receiver` /
/// `Cx::set_with_receiver`, M2-P5-PLAN-D2): a getter and a setter see the receiver as `this`;
/// a data property lands on the receiver, and fails on a read-only or accessor property of
/// it, on a primitive receiver and on a non-extensible one.
#[test]
fn get_and_set_pass_the_receiver() {
    assert_eq!(
        common::run_both(
            r#"
            var src = { get g() { return this.tag; }, set s(v) { this.seen = v; }, d: 1 };
            var recv = { tag: 'receiver' };
            print(Reflect.get(src, 'g'), Reflect.get(src, 'g', recv), Reflect.get(src, 'g', undefined), Reflect.get(src, 'd', recv));
            print(Reflect.get(Object.create(src), 'g', { tag: 'deep' }));
            print(Reflect.set(src, 's', 9, recv), recv.seen, src.seen);
            var target = { d: 1 }, other = {};
            print(Reflect.set(target, 'd', 2, other), target.d, other.d, Object.getOwnPropertyDescriptor(other, 'd').enumerable);
            var ro = {}; Object.defineProperty(ro, 'd', { value: 0, writable: false, configurable: true });
            print(Reflect.set(target, 'd', 3, ro), ro.d);
            var accR = { get d() { return 'a'; } };
            print(Reflect.set(target, 'd', 4, accR), accR.d);
            print(Reflect.set(target, 'n', 5, 1), Reflect.set(target, 'n', 5, null));
            var noSetter = { get only() { return 1; } };
            print(Reflect.set(noSetter, 'only', 2), Reflect.set(Object.create(noSetter), 'only', 2, {}));
            var prim = { get me() { 'use strict'; return typeof this; } };
            print(Reflect.get(prim, 'me', 5), Reflect.get(prim, 'me', 'str'));
            var arr = [1, 2];
            print(Reflect.set({}, 0, 'z', arr), arr[0], arr.length, Reflect.set({}, 5, 'y', arr), arr.length);
            var ne = Object.preventExtensions({});
            print(Reflect.set({}, 'q', 1, ne), 'q' in ne);
"#
        ),
        lines(&[
            "undefined receiver undefined 1",
            "deep",
            "true 9 undefined",
            "true 1 2 true",
            "false 0",
            "false a",
            "false false",
            "false false",
            "number string",
            "true z 2 true 6",
            "false false",
        ])
    );
}

/// Reflect.construct: the prototype comes from newTarget (ordinary, class, Number and Error
/// constructors, a bound function), and IsConstructor is the real answer, as
/// harness/isConstructor.js asks it (M2-P5-PLAN-D12).
#[test]
fn construct_with_new_target() {
    assert_eq!(
        common::run_both(
            r#"
            function F(a) { this.a = a; this.nt = new.target; }
            function G() {}
            G.prototype.mark = 'G';
            var o = Reflect.construct(F, [1]);
            print(o.a, o.nt === F, o instanceof F);
            var p = Reflect.construct(F, [2], G);
            print(p.a, p.nt === G, p.mark, Object.getPrototypeOf(p) === G.prototype, p instanceof F);
            class A { constructor(x) { this.x = x; this.t = new.target.name; } }
            class B extends A {}
            var q = Reflect.construct(A, [3], B);
            print(q.x, q.t, q instanceof B);
            var n = Reflect.construct(Number, ['7'], G);
            print(typeof n, Object.getPrototypeOf(n) === G.prototype, Number.prototype.valueOf.call(Reflect.construct(Number, ['8'])));
            var bound = F.bind(null, 'b');
            var r = Reflect.construct(bound, []);
            print(r.a, r.nt === F);
            var e = Reflect.construct(Error, ['m'], TypeError);
            print(e.message, e instanceof TypeError, Object.prototype.toString.call(e));
            var nt = function () {}.bind();
            var s = Reflect.construct(String, ['ab'], nt);
            print(s.length, typeof s);
            function isConstructor(f) { try { Reflect.construct(String, [], f); } catch (err) { return false; } return true; }
            print(isConstructor(Object), isConstructor(Array), isConstructor(Boolean), isConstructor(Error), isConstructor(Function), isConstructor(Number), isConstructor(String), isConstructor(Promise), isConstructor(TypeError), isConstructor(AggregateError));
            print(isConstructor(Math.max), isConstructor(() => {}), isConstructor(Symbol), isConstructor(parseInt), isConstructor(Reflect.get), isConstructor(function* () {}), isConstructor({ m() {} }.m), isConstructor(class {}));
"#
        ),
        lines(&[
            "1 true true",
            "2 true G true false",
            "3 B true",
            "object true 8",
            "b true",
            "m true [object Error]",
            "2 object",
            "true true true true true true true true true true",
            "false false true false false false false true",
        ])
    );
}

/// M2-P4-REFLECT-PRIV: `[[OwnPropertyKeys]]` never lists a private name (§6.2.12), so
/// Reflect.ownKeys shows no private field, method or accessor, of an instance or of the
/// class (the class of tests/object.rs:573-582, plus a static private field).
#[test]
fn own_keys_lists_no_private_name() {
    assert_eq!(
        common::run_both(
            r#"
            class C {
              #x = 1;
              #m() { return 'm'; }
              get #a() { return this.#x * 10; }
              set #a(v) { this.#x = v; }
              pub = 2;
              static #s = 3;
              static k() { return C.#s; }
              read() { return this.#x + this.#a + this.#m(); }
            }
            var c = new C();
            print(Reflect.ownKeys(c).length, Reflect.ownKeys(c).map(String).join(), c.read());
            print(Reflect.ownKeys(C).map(String).join(), Reflect.ownKeys(C.prototype).map(String).join());
            print(Reflect.has(c, 'pub'), Reflect.preventExtensions(c), c.read(), Reflect.ownKeys(c).join());
"#
        ),
        lines(&[
            "1 pub 11m",
            "length,name,prototype,k constructor,read",
            "true true 11m pub",
        ])
    );
}

/// `%Reflect%`: an ordinary object (prototype %Object.prototype%, extensible, not callable),
/// its keys in V8's order, the global binding {w, c}, the functions {w, c} and not
/// constructors, and `@@toStringTag` "Reflect" {c}.
#[test]
fn reflect_object() {
    assert_eq!(
        common::run_both(
            r#"
            print(typeof Reflect, String(Reflect), Object.getPrototypeOf(Reflect) === Object.prototype);
            print(Reflect.ownKeys(Reflect).map(String).join());
            var d = Object.getOwnPropertyDescriptor(globalThis, 'Reflect');
            print(d.writable, d.enumerable, d.configurable, d.value === Reflect);
            var t = Object.getOwnPropertyDescriptor(Reflect, Symbol.toStringTag);
            print(t.value, t.writable, t.enumerable, t.configurable);
            var g = Object.getOwnPropertyDescriptor(Reflect, 'get');
            print(g.writable, g.enumerable, g.configurable, Reflect.get.name, Reflect.get.length, Reflect.getOwnPropertyDescriptor.name);
            try { new Reflect.apply(); } catch (e) { print(e.name); }
            try { Reflect(); } catch (e) { print(e.name); }
            print('prototype' in Reflect.get, Object.isExtensible(Reflect));
"#
        ),
        lines(&[
            "object [object Reflect] true",
            "defineProperty,deleteProperty,apply,construct,get,getOwnPropertyDescriptor,getPrototypeOf,has,isExtensible,ownKeys,preventExtensions,set,setPrototypeOf,Symbol(Symbol.toStringTag)",
            "true false true true",
            "Reflect false false true",
            "true false true get 2 getOwnPropertyDescriptor",
            "TypeError",
            "TypeError",
            "false true",
        ])
    );
}

/// The TypeErrors of step 1 (and IsCallable / IsConstructor, CreateListFromArrayLike,
/// ToPropertyDescriptor and the proto check), in spec order: a target check comes before
/// ToPropertyKey; abrupt completions of ToPropertyKey, the list and the call pass through.
#[test]
fn non_object_targets_throw() {
    assert_eq!(
        common::run_both(
            r#"
            var names = [];
            function t(f) { try { f(); names.push('none'); } catch (e) { names.push(e.constructor === TypeError ? 'T' : e.name); } }
            t(function () { Reflect.apply(1, null, []); });
            t(function () { Reflect.apply(function () {}, null); });
            t(function () { Reflect.construct(1, []); });
            t(function () { Reflect.construct(function () {}, [], Math.max); });
            t(function () { Reflect.construct(function () {}, [], undefined); });
            t(function () { Reflect.construct(() => {}, []); });
            t(function () { Reflect.construct(function () {}); });
            t(function () { Reflect.defineProperty(1, 'x', {}); });
            t(function () { Reflect.defineProperty({}, 'x', 1); });
            t(function () { Reflect.deleteProperty('s', 'x'); });
            t(function () { Reflect.get(null, 'x'); });
            t(function () { Reflect.getOwnPropertyDescriptor(true, 'x'); });
            t(function () { Reflect.getPrototypeOf(1); });
            t(function () { Reflect.has(Symbol(), 'x'); });
            t(function () { Reflect.isExtensible(1); });
            t(function () { Reflect.ownKeys(undefined); });
            t(function () { Reflect.preventExtensions(1); });
            t(function () { Reflect.set(1, 'x', 1); });
            t(function () { Reflect.setPrototypeOf(1, null); });
            t(function () { Reflect.setPrototypeOf({}, 1); });
            t(function () { Reflect.setPrototypeOf({}); });
            t(function () { Reflect.get({}, { toString: function () { throw new RangeError('k'); } }); });
            t(function () { Reflect.get(1, { toString: function () { throw new RangeError('k'); } }); });
            t(function () { Reflect.apply(function () {}, null, { get length() { throw new RangeError('len'); } }); });
            t(function () { Reflect.apply(function () { throw new SyntaxError('in'); }, null, []); });
            print(names.join());
"#
        ),
        lines(&["T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,RangeError,T,RangeError,SyntaxError",])
    );
}
