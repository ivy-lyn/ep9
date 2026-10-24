//! Helpers for the builtin tests: run JS on an isolate with `ep9_builtins::TABLE` (compiled by
//! ep9_frontend) and collect what `print` wrote. Each test file that uses them says
//! `mod common;`.
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

/// A new isolate with `ep9_builtins::TABLE` and `print`. `stress`: a collection before every
/// allocation, then the heap verifier.
pub fn isolate(stress: bool) -> Isolate {
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
    iso
}

/// Runs the sloppy script `src` on a fresh isolate: the printed lines, or the uncaught
/// exception as "Name: message". Then collects and verifies the heap.
pub fn run(src: &str, stress: bool) -> Result<Vec<String>, String> {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(|e| format!("compile error: {e:?}"))?;
    let mut iso = isolate(stress);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// The global binding `name` as text (`Cx::global_binding`, then `Cx::display`): `None` if
/// there is none. Panics if the read throws.
pub fn global_text(iso: &mut Isolate, name: &str) -> Option<String> {
    iso.with_cx(|cx| Ok(cx.global_binding(name)?.map(|v| cx.display(v))))
        .expect("the read does not throw")
}

/// `run` with and without GC stress; both must agree. Runs on a thread with a big stack
/// (debug builds have no tail calls).
pub fn run_both(src: &'static str) -> Result<Vec<String>, String> {
    let a = on_big_stack(move || run(src, false));
    let b = on_big_stack(move || run(src, true));
    assert_eq!(a, b, "GC stress changed the result of {src:?}");
    a
}

pub fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("a thread")
        .join()
        .expect("no panic")
}
