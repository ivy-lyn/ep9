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
