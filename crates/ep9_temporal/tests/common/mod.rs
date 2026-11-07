//! Helpers for the Temporal tests: run a script on an isolate with `ep9_builtins::TABLE`
//! (which lists the ep9_temporal modules) and collect what `print` wrote. A copy of the
//! small runner of `crates/ep9_builtins/tests/common/mod.rs`.
#![allow(dead_code)] // each test file uses a part

use std::cell::RefCell;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `print(...args)`: ToString of each argument, joined by a space.
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

/// Runs the sloppy script `src` on a fresh isolate: the printed lines, or the uncaught
/// exception as "Name: message". `stress`: a collection before every allocation, then the
/// heap verifier.
pub fn run(src: &str, stress: bool) -> Result<Vec<String>, String> {
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
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("print", 1, print);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// [`run`] without and with GC stress; both must give the same result.
pub fn run_both(src: &str) -> Result<Vec<String>, String> {
    let a = run(src, false);
    let b = run(src, true);
    assert_eq!(a, b, "GC stress changed the result");
    a
}

/// The expected lines.
pub fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Runs `var stage = 'before'; var r = <call>; stage = 'reached';` on a fresh isolate (no
/// GC stress) with the termination requested before the script runs, on its own thread.
/// Nothing polls between the start of the script and the call, so only a poll in the
/// builtin's loop (`Cx::poll_interrupt`, once the budget of 10,000 units runs out) stops it.
/// The result: (terminated, `stage` after the run, `stage` after one more script on the same
/// isolate), or `None` if the run did not end in 60 s (the wait only stops a hung test).
pub fn terminate_during(call: &str) -> Option<(bool, Option<String>, Option<String>)> {
    let src = format!("var stage = 'before'; var r = {call}; stage = 'reached';");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let unit = |src: &str| {
                ep9_frontend::compile(
                    src,
                    SourceOpts::script(Strictness::Sloppy),
                    CompileOptions::default(),
                )
                .expect("compiles")
            };
            let mut iso = Isolate::new(IsolateConfig {
                builtins: Some(&ep9_builtins::TABLE),
                ..IsolateConfig::default()
            })
            .expect("an isolate");
            iso.terminate_handle().terminate();
            let r = iso.with_cx(|cx| cx.run_script(unit(&src), "call.js").map(|_| ()));
            let terminated = r.is_err_and(|e| e.terminated);
            let stage = |iso: &mut Isolate| {
                iso.with_cx(|cx| {
                    let v = cx.global_binding("stage")?;
                    Ok(v.map(|v| cx.display(v)))
                })
                .expect("reads")
            };
            let before = stage(&mut iso);
            // The isolate keeps working.
            iso.with_cx(|cx| {
                cx.run_script(unit("stage = 'again'"), "again.js")
                    .map(|_| ())
            })
            .expect("runs");
            let _ = tx.send((terminated, before, stage(&mut iso)));
        })
        .expect("a thread");
    rx.recv_timeout(std::time::Duration::from_secs(60)).ok()
}
