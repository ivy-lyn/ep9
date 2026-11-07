//! Helpers for the Intl tests: run a script on an isolate with `ep9_builtins::TABLE` (which
//! lists the ep9_intl modules) and collect what `print` wrote. A copy of the runner of
//! `crates/ep9_temporal/tests/common/mod.rs`.
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
