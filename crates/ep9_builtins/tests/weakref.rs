//! WeakRef and FinalizationRegistry (§26.1, §26.2) at the JS level (owner M4-P4 WEAKREF).
//!
//! The synchronous API scripts print node v24.22.1's lines (`print` = the arguments' ToString
//! joined by spaces) and run with and without GC stress (`common::run_both`). The GC part is
//! driven from Rust between scripts, as a host's job loop would: a collection after a stack
//! scrub, then `Cx::clear_kept_objects` and `Cx::cleanup_finalization_registries`. node has
//! no such hook (it runs the cleanup jobs in a later task), so those lines are the spec's
//! (§9.10, §9.13): a WeakRef keeps its target to the end of the job; a dead target's held
//! value goes to the callback once, after the GC, never during it.

mod common;

use std::cell::RefCell;
use std::hint::black_box;

use ep9_vm::{Args, Cx, Isolate, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The own properties of the constructors and prototypes, the global bindings, the tags.
#[test]
fn own_properties_match_node() {
    assert_eq!(
        common::run_both(
            r#"
function show(o) {
  return Reflect.ownKeys(o).map(function (k) {
    var d = Object.getOwnPropertyDescriptor(o, k), f = d.value !== undefined ? d.value : d.get;
    var s = String(k);
    if (typeof f === 'function') s += '(' + f.length + ',' + f.name + ')';
    else if (typeof f === 'string') s += '=' + f;
    return s + (d.get ? '[get]' : '') + (d.enumerable ? 'e' : '') + (d.writable ? 'w' : '') + (d.configurable ? 'c' : '');
  }).join(' ');
}
var names = ['WeakRef', 'FinalizationRegistry'];
for (var i = 0; i < names.length; i++) {
  var C = globalThis[names[i]];
  var g = Object.getOwnPropertyDescriptor(globalThis, names[i]);
  print(names[i], C.length, C.name, show(C), g.writable, g.enumerable, g.configurable);
  print(show(C.prototype));
  print(Object.getPrototypeOf(C.prototype) === Object.prototype, Object.getPrototypeOf(C) === Function.prototype);
}
var o = {};
var w = new WeakRef(o);
print(Object.prototype.toString.call(w), Object.prototype.toString.call(new FinalizationRegistry(function () {})));
print(w.deref() === o, Object.getOwnPropertyNames(w).length, Object.isExtensible(w));
"#
        ),
        lines(&[
            "WeakRef 1 WeakRef lengthc name=WeakRefc prototype true false true",
            "constructor(1,WeakRef)wc deref(0,deref)wc Symbol(Symbol.toStringTag)=WeakRefc",
            "true true",
            "FinalizationRegistry 1 FinalizationRegistry lengthc name=FinalizationRegistryc prototype true false true",
            "constructor(1,FinalizationRegistry)wc register(2,register)wc unregister(1,unregister)wc Symbol(Symbol.toStringTag)=FinalizationRegistryc",
            "true true",
            "[object WeakRef] [object FinalizationRegistry]",
            "true 0 true",
        ])
    );
}

/// The TypeErrors of each step, symbol targets and tokens, unregister results, subclasses
/// and many short-lived objects.
#[test]
fn api_steps_match_node() {
    assert_eq!(
        common::run_both(
            r#"
function t(f) { try { return String(f()); } catch (e) { return e.constructor.name; } }
var o = {}, s = Symbol('s'), rs = Symbol.for('r');
print(t(function () { return WeakRef(o); }), t(function () { return new WeakRef(1); }), t(function () { return new WeakRef(rs); }), t(function () { return new WeakRef(s).deref() === s; }));
print(t(function () { return new WeakRef(Symbol.iterator).deref() === Symbol.iterator; }));
print(t(function () { return WeakRef.prototype.deref.call({}); }), t(function () { return WeakRef.prototype.deref.call(new FinalizationRegistry(function () {})); }));
var fr = new FinalizationRegistry(function () {});
print(t(function () { return FinalizationRegistry(function () {}); }), t(function () { return new FinalizationRegistry({}); }));
print(t(function () { return fr.register(o, o); }), t(function () { return fr.register(1, 2); }), t(function () { return fr.register(o, 1, 2); }), t(function () { return fr.register(o, 1, rs); }));
print(t(function () { return fr.register(o, 1); }), t(function () { return fr.register(o, 1, o); }), t(function () { return fr.register(s, 1, s); }));
print(fr.unregister(o), fr.unregister(o), fr.unregister(s), fr.unregister(s), t(function () { return fr.unregister(1); }), t(function () { return fr.unregister(rs); }));
print(t(function () { return FinalizationRegistry.prototype.register.call(new WeakRef(o), o); }), t(function () { return FinalizationRegistry.prototype.unregister.call({}, o); }));
class W extends WeakRef { constructor(x) { super(x); this.k = 1; } }
var sub = new W(o);
print(sub instanceof W, sub instanceof WeakRef, sub.deref() === o, sub.k);
class F extends FinalizationRegistry {}
var sf = new F(function () {});
print(sf instanceof F, String(sf.register(o, 'h', o)), sf.unregister(o));
var P = function () {}; P.prototype = null;
var ref = Reflect.construct(WeakRef, [o], P);
print(Object.getPrototypeOf(ref) === WeakRef.prototype);
for (var i = 0; i < 200; i++) { new WeakRef({ i: i }).deref(); fr.register({}, i, {}); }
print('done');
"#
        ),
        lines(&[
            "TypeError TypeError TypeError true",
            "true",
            "TypeError TypeError",
            "TypeError TypeError",
            "TypeError TypeError TypeError TypeError",
            "undefined undefined undefined",
            "true false true false TypeError TypeError",
            "TypeError TypeError",
            "true true true 1",
            "true undefined true",
            "true",
            "done",
        ])
    );
}

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `log(...args)`: ToString of each argument, joined by a space.
fn log<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    LOG.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// `cleanupNow()`: the cleanup hook called with JS on the stack (it must throw).
fn cleanup_now<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.cleanup_finalization_registries()?;
    Ok(Local::undefined())
}

fn take_log() -> Vec<String> {
    LOG.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

/// An isolate with `log` and `cleanupNow`.
fn log_isolate(stress: bool) -> Isolate {
    let mut i = common::isolate(stress);
    i.register_host_function("log", 1, log);
    i.register_host_function("cleanupNow", 0, cleanup_now);
    take_log();
    i
}

/// Runs the sloppy script `src` below a pad of zeros, so its native frames leave no stale
/// words above the next collection's stack scan. No microtask checkpoint: [`end_of_job`]
/// runs the two job-end steps, so the test does not depend on when the job loop runs them.
#[inline(never)]
fn run(i: &mut Isolate, src: &str) -> Result<(), String> {
    let mut pad = [0u8; 64 << 10];
    black_box(&mut pad);
    let unit = ep9_frontend::compile(
        src,
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles");
    let r = i
        .with_cx(|cx| cx.run_script(unit, "weakref.js").map(|_| ()))
        .map_err(|e| e.message);
    black_box(&mut pad);
    r
}

/// The end of a job, as a host's job loop runs it: a collection after a stack scrub, then
/// ClearKeptObjects; then another scrubbed collection; then the cleanup jobs.
#[inline(never)]
fn end_of_job(i: &mut Isolate) -> Result<(), String> {
    scrub_and_collect(i);
    i.with_cx(|cx| {
        cx.clear_kept_objects();
        Ok(())
    })
    .unwrap();
    scrub_and_collect(i);
    i.with_cx(|cx| cx.cleanup_finalization_registries())
        .map_err(|e| e.message)
}

#[inline(never)]
fn scrub_and_collect(i: &mut Isolate) {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
    i.collect_garbage();
}

/// A WeakRef target lives to the end of the job, then dies; the registry's callback gets
/// each dead target's held value once, after the GC; unregister by token stops one; a live
/// target and a target of a dead registry get none; a callback can register again.
#[test]
fn targets_die_after_the_job_and_callbacks_run_after_the_gc() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            run(
                &mut i,
                r#"
var keep = { name: 'keep' };
var tok = {};
var fr = new FinalizationRegistry(function (held) {
  log('cleanup', held);
  if (held === 'again') return;
  if (held === 'first') fr.register({}, 'again');
});
(function () {
  for (var n = 0; n < 3; n++) fr.register({ n: n }, 'held' + n);
  fr.register({}, 'unregistered', tok);
  fr.register({}, 'first');
  fr.register(keep, 'kept');
  new FinalizationRegistry(function () { log('dead registry'); }).register({}, 'x');
})();
var w = new WeakRef({ name: 'temp' });
var wk = new WeakRef(keep);
log('same job', w.deref() !== undefined, w.deref().name);
log('unregister', fr.unregister(tok), fr.unregister(tok));
var msg;
try { cleanupNow(); } catch (e) { msg = e.message; }
log('from JS', msg);
"#,
            )
            .unwrap();
            assert_eq!(
                take_log(),
                [
                    "same job true temp",
                    "unregister true false",
                    "from JS FinalizationRegistry cleanup needs an empty JS stack",
                ],
                "stress={stress}"
            );
            end_of_job(&mut i).unwrap();
            let mut got = take_log();
            got.sort();
            assert_eq!(
                got,
                [
                    "cleanup first",
                    "cleanup held0",
                    "cleanup held1",
                    "cleanup held2",
                ],
                "stress={stress}"
            );
            run(
                &mut i,
                "log('next job', w.deref() === undefined, wk.deref() === keep);",
            )
            .unwrap();
            assert_eq!(take_log(), ["next job true true"], "stress={stress}");
            // The cell that the callback registered dies at the next GC.
            end_of_job(&mut i).unwrap();
            assert_eq!(take_log(), ["cleanup again"], "stress={stress}");
            end_of_job(&mut i).unwrap();
            assert_eq!(take_log(), Vec::<String>::new(), "stress={stress}");
            i.verify_heap().expect("a valid heap");
        });
    }
}

/// A callback that throws: the hook gives the error, and the next call runs the rest.
#[test]
fn a_throwing_callback_reports_and_the_rest_runs_later() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            run(
                &mut i,
                r#"
var fr = new FinalizationRegistry(function (held) { log('cleanup', held); throw new RangeError('boom ' + held); });
(function () { fr.register({}, 'a'); fr.register({}, 'b'); })();
"#,
            )
            .unwrap();
            let e1 = end_of_job(&mut i).unwrap_err();
            let e2 = i
                .with_cx(|cx| cx.cleanup_finalization_registries())
                .map_err(|e| e.message)
                .unwrap_err();
            assert!(e1.contains("boom "), "{e1} (stress={stress})");
            assert!(e2.contains("boom ") && e1 != e2, "{e2} (stress={stress})");
            assert_eq!(take_log().len(), 2, "stress={stress}");
            assert_eq!(end_of_job(&mut i), Ok(()), "stress={stress}");
            assert_eq!(take_log(), Vec::<String>::new());
        });
    }
}

/// Runs the sloppy script `src` through the normal embedder path,
/// `Isolate::with_cx_checkpoint` (the CLI's path): the script, then a microtask checkpoint.
/// No job-end hook is called by hand (M4-P4-CX-R1 W-1). Below a pad of zeros, as [`run`].
#[inline(never)]
fn run_checkpoint(i: &mut Isolate, src: &str) -> Result<(), String> {
    let mut pad = [0u8; 64 << 10];
    black_box(&mut pad);
    let unit = ep9_frontend::compile(
        src,
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles");
    let r = i
        .with_cx_checkpoint(|cx| cx.run_script(unit, "weakref.js").map(|_| ()))
        .map_err(|e| e.message);
    black_box(&mut pad);
    r
}

/// The checkpoint clears the kept list (M4-P4-CX-R1 W-1): a WeakRef target with no other
/// reference lives through its script's jobs, and after the checkpoint and a GC it is gone.
/// The control runs the same scripts with `with_cx` (no checkpoint): the kept list still
/// holds the target, so it survives the GC. Only the embedder API is used.
#[test]
fn a_checkpoint_lets_a_weakref_target_die_at_the_next_gc() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let src = r#"
var w = new WeakRef((function () { return { name: 'temp' }; })());
Promise.resolve().then(function () { log('job', w.deref() !== undefined); });
log('script', w.deref().name);
"#;
            let mut i = log_isolate(stress);
            run_checkpoint(&mut i, src).unwrap();
            scrub_and_collect(&mut i);
            run_checkpoint(&mut i, "log('later', w.deref() === undefined);").unwrap();
            assert_eq!(
                take_log(),
                ["script temp", "job true", "later true"],
                "stress={stress}"
            );
            i.verify_heap().expect("a valid heap");

            // Control: no checkpoint, so nothing clears the kept list.
            let mut i = log_isolate(stress);
            run(&mut i, src).unwrap();
            scrub_and_collect(&mut i);
            run(&mut i, "log('later', w.deref() === undefined);").unwrap();
            assert_eq!(
                take_log(),
                ["script temp", "later false"],
                "stress={stress}"
            );
        });
    }
}

/// The checkpoint runs the FinalizationRegistry cleanup (M4-P4-CX-R1 W-1): after a GC that
/// finds the target dead, the next checkpoint calls the callback with the held value once, and
/// a job the callback queues runs in the same checkpoint. Only the embedder API is used.
#[test]
fn a_checkpoint_runs_the_finalization_callback_with_the_held_value() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            run_checkpoint(
                &mut i,
                r#"
var fr = new FinalizationRegistry(function (held) {
  log('cleanup', held);
  Promise.resolve().then(function () { log('job from cleanup', held); });
});
var keep = {};
(function () { fr.register({}, 'held-1'); })();
fr.register(keep, 'kept');
log('registered');
"#,
            )
            .unwrap();
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            // Under GC stress the target may die during the script; the callback then runs in
            // the script's own checkpoint. Either way: once, then its job.
            assert_eq!(
                take_log(),
                ["registered", "cleanup held-1", "job from cleanup held-1"],
                "stress={stress}"
            );
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            assert_eq!(take_log(), Vec::<String>::new(), "stress={stress}");
            i.verify_heap().expect("a valid heap");
        });
    }
}

/// `gc()` for the scripts below: a collection after a stack scrub (the words below this frame
/// are zeroed, so the frames of earlier calls leave no stale pointer for the scan).
#[inline(never)]
fn gc_scrubbed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
    collect_now(cx);
    Ok(Local::undefined())
}

#[inline(never)]
fn collect_now(cx: &mut Cx<'_>) {
    cx.collect_garbage();
}

/// `registerFresh(fr, held)`: `fr.register({}, held)` with the new target made here, in Rust,
/// so no live interpreter frame keeps a stale copy of it for the conservative scan (a debug
/// build keeps every temporary of the dispatch loop in its own stack slot).
#[inline(never)]
fn register_fresh<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let fr = args.get(0);
    let held = args.get(1);
    let o = cx.new_object();
    let register = cx.get(fr.as_object().expect("a registry"), "register")?;
    cx.call(register, fr, &[o.into(), held])?;
    Ok(Local::undefined())
}

/// A GC inside a cleanup callback finds another dead target (R-GC-P4-3; M4-P4 fixer B): the
/// checkpoint serves that cell too before it ends, with no job queued to keep it going. node
/// v24.22.1 `--expose-gc` prints the same 3 lines (scratchpad m1m4/p4/fixB/js/fr-gc-in-cb.js,
/// with `registerFresh = (fr, h) => fr.register({}, h)`: `registered`, `cleanup outer`,
/// `cleanup inner`). Before the fix the checkpoint stopped
/// after `cleanup outer`, and `inner` waited for the next one.
#[test]
fn a_gc_in_a_cleanup_callback_is_served_in_the_same_checkpoint() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            i.register_host_function("gc", 0, gc_scrubbed);
            i.register_host_function("registerFresh", 2, register_fresh);
            run_checkpoint(
                &mut i,
                r#"
var fr = new FinalizationRegistry(function (held) {
  log('cleanup', held);
  if (held === 'outer') {
    registerFresh(fr, 'inner');
    gc();
  }
});
registerFresh(fr, 'outer');
log('registered');
"#,
            )
            .unwrap();
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            // Under GC stress `outer` may die during the script; its callback then runs in the
            // script's own checkpoint. Either way: both, once each, in this order.
            assert_eq!(
                take_log(),
                ["registered", "cleanup outer", "cleanup inner"],
                "stress={stress}"
            );
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            assert_eq!(take_log(), Vec::<String>::new(), "stress={stress}");
            i.verify_heap().expect("a valid heap");
        });
    }
}
