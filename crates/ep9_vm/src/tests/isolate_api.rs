//! The `with_cx`-scoped embedding forms (`Cx::run_script`, `Cx::global_binding`,
//! `Cx::display`), `Isolate::global` and the TDZ, `InterruptHandle::request`, and the heap
//! limit callback (M2-P3 ISOLATE-API; H20-Q4, H23).

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use ep9_bytecode::{RegisterList, RuntimeId, ScopeKind, VariableMode, VerifiedUnit};
use ep9_heap::HeapConfig;

use super::{Asm, config, isolate, on_big_stack, take_output, unit, unit_with_scopes};
use crate::{Args, Cx, Dispatch, IsolateConfig, JsResult, Local};

// ---- scoped forms and globals ----------------------------------------------------------------

/// `let y = 7; var z = 'zz'; 42` (DeclareGlobals, then InitializeGlobal and StaGlobal).
fn globals_script() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let init = s.b.new_register_list(2);
    s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
    s.lda_str("y");
    s.b.star(init.get(0)).lda_smi(7).star(init.get(1));
    s.b.call_runtime(RuntimeId::InitializeGlobal, init, 2);
    s.lda_str("zz");
    s.sta_global("z");
    s.b.lda_smi(42).return_();
    unit_with_scopes(
        "",
        vec![(ScopeKind::Script, None, vec![("y", VariableMode::Let)])],
        vec![s.finish()],
    )
}

/// The scoped forms under every dispatch, with and without GC stress: the script's result and
/// the globals it made survive a collection inside `with_cx`; `global_binding` finds a name
/// on the global object's prototype chain, which `Isolate::global` (own data properties only)
/// does not.
#[test]
fn scoped_forms_run_a_script_and_read_globals() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                let r = iso.with_cx(|cx| {
                    let v = cx.run_script(globals_script(), "globals.js")?;
                    cx.collect_garbage();
                    let y = cx.global_binding("y")?.expect("y");
                    let z = cx.global_binding("z")?.expect("z");
                    let to_string = cx.global_binding("toString")?.expect("toString");
                    let none = cx.global_binding("nope")?;
                    cx.collect_garbage();
                    Ok((
                        cx.display(v),
                        cx.display(y),
                        cx.display(z),
                        to_string.is_callable(),
                        none.is_none(),
                    ))
                });
                let label = format!("{} stress={stress}", d.name());
                assert_eq!(
                    r,
                    Ok(("42".into(), "7".into(), "zz".into(), true, true)),
                    "{label}"
                );
                assert_eq!(
                    iso.global("y").map(|v| iso.display(v)).as_deref(),
                    Some("7")
                );
                assert!(iso.global("toString").is_none(), "{label}");
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// A throw inside `Cx::run_script` is the `Err` of the closure, and `with_cx` reports it.
#[test]
fn a_throw_in_cx_run_script_is_the_exception_of_with_cx() {
    let mut s = Asm::script(0);
    s.lda_str("boom");
    s.b.throw();
    let u = unit("", vec![s.finish()]);
    let mut iso = isolate(IsolateConfig::default());
    let e = iso
        .with_cx(|cx| cx.run_script(u, "throw.js").map(|_| ()))
        .unwrap_err();
    assert_eq!(e.message, "boom");
    assert!(!e.terminated);
    // The isolate keeps working.
    let r = iso.with_cx(|cx| {
        let v = cx.run_script(globals_script(), "again.js")?;
        Ok(cx.display(v))
    });
    assert_eq!(r.as_deref(), Ok("42"));
}

/// `let x = (() => { throw 1 })();` as bytecode: the script declares `x` (TDZ) and throws
/// before it initializes it.
fn tdz_script() -> VerifiedUnit {
    let mut s = Asm::script(0);
    s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
    s.b.lda_smi(1).throw();
    unit_with_scopes(
        "",
        vec![(ScopeKind::Script, None, vec![("x", VariableMode::Let)])],
        vec![s.finish()],
    )
}

/// After the script fails, `x` stays in its TDZ: `Isolate::global` gives `None` (never the
/// hole), and `Cx::global_binding` throws the ReferenceError that a read of `x` throws.
#[test]
fn a_binding_in_its_tdz_is_none_or_a_reference_error() {
    for &d in Dispatch::ALL {
        on_big_stack(move || {
            let mut iso = isolate(config(d, false));
            let e = iso.run_script(tdz_script(), "tdz.js").unwrap_err();
            assert_eq!(e.message, "1");
            assert!(iso.global("x").is_none(), "{}", d.name());
            let e = iso
                .with_cx(|cx| cx.global_binding("x").map(|_| ()))
                .unwrap_err();
            assert_eq!(e.constructor.as_deref(), Some("ReferenceError"));
            assert_eq!(
                e.message,
                "ReferenceError: Cannot access 'x' before initialization"
            );
        });
    }
}

// ---- InterruptHandle -------------------------------------------------------------------------

thread_local! {
    /// Callbacks that ran on this thread (the isolate's thread in the tests below).
    static RAN: Cell<u32> = const { Cell::new(0) };
}

/// `seen()`: how many callbacks ran on this thread so far.
fn seen<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::int32(RAN.with(Cell::get) as i32))
}

/// `for (let i = 0; i < n; i++) {} return seen();`
fn loop_then_seen(n: i32) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let header = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(n).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.ldar(i).inc(s.bin).star(i);
    s.b.jump_loop(header, 0, s.loop_);
    s.b.bind(exit);
    s.lda_global("seen");
    s.b.star(s.f);
    s.b.call_undefined_receiver0(s.f, s.call).return_();
    unit("", vec![s.finish()])
}

/// A callback requested from another thread (spawn, request, join, then run) runs once, on
/// the isolate thread, at the first poll of the next run: with the budget set low, the loop's
/// poll runs it before the script reads `seen()`. Every dispatch (debug: the sibling
/// expansion runs this short loop on a big stack).
#[test]
fn a_requested_callback_runs_at_the_first_poll() {
    for &d in Dispatch::ALL {
        on_big_stack(move || {
            let mut iso = isolate(config(d, false));
            iso.register_host_function("seen", 0, seen);
            let h = iso.interrupt_handle();
            std::thread::spawn(move || {
                h.request(Box::new(|| RAN.with(|r| r.set(r.get() + 1))));
            })
            .join()
            .unwrap();
            assert_eq!(RAN.with(Cell::get), 0, "nothing runs before a poll");
            iso.cx.budget = 3;
            let r = iso.run_script(loop_then_seen(100), "poll.js");
            assert_eq!(
                r.map(|v| iso.display(v)).as_deref(),
                Ok("1"),
                "{}",
                d.name()
            );
            // The bit is clear and the queue empty: the next polls run nothing.
            iso.cx.budget = 3;
            let r = iso.run_script(loop_then_seen(100), "again.js");
            assert_eq!(
                r.map(|v| iso.display(v)).as_deref(),
                Ok("1"),
                "{}",
                d.name()
            );
        });
    }
}

/// With the budget not used up, the run has no poll, so the callback waits for the first
/// poll of a later run. Requests run in request order.
#[test]
fn a_callback_waits_for_a_poll_and_callbacks_keep_their_order() {
    on_big_stack(|| {
        let mut iso = isolate(IsolateConfig::default());
        iso.register_host_function("seen", 0, seen);
        let order = Arc::new(Mutex::new(Vec::new()));
        let h = iso.interrupt_handle();
        for k in 0..3 {
            let o = Arc::clone(&order);
            h.request(Box::new(move || {
                o.lock().unwrap().push(k);
                RAN.with(|r| r.set(r.get() + 1));
            }));
        }
        // 100 back edges of a 10,000 budget: no poll.
        let r = iso.run_script(loop_then_seen(100), "idle.js");
        assert_eq!(r.map(|v| iso.display(v)).as_deref(), Ok("0"));
        assert!(order.lock().unwrap().is_empty());
        iso.cx.budget = 3;
        let r = iso.run_script(loop_then_seen(100), "poll.js");
        assert_eq!(r.map(|v| iso.display(v)).as_deref(), Ok("3"));
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
    });
}

/// `for (;;) {}`
fn endless_loop() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let head = s.b.bound_label();
    s.b.lda_zero().jump_loop(head, 0, s.loop_);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// A live thread requests a callback while the isolate runs an endless loop. The callback
/// runs on the isolate thread and terminates the script from there (it runs no JS; the same
/// poll then stops the loop), and the isolate keeps working.
#[test]
fn a_callback_reaches_a_running_endless_loop() {
    // Debug builds have no tail calls: an endless loop under the sibling expansion would
    // exhaust the native stack before the request comes (as `terminate_from_another_thread`).
    let dispatches: &[Dispatch] = if cfg!(debug_assertions) {
        &[Dispatch::Match]
    } else {
        Dispatch::ALL
    };
    for &d in dispatches {
        let (r, ran_on, iso_thread, after) = on_big_stack(move || {
            let mut iso = isolate(config(d, false));
            let (h, t) = (iso.interrupt_handle(), iso.terminate_handle());
            let ran_on = Arc::new(Mutex::new(None));
            let slot = Arc::clone(&ran_on);
            let requester = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                h.request(Box::new(move || {
                    *slot.lock().unwrap() = Some(std::thread::current().id());
                    t.terminate();
                }));
            });
            take_output();
            let r = iso.run_script(endless_loop(), "loop.js");
            requester.join().unwrap();
            let mut s = Asm::script(0);
            s.lda_str("again");
            s.b.return_();
            let after = iso
                .run_script(unit("", vec![s.finish()]), "again.js")
                .map(|v| iso.display(v));
            let ran_on = *ran_on.lock().unwrap();
            (r.map(|_| ()), ran_on, std::thread::current().id(), after)
        });
        let e = r.unwrap_err();
        assert!(e.terminated, "{}", d.name());
        assert_eq!(
            ran_on,
            Some(iso_thread),
            "the callback ran on the isolate thread"
        );
        assert_eq!(after.as_deref(), Ok("again"));
    }
}

/// The handle is `Send + Sync + Clone`, and a request after the isolate is gone is dropped
/// with the last handle, never run.
#[test]
fn a_request_after_the_isolate_is_dropped_never_runs() {
    fn send_sync<T: Send + Sync + Clone>() {}
    send_sync::<crate::InterruptHandle>();
    let iso = isolate(IsolateConfig::default());
    let h = iso.interrupt_handle();
    drop(iso);
    let dropped = Arc::new(AtomicUsize::new(0));
    struct Mark(Arc<AtomicUsize>);
    impl Drop for Mark {
        fn drop(&mut self) {
            self.0.fetch_add(1, SeqCst);
        }
    }
    let mark = Mark(Arc::clone(&dropped));
    h.request(Box::new(move || {
        let _keep = &mark;
        panic!("must not run");
    }));
    assert_eq!(dropped.load(SeqCst), 0);
    drop(h);
    assert_eq!(dropped.load(SeqCst), 1);
}

// ---- heap limit (H23) ------------------------------------------------------------------------

/// Bytes committed by a new isolate with the VM table (its realm and the first pages).
fn committed_after_init() -> usize {
    isolate(IsolateConfig::default())
        .heap()
        .stats()
        .committed_bytes
}

/// An isolate whose heap limit is 1 MiB above what its init commits.
fn small_heap_isolate(near_limit: fn(usize, usize) -> usize) -> crate::Isolate {
    isolate(IsolateConfig {
        heap: HeapConfig {
            max_heap_bytes: committed_after_init() + (1 << 20),
            near_limit: Some(near_limit),
            ..HeapConfig::default()
        },
        ..IsolateConfig::default()
    })
}

/// Keeps `n` new objects reachable from one array (about 64 bytes each).
fn fill(cx: &mut Cx<'_>, n: u64) -> JsResult<()> {
    let a = cx.new_array();
    for i in 0..n {
        let o = cx.new_object();
        cx.set_index(a, i, o, true)?;
    }
    Ok(())
}

static RAISED: AtomicUsize = AtomicUsize::new(0);

fn raise_by_64_mib(committed: usize, limit: usize) -> usize {
    RAISED.fetch_add(1, SeqCst);
    assert!(committed <= limit);
    limit + (64 << 20)
}

/// With a small `max_heap_bytes`, the near-limit callback runs once and raises the limit, and
/// the allocation that hit the limit goes on.
#[test]
fn the_near_limit_callback_raises_the_heap_limit_once() {
    let mut iso = small_heap_isolate(raise_by_64_mib);
    let limit = iso.heap().max_heap_bytes();
    // About 6 MiB: past the 1 MiB room, well below the raised limit.
    iso.with_cx(|cx| fill(cx, 100_000)).unwrap();
    assert_eq!(RAISED.load(SeqCst), 1, "called once");
    assert_eq!(iso.heap().max_heap_bytes(), limit + (64 << 20));
    assert!(iso.heap().stats().committed_bytes > limit);
    iso.collect_garbage();
    iso.verify_heap().unwrap();
}

static KEPT: AtomicUsize = AtomicUsize::new(0);

fn keep_the_limit(_committed: usize, limit: usize) -> usize {
    KEPT.fetch_add(1, SeqCst);
    limit
}

/// When the callback keeps the limit, the allocation panics with "ep9: heap limit reached"
/// (a release build aborts there; this test runs in the dev profile, which unwinds).
#[test]
fn a_kept_limit_is_the_fatal_heap_limit_panic() {
    let mut iso = small_heap_isolate(keep_the_limit);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = iso.with_cx(|cx| fill(cx, 100_000));
    }));
    let payload = r.expect_err("the heap limit must stop the fill");
    let msg = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<not a string>");
    assert_eq!(msg, "ep9: heap limit reached");
    assert_eq!(KEPT.load(SeqCst), 1);
    // The panic left the isolate in the middle of a builtin call: do not use or drop it.
    std::mem::forget(iso);
}

/// H23-Q3: no default heap limit below the cage.
#[test]
fn the_default_heap_limit_is_the_cage() {
    assert_eq!(HeapConfig::default().max_heap_bytes, usize::MAX);
    assert!(HeapConfig::default().near_limit.is_none());
    let iso = isolate(IsolateConfig::default());
    assert_eq!(iso.heap().max_heap_bytes(), usize::MAX);
}
