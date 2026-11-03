//! `Array.fromAsync` (ES2026 §23.1.2.2; M6-P1 SPEC-A): the job order, the observable reads
//! (`Promise.prototype.constructor` through PromiseResolve, `then` through the resolve
//! functions), iterator closes, custom constructors and rejections instead of throws. The
//! first two scripts print what node v24.22 and v26.13 print (checked when written). The
//! third checks the spec where V8 differs (it closes on a rejected next result and after a
//! failed final length Set, and its close loops or never settles when `return` throws or
//! rejects). Every script runs with and without GC stress (`common::run_both`), so the
//! fromAsync record, reachable only through pending promise reactions, survives a collection
//! at every allocation.

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Sync arrays, array-likes, async generators, mappers (sync and async), thisArg, rejections
/// for a bad mapper and bad items, closes on a mapper throw (sync and async iterators; the
/// async `return` result is awaited before the rejection), custom constructors: node's job order.
#[test]
fn the_job_order_and_closes_follow_node() {
    assert_eq!(
        run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
function ticks(n, tag) { var q = Promise.resolve(); for (let i = 1; i <= n; i++) { q = q.then(function () { p(tag + i); }); } return q; }
// 1. sync array, array-like, async iterable: job order against a tick counter.
Array.fromAsync([1, Promise.resolve(2), 3]).then(function (a) { p('arr:' + a.join('|')); });
Array.fromAsync({ length: 2, 0: 'x', 1: Promise.resolve('y') }).then(function (a) { p('like:' + a.join('|')); });
async function* gen() { yield 'g1'; yield 'g2'; }
Array.fromAsync(gen()).then(function (a) { p('gen:' + a.join('|')); });
Array.fromAsync([1, 2], function (v, k) { p('map' + v + '@' + k + ':' + (this && this.t)); return v * 10; }, { t: 'T' }).then(function (a) { p('mapped:' + a.join('|')); });
Array.fromAsync([1, 2], async function (v) { return v + 0.5; }).then(function (a) { p('amapped:' + a.join('|')); });
ticks(14, 't');
// 2. rejections, not throws.
try { Array.fromAsync([], 5).catch(function (e) { p('mapper:' + e.constructor.name); }); p('no sync throw 1'); } catch (e) { p('sync throw 1'); }
try { Array.fromAsync(undefined).catch(function (e) { p('undef:' + e.constructor.name); }); p('no sync throw 2'); } catch (e) { p('sync throw 2'); }
try { Array.fromAsync({ [Symbol.asyncIterator]: 1 }).catch(function (e) { p('notcallable:' + e.constructor.name); }); } catch (e) { p('sync throw 3'); }
// 3. mapper throw closes a sync iterator; an async iterator's return result is awaited first.
var it = { i: 0, next: function () { p('next' + this.i); return { value: this.i++, done: false }; }, return: function () { p('return sync'); return {}; }, [Symbol.iterator]: function () { return this; } };
Array.fromAsync(it, function (v) { if (v === 1) throw new RangeError('boom'); return v; }).catch(function (e) { p('closed sync:' + e.name); });
var ait = { i: 0, next: function () { p('anext' + this.i); return Promise.resolve({ value: this.i++, done: false }); }, return: function () { p('return async'); return new Promise(function (r) { setTimeoutish(function () { p('return settles'); r({}); }); }); }, [Symbol.asyncIterator]: function () { return this; } };
function setTimeoutish(f) { var q = Promise.resolve(); for (var i = 0; i < 3; i++) q = q.then(function () {}); q.then(f); }
Array.fromAsync(ait, async function (v) { if (v === 1) throw new SyntaxError('aboom'); return v; }).catch(function (e) { p('closed async:' + e.name); });
// 4. a rejecting thenable in a sync iterable closes the iterator.
// 5. a custom constructor.
function C() { p('C(' + arguments.length + (arguments.length ? ':' + arguments[0] : '') + ')'); }
Array.fromAsync.call(C, [7, 8]).then(function (a) { p('C iter:' + (a instanceof C) + ',' + a.length + ',' + a[0] + a[1]); });
Array.fromAsync.call(C, { length: 1, 0: 'z' }).then(function (a) { p('C like:' + (a instanceof C) + ',' + a.length + ',' + a[0]); });
Array.fromAsync.call({}, 'ab').then(function (a) { p('nonctor:' + Array.isArray(a) + ',' + a.join('')); });
p('sync end');
var q = Promise.resolve(); for (var i = 0; i < 40; i++) q = q.then(function () {});
q.then(function () { log.forEach(function (l) { print(l); }); });
"#
        ),
        lines(&[
            "no sync throw 1",
            "no sync throw 2",
            "next0",
            "anext0",
            "C(0)",
            "C(1:1)",
            "sync end",
            "t1",
            "mapper:TypeError",
            "undef:TypeError",
            "notcallable:TypeError",
            "map1@0:T",
            "t2",
            "anext1",
            "C like:true,1,z",
            "like:x|y",
            "t3",
            "next1",
            "t4",
            "return async",
            "map2@1:T",
            "t5",
            "return sync",
            "gen:g1|g2",
            "t6",
            "t7",
            "C iter:true,2,78",
            "nonctor:true,ab",
            "t8",
            "closed sync:RangeError",
            "return settles",
            "arr:1|2|3",
            "mapped:10|20",
            "amapped:1.5|2.5",
            "t9",
            "t10",
            "closed async:SyntaxError",
            "t11",
            "t12",
            "t13",
            "t14",
        ])
    );
}

/// The number of `constructor` reads (PromiseResolve in each Await) and `then` reads (the
/// resolve functions) per call, the `return` variants node handles, strings, numbers, holes and
/// the mapper's `this`: node's counts.
#[test]
fn observable_reads_match_node() {
    assert_eq!(
        run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
var ctorGets = 0, thenGets = 0;
var desc = Object.getOwnPropertyDescriptor(Promise.prototype, 'constructor');
Object.defineProperty(Promise.prototype, 'constructor', { get: function () { ctorGets++; return Promise; }, configurable: true });
Object.defineProperty(Object.prototype, 'then', { get: function () { thenGets++; return undefined; }, configurable: true });
async function run(name, f) {
  ctorGets = 0; thenGets = 0;
  try { var a = await f(); p(name + ' ok ' + JSON.stringify(a)); } catch (e) { p(name + ' err ' + (e && e.name || e)); }
  p(name + ' ctorGets=' + ctorGets + ' thenGets=' + thenGets);
}
(async function () {
  await run('array', function () { return Array.fromAsync([1, Promise.resolve(2)]); });
  await run('like', function () { return Array.fromAsync({ length: 2, 0: 1, 1: Promise.resolve(2) }); });
  await run('mapped', function () { return Array.fromAsync([1, 2], function (x) { return Promise.resolve(x * 2); }); });
  async function* g() { yield 1; yield Promise.resolve(2); }
  await run('gen', function () { return Array.fromAsync(g()); });
  // return method variants on async-iterator close (the throw completion wins).
  function mk(ret) { var o = { next: function () { return { value: 1, done: false }; }, [Symbol.asyncIterator]: function () { return this; } }; Object.defineProperty(o, 'return', ret); return o; }
  await run('ret-non-object', function () { return Array.fromAsync(mk({ value: function () { p('return called'); return 5; } }), function () { throw new RangeError('m'); }); });
  // sync iterator close through return variants.
  function mks(ret) { var o = { next: function () { return { value: 1, done: false }; }, [Symbol.iterator]: function () { return this; } }; if (ret) Object.defineProperty(o, 'return', ret); return o; }
  await run('s-ret-none', function () { return Array.fromAsync(mks(null), function () { throw new RangeError('m'); }); });
  await run('s-ret-value-thenable', function () { return Array.fromAsync(mks({ value: function () { p('s return called'); return { value: { then: function (r) { p('ret value then'); r(1); } }, done: true }; } }), function () { throw new RangeError('m'); }); });
  // define fails -> close
  function Frozen() { return Object.freeze([]); }
  await run('define-fails', function () { return Array.fromAsync.call(Frozen, mks({ value: function () { p('df return'); return {}; } })); });
  // length set fails
  await run('string', function () { return Array.fromAsync('a\u{1F600}b'); });
  await run('number', function () { return Array.fromAsync(5); });
  await run('holes', function () { return Array.fromAsync({ length: 3, 1: 'x' }); });
  await run('this-fn-undefined', function () { return Array.fromAsync([1], function () { 'use strict'; return typeof this; }); });
  await run('this-fn-sloppy', function () { return Array.fromAsync([1], function () { return typeof this; }); });
  Object.defineProperty(Promise.prototype, 'constructor', desc);
  delete Object.prototype.then;
  p('fromAsync.length=' + Array.fromAsync.length + ' name=' + Array.fromAsync.name + ' ' + JSON.stringify(Object.getOwnPropertyDescriptor(Array, 'fromAsync').writable));
  log.forEach(function (l) { print(l); });
})();
"#
        ),
        lines(&[
            "array ok [1,2]",
            "array ctorGets=6 thenGets=4",
            "like ok [1,2]",
            "like ctorGets=3 thenGets=1",
            "mapped ok [2,4]",
            "mapped ctorGets=7 thenGets=4",
            "gen ok [1,2]",
            "gen ctorGets=6 thenGets=4",
            "return called",
            "ret-non-object err RangeError",
            "ret-non-object ctorGets=2 thenGets=1",
            "s-ret-none err RangeError",
            "s-ret-none ctorGets=4 thenGets=2",
            "s return called",
            "ret value then",
            "s-ret-value-thenable err RangeError",
            "s-ret-value-thenable ctorGets=4 thenGets=2",
            "df return",
            "define-fails err TypeError",
            "define-fails ctorGets=4 thenGets=2",
            "string ok [\"a\",\"😀\",\"b\"]",
            "string ctorGets=6 thenGets=5",
            "number ok []",
            "number ctorGets=2 thenGets=1",
            "holes ok [null,\"x\",null]",
            "holes ctorGets=2 thenGets=1",
            "this-fn-undefined ok [\"undefined\"]",
            "this-fn-undefined ctorGets=4 thenGets=3",
            "this-fn-sloppy ok [\"object\"]",
            "this-fn-sloppy ctorGets=4 thenGets=3",
            "fromAsync.length=1 name=fromAsync true",
        ])
    );
}

/// The spec cases where V8 differs (see the module doc): one close per abrupt step, none for
/// a rejected next result or a failed final `length` Set, and the kept throw always wins.
#[test]
fn closes_follow_the_spec_where_v8_differs() {
    assert_eq!(
        run_both(
            r#"
// Spec cases where node v24-v26 (V8) differs: V8 also closes the iterator when the Await of a
// next result rejects or the final length Set throws, and its close loops or hangs when the
// `return` step throws or rejects. ES2026 §23.1.2.2: `? Await(nextResult)` and `? Set` do not
// close; AsyncIteratorClose with a throw completion always gives that throw back.
var log = [];
function p(x) { log.push(x); }
async function run(name, f) {
  try { var a = await f(); p(name + ' ok ' + JSON.stringify(a)); } catch (e) { p(name + ' err ' + (e && e.name || e)); }
}
function mk(ret) { var o = { next: function () { return { value: 1, done: false }; }, [Symbol.asyncIterator]: function () { return this; } }; Object.defineProperty(o, 'return', ret); return o; }
function mks(ret) { var o = { next: function () { return { value: 1, done: false }; }, [Symbol.iterator]: function () { return this; } }; Object.defineProperty(o, 'return', ret); return o; }
function boom() { throw new RangeError('m'); }
(async function () {
  await run('ret-getter-throws', function () { return Array.fromAsync(mk({ get: function () { p('get return'); throw new EvalError('g'); } }), boom); });
  await run('ret-not-callable', function () { return Array.fromAsync(mk({ value: 5 }), boom); });
  await run('ret-rejects', function () { return Array.fromAsync(mk({ value: function () { p('return called'); return Promise.reject(new URIError('r')); } }), boom); });
  await run('s-ret-throws', function () { return Array.fromAsync(mks({ value: function () { p('s return called'); throw new EvalError('x'); } }), boom); });
  var it = { i: 0, next: function () { this.i++; return this.i === 1 ? { value: Promise.reject('R'), done: false } : { done: true }; }, return: function () { p('rejecting value closes once'); return {}; }, [Symbol.iterator]: function () { return this; } };
  await run('rejecting-value', function () { return Array.fromAsync(it); });
  var bad = { next: function () { return 7; }, return: function () { p('not closed'); return {}; }, [Symbol.asyncIterator]: function () { return this; } };
  await run('non-object-result', function () { return Array.fromAsync(bad); });
  function NoLen() { var o = {}; Object.defineProperty(o, 'length', { set: function () { throw new EvalError('len'); } }); return o; }
  var ls = { i: 0, next: function () { return { value: 1, done: this.i++ > 0 }; }, return: function () { p('not closed'); return {}; }, [Symbol.iterator]: function () { return this; } };
  await run('length-fails', function () { return Array.fromAsync.call(NoLen, ls); });
  log.forEach(function (l) { print(l); });
})();
"#
        ),
        lines(&[
            "get return",
            "ret-getter-throws err RangeError",
            "ret-not-callable err RangeError",
            "return called",
            "ret-rejects err RangeError",
            "s return called",
            "s-ret-throws err RangeError",
            "rejecting value closes once",
            "rejecting-value err R",
            "non-object-result err TypeError",
            "length-fails err EvalError",
        ])
    );
}
