//! JS-level tests of `with` and dynamic scope lookup (P2-LOOKUP), run through BytecodeGen.
//! The expected lines come from node v24 (`print` = `console.log`). Every check runs with
//! and without GC stress (`common::run_both`). The hand-built VM tests are in
//! `crates/ep9_vm/src/tests/with_stmt.rs`.

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `with (o) f()` calls `f` with `this === o` (WithBaseObject), also for `(f)()`, a tagged
/// template, a spread call and an optional call; a function found outside the with object
/// gets an undefined receiver.
#[test]
fn a_call_in_with_gets_the_object_as_this() {
    assert_eq!(
        run_both(
            r#"var o = { f: function () { return this === o; } };
with (o) { print(f()); print((f)()); }
var tt = { tag: function (s) { return this === tt; } };
with (tt) { print(tag`x`); }
var sp = { f: function () { return [this === sp, arguments.length]; } };
with (sp) { print(f(...[1, 2, 3]).join()); }
var opt = { m: function () { return this === opt; } };
with (opt) { print(m?.()); }
with ({ oc: undefined }) { print(oc?.()); }
function C() {} C.prototype.who = function () { return this instanceof C; };
with (new C()) { print(who()); }
function outerF() { 'use strict'; return this; }
with (o) { print(outerF() === undefined); }
with ({ nf: 1 }) { try { nf(); } catch (e) { print(e.name); } }"#
        ),
        lines(&[
            "true",
            "true",
            "true",
            "true,3",
            "true",
            "undefined",
            "true",
            "true",
            "TypeError"
        ])
    );
}

/// Reads and stores go to the with object when it has the name, else on to the enclosing
/// function, the global object, or (sloppy store) a new global.
#[test]
fn reads_and_stores() {
    assert_eq!(
        run_both(
            r#"var o = { x: 1 };
with (o) { print(x); x = 2; print(o.x); }
var y = 10;
with (o) { print(y); y = 11; }
print(y);
with (o) { z = 5; }
print(typeof z, z, 'z' in o);
print(typeof undefinedThing);
try { with (o) { nope; } } catch (e) { print(e.name + ': ' + e.message); }
function g() { var a = 1; with ({}) { a = 2; } return a; }
print(g());
var cc = { n: 1 }; with (cc) { n += 5; n++; } print(cc.n);
with (cc) { print(typeof n, typeof missingName); }
with ({ a1: 1 }) { with ({ a2: 2 }) { print(a1 + a2); a1 = 10; } }
with ('abc') { print(length); }
try { with (null) {} } catch (e) { print(e.name); }
var so = { set sv(v) { print('set', v, this === so); }, get sv() { return 'got'; } };
with (so) { sv = 9; print(sv); }
var nw = {}; Object.defineProperty(nw, 'ro', { value: 1, writable: false });
with (nw) { ro = 2; print(ro); (function () { 'use strict'; try { ro = 3; } catch (e) { print(e.name); } })(); }
with ({}) { (function () { 'use strict'; try { undeclared_s = 1; } catch (e) { print(e.name); } })(); }
var fio = { k: 'none' };
with (fio) { for (k in { a: 1 }) {} }
print(fio.k, typeof k);
with ({ e: 'obj' }) { try { throw 'thrown'; } catch (e) { print(e); } print(e); }"#
        ),
        lines(&[
            "1",
            "2",
            "10",
            "11",
            "number 5 false",
            "undefined",
            "ReferenceError: nope is not defined",
            "2",
            "7",
            "number undefined",
            "3",
            "3",
            "TypeError",
            "set 9 true",
            "got",
            "1",
            "TypeError",
            "ReferenceError",
            "a undefined",
            "thrown",
            "obj",
        ])
    );
}

/// `@@unscopables` blocks a name (the walk goes on), and its getter runs before the value's
/// getter (HasBinding, then GetBindingValue).
#[test]
fn unscopables() {
    assert_eq!(
        run_both(
            r#"var u = { w: 'from-o' };
var w = 'outer';
u[Symbol.unscopables] = { w: true };
with (u) { print(w); w = 'set'; }
print(w, u.w);
var order = [];
var t = { get v() { order.push('get v'); return 1; } };
Object.defineProperty(t, Symbol.unscopables, { get: function () { order.push('get unscopables'); return {}; } });
with (t) { v; }
print(order.join(','));
var nb = { q: 1 };
nb[Symbol.unscopables] = { q: false };
with (nb) { print(q); }"#
        ),
        lines(&["outer", "set from-o", "get unscopables,get v", "1"])
    );
}

/// Sloppy `delete name`: a with object's property (true), a declarative binding (false), a
/// global `var` (false), a configurable global property (true), `NaN` (false), an
/// unresolvable name (true).
#[test]
fn delete_a_name() {
    assert_eq!(
        run_both(
            r#"var p = { d: 1 };
with (p) { print(delete d, 'd' in p, delete nonexist); }
function f() { var local = 1; with ({}) { return delete local; } }
print(f());
var vv = 1; print(delete vv);
globalThis.gz = 1;
print(delete gz, typeof gz);
print(delete NaN);
let gl = 1; with ({}) { print(delete gl); }"#
        ),
        lines(&[
            "true false true",
            "false",
            "false",
            "true undefined",
            "false",
            "false"
        ])
    );
}

/// Closures keep the with context; TDZ and `const` through a with context; a `let` per
/// loop iteration inside `with`.
#[test]
fn closures_tdz_and_const() {
    assert_eq!(
        run_both(
            r#"var m = { mm: 'm' };
var fn; with (m) { fn = function () { return mm; }; }
print(fn());
m.mm = 'm2'; print(fn());
function outer() { var ov = 'outer-v'; return function () { with ({}) { return ov; } }; }
print(outer()());
var fs = [];
with ({ q: 'Q' }) { for (let i = 0; i < 3; i++) { fs.push(function () { return q + i; }); } }
print(fs[0]() + ',' + fs[1]() + ',' + fs[2]());
function h() { with ({}) { try { c; } catch (e) { print(e.name); } } let c = 3; return c; }
print(h());
function k() { const q = 1; with ({}) { try { q = 2; } catch (e) { print(e.name + ': ' + e.message); } } return q; }
print(k());
with ({}) { try { typeof tdzv; } catch (e) { print(e.name); } }
let tdzv = 1;
let gl = 'gl'; with ({}) { print(gl); gl = 'gl2'; } print(gl);
const gc = 1; with ({}) { try { gc = 2; } catch (e) { print(e.name); } }
var ao = { t: 1, m: function () { with ({ t: 2 }) { return (() => this.t)(); } } };
print(ao.m());"#
        ),
        lines(&[
            "m",
            "m2",
            "outer-v",
            "Q0,Q1,Q2",
            "ReferenceError",
            "3",
            "TypeError: Assignment to constant variable.",
            "1",
            "ReferenceError",
            "gl",
            "gl2",
            "TypeError",
            "1",
        ])
    );
}

/// A generator suspended inside `with` resumes in the with context (genf::CONTEXT).
#[test]
fn generator_suspended_in_with() {
    assert_eq!(
        run_both(
            r#"function* g(o) { with (o) { yield x; yield x } }
var o = { x: 1 };
var it = g(o);
var r = it.next(); print(r.value, r.done);
o.x = 2;
r = it.next(); print(r.value, r.done);
r = it.next(); print(r.value, r.done);
function* g2(o) { with (o) { yield 0; yield y; x = 5; } }
var o2 = { x: 1 }; var y = 'global-y';
var it2 = g2(o2); it2.next(); print(it2.next().value); o2.y = 'own-y'; it2.next(); print(o2.x);
var it3 = g2({ y: 'own' }); it3.next(); print(it3.next().value);"#
        ),
        lines(&[
            "1 false",
            "2 false",
            "undefined true",
            "global-y",
            "5",
            "own"
        ])
    );
}

/// PutValue goes through the reference that was resolved before the right-hand side ran
/// (§13.15.2 steps 1.a and 1.e, §13.4.2.1, §14.3.2.1): `=`, `op=`, `&&=`, `++` and a `var`
/// initializer resolve the name once, read and store through that binding, and a strict store
/// to a binding that is gone throws. These are the spec answers (JSC agrees, test262
/// S11.13.1_A5_T1, S11.13.2_A5.*, S11.3.1_A5_*, unscopables-inc-dec.js, binding-resolution.js);
/// V8/node v24 resolves again at the store and prints `2 undefined`, `6 undefined`, ...
#[test]
fn the_reference_is_resolved_before_the_right_side() {
    assert_eq!(
        run_both(
            r#"var x = 10, o = { x: 1 };
with (o) { x = (delete o.x, 2); }
print(x, o.x);
var o2 = { y: 1 }, y = 10;
with (o2) { y += (delete o2.y, 5); }
print(y, o2.y);
var o3 = { get z() { delete this.z; return 2; } }, z = 0;
with (o3) { z *= 3; }
print(z, o3.z);
var o4 = { w: 4 }, w = 0;
with (o4) { w ||= (delete o4.w, 9); w &&= (delete o4.w, 7); }
print(w, o4.w);
var calls = 0, flag = true, a = { u: 7 };
var b = { u: 4, get [Symbol.unscopables]() { calls++; return { u: flag = !flag }; } };
with (a) { with (b) { u++; } }
print(calls, a.u, b.u);
var p = {}, q = 1;
with (p) { q = (p.q = 2, 3); }
print(q, p.q);
var r = { v1: 1 };
with (r) { var v1 = delete r.v1; }
print(r.v1, v1);
var keep = { k: 1 };
with (keep) { print(k = (delete keep.k, 5), k++, ++k, k += 1); }
print(keep.k);
function strictStore() {
  var s = { m: 1 };
  with (s) { (function () { 'use strict'; try { m = (delete s.m, 2); print('no throw', s.m); } catch (e) { print(e.name); } })(); }
}
strictStore();
with ({}) { (function () { 'use strict'; try { nf2 = (globalThis.nf2 = 1, 2); print('no throw'); } catch (e) { print(e.name, nf2); } })(); }
with ({}) { fresh1 = (globalThis.fresh1 = 1, 2); }
print(fresh1);
var go = { gx: 0 }, gx = 'outer';
function* gen() { with (go) { gx = yield 1; } }
var gi = gen(); gi.next(); delete go.gx; gi.next(5);
print(go.gx, gx);"#
        ),
        lines(&[
            "10 2",
            "10 6",
            "0 6",
            "0 7",
            "1 7 5",
            "3 2",
            "true undefined",
            "5 5 7 8",
            "8",
            "ReferenceError",
            "ReferenceError 1",
            "2",
            "5 outer",
        ])
    );
}

/// A named function expression's own name is an immutable binding that is not strict
/// (§15.2.5 CreateImmutableBinding(name, false)): a dynamic store to it (inside `with`) is
/// ignored in sloppy code and a TypeError in strict code, `delete` gives false, and a `with`
/// object that has the name still wins. Same answers as node v24.
#[test]
fn a_function_expression_name_is_immutable_through_with() {
    assert_eq!(
        run_both(
            r#"var f = function g() { with ({}) { g = 3; return typeof g; } }; print(f());
var f2 = function g() { with ({}) { return (function () { 'use strict'; try { g = 3; return 'no throw'; } catch (e) { return e instanceof TypeError; } })(); } }; print(f2());
var f3 = function g() { with ({}) { g += 1; g++; return typeof g; } }; print(f3());
var f4 = function g() { var r; with ({}) { r = delete g; } return r + ' ' + typeof g; }; print(f4());
var f5 = function g() { with ({ g: 1 }) { g = 2; return typeof g; } }; print(f5());
var f6 = function* g() { with ({}) { g = 3; yield typeof g; } }; print(f6().next().value);
var f7 = function g() { with ({}) { (function () { 'use strict'; try { g++; } catch (e) { print(e.name); } })(); } return typeof g; }; print(f7());"#
        ),
        lines(&[
            "function",
            "true",
            "function",
            "false function",
            "number",
            "function",
            "TypeError",
            "function",
        ])
    );
}

/// A class heritage and a computed key are strict code even in sloppy code (§11.2.2), so a
/// read there of a `with` binding that is gone by GetValue throws a ReferenceError
/// (§9.4.2 ResolveBinding step 3 takes the strictness of the use site; §6.2.5.5 GetValue
/// step 3.c; §9.1.1.2.6 GetBindingValue step 2.a), before the right-hand side of a compound
/// or logical assignment runs (§13.15.2). Here the `@@unscopables` getter deletes `x` while
/// the name resolves. Plain reads, member bases, calls, `new`, `typeof` (the reference is
/// resolvable, §13.5.3.1 step 3), compound, logical and update assignments, `extends`, and
/// static, accessor and field keys all throw. A sloppy read still gives undefined, and a
/// `with` call in a key still gets the object as `this`. V8/node v24 prints `no throw` for
/// every read here (and runs `rhs` first); these are the spec answers.
#[test]
fn class_keys_and_heritage_read_dynamic_names_in_strict_code() {
    assert_eq!(
        run_both(
            r#"var log = [];
function mk() { return { x: function () { return 1; }, get [Symbol.unscopables]() { delete this.x; return {}; } }; }
function rhs() { log.push('rhs'); return 1; }
function t(name, f) { var r; try { f(mk()); r = 'no throw'; } catch (e) { r = e.name; } print(name + ' ' + r + (log.length ? ' ' + log.join() : '')); log = []; }
t('[x]', function (o) { with (o) { class C { [x]() {} } } });
t('[(0,x)]', function (o) { with (o) { class C { [(0, x)]() {} } } });
t('[x.p]', function (o) { with (o) { class C { [x.p]() {} } } });
t('[x?.p]', function (o) { with (o) { class C { [x?.p]() {} } } });
t('[x()]', function (o) { with (o) { class C { [x()]() {} } } });
t('[x?.()]', function (o) { with (o) { class C { [x?.()]() {} } } });
t('[x``]', function (o) { with (o) { class C { [x``]() {} } } });
t('[new x]', function (o) { with (o) { class C { [new x]() {} } } });
t('[typeof x]', function (o) { with (o) { class C { [typeof x]() {} } } });
t('[x += rhs()]', function (o) { with (o) { class C { [x += rhs()]() {} } } });
t('[x ??= rhs()]', function (o) { with (o) { class C { [x ??= rhs()]() {} } } });
t('[x ||= rhs()]', function (o) { with (o) { class C { [x ||= rhs()]() {} } } });
t('[x &&= rhs()]', function (o) { with (o) { class C { [x &&= rhs()]() {} } } });
t('[x++]', function (o) { with (o) { class C { [x++]() {} } } });
t('[--x]', function (o) { with (o) { class C { [--x]() {} } } });
t('[x = rhs()]', function (o) { with (o) { class C { [x = rhs()]() {} } } });
t('extends', function (o) { with (o) { class C extends (x, Object) {} } });
t('static [x]', function (o) { with (o) { class C { static [x]() {} } } });
t('get [x]', function (o) { with (o) { class C { get [x]() {} } } });
t('field [x]', function (o) { with (o) { class C { [x] = 1; } } });
t('static field [x]', function (o) { with (o) { class C { static [x] = 1; } } });
t('class expr', function (o) { with (o) { (class { [x]() {} }); } });
t('sloppy read', function (o) { with (o) { x; } });
var so = mk(); try { with (so) { class C { [x]() {} } } print('script no throw'); } catch (e) { print('script ' + e.name); }
with ({ k: 'm', f: function () { return this.tag; }, tag: 'w' }) {
  class D { [k]() { return 1; } static [f()]() {} [typeof k]() {} static [typeof nope1]() {} }
  print(new D().m(), typeof D.w, typeof D.prototype.string, typeof D.undefined);
}"#
        ),
        lines(&[
            "[x] ReferenceError",
            "[(0,x)] ReferenceError",
            "[x.p] ReferenceError",
            "[x?.p] ReferenceError",
            "[x()] ReferenceError",
            "[x?.()] ReferenceError",
            "[x``] ReferenceError",
            "[new x] ReferenceError",
            "[typeof x] ReferenceError",
            "[x += rhs()] ReferenceError",
            "[x ??= rhs()] ReferenceError",
            "[x ||= rhs()] ReferenceError",
            "[x &&= rhs()] ReferenceError",
            "[x++] ReferenceError",
            "[--x] ReferenceError",
            "[x = rhs()] ReferenceError rhs",
            "extends ReferenceError",
            "static [x] ReferenceError",
            "get [x] ReferenceError",
            "field [x] ReferenceError",
            "static field [x] ReferenceError",
            "class expr ReferenceError",
            "sloppy read no throw",
            "script ReferenceError",
            "1 function function function",
        ])
    );
}

/// A dynamic identifier target of a destructuring assignment or of a `var` binding pattern
/// is resolved before its value is read (§13.15.5.3 AssignmentProperty: ResolveBinding before
/// GetV; §13.15.5.4-6 Rest, Iterator and Keyed DestructuringAssignmentEvaluation step 1;
/// §8.6.3 KeyedBindingInitialization and §8.6.4 IteratorBindingInitialization: ResolveBinding
/// first), so PutValue goes to the `with` object even when a getter, an iterator step or a
/// default deleted the property in between (§9.1.1.2.5 SetMutableBinding, S false: Set).
/// Covers shorthand, renamed, defaults, nested, array elements, elision, rests, `var`
/// patterns, for-in/of heads with patterns (for-in via a String.prototype getter), a
/// generator suspended inside a default, and the order of the key, the `@@unscopables`
/// lookup and the value. A plain `for (x of ...)` head and a function default name are
/// controls. V8/node v24 resolves at the store and prints `2 undefined`, `key,get,resolve`,
/// `step,resolve,close 3`; these are the spec answers.
#[test]
fn a_destructuring_target_is_resolved_before_its_value() {
    assert_eq!(
        run_both(
            r#"function it(o) { var n = 0; return { [Symbol.iterator]() { return { next() { if (n++ === 0) { delete o.x; return { value: 2, done: false }; } return { value: undefined, done: true }; } }; } }; }
function g2(o) { return { get a() { delete o.x; return 2; } }; }
function show(v) { if (Array.isArray(v)) return '[' + v + ']'; if (typeof v === 'object' && v !== null) { var s = []; for (var k in v) s.push(k + ':' + v[k]); return '{' + s.join() + '}'; } return String(v); }
var cur;
Object.defineProperty(String.prototype, 'x', { get: function () { delete cur.x; return 2; }, configurable: true });
function t(name, f) { var x = 10, o = { x: 1 }; cur = o; print(name + ' ' + f(o) ); }
t('{x}', function (o) { var x = 10; with (o) { ({ x } = { get x() { delete o.x; return 2; } }); } return x + ' ' + show(o.x); });
t('{x=d}', function (o) { var x = 10; with (o) { ({ x = (delete o.x, 2) } = {}); } return x + ' ' + show(o.x); });
t('{a:x}', function (o) { var x = 10; with (o) { ({ a: x } = g2(o)); } return x + ' ' + show(o.x); });
t('{a:x=d}', function (o) { var x = 10; with (o) { ({ a: x = (delete o.x, 2) } = {}); } return x + ' ' + show(o.x); });
t('{a:{b:x}}', function (o) { var x = 10; with (o) { ({ a: { b: x } } = { a: { get b() { delete o.x; return 2; } } }); } return x + ' ' + show(o.x); });
t('[[x]]', function (o) { var x = 10; with (o) { [[x]] = [it(o)]; } return x + ' ' + show(o.x); });
t('[x]', function (o) { var x = 10; with (o) { [x] = it(o); } return x + ' ' + show(o.x); });
t('[x=d]', function (o) { var x = 10; with (o) { [x = (delete o.x, 2)] = []; } return x + ' ' + show(o.x); });
t('[,x=d]', function (o) { var x = 10; with (o) { [, x = (delete o.x, 2)] = [1]; } return x + ' ' + show(o.x); });
t('[...x]', function (o) { var x = 10; with (o) { [...x] = it(o); } return x + ' ' + show(o.x); });
t('{...x}', function (o) { var x = 10; with (o) { ({ ...x } = g2(o)); } return x + ' ' + show(o.x); });
t('var {x}', function (o) { var x = 10; with (o) { var { x } = { get x() { delete o.x; return 2; } }; } return x + ' ' + show(o.x); });
t('var {a:x=d}', function (o) { var x = 10; with (o) { var { a: x = (delete o.x, 2) } = {}; } return x + ' ' + show(o.x); });
t('var [x=d]', function (o) { var x = 10; with (o) { var [x = (delete o.x, 2)] = []; } return x + ' ' + show(o.x); });
t('var [x]', function (o) { var x = 10; with (o) { var [x] = it(o); } return x + ' ' + show(o.x); });
t('var [...x]', function (o) { var x = 10; with (o) { var [...x] = it(o); } return x + ' ' + show(o.x); });
t('var {...x}', function (o) { var x = 10; with (o) { var { ...x } = g2(o); } return x + ' ' + show(o.x); });
t('var {a:[x]}', function (o) { var x = 10; with (o) { var { a: [x] } = { a: it(o) }; } return x + ' ' + show(o.x); });
t('for {x} of', function (o) { var x = 10; with (o) { for ({ x } of [{ get x() { delete o.x; return 2; } }]); } return x + ' ' + show(o.x); });
t('for var {x} of', function (o) { var x = 10; with (o) { for (var { x } of [{ get x() { delete o.x; return 2; } }]); } return x + ' ' + show(o.x); });
t('for [x=d] of', function (o) { var x = 10; with (o) { for ([x = (delete o.x, 2)] of [[]]); } return x + ' ' + show(o.x); });
t('for var [x=d] of', function (o) { var x = 10; with (o) { for (var [x = (delete o.x, 2)] of [[]]); } return x + ' ' + show(o.x); });
t('for {x} in', function (o) { var x = 10; with (o) { for ({ x } in { a: 1 }); } return x + ' ' + show(o.x); });
t('for var {x} in', function (o) { var x = 10; with (o) { for (var { x } in { a: 1 }); } return x + ' ' + show(o.x); });
t('for x of', function (o) { var x = 10; with (o) { for (x of [2]); } return x + ' ' + show(o.x); });
t('{x=fn}', function (o) { var x = 10; with (o) { ({ x = function () {} } = {}); } return x + ' ' + o.x.name; });
delete String.prototype.x;
var log = [];
var lo = { x: 1, get [Symbol.unscopables]() { log.push('resolve'); return {}; } };
with (lo) { ({ [(log.push('key'), 'a')]: x } = { get a() { log.push('get'); return 2; } }); }
print(log.join()); log = [];
with (lo) { var { [(log.push('key'), 'a')]: x } = { get a() { log.push('get'); return 2; } }; }
print(log.join()); log = [];
var stepper = { [Symbol.iterator]() { return { next() { log.push('step'); return { value: 3, done: false }; }, return() { log.push('close'); return {}; } }; } };
with (lo) { [x] = stepper; }
print(log.join(), lo.x); log = [];
with (lo) { var [x] = stepper; }
print(log.join(), lo.x); log = [];
function* gen() { var x = 10, o = { x: 1 }; with (o) { [x = yield o] = []; } return x + ' ' + o.x; }
var gi = gen(); var go = gi.next().value; delete go.x; print(gi.next(2).value);"#
        ),
        lines(&[
            "{x} 10 2",
            "{x=d} 10 2",
            "{a:x} 10 2",
            "{a:x=d} 10 2",
            "{a:{b:x}} 10 2",
            "[[x]] 10 2",
            "[x] 10 2",
            "[x=d] 10 2",
            "[,x=d] 10 2",
            "[...x] 10 [2]",
            "{...x} 10 {a:2}",
            "var {x} 10 2",
            "var {a:x=d} 10 2",
            "var [x=d] 10 2",
            "var [x] 10 2",
            "var [...x] 10 [2]",
            "var {...x} 10 {a:2}",
            "var {a:[x]} 10 2",
            "for {x} of 10 2",
            "for var {x} of 10 2",
            "for [x=d] of 10 2",
            "for var [x=d] of 10 2",
            "for {x} in 10 2",
            "for var {x} in 10 2",
            "for x of 10 2",
            "{x=fn} 10 x",
            "key,resolve,get",
            "key,resolve,get",
            "resolve,step,close 3",
            "resolve,step,close 3",
            "10 2",
        ])
    );
}
