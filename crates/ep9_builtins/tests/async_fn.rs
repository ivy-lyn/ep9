//! Async functions and `await` (§15.8, §27.7; M3-P1 ASYNC): job order, rejections, thenables,
//! methods and the constructor, checked against node v24.22.1 output. Every script runs with
//! and without GC stress (`common::run_both`: `gc_stress: 1, verify_after_gc: true`); the
//! isolate drains the job queue at the end of `run_script`.

mod common;

use common::{on_big_stack, run, run_both};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Await chains interleave with promise chains as in node: no extra job for a native
/// promise (the ES2019 await rule), and a returned promise takes the thenable job.
#[test]
fn await_chains_follow_the_node_job_order() {
    assert_eq!(
        run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
async function a() { p('a1'); await null; p('a2'); await Promise.resolve(); p('a3'); return 'A'; }
async function b() { p('b1'); var v = await a(); p('b2:' + v); return v + 'B'; }
b().then(function (v) { p('done:' + v); });
Promise.resolve().then(function () { p('t1'); }).then(function () { p('t2'); }).then(function () { p('t3'); }).then(function () { p('t4'); }).then(function () { p('t5'); });
async function r() { return Promise.resolve('R'); }
r().then(function (v) { p('r:' + v); });
p('sync');
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(log.join(',')); });
"#
        ),
        lines(&["b1,a1,sync,a2,t1,a3,t2,b2:A,t3,r:R,done:AB,t4,t5",])
    );
}

/// A throw after `await` rejects; `catch` and `finally` with `await` inside; a `finally`
/// that throws overrides a `return`; an abrupt parameter default rejects (no sync throw).
#[test]
fn a_throw_after_await_rejects_and_finally_runs() {
    assert_eq!(
        run_both(
            r#"
async function t1() { await 1; throw new TypeError('after await'); }
t1().catch(function (e) { print('t1', e.name, e.message); });
async function t2() { try { await Promise.reject(new Error('rej')); } catch (e) { return 'caught ' + e.message; } }
t2().then(function (v) { print('t2', v); });
async function t3() { try { return await 5; } finally { print('t3 finally'); await 0; print('t3 after finally await'); } }
t3().then(function (v) { print('t3', v); });
async function t4() { try { return 1; } finally { throw 'override'; } }
t4().then(function (v) { print('t4 ok', v); }, function (e) { print('t4 rejected', e); });
async function t5(a = (function () { throw new RangeError('param'); })()) { print('never'); }
var p5 = t5(); print('t5 sync', p5 instanceof Promise);
p5.catch(function (e) { print('t5', e.name, e.message); });
async function t6() { for (var i = 0; i < 3; i++) { try { await (i % 2 ? Promise.reject(i) : i); print('t6 ok', i); } catch (e) { print('t6 caught', e); } } return 'end'; }
t6().then(function (v) { print('t6', v); });
"#
        ),
        lines(&[
            "t5 sync true",
            "t3 finally",
            "t4 rejected override",
            "t5 RangeError param",
            "t6 ok 0",
            "t1 TypeError after await",
            "t2 caught rej",
            "t3 after finally await",
            "t6 caught 1",
            "t3 5",
            "t6 ok 2",
            "t6 end",
        ])
    );
}

/// `await` of a non-promise, of a thenable (its `then` runs in a job), of a non-callable
/// `then`; PromiseResolve reads `constructor` once and its throw is a throw at the `await`;
/// a throwing `then` getter rejects.
#[test]
fn await_of_a_thenable_and_of_non_promises() {
    assert_eq!(
        run_both(
            r#"
var log = [];
async function f() {
  var a = await 1;
  var b = await { then: function (r) { log.push('then called'); r(2); } };
  var c = await { then: 5 };
  var d = await undefined;
  var e = await Promise.resolve(4);
  log.push([a, b, typeof c, d, e].join('|'));
}
f().then(function () { print(log.join(',')); });
var count = 0;
var p = Promise.resolve(7);
Object.defineProperty(p, 'constructor', { get: function () { count++; return Promise; } });
async function g() { var v = await p; print('g', v, count); }
g();
var bad = Promise.resolve(8);
Object.defineProperty(bad, 'constructor', { get: function () { throw new Error('ctor getter'); } });
async function h() { try { await bad; } catch (e) { print('h', e.message); } }
h();
async function k() { await { get then() { throw 'then getter'; } }; }
k().catch(function (e) { print('k', e); });
"#
        ),
        lines(&[
            "h ctor getter",
            "g 7 1",
            "k then getter",
            "then called,1|2|object||4",
        ])
    );
}

/// Async methods (object, class, static, private, `super`), an async arrow's lexical
/// `this` and `arguments` after `await`, `arguments` and `new.target` of an async function,
/// and the function object: no `prototype`, not a constructor.
#[test]
fn async_methods_arrows_this_and_arguments() {
    assert_eq!(
        run_both(
            r#"
var o = { x: 1, async m(a) { var v = await a; return this.x + v; } };
o.m(2).then(function (v) { print('obj', v); });
class B { base() { return 'base'; } }
class C extends B {
  #p = 10;
  constructor() { super(); this.y = 3; }
  async m() { await 0; return super.base() + this.y; }
  static async s(v) { return await v * 2; }
  async #priv() { await null; return this.#p; }
  callPriv() { return this.#priv(); }
}
var c = new C();
c.m().then(function (v) { print('method', v); });
C.s(21).then(function (v) { print('static', v); });
c.callPriv().then(function (v) { print('private', v); });
function outer() {
  var arrow = async () => { await 0; return [this.tag, arguments.length, arguments[0]].join(':'); };
  return arrow();
}
outer.call({ tag: 'T' }, 'first', 'second').then(function (v) { print('arrow', v); });
async function args(a, b) { await 0; return arguments.length + ':' + a + b + ':' + typeof new.target; }
args(1, 2, 3).then(function (v) { print('args', v); });
async function named() {}
print(named.length, named.name, 'prototype' in named, typeof named());
try { new named(); } catch (e) { print(e.name); }
var AsyncFunction = (async function () {}).constructor;
print(typeof AsyncFunction);
"#
        ),
        lines(&[
            "0 named false object",
            "TypeError",
            "function",
            "obj 3",
            "method base3",
            "static 42",
            "private 10",
            "arrow T:2:first",
            "args 3:12:undefined",
        ])
    );
}

/// M2-P4-JOBS-STOP (V8-C8): an await-resume job whose body throws rejects its promise; the
/// throw does not leave the job, so the checkpoint runs the jobs behind it.
#[test]
fn an_await_resume_throw_rejects_and_the_checkpoint_goes_on() {
    assert_eq!(
        run_both(
            r#"
var order = [];
async function boom() { await 0; order.push('boom resumes'); throw new Error('boom'); }
var pb = boom();
Promise.resolve().then(function () { order.push('j1'); }).then(function () { order.push('j2'); }).then(function () { order.push('j3'); });
pb.catch(function (e) { order.push('caught ' + e.message); });
async function silent() { await 0; throw 'unhandled'; }
silent().then(null, function () {});
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(order.join(',')); });
"#
        ),
        lines(&["boom resumes,j1,caught boom,j2,j3",])
    );
}

/// `await` in a loop and in a recursion (GC stress: the suspended frames and their
/// promises are heap cells).
#[test]
fn await_in_a_loop_and_in_recursion() {
    assert_eq!(
        run_both(
            r#"
async function many(n) { var s = 0; for (var i = 0; i < n; i++) { s += await i; } return s; }
many(300).then(function (v) { print('sum', v); });
async function rec(n) { if (n === 0) return 0; return 1 + await rec(n - 1); }
rec(100).then(function (v) { print('rec', v); });
"#
        ),
        lines(&["rec 100", "sum 44850",])
    );
}

/// One Rust level per resume job, never one per `await` (H14): 100,000 awaits in one frame
/// and a deep chain of pending async calls finish on the default stack. No GC stress (too
/// slow at this size; `await_in_a_loop_and_in_recursion` covers it).
#[test]
fn many_awaits_take_no_native_recursion() {
    let r = on_big_stack(|| {
        run(
            r#"
async function many(n) { var s = 0; for (var i = 0; i < n; i++) { s += await i; } return s; }
many(100000).then(function (v) { print('sum', v); });
async function rec(n) { if (n === 0) return 0; return 1 + await rec(n - 1); }
rec(2000).then(function (v) { print('rec', v); });
"#,
            false,
        )
    });
    assert_eq!(r, lines(&["rec 2000", "sum 4999950000"]));
}

/// An async function that throws with no handler leaves an unhandled rejection (the CLI
/// reports it; P4), and the script itself completes.
#[test]
fn an_unhandled_async_rejection_is_reported() {
    let unit = ep9_frontend::compile(
        "async function f() { await 0; throw 'lost'; } f(); print('end');",
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles");
    let mut iso = common::isolate(true);
    iso.run_script(unit, "test.js").expect("no uncaught error");
    let reasons = iso.take_unhandled_rejections();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].message, "lost");
}
