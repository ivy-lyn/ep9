//! P5-VM-SMALL: the VM-side fixes that only JS can test (ep9_vm tests cannot compile JS).
//! Every check runs with and without GC stress (`common::run_both`: a collection before
//! every allocation, then the heap verifier). Expected outputs come from node v24.22.1,
//! except where a test says V8's message differs.
//!
//! | Area | VM code |
//! |---|---|
//! | sloppy primitive `this` (OrdinaryCallBindThis step 6.b, H14-Q2) | `call::wraps_this`/`wrap_this`, from `call_fast` (cold), `slow::call`, `call_function_object` |
//! | `instanceof` and `Function.prototype[@@hasInstance]` | `property::instance_of`, `natives::function_has_instance` |
//! | `%Iterator.prototype%[@@toStringTag]` accessor | `natives::iterator_to_string_tag_{get,set}` |
//! | `Object.prototype.valueOf`, `Array.prototype.join`/`toString` ToObject | `natives.rs` |
//! | `Array` with a NewTarget (subclass; `Reflect.construct`) | `natives::array_constructor` |
//! | NativeError constructors inherit from `%Error%` (step 2) | `builtins::install` |
//! | `Array.prototype[@@unscopables]` (step 2) | `natives.rs` VM table, `IntrinsicId::ArrayUnscopables` |

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

// ---- sloppy primitive this ---------------------------------------------------------------------

/// A sloppy callee gets a wrapper object for a number, string, boolean or symbol receiver,
/// and the global object for undefined; a strict callee gets the value as it is. Each call
/// shape: `call`/`apply`/bound (Rust `call_function_object`), a method call on a primitive
/// (`CallProperty*`: T0 `call_fast` goes cold, `slow::call` wraps), a getter (Rust).
#[test]
fn sloppy_callee_wraps_a_primitive_receiver() {
    let src = "
        function sl() { return typeof this; }
        function st() { 'use strict'; return typeof this; }
        print(sl.call(1), sl.call('s'), sl.call(true), sl.call(Symbol()), sl.call(undefined) === typeof globalThis);
        print(st.call(1), st.call('s'), st.call(true), st.call(undefined), st.call(null));
        print(sl.apply(2, []), sl.bind(3)(), st.bind(3)());
        Number.prototype.me = function () { return this; };
        String.prototype.me = function () { return this; };
        Boolean.prototype.st = function () { 'use strict'; return this; };
        var n = (5).me();
        print(typeof n, n instanceof Number, n.valueOf() === 5, n == 5);
        var a = 'x'.me(), b = 'x'.me();
        print(typeof a, a === b, a.length, String(a));
        print(typeof true.st(), true.st());
        Object.defineProperty(Number.prototype, 'gt', { get: function () { return typeof this; }, configurable: true });
        print((1).gt);
        var r = [];
        [1, 2].forEach(function () { r.push(typeof this); }, 7);
        print(r.join());
    ";
    assert_eq!(
        run_both(src),
        lines(&[
            "object object object object true",
            "number string boolean undefined object",
            "object object number",
            "object true true true",
            "object false 1 x",
            "boolean true",
            "object",
            "object,object",
        ])
    );
}

/// The wrap in a hot loop: past the feedback threshold the T0 fast path keeps sending the
/// call cold, and every call gets a fresh wrapper; an object receiver stays on the fast path.
#[test]
fn sloppy_primitive_receiver_in_a_loop() {
    let src = "
        String.prototype.self = function () { return this; };
        var seen = [], o = { f: function () { return this; } };
        for (var i = 0; i < 50; i++) {
            var w = 'ab'.self();
            if (typeof w !== 'object' || w.length !== 2) throw new Error('bad wrap at ' + i);
            seen.push(w);
            if (o.f() !== o) throw new Error('bad object this at ' + i);
        }
        print(seen.length, seen[0] === seen[1], seen[49] + '');
    ";
    assert_eq!(run_both(src), lines(&["50 false ab"]));
}

/// A host function (here `Array.prototype.push` through `call`, and
/// `Object.prototype.toString`) gets the raw receiver: no wrap for host callees.
#[test]
fn host_callee_gets_the_raw_receiver() {
    let src = "
        print(Object.prototype.toString.call(1), Object.prototype.toString.call('s'));
        print(typeof Object.prototype.valueOf.call(true));
    ";
    assert_eq!(
        run_both(src),
        lines(&["[object Number] [object String]", "object"])
    );
}

// ---- instanceof and @@hasInstance --------------------------------------------------------------

#[test]
fn instanceof_uses_has_instance() {
    let src = "
        function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name + ': ' + e.message); } }
        t(function () { return 1 instanceof 1; });
        t(function () { return 1 instanceof {}; });
        t(function () { return ({}) instanceof { [Symbol.hasInstance]: null }; });
        t(function () { return ({}) instanceof { [Symbol.hasInstance]: function (v) { return 'x'; } }; });
        t(function () { return ({}) instanceof { [Symbol.hasInstance]: function (v) { return 0; } }; });
        var seen;
        var O = { [Symbol.hasInstance]: function (v) { seen = [this === O, v]; return 1; } };
        t(function () { return (7 instanceof O) + ' ' + seen.join(); });
        var d = Object.getOwnPropertyDescriptor(Function.prototype, Symbol.hasInstance);
        t(function () { return [d.writable, d.enumerable, d.configurable, d.value.name, d.value.length].join(); });
        t(function () { return Function.prototype[Symbol.hasInstance].call({}, {}); });
        t(function () { return Function.prototype[Symbol.hasInstance].call(Object, {}); });
        t(function () { return Function.prototype[Symbol.hasInstance].call(Array, {}); });
        function F() {}
        var B = F.bind(null), BB = B.bind(null);
        t(function () { return (new F instanceof B) + ' ' + (new F instanceof BB) + ' ' + Function.prototype[Symbol.hasInstance].call(BB, new F); });
        Object.defineProperty(F, Symbol.hasInstance, { value: function () { return true; } });
        t(function () { return (1 instanceof B) + ' ' + (1 instanceof F); });
        t(function () { return [] instanceof Array; });
        function G() {}
        G.prototype = 3;
        t(function () { return ({}) instanceof G; });
        t(function () { return 1 instanceof G; });
    ";
    // V8 adds the value to the non-object prototype message ("... prototype '3' in ..."); ep9
    // keeps its older text.
    assert_eq!(
        run_both(src),
        lines(&[
            "TypeError: Right-hand side of 'instanceof' is not an object",
            "TypeError: Right-hand side of 'instanceof' is not callable",
            "TypeError: Right-hand side of 'instanceof' is not callable",
            "true",
            "false",
            "true true,7",
            "false,false,false,[Symbol.hasInstance],1",
            "false",
            "true",
            "false",
            "true true true",
            "true true",
            "true",
            "TypeError: Function has non-object prototype in instanceof check",
            "false",
        ])
    );
}

/// A non-callable `@@hasInstance` throws; a getter for it runs once per `instanceof`.
#[test]
fn has_instance_getter_and_non_callable() {
    let src = "
        var n = 0;
        var O = {};
        Object.defineProperty(O, Symbol.hasInstance, { get: function () { n++; return function () { return true; }; } });
        print(1 instanceof O, 2 instanceof O, n);
        try { ({}) instanceof { [Symbol.hasInstance]: 1 }; } catch (e) { print(e.constructor.name); }
    ";
    assert_eq!(run_both(src), lines(&["true true 2", "TypeError"]));
}

// ---- %Iterator.prototype%[@@toStringTag] -------------------------------------------------------

#[test]
fn iterator_prototype_to_string_tag_is_an_accessor() {
    let src = "
        function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name + ': ' + e.message); } }
        var IP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
        var g = Object.getOwnPropertyDescriptor(IP, Symbol.toStringTag);
        t(function () { return [typeof g.get, typeof g.set, g.enumerable, g.configurable, g.get.name, g.set.name, g.get.length, g.set.length].join(); });
        t(function () { return Object.prototype.toString.call([].keys()) + ' ' + IP[Symbol.toStringTag] + ' ' + g.get.call(1); });
        t(function () { return Object.prototype.toString.call(Object.create(IP)); });
        t(function () { g.set.call(IP, 'x'); });
        t(function () { IP[Symbol.toStringTag] = 'x'; });
        t(function () { g.set.call(1, 'x'); });
        t(function () {
            var o = Object.create(IP);
            o[Symbol.toStringTag] = 'Q';
            var d = Object.getOwnPropertyDescriptor(o, Symbol.toStringTag);
            return [d.value, d.writable, d.enumerable, d.configurable].join() + ' ' + Object.prototype.toString.call(o);
        });
        t(function () { var o = { [Symbol.toStringTag]: 'aa' }; g.set.call(o, 'a'); return o[Symbol.toStringTag]; });
        t(function () { g.set.call(Object.preventExtensions({}), 'a'); });
        t(function () { return IP[Symbol.toStringTag]; });
    ";
    // V8 appends " '[object Object]'" to the two read-only messages.
    assert_eq!(
        run_both(src),
        lines(&[
            "function,function,false,true,get [Symbol.toStringTag],set [Symbol.toStringTag],0,1",
            "[object Array Iterator] Iterator Iterator",
            "[object Iterator]",
            "TypeError: Cannot assign to read only property 'Symbol(Symbol.toStringTag)' of object",
            "TypeError: Cannot assign to read only property 'Symbol(Symbol.toStringTag)' of object",
            "TypeError: set Iterator.prototype[Symbol.toStringTag] called on non-object",
            "Q,true,true,true [object Q]",
            "a",
            "TypeError: Cannot define property Symbol(Symbol.toStringTag), object is not extensible",
            "Iterator",
        ])
    );
}

// ---- Object.prototype.valueOf, Array.prototype.join / toString ---------------------------------

#[test]
fn value_of_join_and_to_string_apply_to_object() {
    let src = "
        function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name + ': ' + e.message); } }
        t(function () { var w = Object.prototype.valueOf.call(1); return typeof w + ' ' + (w instanceof Number) + ' ' + (w + 1); });
        t(function () { return typeof Object.prototype.valueOf.call('s') + ' ' + typeof Object.prototype.valueOf.call(Symbol()); });
        t(function () { return Object.prototype.valueOf.call(undefined); });
        t(function () { return Array.prototype.join.call(undefined); });
        t(function () { return Array.prototype.toString.call(null); });
        t(function () { return Array.prototype.join.call('abc', '-'); });
        t(function () { return Array.prototype.join.call(5); });
        t(function () { return Array.prototype.join.call({ length: 2, 0: 'a', 1: 'b' }, '+'); });
        t(function () { return Array.prototype.toString.call({ join: function () { return 'J' + typeof this; } }); });
        t(function () { return Array.prototype.toString.call({ join: 1 }); });
        t(function () { return Array.prototype.toString.call(1); });
        t(function () { return Array.prototype.toString.call('ab'); });
        t(function () { return [1, [2, 3], null, undefined].toString(); });
        t(function () { var a = [1, 2]; a.join = function () { return 'own'; }; return String(a); });
        t(function () {
            var saved = Array.prototype.join;
            Array.prototype.join = function () { return 'patched'; };
            var r = [1, 2].toString();
            Array.prototype.join = saved;
            return r + ' ' + [1, 2].toString();
        });
    ";
    assert_eq!(
        run_both(src),
        lines(&[
            "object true 2",
            "object object",
            "TypeError: Cannot convert undefined or null to object",
            "TypeError: Cannot convert undefined or null to object",
            "TypeError: Cannot convert undefined or null to object",
            "a-b-c",
            "",
            "a+b",
            "Jobject",
            "[object Object]",
            "[object Number]",
            "[object String]",
            "1,2,3,,",
            "own",
            "patched 1,2",
        ])
    );
}

// ---- Array with a NewTarget --------------------------------------------------------------------

#[test]
fn array_constructor_takes_the_new_target_prototype() {
    let src = "
        function t(f) { try { print(String(f())); } catch (e) { print(e.constructor.name + ': ' + e.message); } }
        class Sub extends Array {}
        t(function () { var s = new Sub(3); return [s instanceof Sub, s instanceof Array, Array.isArray(s), s.length, Object.getPrototypeOf(s) === Sub.prototype].join(); });
        t(function () { var s = new Sub(1, 2); s.push(3); return [s instanceof Sub, s.length, s.join('')].join(); });
        t(function () { var s = new Sub(); return [s.length, Object.getPrototypeOf(s) === Sub.prototype].join(); });
        t(function () { return new Sub(-1); });
        t(function () { return Object.getPrototypeOf(new Array(2)) === Array.prototype; });
        t(function () { return Object.getPrototypeOf(Array(2)) === Array.prototype; });
        class Sub2 extends Array { constructor(n) { super(n); } }
        t(function () {
            var B = Sub2.bind(null);
            var x = new B(4);
            return [x instanceof Sub2, x.length].join();
        });
        var arrs = [];
        for (var i = 0; i < 25; i++) arrs.push(new Sub(1));
        arrs[0].p = 1;
        arrs.push(new Sub(1));
        t(function () { return arrs.every(function (a) { return Object.getPrototypeOf(a) === Sub.prototype; }); });
    ";
    assert_eq!(
        run_both(src),
        lines(&[
            "true,true,true,3,true",
            "true,3,123",
            "0,true",
            "RangeError: Invalid array length",
            "true",
            "true",
            "true,4",
            "true",
        ])
    );
}

/// `Reflect.construct` (B-REFLECT-GLOBALS) gives `Array` any NewTarget: its `prototype` when it
/// is an object, else the realm's `%Array.prototype%` (GetPrototypeFromConstructor step 4).
#[test]
fn array_constructor_with_reflect_construct() {
    let src = "
        function F() {}
        F.prototype = { tag: 'F' };
        var a = Reflect.construct(Array, [3], F);
        print(Object.getPrototypeOf(a) === F.prototype, Array.isArray(a), a.length, a.tag);
        F.prototype = 1;
        var b = Reflect.construct(Array, [1, 2], F);
        print(Object.getPrototypeOf(b) === Array.prototype, b.join());
        print(Object.getPrototypeOf(Reflect.construct(Array, [], Object)) === Object.prototype);
    ";
    assert_eq!(run_both(src), lines(&["true true 3 F", "true 1,2", "true"]));
}

// ---- NativeError [[Prototype]] (step 2) --------------------------------------------------------

/// §20.5.6.2: each NativeError constructor's [[Prototype]] is %Error%, so statics on `Error`
/// are inherited, and a subclass chain reaches `Error`.
#[test]
fn native_error_constructors_inherit_from_error() {
    let src = "
        var es = [EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError, AggregateError];
        print(es.map(function (E) { return Object.getPrototypeOf(E) === Error; }).join());
        print(Object.getPrototypeOf(Error) === Function.prototype, Error.isPrototypeOf(URIError));
        Error.inherited = 'inh';
        print(TypeError.inherited, RangeError.hasOwnProperty('inherited'), Object.getOwnPropertyNames(SyntaxError).sort().join());
        delete Error.inherited;
        print(TypeError.inherited, TypeError.name, TypeError.length, Object.getPrototypeOf(TypeError.prototype) === Error.prototype);
        class MyErr extends RangeError {}
        var m = new MyErr('m');
        print(Object.getPrototypeOf(MyErr) === RangeError, m instanceof RangeError, m instanceof Error, m.message, m.name, String(m));
        print(Object.prototype.toString.call(m), Object.prototype.toString.call(EvalError));
    ";
    assert_eq!(
        run_both(src),
        lines(&[
            "true,true,true,true,true,true,true",
            "true true",
            "inh false length,name,prototype",
            "undefined TypeError 1 true",
            "true true true m RangeError RangeError: m",
            "[object Error] [object Function]",
        ])
    );
}

// ---- Array.prototype[@@unscopables] (step 2) ---------------------------------------------------

/// §23.1.3.41: a null-prototype object with 16 `true` {w, e, c} keys in spec order, on
/// `Array.prototype` with {c}; `with` skips those names on an array (§9.1.1.2.1 step 6).
#[test]
fn array_prototype_unscopables() {
    let src = "
        var u = Array.prototype[Symbol.unscopables];
        print(Object.getPrototypeOf(u), Reflect.ownKeys(u).join());
        var d = Object.getOwnPropertyDescriptor(u, 'flat');
        print(d.value, d.writable, d.enumerable, d.configurable, Object.keys(u).length);
        var p = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.unscopables);
        print(p.writable, p.enumerable, p.configurable, p.value === u, 'get' in p);
        print(Object.getOwnPropertySymbols(Array.prototype).map(String).join());
        var keys = 'k', values = 'v', at = 'a', join = 'j';
        with ([1, 2]) { print(keys, values, at, typeof join, length); }
        u.join = true;
        with ([]) { print(typeof join); }
        delete u.join;
        delete Array.prototype[Symbol.unscopables];
        with ([]) { print(typeof keys, typeof values); }
        print(Symbol.unscopables in Array.prototype, Object.getOwnPropertySymbols(u).length);
    ";
    assert_eq!(
        run_both(src),
        lines(&[
            "null at,copyWithin,entries,fill,find,findIndex,findLast,findLastIndex,flat,flatMap,includes,keys,toReversed,toSorted,toSpliced,values",
            "true true true true 16",
            "false false true true false",
            "Symbol(Symbol.iterator),Symbol(Symbol.unscopables)",
            "k v a function 2",
            "string",
            "function function",
            "false 0",
        ])
    );
}

/// Reflect (B-REFLECT-GLOBALS) calls accessors and functions from Rust
/// (`call_function_object`), so a sloppy getter, setter or function gets a wrapper for a
/// primitive receiver (H14-Q2), and a strict one the value as it is.
#[test]
fn reflect_wraps_a_primitive_receiver_for_sloppy_callees() {
    let src = "
        var r = [];
        var o = { get x() { return typeof this; }, set y(v) { r.push(typeof this); } };
        print(Reflect.get(o, 'x', 1), Reflect.get(o, 'x', 's'), Reflect.get(o, 'x'));
        Reflect.set(o, 'y', 0, true);
        print(r.join());
        print(Reflect.apply(function () { return typeof this; }, 2, []), Reflect.apply(function () { 'use strict'; return typeof this; }, 2, []));
        print(Reflect.construct(function () { this.t = typeof this; }, []).t);
    ";
    assert_eq!(
        run_both(src),
        lines(&["object object object", "object", "object number", "object"])
    );
}
