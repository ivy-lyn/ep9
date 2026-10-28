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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ep9_vm::{Args, Cx, Isolate, JsResult, Local, TerminateHandle};

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

/// A collection after a stack scrub: [`scrub_below`] zeroes the stack below this frame, then
/// the collection runs from here, so its own frames (`Isolate::collect_garbage` down to the
/// spill trampoline, which the conservative scan reads) lie on zeros. The scrub must be a
/// call of its own (fixer E): with the zeroed array in this frame, the collection's frames
/// lay below the array, on the stale words of a script's frames (a debug match loop takes
/// about 200 KiB of stack, so a script run below a 64 KiB pad reaches past a 256 KiB array),
/// and a slot that those frames leave unwritten could hold a dead target's address. On
/// linux-x64 debug that kept the target of
/// [`a_checkpoint_lets_a_weakref_target_die_at_the_next_gc`] alive (CI run 40564429974).
/// The second scrub wipes the collection's own frames before the next checkpoint's frames lie
/// over them: the collection's deeper frames (the work after the scan) leave the addresses
/// of live cells, and a slot that a later live frame leaves unwritten keeps such a word for
/// that frame's own collections. Without it a chain target's one-past-end address, left
/// about 5 KiB below this frame, stopped the 30-link chain after 2 links (mac a64 debug,
/// `EP9_HASH_SEED` 59; verifier E, V-E1).
#[inline(never)]
fn scrub_and_collect(i: &mut Isolate) {
    scrub_below();
    i.collect_garbage();
    scrub_below();
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

/// `gc()` for the scripts below: a collection after a stack scrub ([`scrub_then_collect`]: the
/// words below this frame are zeroed first, and the collection's frames then lie on them, as
/// in [`scrub_and_collect`]), so the frames of earlier calls leave no stale pointer for the
/// scan.
#[inline(never)]
fn gc_scrubbed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    scrub_then_collect(cx);
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

// ---- the cleanup drain of a checkpoint (M4-P4 fixer C; P4-ORCH-8; Codex C2-1, C2-2, B1-2) ----

/// `dropAndCollect(keep, k)`: `keep[k] = null`, then a collection after a stack scrub. The
/// store runs here, in Rust: a debug build's keyed store from JS left the old element (the
/// target) in a live dispatch-loop slot, so the collection that followed in the same callback
/// still saw it (fixer C probe). The collection runs where a scrub has just zeroed 512 KiB
/// (the store's frames and the last collection's frames lie there).
#[inline(never)]
fn drop_and_collect<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    drop_below(cx, args.get(0), args.get(1))?;
    scrub_then_collect(cx);
    Ok(Local::undefined())
}

#[inline(never)]
fn drop_below<'s>(cx: &mut Cx<'s>, keep: Local<'s>, k: Local<'s>) -> JsResult<()> {
    let keep = keep.as_object().expect("an array");
    let k = u64::try_from(k.as_int32().expect("an index")).expect("an index");
    cx.set_index(keep, k, Local::null(), true).map(|_| ())
}

/// A collection after a stack scrub, from inside a host function ([`scrub_and_collect`] from
/// Rust): the scrub is a call of its own, so the collection's frames lie on its zeros.
#[inline(never)]
fn scrub_then_collect(cx: &mut Cx<'_>) {
    scrub_below();
    collect_now(cx);
}

/// Zeroes 512 KiB of stack below the caller's frame. The caller's next call (a collection)
/// then has zeros under its frames, where the conservative scan reads.
#[inline(never)]
fn scrub_below() {
    let mut words = [0usize; 1 << 16];
    black_box(&mut words);
}

/// A chain of cleanup rounds with no promise job between them: `links` targets are registered
/// up front (held value `k`), each one kept alive only by `keep[k]`; the script lets target 0
/// go, and each callback lets the next target go and collects (`dropAndCollect`), so each
/// round's GC finds the next dead target. `body` runs after that. The targets are made by the
/// script, not in a callback: a target made in a callback (as by [`register_fresh`]) can sit
/// where a freed cell was that a stale word of a live debug frame still names, and such a
/// chain stalled at link 9 (fixer C probe). A sentinel cell, registered first, stays alive.
///
/// Each target is followed by a gap object (kept alive by `gaps`), so no cell starts where a
/// target ends. The cleanup hook reads the cells, and its own live frame may keep a cell's
/// address while a callback runs: on linux-x64 release, `Cx::cleanup_finalization_registries`
/// keeps the `prev` cell of its collect pass (the cell of target `h + 1`) in a stack slot
/// across the callback of link `h`. The conservative scan's end rule reads a cell's address as
/// one past the end of the cell before it. When a cell was made right after its target, that
/// kept target `h + 1` alive, and both chain tests stopped after 1 link (CI run 40715227268).
/// A mac a64 release probe that holds that `prev` word (or every cell address of the collect
/// pass) in that frame stops both tests the same way with the old layout, and not with this
/// one.
fn chain_script(links: usize, body: &str) -> String {
    format!(
        r#"
var n = 0, keep = [], gaps = [], sentinel = {{}};
var fr = new FinalizationRegistry(function (h) {{
  n++;
  log('cb', h);
  if (h + 1 < {links}) dropAndCollect(keep, h + 1);
  {body}
}});
fr.register(sentinel, -1);
(function () {{ for (var k = 0; k < {links}; k++) {{ var t = {{}}; gaps.push({{}}); keep.push(t); fr.register(t, k); }} }})();
keep[0] = null;
log('registered');
"#
    )
}

/// The log of a chain of `links` that runs to its end.
fn chain_log(links: usize) -> Vec<String> {
    core::iter::once("registered".to_string())
        .chain((0..links).map(|k| format!("cb {k}")))
        .collect()
}

/// `n`, the number of chain callbacks that started.
fn chain_count(i: &mut Isolate) -> usize {
    run(i, "log(n);").unwrap();
    take_log()[0].parse().unwrap()
}

/// Runs the rest of a chain that a checkpoint left: lets target `n` go (a stopped callback
/// may not have), then a scrubbed collection and a checkpoint, until all `links` started (at
/// most 8 times). A run of the chain can end before its last link even with no cap: the
/// conservative stack scan may keep a target that the chain let go alive through the
/// collection of its round, so no new dead target is found (a word equal to the target's
/// address in a live frame: under GC stress in a debug build the 30-link chain stopped after
/// link 28; fixer C probe with `record_conservative_roots`). The next scrubbed collection
/// from here frees it. The lines of these checkpoints are added to `got`.
fn finish_chain(i: &mut Isolate, links: usize, got: &mut Vec<String>) {
    for _ in 0..8 {
        if chain_count(i) == links {
            return;
        }
        run(i, "if (n < keep.length) keep[n] = null;").unwrap();
        scrub_and_collect(i);
        i.perform_microtask_checkpoint()
            .map_err(|e| e.message)
            .unwrap();
        got.extend(take_log());
    }
    panic!("the chain did not end: {} of {links}", chain_count(i));
}

/// A chain of 30 cleanup callbacks, each of which lets one more target die and collects,
/// with no promise job between them: one checkpoint runs them all, as node v24.22.1
/// `--expose-gc` does (scratchpad m1m4/p4/verB/js/fr-loop.js prints `cb 1` to `cb 30`; the ep9
/// release CLI prints the same). Before the fix the checkpoint stopped after 8 cleanup rounds
/// (`MAX_CLEANUP_ROUNDS`) and nothing ran the rest (Codex B1-2). The first checkpoint must
/// run more than 8 links; a debug build may stop it early at a conservative-scan root
/// ([`finish_chain`]), so the rest runs in the next ones, each link once, in order.
#[test]
fn a_chain_of_30_cleanup_callbacks_runs_in_one_checkpoint() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            i.register_host_function("dropAndCollect", 2, drop_and_collect);
            run_checkpoint(&mut i, &chain_script(30, "")).unwrap();
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            // Under GC stress target 0 may die during the script: the chain then runs in the
            // script's own checkpoint. Either way the first checkpoint that ran it:
            let mut got = take_log();
            let first = chain_count(&mut i);
            assert!(
                first > 8,
                "{first} links in one checkpoint (stress={stress})"
            );
            if cfg!(not(debug_assertions)) && !stress {
                assert_eq!(first, 30, "a release build runs all 30 at once");
            }
            finish_chain(&mut i, 30, &mut got);
            assert_eq!(got, chain_log(30), "first run {first} (stress={stress})");
            i.verify_heap().expect("a valid heap");
        });
    }
}

thread_local! {
    static TERM: RefCell<Option<TerminateHandle>> = const { RefCell::new(None) };
}

/// `stop()`: requests a termination; it stops the running code at its next interrupt poll.
fn stop<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    TERM.with(|t| t.borrow().as_ref().expect("a handle").terminate());
    Ok(Local::undefined())
}

/// A termination during a long cleanup chain (300 links; from link 6 on each callback also
/// runs a loop of 2,000 steps, so the interrupt budget runs out within about 6 links) stops
/// the checkpoint (it cannot be caught); the cells not yet served stay, and later checkpoints
/// run the rest of the chain, in order, each link once. The callback that the termination
/// stopped may miss its line (a poll before its `log`; its cell was taken off `[[Cells]]`
/// before the call, §9.13.1 step 3.b). Before the fix the first checkpoint ended `Ok` after
/// 8 cleanup rounds.
#[test]
fn a_termination_stops_a_cleanup_chain_and_the_rest_runs_later() {
    const LINKS: usize = 300;
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            i.register_host_function("dropAndCollect", 2, drop_and_collect);
            i.register_host_function("stop", 0, stop);
            TERM.with(|t| *t.borrow_mut() = Some(i.terminate_handle()));
            let body = "if (h === 6) stop(); if (h >= 6) for (var k = 0; k < 2000; k++) {}";
            // Under GC stress target 0 may die during the script: the chain then starts, and
            // stops, in the script's own checkpoint.
            let e = match run_checkpoint(&mut i, &chain_script(LINKS, body)) {
                Err(m) => m,
                Ok(()) => {
                    scrub_and_collect(&mut i);
                    i.perform_microtask_checkpoint()
                        .expect_err("the termination stops the chain")
                        .message
                }
            };
            assert_eq!(e, "execution terminated", "stress={stress}");
            let mut got = take_log();
            let n = chain_count(&mut i);
            assert!(n > 7 && n < LINKS, "n = {n} (stress={stress})");
            finish_chain(&mut i, LINKS, &mut got);
            let full = chain_log(LINKS);
            let mut without = full.clone();
            without.remove(n);
            assert!(
                got == full || got == without,
                "{} lines, n = {n} (stress={stress})",
                got.len()
            );
            i.verify_heap().expect("a valid heap");
        });
    }
}

/// A builtin cleanup callback (`Boolean`) takes a budget unit and polls per cell, as a job
/// does (Codex C2-1): an `InterruptHandle` request runs during a cleanup of 12,000 cells (more
/// than the 10,000 units of one budget). With a request that also terminates, the cleanup
/// stops and the cells not yet served stay in the registry (`unregister` finds them); the next
/// checkpoint, with no GC between, serves them (the pending flag is set again when the poll
/// stops the cleanup; verifier C V-C1). With a request that only marks, every cell is served.
/// Before the fix a builtin callback ran no poll, so the request waited for the next one. No
/// GC stress (12,000 registrations).
#[test]
fn a_builtin_cleanup_callback_polls_for_interrupts() {
    // (terminate, a second checkpoint after the first)
    for (terminate, again) in [(true, false), (true, true), (false, false)] {
        common::on_big_stack(move || {
            let mut i = log_isolate(false);
            run_checkpoint(
                &mut i,
                r#"
var tok = {};
var fr = new FinalizationRegistry(Boolean);
(function () { for (var k = 0; k < 12000; k++) fr.register({}, k, tok); })();
"#,
            )
            .unwrap();
            scrub_and_collect(&mut i);
            let ran = Arc::new(AtomicBool::new(false));
            let (r, t) = (Arc::clone(&ran), i.terminate_handle());
            i.interrupt_handle().request(Box::new(move || {
                r.store(true, Ordering::SeqCst);
                if terminate {
                    t.terminate();
                }
            }));
            let res = i.perform_microtask_checkpoint();
            assert!(ran.load(Ordering::SeqCst), "the request ran in the cleanup");
            match res {
                Err(e) => assert!(terminate && e.terminated, "{}", e.message),
                Ok(()) => assert!(!terminate, "the termination stops the cleanup"),
            }
            if again {
                i.perform_microtask_checkpoint()
                    .map_err(|e| e.message)
                    .unwrap();
            }
            run(&mut i, "log('left', fr.unregister(tok));").unwrap();
            assert_eq!(take_log(), [format!("left {}", terminate && !again)]);
            i.verify_heap().expect("a valid heap");
        });
    }
}

/// ClearKeptObjects runs at the end of the checkpoint, after the cleanup (Codex C2-2): a
/// callback that makes a WeakRef and queues no job leaves its target collectible, so the
/// next GC (no checkpoint between) frees it. Before the fix the target stayed in the kept
/// list until the next checkpoint.
#[test]
fn a_weakref_made_in_a_cleanup_callback_is_not_kept_after_the_checkpoint() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = log_isolate(stress);
            run_checkpoint(
                &mut i,
                r#"
var w;
var fr = new FinalizationRegistry(function (held) {
  w = new WeakRef((function () { return { name: 'made in cleanup' }; })());
  log('cleanup', held, w.deref().name);
});
(function () { fr.register({}, 'held'); })();
log('registered');
"#,
            )
            .unwrap();
            scrub_and_collect(&mut i);
            i.perform_microtask_checkpoint()
                .map_err(|e| e.message)
                .unwrap();
            scrub_and_collect(&mut i);
            run(&mut i, "log('after', w.deref() === undefined);").unwrap();
            assert_eq!(
                take_log(),
                ["registered", "cleanup held made in cleanup", "after true"],
                "stress={stress}"
            );
            i.verify_heap().expect("a valid heap");
        });
    }
}
