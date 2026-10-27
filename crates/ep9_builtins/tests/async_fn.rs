//! Async functions and `await` (§15.8, §27.7; M3-P1 ASYNC): job order, rejections, thenables,
//! methods and the constructor, checked against node v24.22.1 output. Every script runs with
//! and without GC stress (`common::run_both`: `gc_stress: 1, verify_after_gc: true`); the
//! isolate drains the job queue at the end of `run_script`.

mod common;

use std::cell::RefCell;

use common::{on_big_stack, run, run_both};
use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Dispatch, HostFn, IntrinsicId, Isolate,
    IsolateConfig, JsResult, Local, PropertyDescriptor,
};

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
    iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
        .expect("no uncaught error");
    let reasons = iso.take_unhandled_rejections();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].message, "lost");
}

// ---- %AsyncFunction% (CreateDynamicFunction kind async) on isolates with compiler hooks ----
// The `common` isolates have no compiler hooks. Async functions still inherit from
// %Function.prototype% (fixer item), so `asyncFunction()` (a host function) gives
// %AsyncFunction% of its own realm (H20 R5), and `createRealm()` makes a second realm.

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// `asyncFunction()`: %AsyncFunction% of the current realm.
fn async_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.intrinsic(IntrinsicId::AsyncFunction).into())
}

/// The host functions of every realm.
const HOST: [(&str, u32, HostFn); 3] = [
    ("print", 1, print),
    ("createRealm", 0, create_realm),
    ("asyncFunction", 0, async_function),
];

/// `createRealm()`: a new realm with the host functions, made in it; returns its global.
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let global = cx.create_realm()?;
    cx.with_realm(global, |cx| {
        for (name, length, f) in HOST {
            let func = cx.new_builtin_closure(f, name, length, Local::undefined());
            let desc = PropertyDescriptor::data(func.into(), true, false, true);
            cx.define_property_or_throw(global, name, desc)?;
        }
        Ok(())
    })?;
    Ok(global.into())
}

fn failure(e: CodegenError) -> CompileFailure {
    match e {
        CodegenError::Compile(CompileError::Syntax(errors)) => {
            CompileFailure::Syntax(errors.first().message.clone())
        }
        CodegenError::Compile(CompileError::Range(r)) => CompileFailure::Range(r.message),
        CodegenError::Unsupported(u) => CompileFailure::Unsupported(u.feature.to_string()),
        CodegenError::Limit(m) => CompileFailure::Range(m),
        CodegenError::Internal(m) => CompileFailure::Internal(m),
    }
}

fn hook_eval(source: &str, ctx: &EvalContext) -> CompileResult {
    ep9_frontend::compile_eval(source, ctx, CompileOptions::default()).map_err(failure)
}

fn hook_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    ep9_frontend::compile_function_ctor(kind, params, body, CompileOptions::default())
        .map_err(failure)
}

fn hook_script(source: &str) -> CompileResult {
    let opts = SourceOpts::script(Strictness::Sloppy);
    ep9_frontend::compile(source, opts, CompileOptions::default()).map_err(failure)
}

static HOOKS: CompilerHooks = CompilerHooks {
    compile_eval: hook_eval,
    compile_function: hook_function,
    compile_script: hook_script,
};

/// Runs the sloppy script `src` on an isolate with the table, [`HOOKS`] and [`HOST`] under
/// `dispatch`: the printed lines (after the microtask checkpoint), or the uncaught exception.
/// Then collects and verifies the heap.
fn run_hooked(src: &str, dispatch: Dispatch, stress: bool) -> Result<Vec<String>, String> {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(|e| format!("compile error: {e:?}"))?;
    let mut iso = Isolate::new(IsolateConfig {
        heap: HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..HeapConfig::default()
        },
        dispatch,
        builtins: Some(&ep9_builtins::TABLE),
        compiler: Some(&HOOKS),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    for (name, length, f) in HOST {
        iso.register_host_function(name, length, f);
    }
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// [`run_hooked`] under every dispatch expansion, with and without GC stress; all must agree.
fn run_hooked_every(src: &'static str) -> Result<Vec<String>, String> {
    let mut first: Option<(String, Result<Vec<String>, String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = on_big_stack(move || run_hooked(src, d, stress));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0} for {src:?}"),
            }
        }
    }
    first.expect("at least one dispatch").1
}

/// `%AsyncFunction%` (§27.7.1.1) makes async functions through CreateDynamicFunction kind
/// async. Called, constructed, or through `Reflect.construct` with a `newTarget` whose
/// `prototype` is not an object, the fallback is `%AsyncFunction.prototype%` of
/// GetFunctionRealm(newTarget) (§20.2.1.1.1 step 5, §10.1.14 step 4), here of a second
/// realm; a subclass gets its own prototype. Expected lines: node v24.22.1 with
/// `createRealm` = `vm.createContext` (scratchpad m0m3/p1/async/step2/run-node.js).
#[test]
fn the_async_function_constructor_takes_the_realm_fallback_prototype() {
    assert_eq!(
        run_hooked_every(
            r#"
var AF = asyncFunction();
var f = new AF('a', 'return await a;');
print(Object.getPrototypeOf(f) === AF.prototype, f.name, f.length, 'prototype' in f);
f(5).then(function (v) { print('f', v); });
var g = AF('await 0; return 1');
print(Object.getPrototypeOf(g) === AF.prototype, Object.getPrototypeOf(AF.prototype) === Function.prototype);
function nt() {}
nt.prototype = null;
var h = Reflect.construct(AF, [], nt);
print(Object.getPrototypeOf(h) === AF.prototype);
var other = createRealm();
var OAF = other.asyncFunction();
var C = new other.Function();
C.prototype = 1;
var o = Reflect.construct(AF, ['return 2'], C);
print(OAF !== AF, Object.getPrototypeOf(o) === OAF.prototype, Object.getPrototypeOf(o) === AF.prototype);
o().then(function (v) { print('o', v, v === 2); });
class Sub extends AF {}
var s = new Sub('return 3');
print(Object.getPrototypeOf(s) === Sub.prototype, s instanceof AF);
s().then(function (v) { print('s', v); });
"#
        ),
        lines(&[
            "true anonymous 1 false",
            "true true",
            "true",
            "true true false",
            "true true",
            "o 2 true",
            "s 3",
            "f 5",
        ])
    );
}

/// An async function of another realm (REALMS: `call::run_resumed` switches to the callee
/// realm): each resume job runs it in its own realm, so its objects, its errors from bytecode
/// and its promise are that realm's; the job order equals node's.
#[test]
fn an_async_function_resumes_in_its_own_realm() {
    assert_eq!(
        run_hooked_every(
            r#"
var other = createRealm();
var of = other.eval('(async function (p) { var a = await p; var e; try { await Promise.reject(1); } catch (x) { e = new TypeError("t"); } return [a, e]; })');
of(Promise.resolve('main')).then(function (v) {
  print(v[0], Array.isArray(v), v instanceof Array, v instanceof other.Array, v[1] instanceof other.TypeError);
});
var thrower = other.eval('(async function () { await 0; null.x; })');
thrower().catch(function (e) { print(e instanceof TypeError, e instanceof other.TypeError); });
async function mainAwaitsOther() { var r = await of(1); print('main', r[0], r instanceof other.Array, [] instanceof Array); }
mainAwaitsOther();
print('sync', Object.getPrototypeOf(of(0)) === other.Promise.prototype);
"#
        ),
        lines(&[
            "sync true",
            "false true",
            "main 1 true true",
            "main true false true true",
        ])
    );
}
