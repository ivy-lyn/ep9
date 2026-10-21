//! GC stress (gate "gc-stress-interp"): `run_all` runs every program with a collection before
//! every allocation plus the heap verifier after each one. These tests add programs that
//! keep many fresh cells only in registers, contexts and Rust locals.

use ep9_bytecode::{FunctionId, ScopeId, ScopeKind, VariableMode, VerifiedUnit};

use super::{Asm, config, isolate, meta, ok, on_big_stack, run_all, take_output, unit_with_scopes};
use crate::{Args, Dispatch, HostCx, JsResult, Value};

fn gc(cx: &mut HostCx<'_>, _this: Value, _args: Args<'_>, _nt: Value) -> JsResult<Value> {
    // A value that lives only in this Rust local across the collection (conservative scan).
    let s = cx.new_string("kept by the stack scan");
    cx.collect_garbage();
    let t = cx.to_rust_string(s)?;
    assert_eq!(t, "kept by the stack scan");
    Ok(Value::UNDEFINED)
}

#[test]
fn allocation_loop_under_stress() {
    fn make() -> VerifiedUnit {
        // var s = ""; var a = []; for (i = 0; i < 300; i++) { s = s + "x"; a.push({i: i}); }
        // print(s.length, a.length, a[299].i)
        let mut s = Asm::script(0);
        let (str_, arr, i, o) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        s.lda_str("");
        s.b.star(str_)
            .create_empty_array_literal(s.lit)
            .star(arr)
            .lda_zero()
            .star(i);
        let head = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(300)
            .test_less_than(i, s.cmp)
            .jump_if_false(exit);
        s.lda_str("x");
        s.b.add(str_, s.bin).star(str_);
        s.b.create_empty_object_literal().star(o).ldar(i);
        s.set_named(o, "i");
        s.get_named(arr, "push");
        s.b.star(s.f).call_property1(s.f, arr, o, s.call);
        s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
        s.b.bind(exit);
        s.get_named(str_, "length");
        s.print_acc();
        s.get_named(arr, "length");
        s.print_acc();
        s.b.lda_smi(299).get_keyed_property(arr, s.keyed).star(o);
        s.get_named(o, "i");
        s.print_acc();
        s.b.ldar(str_).return_();
        unit_with_scopes("", vec![], vec![s.finish()])
    }
    let r = run_all(make);
    assert_eq!(ok(&r).len(), 300);
    assert_eq!(r.output, ["300", "300", "299"]);
    // Stress really collects: at least one collection per string and object of the loop.
    for stress in [false, true] {
        let n = on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            iso.run_script(make(), "stress.js").unwrap();
            take_output();
            iso.heap().stats().collections
        });
        eprintln!("collections (stress={stress}): {n}");
        if stress {
            assert!(n >= 600, "{n} collections under stress");
        } else {
            assert!(n < 10, "{n} collections without stress");
        }
    }
}

#[test]
fn collection_in_the_middle_of_js() {
    // Fresh cells in registers, a context and the accumulator survive a GC that a host
    // function runs while JS frames are on the VM stack.
    let make = || {
        // f1: function keep() { let v = "a" + 1; return function () { return v; }; }
        let mut keep = Asm::new(meta("keep", 0, 0, 0, 0));
        let ctx = keep.b.new_register();
        let ks = keep.b.scope_constant(ScopeId(0));
        keep.b.create_function_context(ks, 1).push_context(ctx);
        keep.lda_str("a");
        keep.b
            .star(keep.f)
            .lda_smi(1)
            .add(keep.f, keep.bin)
            .sta_current_context_slot(0);
        let kf = keep.b.function_constant(FunctionId(2));
        keep.b.create_closure(kf, keep.closure, 0).return_();
        // f2: the closure
        let mut get = Asm::new(meta("", 0, 1, 0, 0));
        get.b.lda_current_context_slot(0).return_();

        let mut s = Asm::script(0);
        let (o, c, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0)
            .star(s.f)
            .call_undefined_receiver0(s.f, s.call);
        s.b.star(c);
        s.b.create_empty_object_literal().star(o);
        s.lda_str("p");
        s.b.star(t).lda_smi(2).add(t, s.bin);
        s.set_named(o, "p");
        s.lda_global("gc");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.get_named(o, "p");
        s.print_acc();
        s.b.call_undefined_receiver0(c, s.call);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit_with_scopes(
            "",
            vec![(ScopeKind::Function, None, vec![("v", VariableMode::Let)])],
            vec![s.finish(), keep.finish(), get.finish()],
        )
    };
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let out = on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                iso.register_host_function("gc", 0, gc);
                take_output();
                let r = iso.run_script(make(), "gc.js");
                assert!(r.is_ok(), "{r:?}");
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                take_output()
            });
            assert_eq!(out, ["p2", "a1"], "{} stress={stress}", d.name());
        }
    }
}

/// Slots that `fill_scratch` and `scratch_values` use above the host frame.
const SCRATCH: usize = 64;

/// Writes `SCRATCH` fresh strings into the VM stack from `top` up, raising the scratch top
/// past each one. Every allocation is a GC point (and, under GC stress, a collection), so the
/// earlier strings live only in those slots. Out of line: no Rust local of the caller holds
/// them afterwards.
#[inline(never)]
fn fill_scratch(rt: &crate::runtime::Runtime, top: *mut u64) {
    for i in 0..SCRATCH {
        let s = crate::string::new_string(rt, &format!("scratch {i}"));
        // SAFETY: `top + i` lies above the live host frame and far below the end of the
        // 2^20-slot VM stack; no frame uses it while the host function runs.
        unsafe { *top.add(i) = s };
        rt.stack().set_scratch_top(top.wrapping_add(i + 1));
    }
}

/// `scratchValues()`: keeps fresh strings only between the host frame's top and the scratch
/// top across many collections, then checks every one.
fn scratch_values(
    cx: &mut HostCx<'_>,
    _this: Value,
    _args: Args<'_>,
    _nt: Value,
) -> JsResult<Value> {
    let rt = cx.cx.rt();
    let top = crate::runtime::frame_top(rt.stack().fp());
    fill_scratch(rt, top);
    for _ in 0..16 {
        cx.new_string("an allocation, so a collection under stress");
    }
    cx.collect_garbage();
    for i in 0..SCRATCH {
        // SAFETY: `fill_scratch` wrote slot `top + i` with a string Value.
        let s = unsafe { *top.add(i) };
        assert!(crate::kinds::is_string(s), "slot {i} lost its string");
        let text = crate::string::str_view(s).to_string_lossy();
        assert_eq!(text, format!("scratch {i}"));
    }
    Ok(Value::UNDEFINED)
}

#[test]
fn values_below_the_scratch_top_survive_gc_stress() {
    // A host frame's top is where Rust code builds call arguments (H20 ArgsBuilder): the GC
    // scans up to the scratch top, and a host call clears it again on return.
    for stress in [false, true] {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        iso.register_host_function("scratchValues", 0, scratch_values);
        let f = iso.global("scratchValues").unwrap();
        let before = iso.heap().stats().collections;
        let r = iso.call(f, Value::UNDEFINED, &[]);
        assert!(r.is_ok(), "{r:?}");
        let collections = iso.heap().stats().collections - before;
        if stress {
            assert!(
                collections > SCRATCH as u64,
                "{collections} collections under stress"
            );
        }
        assert!(
            iso.rt.stack().scratch_top().is_null(),
            "the host return clears it"
        );
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    }
}
