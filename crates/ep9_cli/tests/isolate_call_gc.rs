//! `Isolate::call` from an embedder, under GC stress (a collection before every allocation,
//! then the heap verifier): the arguments of the call live only in the embedder's heap `Vec`
//! and in the heap copy that `Isolate::call` makes, which the GC does not scan. So every GC
//! point of the call must come after the callee frame has copied them (P5 fixer, R2). The wrap
//! of a primitive `this` for a sloppy callee (OrdinaryCallBindThis step 6.b, H14-Q2) is such a
//! GC point: `call::call_function_object` pushes the frame first, then wraps.
//!
//! This lives in `ep9_cli` (an embedder) because the test holds raw `ep9_vm::Value`s, which
//! the `ep9_builtins` clippy bans forbid there.

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Isolate, IsolateConfig, Value};

/// Makes one argument with one API call and puts it straight into a heap `Vec`, so the only
/// copy of the Value is memory the GC does not scan. `inline(never)`: its frame is gone
/// before the call under test.
#[inline(never)]
fn arg_in_a_vec(iso: &mut Isolate) -> Vec<Value> {
    let mk = iso.global("mk").expect("mk");
    vec![iso.call(mk, Value::UNDEFINED, &[]).expect("mk runs")]
}

/// Plain Rust with no isolate call: zeroes a part of the native stack below the caller, so no
/// old copy of a Value stays there for the conservative scan to find.
#[inline(never)]
fn clear_stack() {
    let mut a = [1u64; 16384];
    for x in a.iter_mut() {
        // SAFETY: a valid, aligned element of the local array.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
    std::hint::black_box(&a);
}

/// Calls the global `name` with `this` and one argument held only in a `Vec`; the result as
/// text, or "Err: <message>".
fn call_with_an_arg_in_a_vec(name: &'static str, this: Value) -> String {
    let run = move || {
        let src = "
            globalThis.mk = function () { return { a: 7, b: 'x'.repeat(3) + 'y' }; };
            globalThis.f = function (o) { return typeof this + ':' + o.a + ':' + o.b; };
            globalThis.g = function (o) { 'use strict'; return typeof this + ':' + o.a + ':' + o.b; };
            globalThis.bf = f.bind(1);
        ";
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("compiles");
        let mut iso = Isolate::new(IsolateConfig {
            heap: HeapConfig {
                gc_stress: 1,
                verify_after_gc: true,
                ..HeapConfig::default()
            },
            builtins: Some(&ep9_builtins::TABLE),
            ..IsolateConfig::default()
        })
        .expect("an isolate");
        iso.run_script(unit, "r2.js").expect("runs");
        let f = iso.global(name).expect("callee");
        let args = arg_in_a_vec(&mut iso);
        clear_stack();
        let r = iso
            .call(f, this, &args)
            .map(|v| iso.display(v))
            .map_err(|e| e.message);
        iso.verify_heap().expect("a valid heap");
        match r {
            Ok(s) => s,
            Err(m) => format!("Err: {m}"),
        }
    };
    // Debug builds have no tail calls: run on a big stack, as the other e2e tests do.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(run)
        .expect("a thread")
        .join()
        .expect("no panic")
}

/// node v24.22.1: `f.call(1, {a: 7, b: 'xxxy'})` and `f.bind(1)(...)` give "object:7:xxxy";
/// the strict `g.call(1, ...)` gives "number:7:xxxy". Before the fix, the sloppy calls with a
/// primitive `this` gave "TypeError: Cannot read properties of object (reading 'a')": the
/// argument was freed at the wrap.
#[test]
fn isolate_call_wraps_this_after_the_arguments_are_copied() {
    assert_eq!(
        call_with_an_arg_in_a_vec("f", Value::UNDEFINED),
        "object:7:xxxy"
    );
    assert_eq!(
        call_with_an_arg_in_a_vec("f", Value::int32(1)),
        "object:7:xxxy"
    );
    assert_eq!(call_with_an_arg_in_a_vec("f", Value::TRUE), "object:7:xxxy");
    assert_eq!(
        call_with_an_arg_in_a_vec("g", Value::int32(1)),
        "number:7:xxxy"
    );
    assert_eq!(
        call_with_an_arg_in_a_vec("bf", Value::UNDEFINED),
        "object:7:xxxy"
    );
}
