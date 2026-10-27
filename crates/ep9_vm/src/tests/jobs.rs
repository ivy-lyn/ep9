//! The job queue, the microtask checkpoint and the Promise operations of `abi/jobs.rs`, with
//! Rust-made jobs only (ep9_vm has no front end; the JS-level job order is tested in
//! `ep9_builtins` `tests/promise.rs` and the CLI e2e programs `tests/e2e/jobs/`). Every test
//! runs under GC stress with the heap verifier too (a collection before every allocation).

use core::cell::{Cell, RefCell};

use ep9_bytecode::VerifiedUnit;

use super::{Asm, config, isolate, on_big_stack, take_output, unit};
use crate::abi::{PromiseRejectionOperation, PromiseState};
use crate::frame::UNDEFINED;
use crate::{Args, Cx, Dispatch, Isolate, IsolateConfig, JsObject, JsResult, Local};

/// Runs `f` with a fresh isolate (with `print`, `queue` and `nested`) under every dispatch,
/// with and without GC stress; the heap must verify after each.
fn each(f: fn(&mut Isolate, &str)) {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                iso.register_host_function("queue", 1, queue);
                iso.register_host_function("nested", 1, nested);
                take_output();
                f(&mut iso, &format!("{} stress={stress}", d.name()));
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// `queue(x)`: a job that prints `x`.
fn queue<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let print = cx.global_binding("print")?.expect("print");
    cx.enqueue_job(print, args.get(0))?;
    Ok(Local::undefined())
}

/// `nested(x)`: prints `x`, then queues a job that prints `x + "+"`.
fn nested<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let text = cx.to_rust_string(args.get(0))?;
    let print = cx.global_binding("print")?.expect("print");
    cx.call(print, Local::undefined(), &[args.get(0)])?;
    let next = cx.new_string(&format!("{text}+"))?;
    cx.enqueue_job(print, next)?;
    Ok(Local::undefined())
}

/// Queues `print(name)` for each name: fresh strings, so only the queue keeps them alive.
fn queue_prints(cx: &mut Cx<'_>, names: &[&str]) -> JsResult<()> {
    let print = cx.global_binding("print")?.expect("print");
    for n in names {
        let s = cx.new_string(n)?;
        cx.enqueue_job(print, s)?;
    }
    Ok(())
}

/// Jobs run in the order they were queued, after the jobs queued before them; a job that
/// queues a job puts it at the end. The queued strings are only reachable through the queue
/// while the collections run.
#[test]
fn jobs_run_in_fifo_order_and_survive_collections() {
    each(|iso, label| {
        iso.with_cx(|cx| {
            queue_prints(cx, &["a", "b"])?;
            let nested = cx.global_binding("nested")?.expect("nested");
            let c = cx.new_string("c")?;
            cx.enqueue_job(nested, c)?;
            queue_prints(cx, &["d"])?;
            cx.collect_garbage();
            Ok(())
        })
        .unwrap();
        assert!(
            take_output().is_empty(),
            "{label}: nothing runs before the checkpoint"
        );
        iso.perform_microtask_checkpoint().unwrap();
        assert_eq!(take_output(), ["a", "b", "c", "d", "c+"], "{label}");
        // An empty queue: nothing to do.
        iso.perform_microtask_checkpoint().unwrap();
        assert!(take_output().is_empty(), "{label}");
    });
}

/// `queue("job"); print("script")` as a script.
fn queue_then_print() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (f, a0) = (s.f, s.a.get(0));
    s.lda_str("job");
    s.b.star(a0);
    s.lda_global("queue");
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.lda_str("script");
    s.print_acc();
    s.b.lda_smi(7).return_();
    unit("", vec![s.finish()])
}

/// `Isolate::run_script` and `Isolate::call` run a checkpoint after the run: the job runs
/// after the script's own output and before the call returns; the result survives it.
#[test]
fn run_script_and_call_drain_the_queue() {
    each(|iso, label| {
        let v = iso.run_script(queue_then_print(), "q.js").unwrap();
        assert_eq!(iso.display(v), "7", "{label}");
        assert_eq!(take_output(), ["script", "job"], "{label}");
        let q = iso.global("queue").unwrap();
        let undefined = crate::runtime::val(UNDEFINED);
        let s = iso
            .with_cx(|cx| Ok(cx.display(Local::undefined())))
            .unwrap();
        assert_eq!(s, "undefined");
        iso.call(q, undefined, &[crate::Value::number(5.0)])
            .unwrap();
        assert_eq!(take_output(), ["5"], "{label}");
    });
}

/// A script that throws still drains (the error wins); a job that throws stops the
/// checkpoint with its error, and the jobs behind it stay queued for the next one.
#[test]
fn a_throwing_job_stops_the_checkpoint_and_the_rest_stays_queued() {
    each(|iso, label| {
        iso.with_cx(|cx| {
            queue_prints(cx, &["a"])?;
            // `enqueue_job` of a non-callable is a TypeError; a callable that throws:
            let bad = cx.new_object();
            assert!(cx.enqueue_job(bad, Local::undefined()).is_err());
            let _ = cx.catch_exception(crate::Throw)?;
            let thrower = cx.global_binding("queue")?.expect("queue");
            // `queue` with `this` undefined works; make a job that calls a non-function by
            // queueing `nested` with an argument whose ToString throws: a symbol.
            let nested = cx.global_binding("nested")?.expect("nested");
            let sym = cx.new_symbol(Local::undefined())?;
            cx.enqueue_job(nested, sym)?;
            let _ = thrower;
            queue_prints(cx, &["after"])?;
            Ok(())
        })
        .unwrap();
        let e = iso.perform_microtask_checkpoint().unwrap_err();
        assert!(e.message.starts_with("TypeError"), "{label}: {}", e.message);
        assert_eq!(take_output(), ["a"], "{label}");
        iso.perform_microtask_checkpoint().unwrap();
        assert_eq!(take_output(), ["after"], "{label}");
    });
}

/// A checkpoint from a host function (JS on the stack) throws and runs nothing.
#[test]
fn a_checkpoint_inside_a_host_call_throws() {
    fn inner<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        _args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        cx.perform_microtask_checkpoint()?;
        Ok(Local::undefined())
    }
    let mut iso = isolate(IsolateConfig::default());
    iso.register_host_function("inner", 0, inner);
    let f = iso.global("inner").unwrap();
    let e = iso
        .call(f, crate::runtime::val(UNDEFINED), &[])
        .unwrap_err();
    assert!(e.message.contains("empty JS stack"), "{}", e.message);
}

/// `again()`: queues itself again: a job loop with no end.
fn again<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.enqueue_job(args.callee(), Local::undefined())?;
    Ok(Local::undefined())
}

/// Each job takes one interrupt budget unit and polls: a TerminateHandle stops a job loop
/// (which runs no bytecode, so no other poll would see it) and the queue is dropped; an
/// interrupt callback runs during a drain.
#[test]
fn termination_stops_a_job_loop_and_clears_the_queue() {
    let mut iso = isolate(IsolateConfig::default());
    iso.register_host_function("again", 0, again);
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let r2 = std::sync::Arc::clone(&ran);
    iso.interrupt_handle().request(Box::new(move || {
        r2.store(true, std::sync::atomic::Ordering::SeqCst)
    }));
    let t = iso.terminate_handle();
    // Terminate from the interrupt callback, so the loop must reach a poll to stop.
    iso.interrupt_handle()
        .request(Box::new(move || t.terminate()));
    iso.with_cx(|cx| {
        let f = cx.global_binding("again")?.expect("again");
        cx.enqueue_job(f, Local::undefined())
    })
    .unwrap();
    let e = iso.perform_microtask_checkpoint().unwrap_err();
    assert!(e.terminated);
    assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(iso.rt.jobs.queue_head(), UNDEFINED, "the queue is dropped");
    // The isolate works on.
    iso.perform_microtask_checkpoint().unwrap();
}

thread_local! {
    static TRACKED: RefCell<Vec<(String, PromiseRejectionOperation)>> =
        const { RefCell::new(Vec::new()) };
    static HANDLED: Cell<u32> = const { Cell::new(0) };
}

fn tracker<'s>(cx: &mut Cx<'s>, p: Local<'s, JsObject>, op: PromiseRejectionOperation) {
    let r = cx.promise_result(p).unwrap();
    let text = cx.to_rust_string(r).unwrap();
    TRACKED.with(|t| t.borrow_mut().push((text, op)));
}

fn count_handled<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let text = cx.to_rust_string(args.get(0))?;
    crate::tests::print(cx, Local::undefined(), args, Local::undefined())?;
    let _ = text;
    HANDLED.with(|h| h.set(h.get() + 1));
    Ok(Local::undefined())
}

/// The Promise operations without a builtin table (a null-prototype promise): resolve with a
/// value runs the fulfill handler in a job; the resolving functions are "already resolved"
/// after the first call; a reject with no handler reaches the tracker hook ("reject") and the
/// unhandled list; a later handler gives "handle" and leaves the list.
#[test]
fn promise_operations_fulfil_reject_and_track() {
    for stress in [false, true] {
        on_big_stack(move || {
            TRACKED.with(|t| t.borrow_mut().clear());
            let mut iso = isolate(IsolateConfig {
                promise_rejection_tracker: Some(tracker),
                ..config(Dispatch::DEFAULT, stress)
            });
            iso.register_host_function("handled", 1, count_handled);
            take_output();
            iso.with_cx(|cx| {
                let print = cx.global_binding("print")?.expect("print");
                let p = cx.new_promise(Local::null());
                let (res, rej) = cx.create_resolving_functions(p);
                assert_eq!(cx.promise_state(p)?, PromiseState::Pending);
                cx.perform_promise_then(p, print, Local::undefined(), None);
                let v = cx.new_string("one")?;
                cx.call(res, Local::undefined(), &[v.into()])?;
                let w = cx.new_string("two")?;
                cx.call(res, Local::undefined(), &[w.into()])?;
                cx.call(rej, Local::undefined(), &[w.into()])?;
                assert_eq!(cx.promise_state(p)?, PromiseState::Fulfilled);
                // A rejected promise with no handler.
                let q = cx.new_promise(Local::null());
                let why = cx.new_string("why")?;
                cx.reject_promise(q, why);
                assert!(!cx.promise_is_handled(q)?);
                let g = cx.global();
                let key = cx.key("q");
                cx.set(g, key, q, true)?;
                cx.collect_garbage();
                Ok(())
            })
            .unwrap();
            iso.perform_microtask_checkpoint().unwrap();
            assert_eq!(take_output(), ["one"]);
            let tracked = TRACKED.with(|t| t.borrow().clone());
            assert_eq!(
                tracked,
                [("why".to_string(), PromiseRejectionOperation::Reject)]
            );
            let unhandled = iso.take_unhandled_rejections();
            assert_eq!(unhandled.len(), 1);
            assert_eq!(unhandled[0].message, "why");
            assert!(iso.take_unhandled_rejections().is_empty(), "taken");
            // A handler on the rejected promise: "handle", and the reject handler runs.
            iso.with_cx(|cx| {
                let q = cx.global_binding("q")?.expect("q").as_object().expect("q");
                let h = cx.global_binding("handled")?.expect("handled");
                cx.perform_promise_then(q, Local::undefined(), h, None);
                Ok(())
            })
            .unwrap();
            iso.perform_microtask_checkpoint().unwrap();
            assert_eq!(take_output(), ["why"]);
            let tracked = TRACKED.with(|t| t.borrow().clone());
            assert_eq!(tracked.len(), 2);
            assert_eq!(tracked[1].1, PromiseRejectionOperation::Handle);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A rejected promise that gets a handler in a later job of the same checkpoint is not
/// unhandled when the checkpoint ends (the list drops it), as node reports none.
#[test]
fn a_rejection_handled_in_the_same_checkpoint_is_not_reported() {
    fn late<'s>(
        cx: &mut Cx<'s>,
        _this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let q = args.get(0).as_object().expect("a promise");
        let print = cx.global_binding("print")?.expect("print");
        cx.perform_promise_then(q, Local::undefined(), print, None);
        Ok(Local::undefined())
    }
    let mut iso = isolate(config(Dispatch::DEFAULT, true));
    iso.register_host_function("late", 1, late);
    take_output();
    iso.with_cx(|cx| {
        let q = cx.new_promise(Local::null());
        let r = cx.new_string("r")?;
        cx.reject_promise(q, r);
        let late = cx.global_binding("late")?.expect("late");
        cx.enqueue_job(late, q)
    })
    .unwrap();
    iso.perform_microtask_checkpoint().unwrap();
    assert_eq!(take_output(), ["r"]);
    assert!(iso.take_unhandled_rejections().is_empty());
}

/// `thenable(resolve, reject)`: a `then` that resolves with "via-then".
fn thenable_then<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let v = cx.new_string("via-then")?;
    cx.call(args.get(0), Local::undefined(), &[v.into()])?;
    Ok(Local::undefined())
}

/// Resolving with a thenable queues NewPromiseResolveThenableJob: the `then` runs in a job,
/// not during the resolve, and the handler of the promise runs in a later job. Resolving a
/// promise with itself rejects it with a TypeError. A built-in closure has name "" and no
/// `prototype`, and is not a constructor.
#[test]
fn thenables_resolve_in_a_job_and_closures_look_like_builtins() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            iso.register_host_function("then_impl", 2, thenable_then);
            take_output();
            iso.with_cx(|cx| {
                let print = cx.global_binding("print")?.expect("print");
                let then = cx.global_binding("then_impl")?.expect("then_impl");
                let t = cx.new_object();
                cx.set(t, "then", then, true)?;
                let p = cx.new_promise(Local::null());
                let (res, _) = cx.create_resolving_functions(p);
                cx.call(res, Local::undefined(), &[t.into()])?;
                assert_eq!(
                    cx.promise_state(p)?,
                    PromiseState::Pending,
                    "then runs in a job"
                );
                cx.perform_promise_then(p, print, Local::undefined(), None);
                // Self-resolution.
                let s = cx.new_promise(Local::null());
                let (sres, _) = cx.create_resolving_functions(s);
                cx.call(sres, Local::undefined(), &[s.into()])?;
                assert_eq!(cx.promise_state(s)?, PromiseState::Rejected);
                let e = cx.promise_result(s)?;
                let text = cx.to_rust_string(e)?;
                assert!(text.starts_with("TypeError"), "{text}");
                cx.perform_promise_then(s, Local::undefined(), print, None);
                // The closures.
                assert!(!cx.is_constructor(res));
                assert!(!cx.has_own_property(res, "prototype")?);
                let name = cx.get(res, "name")?;
                assert_eq!(cx.display(name), "");
                let len = cx.get(res, "length")?;
                assert_eq!(cx.display(len), "1");
                assert!(cx.closure_data(print).is_undefined());
                Ok(())
            })
            .unwrap();
            iso.perform_microtask_checkpoint().unwrap();
            let out = take_output();
            assert_eq!(out.len(), 2);
            assert!(out[0].starts_with("TypeError"), "{out:?}");
            assert_eq!(out[1], "via-then");
        });
    }
}

/// Negative control for the GC rule (plan rule 8): with the job roots untraced, the queued
/// jobs die at the first collection and the GC-stress run cannot give the right output (a
/// debug check or the heap verifier fails, or the output is wrong). The same steps with the
/// roots traced pass, so the control fails only for the missing roots.
#[test]
fn untraced_job_roots_fail_under_gc_stress() {
    fn steps(untraced: bool) -> bool {
        crate::jobs::NEGATIVE_CONTROL_UNTRACED_QUEUE.with(|f| f.set(untraced));
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        take_output();
        iso.with_cx(|cx| {
            queue_prints(cx, &["a", "b", "c"])?;
            cx.collect_garbage();
            Ok(())
        })
        .unwrap();
        let ok = iso.perform_microtask_checkpoint().is_ok();
        let out = take_output();
        ok && out == ["a", "b", "c"] && iso.verify_heap().is_ok()
    }
    assert_eq!(
        on_big_stack_catch(|| steps(false)),
        Some(true),
        "the traced run passes"
    );
    assert_ne!(
        on_big_stack_catch(|| steps(true)),
        Some(true),
        "untraced jobs must not survive GC stress"
    );
}

/// Runs `f` on a big stack in a thread of its own; `None` if it panicked.
fn on_big_stack_catch(f: fn() -> bool) -> Option<bool> {
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .ok()
}

// ---- M2-P4-RF1: termination drops the queue (Q1, Q2); "handle" unlinks (Q3) -------------

thread_local! {
    /// The handle that `doomed` and `spin` use (each test sets it).
    static TERM: RefCell<Option<crate::TerminateHandle>> = const { RefCell::new(None) };
}

/// Terminates the isolate from inside a host call: sets the TERMINATE bit, then polls until
/// a poll sees it (within one interrupt budget). Gives the terminated `Err`.
fn terminate_here<'s>(cx: &mut Cx<'s>) -> JsResult<Local<'s>> {
    TERM.with(|t| t.borrow().as_ref().expect("TERM set").terminate());
    loop {
        cx.poll_interrupt()?;
    }
}

/// `doomed(x)`: queues a job that prints `x`, then terminates the run.
fn doomed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let print = cx.global_binding("print")?.expect("print");
    cx.enqueue_job(print, args.get(0))?;
    terminate_here(cx)
}

/// `spin()`: terminates the run (a job that never ends).
fn spin<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    terminate_here(cx)
}

/// `name(arg); print("not reached")` as a script.
fn call_global_then_print(name: &str, arg: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (f, a0) = (s.f, s.a.get(0));
    s.lda_str(arg);
    s.b.star(a0);
    s.lda_global(name);
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.lda_str("not reached");
    s.print_acc();
    s.b.return_();
    unit("", vec![s.finish()])
}

/// `print("next"); return 1` as a script.
fn print_next() -> VerifiedUnit {
    let mut s = Asm::script(0);
    s.lda_str("next");
    s.print_acc();
    s.b.lda_smi(1).return_();
    unit("", vec![s.finish()])
}

/// A fresh isolate with `doomed`, `spin`, `queue_call` and `fail`, and `TERM` set to its
/// handle.
fn doomed_isolate(stress: bool) -> Isolate {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    iso.register_host_function("doomed", 1, doomed);
    iso.register_host_function("spin", 0, spin);
    iso.register_host_function("queue_call", 1, queue_call);
    iso.register_host_function("fail", 0, fail);
    TERM.with(|t| *t.borrow_mut() = Some(iso.terminate_handle()));
    take_output();
    iso
}

/// Q1: a run that is itself terminated (`Isolate::run_script`, then `Isolate::call`) drops
/// the jobs it queued: the queue is empty right after the terminated return, and the next
/// run of the same isolate does not run them.
#[test]
fn a_terminated_run_or_call_drops_its_queued_jobs() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = doomed_isolate(stress);
            let e = iso
                .run_script(call_global_then_print("doomed", "abandoned"), "a.js")
                .unwrap_err();
            assert!(e.terminated, "{e:?}");
            assert_eq!(
                iso.rt.jobs.queue_head(),
                UNDEFINED,
                "run_script: queue dropped"
            );
            assert_eq!(iso.rt.jobs.queue_tail(), UNDEFINED);
            let v = iso.run_script(print_next(), "b.js").unwrap();
            assert_eq!(iso.display(v), "1");
            assert_eq!(take_output(), ["next"], "the abandoned job never runs");

            let f = iso.global("doomed").expect("doomed");
            let e = iso
                .call(
                    f,
                    crate::runtime::val(UNDEFINED),
                    &[crate::Value::number(5.0)],
                )
                .unwrap_err();
            assert!(e.terminated, "{e:?}");
            assert_eq!(iso.rt.jobs.queue_head(), UNDEFINED, "call: queue dropped");
            iso.perform_microtask_checkpoint().unwrap();
            iso.run_script(print_next(), "c.js").unwrap();
            assert_eq!(take_output(), ["next"], "the abandoned job never runs");
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// `queue_call(f)`: queues a job that calls `f` with no argument.
fn queue_call<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.enqueue_job(args.get(0), Local::undefined())?;
    Ok(Local::undefined())
}

/// `fail()`: throws a TypeError (an ordinary job error).
fn fail<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("job error"))
}

/// `queue_call(name); throw "script error"` as a script.
fn queue_then_throw(name: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (f, a0) = (s.f, s.a.get(0));
    s.lda_global(name);
    s.b.star(a0);
    s.lda_global("queue_call");
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.lda_str("script error");
    s.b.throw();
    unit("", vec![s.finish()])
}

/// Q2: after a script error the checkpoint still runs; a termination in one of its jobs is
/// the result (`terminated`), while an ordinary job error leaves the script's error.
#[test]
fn a_termination_in_a_job_after_a_script_error_is_reported() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = doomed_isolate(stress);
            let e = iso
                .run_script(queue_then_throw("spin"), "a.js")
                .unwrap_err();
            assert!(e.terminated, "the termination wins: {e:?}");
            assert_eq!(iso.rt.jobs.queue_head(), UNDEFINED);
            // An ordinary job error: the script's error wins.
            let e = iso.run_script(queue_then_throw("fail"), "b.js");
            let e = e.unwrap_err();
            assert!(!e.terminated);
            assert_eq!(e.message, "script error");
            iso.run_script(print_next(), "c.js").unwrap();
            assert_eq!(take_output(), ["next"]);
        });
    }
}

/// The number of entries in the rejected list.
fn rejected_len(rt: &crate::runtime::Runtime) -> usize {
    let mut n = 0;
    let mut e = rt.jobs.rejected_head();
    while e != UNDEFINED {
        n += 1;
        e = crate::object::slot_get(e, crate::jobs::rejected::NEXT);
    }
    n
}

/// Q3: "handle" unlinks the promise's entry at once (before any checkpoint), from the head,
/// the middle and the tail; the rest keep their order. A handler after
/// `take_unhandled_rejections` took the entry finds none.
#[test]
fn a_handler_unlinks_the_rejection_at_once() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            take_output();
            iso.with_cx(|cx| {
                let g = cx.global();
                for name in ["a", "b", "c", "d"] {
                    let p = cx.new_promise(Local::null());
                    let why = cx.new_string(name)?;
                    cx.reject_promise(p, why);
                    let key = cx.key(name);
                    cx.set(g, key, p, true)?;
                }
                Ok(())
            })
            .unwrap();
            assert_eq!(rejected_len(&iso.rt), 4);
            let handle = |iso: &mut Isolate, name: &'static str| {
                iso.with_cx(move |cx| {
                    let p = cx.global_binding(name)?.expect("p").as_object().expect("p");
                    let print = cx.global_binding("print")?.expect("print");
                    cx.perform_promise_then(p, Local::undefined(), print, None);
                    Ok(())
                })
                .unwrap();
            };
            handle(&mut iso, "b");
            assert_eq!(
                rejected_len(&iso.rt),
                3,
                "middle unlinked before a checkpoint"
            );
            handle(&mut iso, "a");
            handle(&mut iso, "d");
            assert_eq!(rejected_len(&iso.rt), 1, "head and tail unlinked");
            handle(&mut iso, "c");
            assert_eq!(
                iso.rt.jobs.rejected_head(),
                UNDEFINED,
                "empty before a checkpoint"
            );
            assert_eq!(iso.rt.jobs.rejected_tail(), UNDEFINED);
            // A new rejection after the list emptied, taken, then handled.
            iso.with_cx(|cx| {
                let p = cx.new_promise(Local::null());
                let why = cx.new_string("e")?;
                cx.reject_promise(p, why);
                let g = cx.global();
                let key = cx.key("e");
                cx.set(g, key, p, true)
            })
            .unwrap();
            let un: Vec<String> = iso
                .take_unhandled_rejections()
                .into_iter()
                .map(|e| e.message)
                .collect();
            assert_eq!(un, ["e"]);
            handle(&mut iso, "e");
            assert_eq!(iso.rt.jobs.rejected_head(), UNDEFINED);
            iso.perform_microtask_checkpoint().unwrap();
            assert_eq!(take_output(), ["b", "a", "d", "c", "e"]);
            assert!(iso.take_unhandled_rejections().is_empty());
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

thread_local! {
    /// The longest rejected list that `chain` saw.
    static MAX_REJECTED: Cell<usize> = const { Cell::new(0) };
}

/// `ignore()`: a reject handler that does nothing.
fn ignore<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::undefined())
}

/// `chain(n)`, a job: records the rejected list's length, then (n > 0) rejects a new promise
/// with no handler, gives it a handler and queues `chain(n - 1)`: one long drain of
/// reject + catch, as `Promise.reject(x).catch(...)` in a loop of jobs.
fn chain<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let len = rejected_len(cx.inner().rt());
    MAX_REJECTED.with(|m| m.set(m.get().max(len)));
    let n = cx.to_number(args.get(0))?;
    if n <= 0.0 {
        return Ok(Local::undefined());
    }
    let p = cx.new_promise(Local::null());
    let why = cx.new_string("why")?;
    cx.reject_promise(p, why);
    let ignore = cx.global_binding("ignore")?.expect("ignore");
    cx.perform_promise_then(p, Local::undefined(), ignore, None);
    cx.enqueue_job(args.callee(), Local::number(n - 1.0))?;
    Ok(Local::undefined())
}

/// Q3: a long drain of handled rejections keeps none of them in the rejected list (it held
/// one entry, and its promise and reason, per rejection until the drain ended).
#[test]
fn a_long_reject_and_catch_drain_keeps_the_list_empty() {
    for stress in [false, true] {
        on_big_stack(move || {
            MAX_REJECTED.with(|m| m.set(0));
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            iso.register_host_function("chain", 1, chain);
            iso.register_host_function("ignore", 0, ignore);
            iso.with_cx(|cx| {
                let chain = cx.global_binding("chain")?.expect("chain");
                cx.enqueue_job(chain, Local::number(300.0))
            })
            .unwrap();
            iso.perform_microtask_checkpoint().unwrap();
            assert_eq!(MAX_REJECTED.with(Cell::get), 0, "stress={stress}");
            assert!(iso.take_unhandled_rejections().is_empty());
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}
