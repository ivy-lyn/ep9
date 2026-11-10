//! M7-P5 SPEC-BUILTINS5: the VM language residue of the V12-D25 pick (P5-DL-1): frozen
//! template objects, the HasProperty step of an object record's SetMutableBinding, and
//! `delete` on a primitive base.
//!
//! `common::run_both` runs each script with and without GC stress (a collection before every
//! allocation, then the heap verifier); `print` is the arguments' ToString joined by spaces.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// GetTemplateObject (§13.2.8.4): SetIntegrityLevel(rawObj, frozen), then the `raw`
/// property {w: false, e: false, c: false}, then SetIntegrityLevel(template, frozen). Both
/// arrays are frozen and not extensible; each element and `length` are {w: false, c: false};
/// a sloppy write does nothing and a strict write, an add, `push`, `defineProperty` and
/// `setPrototypeOf` throw a TypeError. One site gives the same object each time. The lines
/// are node v26.13.0's for the same script. On the old code the second line is
/// "false false true true", and the script throws a TypeError later (`setPrototypeOf`
/// succeeds, so `s.map` is gone).
#[test]
fn template_objects_are_frozen() {
    assert_eq!(
        common::run_both(
            r#"
            function d(o, k) { var x = Object.getOwnPropertyDescriptor(o, k); return x ? [x.writable, x.enumerable, x.configurable].join('/') : 'none'; }
            function t(f) { try { return 'ok:' + f(); } catch (e) { return e.constructor.name; } }
            function tag(s) { return s; }
            function site() { return tag`a${1}b${2}\u{`; }
            var s = site();
            print(s === site());
            print(Object.isFrozen(s), Object.isFrozen(s.raw), Object.isExtensible(s), Object.isExtensible(s.raw));
            print(JSON.stringify(Array.from(s)), JSON.stringify(Array.from(s.raw)));
            print(d(s, '0'), d(s, 'length'), d(s, 'raw'), d(s.raw, '0'), d(s.raw, 'length'));
            print(Object.getOwnPropertyNames(s).join(), Object.getOwnPropertyNames(s.raw).join());
            s[0] = 'z'; s.length = 0; s.x = 1; s.raw[0] = 'z';
            print(s[0], s.length, s.x, s.raw[0], delete s[0], delete s.raw);
            print(t(function () { return s.push(1); }), t(function () { 'use strict'; s[0] = 1; }), t(function () { 'use strict'; s.raw.y = 1; }), t(function () { 'use strict'; s.length = 9; }));
            print(t(function () { Object.defineProperty(s, '0', { value: 'q' }); }), t(function () { return Object.setPrototypeOf(s, null) === s; }), Object.getPrototypeOf(s) === Array.prototype);
            var many = (function (x) { return x; })`${0}${1}${2}${3}${4}${5}${6}${7}${8}${9}${10}${11}${12}${13}${14}${15}${16}${17}${18}${19}`;
            var e = (function (x) { return x; })``;
            print(many.length, Object.isFrozen(many), Object.isFrozen(many.raw), e.length, Object.isFrozen(e), JSON.stringify(e.raw));
            print(String.raw`x\ny${1}z`, Object.isFrozen(s.slice()), JSON.stringify(s.map(function (x) { return x === undefined ? 'U' : x; })));
"#
        ),
        lines(&[
            "true",
            "true true false false",
            r#"["a","b",null] ["a","b","\\u{"]"#,
            "false/true/false false/false/false false/false/false false/true/false false/false/false",
            "0,1,2,length,raw 0,1,2,length",
            "a 3 undefined a false false",
            "TypeError TypeError TypeError TypeError",
            "TypeError TypeError true",
            "21 true true 1 true [\"\"]",
            r#"x\ny1z false ["a","b","U"]"#,
        ])
    );
}

/// SetMutableBinding of an object record (§9.1.1.2.5): step 2 HasProperty runs in sloppy
/// code too, so a proxy's `has` trap sees it after HasBinding's (and after
/// GetBindingValue's in a compound assignment); step 3 throws a ReferenceError only in
/// strict code; step 4 sets the property again when it is gone (sloppy). node v26.13.0 gives
/// "has:p,set:p 1", "has:p,has:p,set:p 3" and "v 3 undefined" (V8 leaves out step 2) and
/// `ok:undefined ["r"]` for the strict line (V8 leaves out step 3), so those 4 lines come
/// from the spec steps; the first 2 are the forms that test262
/// statements/with/set-mutable-binding-idref-with-proxy-env.js and
/// -compound-assign-with-proxy-env.js assert. The other lines are node's. On the old code the
/// first line is "has:p,set:p 1".
#[test]
fn with_set_mutable_binding_asks_has_property() {
    assert_eq!(
        common::run_both(
            r#"
            function t(f) { try { return 'ok:' + f(); } catch (e) { return e.constructor.name; } }
            var log = [];
            var env = { p: 0 };
            var px = new Proxy(env, {
              has: function (o, k) { log.push('has:' + String(k)); return Reflect.has(o, k); },
              set: function (o, k, v, r) { log.push('set:' + String(k)); return Reflect.set(o, k, v, r); },
            });
            with (px) { p = 1; }
            print(log.join(), env.p);
            log = [];
            with (px) { p += 2; }
            print(log.join(), env.p);
            var o2 = { get [Symbol.unscopables]() { delete o2.q; return undefined; }, q: 1 };
            with (o2) { q = 5; }
            print(JSON.stringify(o2));
            var o3 = { get [Symbol.unscopables]() { delete o3.r; return undefined; }, r: 1 };
            var res;
            with (o3) { res = t(function () { 'use strict'; r = 7; }); }
            print(res, JSON.stringify(Object.keys(o3)));
            var o4 = { s: 1 };
            with (o4) { (function () { 'use strict'; s = 2; })(); }
            print(o4.s);
            var log5 = [];
            var target5 = { v: 1 };
            var px5 = new Proxy(target5, { has: function (o, k) { log5.push(String(k)); return log5.length < 2; } });
            with (px5) { v = 3; }
            print(log5.join(), target5.v, typeof globalThis.v);
"#
        ),
        lines(&[
            "has:p,has:p,set:p 1",
            "has:p,has:p,has:p,set:p 3",
            r#"{"q":5}"#,
            "ReferenceError []",
            "2",
            "v,v 3 undefined",
        ])
    );
}

/// `delete` on a primitive base (§13.5.1.2): `ToObject(base).[[Delete]](key)`. A String's
/// indices below its length and its `length` are own and not configurable: false, and a
/// TypeError in strict code. Every other key (an index past the end, "-0", "01", a symbol, an
/// inherited method) and every key of another primitive gives true. The key is converted
/// once; `null` and `undefined` throw a TypeError. The lines are node v26.13.0's for the
/// same script. On the old code the first line is "false false false false" and the strict
/// index line is "ok:false".
#[test]
fn delete_on_a_primitive_base() {
    assert_eq!(
        common::run_both(
            r#"
            function t(f) { try { return 'ok:' + f(); } catch (e) { return e.constructor.name; } }
            print(delete 'abc'[0], delete 'abc'[2], delete 'abc'[3], delete 'abc'[100]);
            print(delete 'abc'.length, delete 'abc'.foo, delete 'abc'['-0'], delete 'abc'['1.0'], delete 'abc'['01']);
            print(delete 'abc'[Symbol.iterator], delete 'abc'.charAt, delete ''[0], delete ''.length);
            print(delete (5).toFixed, delete (5)[0], delete true.x, delete Symbol().description, delete (1n).x);
            print(t(function () { 'use strict'; return delete 'abc'[1]; }), t(function () { 'use strict'; return delete 'abc'.length; }));
            print(t(function () { 'use strict'; return delete 'abc'[5]; }), t(function () { 'use strict'; return delete 'abc'.x; }), t(function () { 'use strict'; return delete (5).x; }));
            var k = 0;
            var ko = { toString: function () { k++; return '1'; } };
            print(delete 'abc'[ko], k);
            print(t(function () { 'use strict'; return delete null.x; }), t(function () { return delete undefined[0]; }));
            var big = 'x'.repeat(70000);
            print(delete big[69999], delete big[70000], delete new String('abc')[0], delete new String('abc')[5]);
"#
        ),
        lines(&[
            "false false true true",
            "false true true true true",
            "true true true false",
            "true true true true true",
            "TypeError TypeError",
            "ok:true ok:true ok:true",
            "false 1",
            "TypeError TypeError",
            "false true false true",
        ])
    );
}
