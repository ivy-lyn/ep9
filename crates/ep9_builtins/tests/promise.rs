//! Promise and AggregateError (§27.2, §20.5.7.1): the job order of scripts, checked against
//! node v24.22.1 output. Every script runs with and without GC stress (`common::run_both`); the
//! isolate drains the job queue at the end of `run_script`.

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The order of reaction jobs, thenable jobs (one extra job) and a promise resolved with a promise (two extra jobs).
#[test]
fn job_order_follows_the_spec() {
    assert_eq!(
        run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
Promise.resolve(1).then(function (v) { p('a' + v); return 2; }).then(function (v) { p('b' + v); });
Promise.reject(3).catch(function (e) { p('c' + e); }).finally(function () { p('fin'); });
var t = { then: function (r) { p('then-called'); r(4); } };
Promise.resolve(t).then(function (v) { p('t' + v); });
new Promise(function (r) { p('exec'); r(Promise.resolve(5)); }).then(function (v) { p('nested' + v); });
p('sync');
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(log.join(',')); });
"#
        ),
        lines(&["exec,sync,a1,c3,then-called,b2,fin,t4,nested5",])
    );
}

/// Promise.all, race, any, allSettled, withResolvers and try, with their job order.
#[test]
fn the_combinators_withresolvers_and_try() {
    assert_eq!(
        run_both(
            r#"
var t = { then: function (r) { r(4); } };
Promise.all([1, Promise.resolve(2), t]).then(function (v) { print('all', v.join('|'), v.length); });
Promise.race([new Promise(function () {}), Promise.resolve('r')]).then(function (v) { print('race', v); });
Promise.any([Promise.reject(1), Promise.reject(2)]).catch(function (e) { print(e.name, e.errors.join('|'), e instanceof AggregateError, e instanceof Error, e.message); });
Promise.any([Promise.reject(1), Promise.resolve(2)]).then(function (v) { print('any', v); });
Promise.allSettled([Promise.resolve(1), Promise.reject(2)]).then(function (r) { print(r[0].status, r[0].value, r[1].status, r[1].reason); });
var w = Promise.withResolvers(); w.promise.then(function (v) { print('wr', v); }); w.resolve(9);
Promise.try(function (a, b) { return a + b; }, 3, 4).then(function (v) { print('try', v); });
Promise.try(function () { throw 'x'; }).catch(function (e) { print('tryerr', e); });
Promise.all([]).then(function (v) { print('empty', v.length); });
Promise.race([]).then(function () { print('never'); });
Promise.any([]).catch(function (e) { print('anyempty', e.errors.length); });
"#
        ),
        lines(&[
            "wr 9",
            "try 7",
            "tryerr x",
            "empty 0",
            "anyempty 0",
            "race r",
            "AggregateError 1|2 true true All promises were rejected",
            "any 2",
            "fulfilled 1 rejected 2",
            "all 1|2|4 3",
        ])
    );
}

/// Lengths, names, @@toStringTag, @@species, AggregateError, and the TypeErrors of the constructor and `then`.
#[test]
fn the_shapes_and_the_type_errors() {
    assert_eq!(
        run_both(
            r#"
print(typeof Promise, Promise.length, Promise.name, Promise.prototype.then.length, Promise.prototype.catch.length, Promise.prototype.finally.length);
print(Promise.all.length, Promise.race.length, Promise.resolve.length, Promise.reject.length, Promise.withResolvers.length, Promise.try.length);
print(Promise.prototype[Symbol.toStringTag], Promise[Symbol.species] === Promise, AggregateError.length, AggregateError.prototype.name, AggregateError.prototype.message === '');
try { Promise(function () {}); } catch (e) { print(e.name); }
try { new Promise(1); } catch (e) { print(e.name); }
try { Promise.resolve.call(1, 2); } catch (e) { print(e.name); }
try { Promise.prototype.then.call({}, function () {}); } catch (e) { print(e.name); }
var e = new AggregateError([1, 2], 'msg', { cause: 'c' });
print(e.message, e.errors.length, e.cause, e.name, String(e));
var p = Promise.resolve(1);
print(Promise.resolve(p) === p, p instanceof Promise, p.constructor === Promise);
"#
        ),
        lines(&[
            "function 1 Promise 2 1 1",
            "1 1 1 1 0 1",
            "Promise true 2 AggregateError true",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "msg 2 c AggregateError AggregateError: msg",
            "true true true",
        ])
    );
}

/// A subclass gets its own promises from `then`; a non-Promise constructor goes through NewPromiseCapability; `constructor` undefined falls back to %Promise%.
#[test]
fn subclasses_and_species() {
    assert_eq!(
        run_both(
            r#"
class MyP extends Promise {}
var m = MyP.resolve(1);
var m2 = m.then(function (v) { return v + 1; });
print(m instanceof MyP, m2 instanceof MyP, m2.constructor === MyP);
m2.then(function (v) { print('sub', v); });
var calls = 0;
function C(executor) { calls++; return new Promise(executor); }
var r = Promise.resolve.call(C, 5);
print(calls, r instanceof Promise);
var q = Promise.resolve(2);
q.constructor = undefined;
q.then(function (v) { print('ctor-undefined', v); });
"#
        ),
        lines(&["true true true", "1 true", "ctor-undefined 2", "sub 2",])
    );
}

/// A 300-long `then` chain and a 100-element Promise.all that allocate in every job: the reactions and jobs live only in heap records.
#[test]
fn long_chains_under_collections() {
    assert_eq!(
        run_both(
            r#"
var head = Promise.resolve(0);
for (var i = 0; i < 300; i++) {
  head = head.then(function (v) { return { n: v.n === undefined ? v + 1 : v.n + 1, pad: [v, 'x' + v] }; });
}
head.then(function (v) { print('chain', v.n); });
var all = [];
for (var j = 0; j < 100; j++) all.push(new Promise(function (r) { r('s' + j); }));
Promise.all(all).then(function (v) { print('all', v.length, v[0], v[99]); });
var seen = '';
for (var k = 0; k < 5; k++) (function (k) { Promise.resolve().then(function () { seen += k; }); })(k);
Promise.resolve().then(function () {}).then(function () { print('seen', seen); });
"#
        ),
        lines(&["all 100 s0 s99", "seen 01234", "chain 300",])
    );
}

/// `finally` keeps the value or overrides with its throw; the first resolve wins; an executor throw rejects; self-resolution is a TypeError.
#[test]
fn finally_executor_and_self_resolution() {
    assert_eq!(
        run_both(
            r#"
var log = '';
Promise.reject(new Error('boom')).finally(function () { log += 'f'; }).catch(function (e) { print('caught', e.message, log); });
Promise.resolve(1).finally(function () { return 99; }).then(function (v) { print('keeps', v); });
Promise.resolve(1).finally(function () { throw 'over'; }).catch(function (e) { print('overrides', e); });
var p = new Promise(function (res, rej) { res(1); rej(2); res(3); });
p.then(function (v) { print('first wins', v); });
new Promise(function () { throw 'exec-throw'; }).catch(function (e) { print('exec', e); });
new Promise(function (res) { res(); throw 'ignored'; }).then(function () { print('resolved before throw'); });
var self; self = new Promise(function (r) { Promise.resolve().then(function () { r(self); }); });
self.catch(function (e) { print(e.name); });
"#
        ),
        lines(&[
            "first wins 1",
            "exec exec-throw",
            "resolved before throw",
            "overrides over",
            "TypeError",
            "caught boom f",
            "keeps 1",
        ])
    );
}
