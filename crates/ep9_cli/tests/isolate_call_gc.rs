//! `Cx::call` from an embedder (inside `Isolate::with_cx_checkpoint`), under GC stress (a
//! collection before every allocation, then the heap verifier): a call with a primitive
//! `this` for a sloppy callee wraps `this` (OrdinaryCallBindThis step 6.b, H14-Q2), a GC
//! point, and the argument that one call made reaches the next call intact.
//!
//! Until M3-P3 this file tested the raw `Isolate::call` with the argument only in a heap
//! `Vec` (the R2 rule, M2-P5-FIX-R2: every GC point of the call comes after the callee frame
//! copied the arguments). The scoped form keeps every argument in a `Local` on the native
//! stack, so an embedder cannot make that case any more; the R2 rule for the Rust callers of
//! `call::call_with` that remain (the spike-native `call_from_rust`, the in-crate tests) is
//! tested in `ep9_vm` `tests::calls::a_rust_call_copies_the_arguments_before_the_this_wrap`.

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Isolate, IsolateConfig, Local};

/// The `this` of the call under test.
#[derive(Clone, Copy)]
enum This {
    Undefined,
    One,
    True,
}

/// Calls the global `name` with `this` and one argument that `mk()` made; the result as text,
/// or "Err: <message>".
fn call_with_a_new_arg(name: &'static str, this: This) -> String {
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
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "r2.js").map(|_| ()))
            .expect("runs");
        let r = iso
            .with_cx_checkpoint(|cx| {
                let mk = cx.global_binding("mk")?.expect("mk");
                let o = cx.call(mk, Local::undefined(), &[])?;
                let f = cx.global_binding(name)?.expect("callee");
                let this = match this {
                    This::Undefined => Local::undefined(),
                    This::One => Local::int32(1),
                    This::True => Local::boolean(true),
                };
                let v = cx.call(f, this, &[o])?;
                cx.to_rust_string(v)
            })
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
/// the strict `g.call(1, ...)` gives "number:7:xxxy".
#[test]
fn cx_call_wraps_a_primitive_this_under_gc_stress() {
    assert_eq!(call_with_a_new_arg("f", This::Undefined), "object:7:xxxy");
    assert_eq!(call_with_a_new_arg("f", This::One), "object:7:xxxy");
    assert_eq!(call_with_a_new_arg("f", This::True), "object:7:xxxy");
    assert_eq!(call_with_a_new_arg("g", This::One), "number:7:xxxy");
    assert_eq!(call_with_a_new_arg("bf", This::Undefined), "object:7:xxxy");
}
