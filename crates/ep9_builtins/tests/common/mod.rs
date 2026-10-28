//! Helpers for the builtin tests: run JS on an isolate with `ep9_builtins::TABLE` (compiled by
//! ep9_frontend) and collect what `print` wrote. Each test file that uses them says
//! `mod common;`.
//!
//! The GC-timing helpers (plan v10 rule 35, M5-P1 GC5; moved here from `weakref.rs`): a test
//! that asserts an object dies after a GC collects through [`scrub_and_collect`] (from Rust)
//! or [`scrub_then_collect`] (from a host function), so the conservative stack scan reads
//! zeros below the collection instead of the stale words of earlier frames, and holds no
//! struct with padding across the collection (a `GcStats` copy kept a dead shape alive on
//! windows-x64, 64efe18; read it only through [`conservative_roots`]). [`assert_dies`] then
//! tells a stale stack word ("kept by the conservative scan") from a real reference.
#![allow(dead_code)] // each test file uses a part

use std::cell::RefCell;
use std::hint::black_box;

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

/// Zeroes 512 KiB of stack below the caller's frame. The caller's next call (a collection)
/// then has zeros under its frames, where the conservative scan reads. A call of its own
/// (fixer E, M4-P4): with the zeroed array in the caller's frame, the collection's frames lay
/// below the array, on the stale words of a script's frames (a debug match loop takes about
/// 200 KiB of stack, so a script run below a 64 KiB pad reaches past a 256 KiB array), and a
/// slot that those frames leave unwritten could hold a dead target's address (linux-x64
/// debug, CI run 40564429974).
#[inline(never)]
pub fn scrub_below() {
    let mut words = [0usize; 1 << 16];
    black_box(&mut words);
}

/// A collection after a stack scrub, from Rust: [`scrub_below`] zeroes the stack below this
/// frame, the collection runs from here (its frames, `Isolate::collect_garbage` down to the
/// spill trampoline, lie on zeros), then a second scrub wipes the collection's own frames.
/// The collection's deeper frames (the work after the scan) leave the addresses of live
/// cells, and a slot that a later live frame leaves unwritten keeps such a word for that
/// frame's own collections: without the second scrub a chain target's one-past-end address
/// stopped the 30-link weakref chain after 2 links (mac a64 debug, `EP9_HASH_SEED` 59;
/// verifier E, V-E1; 4941d6a).
///
/// Inlined (rule 35: the scrubs run in the caller's own frame), so the zeros also lie under
/// the frame of [`collect_isolate`]. An out-of-line helper's own frame is not under its own
/// scrub, and a slot that it never writes (on Win64 the home space it gives its callees) keeps
/// what the last call at that depth left there (ep9_vm `tests/ordered_table.rs` `collect`:
/// CI run 40733504045).
#[inline(always)]
pub fn scrub_and_collect(i: &mut Isolate) {
    scrub_below();
    collect_isolate(i);
    scrub_below();
}

/// A full collection, out of line, so that its frames lie on the zeros of the caller's scrub.
#[inline(never)]
fn collect_isolate(i: &mut Isolate) {
    i.collect_garbage();
}

/// A collection after a stack scrub, from inside a host function ([`scrub_and_collect`] from
/// Rust): the scrub is a call of its own, so the collection's frames lie on its zeros. Inlined,
/// as [`scrub_and_collect`]: the scrub runs in the host function's frame, so its zeros lie
/// under the frame of [`collect_now`] too.
#[inline(always)]
pub fn scrub_then_collect(cx: &mut Cx<'_>) {
    scrub_below();
    collect_now(cx);
}

#[inline(never)]
fn collect_now(cx: &mut Cx<'_>) {
    cx.collect_garbage();
}

/// The number of cells that the last collection's conservative stack scan marked first
/// (`GcStats::last_conservative_roots`). The `GcStats` copy (7 padding bytes after
/// `last_minor`, 64efe18) lives only in this frame, which the next scrub zeroes.
#[inline(never)]
pub fn conservative_roots(i: &Isolate) -> usize {
    i.gc_stats().last_conservative_roots
}

/// Rule 35's check of "this object dies at the next GC": a scrubbed collection
/// ([`scrub_and_collect`]), then `dead`. If the object is still alive, a second scrubbed
/// collection from a fresh frame tells the two causes apart, and the test fails with a text
/// that names the cause:
/// - dead after the second collection: "kept by the conservative scan" (a stale stack word
///   kept it one more cycle, `crates/ep9_heap/src/css.rs` module doc: the test's frames, not
///   the engine, are at fault; scrub in the test's own frame before and after the collection,
///   and make targets in Rust or in a frame that has returned);
/// - still alive: "still alive" (a real reference keeps it).
///
/// `dead` must neither keep the object alive nor let it go: no `WeakRef.prototype.deref` (it
/// adds the target to the kept list) and no microtask checkpoint (it clears the kept list).
/// A FinalizationRegistry callback is the usual probe: `dead` runs
/// `Cx::cleanup_finalization_registries` and reads what the callback logged.
pub fn assert_dies(i: &mut Isolate, what: &str, mut dead: impl FnMut(&mut Isolate) -> bool) {
    scrub_and_collect(i);
    let roots = conservative_roots(i);
    if dead(i) {
        return;
    }
    scrub_and_collect(i);
    if dead(i) {
        panic!(
            "{what}: kept by the conservative scan for one more GC ({roots} conservative roots \
             in that GC): a stale stack word, not a reference (plan rule 35)"
        );
    }
    panic!("{what}: still alive after two scrubbed collections: a reference keeps it");
}
