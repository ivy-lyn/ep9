//! Async generators and `for await` (§27.6, §14.7.5; M3-P2 ASYNCGEN): the request queue, job
//! order, abrupt requests, `yield*`, async iterator close, CreateAsyncFromSyncIterator and
//! %AsyncGeneratorFunction%, checked against node v24.22.1 output (scratchpad
//! m0m3/p2/asyncgen/probe). Every script runs with and without GC stress (`common::run_both`:
//! `gc_stress: 1, verify_after_gc: true`); the isolate drains the job queue at the end of
//! `run_script`.

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

/// The printed lines, each split at its newlines (scripts print `log.join('\n')`).
fn split(r: Result<Vec<String>, String>) -> Result<Vec<String>, String> {
    r.map(|v| {
        v.iter()
            .flat_map(|s| s.split('\n'))
            .map(str::to_string)
            .collect()
    })
}

/// Requests queue while the generator runs; each `yield` awaits its operand, a completed
/// generator resolves its queued requests in order, and the job order of `next`, `await`,
/// `yield` and `return` equals node's (no extra job for `return;` and `return undefined;`).
#[test]
fn the_job_order_follows_node() {
    assert_eq!(
        split(run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
async function* g() { p('g start'); var a = yield 'y1'; p('g got ' + a); await null; p('g after await'); var b = yield Promise.resolve('y2'); p('g got ' + b); return 'R'; }
var it = g();
p('made');
it.next('n1').then(function (r) { p('r1 ' + r.value + ' ' + r.done); });
it.next('n2').then(function (r) { p('r2 ' + r.value + ' ' + r.done); });
it.next('n3').then(function (r) { p('r3 ' + r.value + ' ' + r.done); });
it.next('n4').then(function (r) { p('r4 ' + r.value + ' ' + r.done); });
Promise.resolve().then(function () { p('t1'); }).then(function () { p('t2'); }).then(function () { p('t3'); }).then(function () { p('t4'); }).then(function () { p('t5'); }).then(function () { p('t6'); });
async function* g1() {}
async function* g2() { return; }
async function* g3() { return undefined; }
g1().next().then(function () { p('g1 ret'); });
g2().next().then(function () { p('g2 ret'); });
g3().next().then(function () { p('g3 ret'); });
p('sync end');
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(log.join(',')); });
"#
        )),
        lines(&[
            "made,g start,sync end,g got n2,t1,g1 ret,g2 ret,r1 y1 false,g after await,t2,g3 ret,g got n3,t3,r2 y2 false,t4,r3 R true,r4 undefined true,t5,t6",
        ])
    );
}

/// `return` and `throw` requests on a suspended-start, a suspended-yield and a completed
/// generator (AsyncGeneratorAwaitReturn awaits the `return` value, a rejected one rejects),
/// `finally` blocks with `await`, a throw in the body, and a wrong receiver (a rejected promise,
/// no sync throw).
#[test]
fn return_and_throw_requests_and_wrong_receivers() {
    assert_eq!(
        split(run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
function on(tag, pr) { pr.then(function (r) { p(tag + ' ok ' + r.value + ' ' + r.done); }, function (e) { p(tag + ' err ' + (e && e.message || e)); }); }
async function* a() { try { p('a body'); yield 1; p('a never'); } finally { p('a finally'); } }
var ia = a();
on('a-ret', ia.return(Promise.resolve('early')));
on('a-next', ia.next());
var ib = a();
on('b-throw', ib.throw(new Error('boom')));
on('b-next', ib.next());
var ic = a();
on('c-next', ic.next());
on('c-ret', ic.return('cv'));
on('c-next2', ic.next());
async function* d() { try { yield 1; } catch (e) { p('d caught ' + e); yield 'recovered'; } return 'd end'; }
var id = d();
on('d1', id.next());
on('d2', id.throw('E1'));
on('d3', id.next());
on('d4', id.next());
async function* e() { yield 1; }
var ie = e();
on('e1', ie.next());
on('e2', ie.return(Promise.reject(new Error('rej'))));
on('e3', ie.next());
async function* f() { try { yield 1; } finally { await null; p('f finally awaited'); } }
var iff = f();
on('f1', iff.next());
on('f2', iff.return('fv'));
async function* t() { await null; throw new TypeError('in body'); }
var it = t();
on('t1', it.next());
on('t2', it.next());
var AGP = Object.getPrototypeOf(ia);
AGP.next.call({}).then(null, function (e) { p('recv ' + e.constructor.name + ' ' + e.message.slice(0, 50)); });
AGP.throw.call(1).then(null, function (e) { p('recv2 ' + e.constructor.name); });
print(typeof AGP.next, Object.getPrototypeOf(AGP) === Object.getPrototypeOf(Object.getPrototypeOf(AGP)) ? 'x' : 'ok');
var q = Promise.resolve(); for (var i = 0; i < 20; i++) q = q.then(function () {}); q.then(function () { print(log.join('\n')); });
"#
        )),
        lines(&[
            "function ok",
            "a body",
            "b-throw err boom",
            "b-next ok undefined true",
            "d caught E1",
            "recv TypeError Method [AsyncGenerator].prototype.next called on i",
            "recv2 TypeError",
            "a-ret ok early true",
            "a-next ok undefined true",
            "c-next ok 1 false",
            "a finally",
            "d1 ok 1 false",
            "e1 ok 1 false",
            "f1 ok 1 false",
            "t1 err in body",
            "t2 ok undefined true",
            "c-ret ok cv true",
            "c-next2 ok undefined true",
            "d2 ok recovered false",
            "e2 err rej",
            "e3 ok undefined true",
            "f finally awaited",
            "d3 ok d end true",
            "d4 ok undefined true",
            "f2 ok fv true",
        ])
    );
}

/// `yield*` over an async iterator (its `next` gets the sent values, the done value is the
/// result), over a sync iterable (CreateAsyncFromSyncIterator awaits each value), a `return`
/// request that closes the inner iterator through `finally`, an inner iterator with no
/// `throw` (closed, then a TypeError), a rejected inner `next`, and an inner async generator.
#[test]
fn yield_star_in_an_async_generator() {
    assert_eq!(
        split(run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
var asyncIt = {
  [Symbol.asyncIterator]() { p('get async iter'); return this; },
  i: 0,
  next(v) { p('inner next ' + v); return this.i < 2 ? { value: 'a' + this.i++, done: false } : { value: 'aret', done: true }; },
  return(v) { p('inner return ' + v); return { value: 'r:' + v, done: true }; },
};
async function* g1() { var r = yield* asyncIt; p('g1 r ' + r); return r; }
var i1 = g1();
i1.next('x0').then(function (r) { p('1a ' + r.value); return i1.next('x1'); })
  .then(function (r) { p('1b ' + r.value); return i1.next('x2'); })
  .then(function (r) { p('1c ' + r.value + ' ' + r.done); });
async function* g2() { try { yield* [1, Promise.resolve(2), 3]; } finally { p('g2 finally'); } }
var i2 = g2();
i2.next().then(function (r) { p('2a ' + r.value); return i2.next(); })
  .then(function (r) { p('2b ' + r.value); return i2.return('stop'); })
  .then(function (r) { p('2c ' + r.value + ' ' + r.done); });
var noThrow = { [Symbol.asyncIterator]() { return this; }, next() { return { value: 1, done: false }; }, return() { p('noThrow closed'); return {}; } };
async function* g3() { yield* noThrow; }
var i3 = g3();
i3.next().then(function () { return i3.throw('T'); }).then(null, function (e) { p('3 ' + e.constructor.name); });
async function* g4() { yield* { [Symbol.asyncIterator]() { return { next() { return Promise.reject('inner rej'); } }; } }; }
g4().next().then(null, function (e) { p('4 ' + e); });
async function* g5() { var x = yield* (async function* () { yield 'in'; return 'inner done'; })(); yield x; }
var i5 = g5();
i5.next().then(function (r) { p('5a ' + r.value); return i5.next(); }).then(function (r) { p('5b ' + r.value); });
var q = Promise.resolve(); for (var i = 0; i < 30; i++) q = q.then(function () {}); q.then(function () { print(log.join('\n')); });
"#
        )),
        lines(&[
            "get async iter",
            "inner next undefined",
            "1a a0",
            "inner next x1",
            "noThrow closed",
            "4 inner rej",
            "2a 1",
            "5a in",
            "1b a1",
            "inner next x2",
            "3 TypeError",
            "g1 r aret",
            "2b 2",
            "5b inner done",
            "1c aret true",
            "g2 finally",
            "2c stop true",
        ])
    );
}

/// `for await` over a sync iterator (values awaited, a rejected value closes nothing and
/// throws, `break` closes the sync iterator), over an async iterator (a throw in the body
/// closes it), labeled `continue` across two loops, a non-iterable, and a non-object result.
#[test]
fn for_await_with_async_and_sync_iterators() {
    assert_eq!(
        split(run_both(
            r#"
var log = [];
function p(x) { log.push(x); }
function syncIter(vals) { var i = 0; return { [Symbol.iterator]() { return { next() { return i < vals.length ? { value: vals[i++], done: false } : { done: true }; }, return() { p('sync return'); return {}; } }; } }; }
async function f1() { for await (var x of syncIter([1, Promise.resolve(2), 3])) { p('f1 ' + x); if (x === 2) break; } }
f1().then(function () { p('f1 done'); });
async function f2() { try { for await (var x of syncIter([1, Promise.reject('bad'), 3])) p('f2 ' + x); } catch (e) { p('f2 caught ' + e); } }
f2();
var asyncIt = { [Symbol.asyncIterator]() { var n = 0; return { next() { n++; return Promise.resolve({ value: n, done: n > 3 }); }, return() { p('async return'); return Promise.resolve({}); } }; } };
async function* g() { for await (const v of asyncIt) { if (v === 2) throw new Error('in body'); yield v; } }
var it = g();
it.next().then(function (r) { p('g1 ' + r.value); return it.next(); }).then(null, function (e) { p('g caught ' + e.message); });
async function f3() { label: for await (var x of asyncIt) { for await (var y of [10, 20]) { p('f3 ' + x + ' ' + y); continue label; } } return 'f3 end'; }
f3().then(function (v) { p(v); });
async function f4() { for await (var x of 5) ; }
f4().then(null, function (e) { p('f4 ' + e.constructor.name + ': ' + e.message); });
async function f5() { var o = { [Symbol.asyncIterator]() { return { next() { return 1; } }; } }; for await (var x of o) ; }
f5().then(null, function (e) { p('f5 ' + e.constructor.name); });
var q = Promise.resolve(); for (var i = 0; i < 40; i++) q = q.then(function () {}); q.then(function () { print(log.join('\n')); });
"#
        )),
        lines(&[
            "f4 TypeError: 5 is not async iterable",
            "f1 1",
            "f2 1",
            "f5 TypeError",
            "sync return",
            "g1 1",
            "f3 1 10",
            "f1 2",
            "sync return",
            "f2 caught bad",
            "async return",
            "f1 done",
            "g caught in body",
            "f3 2 10",
            "f3 3 10",
            "f3 end",
        ])
    );
}

/// M2-P4-JOBS-STOP (V8-C8): an await-resume job of an async generator whose body throws
/// rejects the request promise; the throw does not leave the job, so the checkpoint runs the
/// jobs behind it.
#[test]
fn an_await_resume_throw_rejects_and_the_checkpoint_goes_on() {
    assert_eq!(
        run_both(
            r#"
var order = [];
async function* boom() { await 0; order.push('boom resumes'); throw new Error('boom'); }
var pb = boom().next();
Promise.resolve().then(function () { order.push('j1'); }).then(function () { order.push('j2'); }).then(function () { order.push('j3'); });
pb.catch(function (e) { order.push('caught ' + e.message); });
async function* silent() { await 0; throw 'unhandled'; }
silent().next().then(null, function () {});
Promise.resolve().then(function(){}).then(function(){}).then(function(){}).then(function(){}).then(function () { print(order.join(',')); });
"#
        ),
        lines(&["boom resumes,j1,caught boom,j2,j3",])
    );
}

/// Many suspended async generators with queued requests and objects that only their frames
/// hold, under GC stress (`run_both`).
#[test]
fn suspended_async_generators_survive_gc_stress() {
    assert_eq!(
        run_both(
            r#"
var out = [];
async function* counter(n, tag) { var objs = []; for (var i = 0; i < n; i++) { objs.push({ i: i, s: tag + i }); var got = yield objs[i].s; if (got) objs.push({ got: got }); await { then(r) { r(i); } }; } return objs.length; }
var gens = [];
for (var k = 0; k < 4; k++) gens.push(counter(5, 'g' + k + ':'));
var pending = [];
for (var k = 0; k < 4; k++) for (var j = 0; j < 7; j++) pending.push(gens[k].next('v' + j));
Promise.all(pending).then(function (rs) { out.push(rs.map(function (r) { return r.value + (r.done ? '!' : ''); }).join(' ')); }).then(function () { print(out.join('\n')); });
"#
        ),
        lines(&[
            "g0:0 undefined g0:1 undefined g0:2 10! undefined! g1:0 undefined g1:1 undefined g1:2 10! undefined! g2:0 undefined g2:1 undefined g2:2 10! undefined! g3:0 undefined g3:1 undefined g3:2 10! undefined!",
        ])
    );
}

/// An async generator suspended at an `await`, with a queued request, that no script variable
/// holds (only the pending promise's reaction): the isolate collects and verifies the heap
/// between the two scripts (GC stress on, every dispatch), then the second script resumes it.
#[test]
fn a_suspended_async_generator_survives_a_collection() {
    for &d in Dispatch::ALL {
        let out = on_big_stack(move || {
            let mut iso = hooked_isolate(d, true);
            OUT.with(|o| o.borrow_mut().clear());
            iso.run_script(compile(FIRST).expect("compiles"), "first.js")
                .expect("first script");
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            iso.run_script(compile(SECOND).expect("compiles"), "second.js")
                .expect("second script");
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            OUT.with(|o| core::mem::take(&mut *o.borrow_mut()))
        });
        assert_eq!(
            split(Ok(out)),
            lines(&[
                "resumed go kept 1+2+3",
                "next got second",
                "p1 3 false",
                "p2 kept true",
            ]),
            "{}",
            d.name()
        );
    }
}

/// The first script of [`a_suspended_async_generator_survives_a_collection`].
const FIRST: &str = r#"
var log = [];
var resume;
async function* held(tag) {
  var keep = { tag: tag, list: [1, 2, 3] };
  var got = await new Promise(function (r) { resume = r; });
  log.push('resumed ' + got + ' ' + keep.tag + ' ' + keep.list.join('+'));
  var x = yield keep.list.length;
  log.push('next got ' + x.v);
  return keep.tag;
}
var it = held('kept');
var p1 = it.next();
var p2 = it.next({ v: 'second' });
p1.then(function (r) { log.push('p1 ' + r.value + ' ' + r.done); });
p2.then(function (r) { log.push('p2 ' + r.value + ' ' + r.done); });
it = null; p1 = null; p2 = null;
"#;

/// The second script: it resumes the generator.
const SECOND: &str = r#"
resume('go');
var q = Promise.resolve(); for (var i = 0; i < 10; i++) q = q.then(function () {});
q.then(function () { print(log.join('\n')); });
"#;

/// One Rust level per resume job, never one per `yield` or queued request (H14): a
/// `for await` over 100,000 yields and 50,001 requests queued on one generator finish. No GC
/// stress (too slow at this size; the tests above cover it).
#[test]
fn many_yields_and_queued_requests_take_no_native_recursion() {
    let r = on_big_stack(|| {
        run(
            r#"
async function* many(n) { for (var i = 0; i < n; i++) yield i; }
(async function () { var s = 0; for await (var v of many(100000)) s += v; print('sum', s); })();
var g = many(50000); var ps = []; for (var i = 0; i < 50001; i++) ps.push(g.next());
Promise.all(ps).then(function (rs) { print('queued', rs.length, rs[49999].value, rs[50000].done); });
"#,
            false,
        )
    });
    assert_eq!(r, lines(&["queued 50001 49999 true", "sum 4999950000",]));
}

// ---- %AsyncGeneratorFunction% (CreateDynamicFunction kind asyncGenerator) with compiler hooks ----
// The `common` isolates have no compiler hooks. `asyncGeneratorFunction()` (a host function)
// gives %AsyncGeneratorFunction% of its own realm (H20 R5), and `createRealm()` makes a
// second realm.

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

/// `asyncGeneratorFunction()`: %AsyncGeneratorFunction% of the current realm.
fn async_generator_function<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.intrinsic(IntrinsicId::AsyncGeneratorFunction).into())
}

/// The host functions of every realm.
const HOST: [(&str, u32, HostFn); 3] = [
    ("print", 1, print),
    ("createRealm", 0, create_realm),
    ("asyncGeneratorFunction", 0, async_generator_function),
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
    compile(source).map_err(failure)
}

static HOOKS: CompilerHooks = CompilerHooks {
    compile_eval: hook_eval,
    compile_function: hook_function,
    compile_script: hook_script,
};

/// The sloppy script `src`, compiled.
fn compile(src: &str) -> Result<ep9_bytecode::VerifiedUnit, CodegenError> {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
}

/// An isolate with the table, [`HOOKS`] and [`HOST`] under `dispatch`.
fn hooked_isolate(dispatch: Dispatch, stress: bool) -> Isolate {
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
    iso
}

/// Runs the sloppy script `src` on [`hooked_isolate`]: the printed lines (after the microtask
/// checkpoint), or the uncaught exception. Then collects and verifies the heap.
fn run_hooked(src: &str, dispatch: Dispatch, stress: bool) -> Result<Vec<String>, String> {
    let unit = compile(src).map_err(|e| format!("compile error: {e:?}"))?;
    let mut iso = hooked_isolate(dispatch, stress);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.run_script(unit, "test.js");
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

/// `%AsyncGeneratorFunction%` (§27.3.1.1) and its prototype objects (§27.3.3, §27.6.1,
/// §27.1.3), and CreateDynamicFunction kind asyncGenerator: called, constructed, through
/// `Reflect.construct` with a `newTarget` whose `prototype` is not an object (the fallback is
/// `%AsyncGeneratorFunction.prototype%` of GetFunctionRealm(newTarget), here of a second
/// realm), a subclass, and early errors in the parameters. Expected lines: node v24.22.1 with
/// `createRealm` = `vm.createContext` (scratchpad m0m3/p2/asyncgen/probe/hooked/run-node.js).
#[test]
fn the_async_generator_function_constructor_and_its_prototypes() {
    assert_eq!(
        run_hooked_every(
            r#"
var AGF = asyncGeneratorFunction();
var AGFP = AGF.prototype;
print(AGF.name, AGF.length, Object.getPrototypeOf(AGF) === Function, Object.getPrototypeOf(AGFP) === Function.prototype);
print(AGFP[Symbol.toStringTag], AGFP.prototype[Symbol.toStringTag], AGFP.constructor === AGF);
var AGP = AGFP.prototype;
var AIP = Object.getPrototypeOf(AGP);
print(typeof AIP[Symbol.asyncIterator], AIP[Symbol.asyncIterator].call(7) === 7, Object.getPrototypeOf(AIP) === Object.prototype);
var f = new AGF('a', 'var b = yield a; yield await b; return "end";');
print(Object.getPrototypeOf(f) === AGFP, f.name, f.length);
var it = f(1);
print(Object.getPrototypeOf(Object.getPrototypeOf(it)) === AGP || Object.getPrototypeOf(it) === AGP);
it.next().then(function (r) { print('n1', r.value, r.done); return it.next(Promise.resolve(2)); })
  .then(function (r) { print('n2', r.value, r.done); return it.next(); })
  .then(function (r) { print('n3', r.value, r.done); });
var g = AGF('yield 9');
print(Object.getPrototypeOf(g) === AGFP);
function nt() {}
nt.prototype = null;
var h = Reflect.construct(AGF, [], nt);
print(Object.getPrototypeOf(h) === AGFP);
var other = createRealm();
var OAGF = other.asyncGeneratorFunction();
var C = new other.Function();
C.prototype = 1;
var o = Reflect.construct(AGF, ['yield 2'], C);
print(OAGF !== AGF, Object.getPrototypeOf(o) === OAGF.prototype, Object.getPrototypeOf(o) === AGFP);
class Sub extends AGF {}
var s = new Sub('yield 3');
print(Object.getPrototypeOf(s) === Sub.prototype, s instanceof AGF);
s().next().then(function (r) { print('s', r.value); });
try { AGF('a = await 1', ''); } catch (e) { print('param await', e.constructor === SyntaxError); }
try { AGF('yield', ''); } catch (e) { print('param yield', e.constructor === SyntaxError); }
"#
        ),
        lines(&[
            "AsyncGeneratorFunction 1 true true",
            "AsyncGeneratorFunction AsyncGenerator true",
            "function true true",
            "true anonymous 1",
            "true",
            "true",
            "true",
            "true true false",
            "true true",
            "param await true",
            "param yield true",
            "n1 1 false",
            "s 3",
            "n2 2 false",
            "n3 end true",
        ])
    );
}
