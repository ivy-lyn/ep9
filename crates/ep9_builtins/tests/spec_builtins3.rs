//! M7-P4 SPEC-BUILTINS3: the Error package of the V12-D25 pick (route ERROR-STACK).
//!
//! `common::run_both` runs each script with and without GC stress (a collection before every
//! allocation, then the heap verifier); `print` is the arguments' ToString joined by spaces.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// InstallErrorCause (§20.5.8.1) in `Error` and the 6 native errors: an own {w, c} `cause`
/// only when `options` is an object that has `cause` (an inherited one too); ToString of the
/// message first, then HasProperty, then Get; a throw of the getter. The lines are node
/// v26.13.0's for the same script. On the old code the first `cause` read is undefined, so
/// the script throws at its first line.
#[test]
fn error_constructors_install_the_cause() {
    assert_eq!(
        common::run_both(
            r#"
            function t(f) { try { return f(); } catch (e) { return 'THROW ' + (e && e.message || e); } }
            var c = { m: 1 };
            var e = new Error('m', { cause: c });
            var d = Object.getOwnPropertyDescriptor(e, 'cause');
            print(d.value === c, d.writable, d.enumerable, d.configurable);
            print('cause' in new Error('m'), Object.hasOwn(new Error('m', {}), 'cause'), Object.hasOwn(new Error('m', { cause: undefined }), 'cause'));
            [Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError].forEach(function (C) { print(C.name, new C('x', { cause: 5 }).cause, C('y', { cause: 6 }).cause, C.length); });
            var log = [];
            var opts = new Proxy({ cause: 1 }, { has(t, k) { log.push('has ' + String(k)); return k in t; }, get(t, k) { log.push('get ' + String(k)); return t[k]; } });
            new Error({ toString() { log.push('toString'); return 'm'; } }, opts);
            print(log.join());
            print(new Error('m', Object.create({ cause: 'proto' })).cause);
            print(Object.hasOwn(new Error('m', 'cause'), 'cause'), Object.hasOwn(new Error('m', 1), 'cause'), Object.hasOwn(new Error(undefined, { cause: 0 }), 'message'));
            print(t(function () { return new Error('m', { get cause() { throw new Error('g'); } }); }));
            print(Object.getOwnPropertyNames(new Error('m', { cause: 1 })).filter(function (k) { return k !== 'stack'; }).join());
            class E2 extends RangeError {}
            var e2 = new E2('m', { cause: 9 });
            print(e2.cause, e2 instanceof RangeError, Object.getPrototypeOf(e2) === E2.prototype);
            var e3 = Reflect.construct(TypeError, ['m', { cause: 'r' }], Object);
            print(e3.cause, Object.getPrototypeOf(e3) === Object.prototype);
            print(String(new Error('m', { cause: 'c' })), JSON.stringify(new Error('m', { cause: 'c' })));
"#
        ),
        lines(&[
            "true true false true",
            "false false true",
            "Error 5 6 1",
            "EvalError 5 6 1",
            "RangeError 5 6 1",
            "ReferenceError 5 6 1",
            "SyntaxError 5 6 1",
            "TypeError 5 6 1",
            "URIError 5 6 1",
            "toString,has cause,get cause",
            "proto",
            "false false false",
            "THROW g",
            "message,cause",
            "9 true true",
            "r true",
            "Error: m {}",
        ])
    );
}

/// `Error.prototype.stack` (error-stack-accessor, Stage 3), form H (M6-ORCH-296): the
/// accessor {e: false, c: true} with "get stack" (length 0) and "set stack" (length 1), not
/// constructors; the getter gives a string for every object with `[[ErrorData]]` (also
/// AggregateError, SuppressedError, a subclass, a foreign new.target) and undefined for any
/// other object (a Proxy of an error too), a TypeError for a primitive; it reads no own
/// `stack`. The setter takes only a string, throws on %Error.prototype% and runs
/// SetterThatIgnoresPrototypeProperties (CreateDataPropertyOrThrow, or Set, with the traps of
/// a Proxy). Each new error still has its own non-enumerable `stack` (form H). node v26.13.0
/// has no `Error.prototype.stack` (an own accessor on each instance instead), so these lines
/// come from the proposal steps, not from node. On the old code the first line throws (no
/// accessor).
#[test]
fn error_prototype_stack_is_an_accessor() {
    assert_eq!(
        common::run_both(
            r#"
            function t(f) { try { return 'ok ' + f(); } catch (e) { return 'THROW ' + e.constructor.name; } }
            var d = Object.getOwnPropertyDescriptor(Error.prototype, 'stack');
            var get = d.get, set = d.set;
            print(typeof get, typeof set, d.enumerable, d.configurable, get.name, get.length, set.name, set.length);
            print(t(function () { return new get(); }), t(function () { return new set('s'); }), Object.hasOwn(get, 'prototype'));
            var errs = [new Error('m'), new TypeError('m'), RangeError('m'), new AggregateError([], 'm'), new SuppressedError(1, 2, 'm'), new (class extends URIError {})('m'), Reflect.construct(EvalError, ['m'], Object)];
            print(errs.map(function (e) { return typeof get.call(e); }).join());
            var e = (function thrower() { return new TypeError('boom'); })();
            var lines = get.call(e).split('\n');
            print(lines.length, lines[0], lines[1].indexOf('    at test.js:') === 0);
            print([{}, [], function () {}, /r/, new Date(0), Object.create(Error.prototype), Error.prototype, TypeError.prototype, new Proxy(new Error('p'), {})].map(function (o) { return get.call(o); }).join());
            print([undefined, null, true, 1, '', Symbol(), 1n].map(function (v) { return t(function () { return get.call(v); }); }).join());
            print(Error.prototype.stack, Object.hasOwn(e, 'stack'), Object.getOwnPropertyDescriptor(e, 'stack').enumerable);
            Object.defineProperty(e, 'stack', { value: 'X', configurable: true, writable: true });
            print(e.stack, get.call(e) !== 'X', delete e.stack, typeof e.stack, e.stack === get.call(e));
            print([undefined, null, 1, {}, new String('s'), Symbol()].map(function (v) { return t(function () { return set.call(e, v); }); }).join());
            print([undefined, null, 1, 'x'].map(function (v) { return t(function () { return set.call(v, 's'); }); }).join());
            print(t(function () { return set.call(Error.prototype, 's'); }), t(function () { 'use strict'; Error.prototype.stack = 's'; }), t(function () { Error.prototype.stack = 's'; }), typeof Object.getOwnPropertyDescriptor(Error.prototype, 'stack').get);
            var p = {}; print(set.call(p, 'v'), JSON.stringify(Object.getOwnPropertyDescriptor(p, 'stack')));
            var f = new Error('f'); print(set.call(f, 'w'), JSON.stringify(Object.getOwnPropertyDescriptor(f, 'stack')));
            var nw = new Error('n'); Object.defineProperty(nw, 'stack', { value: 'o', writable: false, configurable: true });
            print(t(function () { return set.call(nw, 'u'); }), nw.stack);
            var fr = Object.freeze({}); print(t(function () { return set.call(fr, 'u'); }), Object.hasOwn(fr, 'stack'));
            var acc, wa = new Error('a'); Object.defineProperty(wa, 'stack', { get: function () { return 'g'; }, set: function (v) { acc = v; }, configurable: true });
            print(set.call(wa, 'z'), acc);
            var log = [];
            var px = new Proxy({}, { getOwnPropertyDescriptor: function (t, k) { log.push('gopd ' + k); return Reflect.getOwnPropertyDescriptor(t, k); }, defineProperty: function (t, k, dd) { log.push('define ' + k + ' ' + dd.value + dd.writable + dd.enumerable + dd.configurable); return Reflect.defineProperty(t, k, dd); } });
            set.call(px, 'q'); print(log.join());
            var o2 = Object.create(Error.prototype); o2.stack = 'own'; print(JSON.stringify(Object.getOwnPropertyDescriptor(o2, 'stack')));
            var base = new Error('b'); delete base.stack; var child = { __proto__: base }; print(child.stack);
"#
        ),
        lines(&[
            "function function false true get stack 0 set stack 1",
            "THROW TypeError THROW TypeError false",
            "string,string,string,string,string,string,string",
            "2 TypeError: boom true",
            ",,,,,,,,",
            "THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError",
            "undefined true false",
            "X true true string true",
            "THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError",
            "THROW TypeError,THROW TypeError,THROW TypeError,THROW TypeError",
            "THROW TypeError THROW TypeError THROW TypeError function",
            "undefined {\"value\":\"v\",\"writable\":true,\"enumerable\":true,\"configurable\":true}",
            "undefined {\"value\":\"w\",\"writable\":true,\"enumerable\":false,\"configurable\":true}",
            "THROW TypeError o",
            "THROW TypeError false",
            "undefined z",
            "gopd stack,define stack qtruetruetrue",
            "{\"value\":\"own\",\"writable\":true,\"enumerable\":true,\"configurable\":true}",
            "undefined",
        ])
    );
}
