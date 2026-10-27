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

// ---- M2-P4-RF1: termination and the job queue (Q1, Q2); handled rejections (Q3) ----------

thread_local! {
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static GCS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `log(...args)`: like `print`, into `LOG`.
fn log<'s>(
    cx: &mut ep9_vm::Cx<'s>,
    _this: ep9_vm::Local<'s>,
    args: ep9_vm::Args<'s>,
    _nt: ep9_vm::Local<'s>,
) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    LOG.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(ep9_vm::Local::undefined())
}

/// `gc()`: a full collection now (its marked bytes stay in the heap stats).
fn gc<'s>(
    cx: &mut ep9_vm::Cx<'s>,
    _this: ep9_vm::Local<'s>,
    _args: ep9_vm::Args<'s>,
    _nt: ep9_vm::Local<'s>,
) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
    cx.collect_garbage();
    GCS.with(|g| g.set(g.get() + 1));
    Ok(ep9_vm::Local::undefined())
}

fn compile(src: &str) -> ep9_bytecode::VerifiedUnit {
    ep9_frontend::compile(
        src,
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles")
}

fn take_log() -> Vec<String> {
    LOG.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

/// A fresh isolate with `log` and `gc`.
fn log_isolate(stress: bool) -> ep9_vm::Isolate {
    let mut i = common::isolate(stress);
    i.register_host_function("log", 1, log);
    i.register_host_function("gc", 0, gc);
    take_log();
    i
}

/// The next interrupt poll terminates the run.
fn terminate_at_next_poll(i: &ep9_vm::Isolate) {
    let t = i.terminate_handle();
    i.interrupt_handle()
        .request(Box::new(move || t.terminate()));
}

/// Q1: a script that queues a reaction and is then terminated drops the reaction: the next
/// script on the same isolate does not run it (it ran there before the fix).
#[test]
fn a_terminated_script_drops_its_reactions() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            terminate_at_next_poll(&i);
            let e = i
                .run_script(
                    compile(
                        "Promise.resolve().then(function () { log('abandoned'); }); for (;;) {}",
                    ),
                    "a.js",
                )
                .unwrap_err();
            assert!(e.terminated, "{e:?}");
            i.run_script(compile("log('second');"), "b.js").unwrap();
            assert_eq!(take_log(), ["second"]);
        });
    }
}

/// Q1 for `Isolate::call`: a terminated call drops the reactions it queued.
#[test]
fn a_terminated_call_drops_its_reactions() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            i.run_script(
                compile(
                    "function f() { Promise.resolve().then(function () { log('abandoned'); }); for (;;) {} }",
                ),
                "def.js",
            )
            .unwrap();
            let f = i.global("f").expect("f");
            terminate_at_next_poll(&i);
            // `this` = undefined (the builtin tests name no raw Value type).
            let undefined = i.global("undefined").expect("undefined");
            let e = i.call(f, undefined, &[]).unwrap_err();
            assert!(e.terminated, "{e:?}");
            i.run_script(compile("log('next');"), "b.js").unwrap();
            assert_eq!(take_log(), ["next"]);
        });
    }
}

/// Q2: a script throws, and a reaction that runs after it is terminated: the result is the
/// termination, not the script's TypeError. The isolate works on.
#[test]
fn a_termination_after_a_script_error_is_reported_as_one() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            terminate_at_next_poll(&i);
            let e = i
                .run_script(
                    compile(
                        "Promise.resolve().then(function () { log('job'); for (;;) {} }); null.x;",
                    ),
                    "a.js",
                )
                .unwrap_err();
            assert!(e.terminated, "{e:?}");
            assert_eq!(take_log(), ["job"]);
            // An ordinary job error after a script error: the script's error stays.
            let e = i
                .run_script(
                    compile("Promise.resolve().then(function () { log('job2'); }); null.y;"),
                    "b.js",
                )
                .unwrap_err();
            assert!(!e.terminated);
            assert_eq!(e.constructor.as_deref(), Some("TypeError"));
            assert_eq!(take_log(), ["job2"]);
        });
    }
}

/// `n` jobs in one drain, each `Promise.reject(new Error(..)).catch(next)`; the last job runs a
/// full collection. The marked bytes of that collection.
fn marked_at_the_last_job(n: u32) -> usize {
    let src = format!(
        "function loop(i) {{ if (i === 0) {{ gc(); return; }} \
         Promise.reject(new Error('x' + i)).catch(function () {{ loop(i - 1); }}); }}\nloop({n});"
    );
    let mut i = log_isolate(false);
    GCS.with(|g| g.set(0));
    i.run_script(compile(&src), "q3.js").unwrap();
    assert_eq!(GCS.with(std::cell::Cell::get), 1);
    assert!(i.take_unhandled_rejections().is_empty());
    i.heap().stats().last_marked_bytes
}

/// Q3: a long drain of handled rejections keeps a bounded heap: each handled rejection
/// leaves the rejected list when its handler is attached, so the live heap at the last job
/// does not grow with the number of jobs (it grew by about 320 bytes per rejection).
#[test]
fn a_long_reject_and_catch_drain_keeps_a_bounded_heap() {
    let (small, big) =
        common::on_big_stack(|| (marked_at_the_last_job(100), marked_at_the_last_job(3100)));
    assert!(
        big < small + 64 * 1024,
        "the live heap grew with the drain: {small} B at 100 jobs, {big} B at 3100 jobs"
    );
}
