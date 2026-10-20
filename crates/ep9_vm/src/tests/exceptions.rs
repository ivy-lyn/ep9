//! Exceptions across frames, source locations (UTF-16 columns), stack limits, "not
//! implemented", TDZ and termination (gates "exceptions", "stack-limit-rangeerror").

use ep9_bytecode::{
    FeedbackSlotKind, HandlerKind, Label, Register, RegisterList, RuntimeId, ScopeKind,
    VariableMode, VerifiedUnit,
};

use super::{
    Asm, config, isolate, meta, ok, on_big_stack, pos, run_all, run_in, take_output, unit,
    unit_with_scopes,
};
use crate::{Args, Dispatch, HostCx, IsolateConfig, JsResult, Value};

/// `try { body } catch (e) { print(e.message) }` around what `body` emits.
fn try_print_message(s: &mut Asm, ctx: Register, body: impl FnOnce(&mut Asm)) {
    let (start, end, handler, done): (Label, Label, Label, Label) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    let e = s.a.get(2);
    s.b.star(e);
    s.get_named(e, "message");
    s.print_acc();
    s.b.bind(done);
}

/// The source of the location tests: a non-BMP character before `new Error` on line 2, so
/// the UTF-16 column (16) differs from the byte column (18) and the char column (15).
const SRC: &str = "function f() {\n  /*\u{1F600}*/ throw new Error('boom');\n}\n\
function g() { return f(); }\n\
function h() { throw 'x'; }\n\
try { g(); } catch (e) { print(e.message); }\n\
g();\n";

/// f1 f, f2 g, f3 h; the script runs `try { g() } catch`, then `h` in a try, then `g()`
/// uncaught (`uncaught`), or returns.
fn located(uncaught: bool) -> VerifiedUnit {
    let src = SRC;
    let p = |n: &str| pos(src, n);
    let line_end = |from: &str| p(from) + src[p(from) as usize..].find('\n').unwrap() as u32;
    // f: throw new Error('boom')
    let mut f = Asm::new(meta("f", 0, 0, p("function f"), line_end("}")));
    let args = RegisterList::new(f.a.get(1).local_index().unwrap(), 1);
    f.b.set_statement_position(p("throw new"));
    f.lda_str("boom");
    f.b.star(args.get(0));
    f.b.set_expression_position(p("new Error"));
    f.construct_global("Error", args);
    f.b.throw();
    // g: return f()
    let mut g = Asm::new(meta("g", 0, 0, p("function g"), line_end("function g")));
    g.b.set_statement_position(p("return f()"));
    g.lda_global("f");
    g.b.star(g.f);
    g.b.set_expression_position(p("f();"));
    g.b.call_undefined_receiver0(g.f, g.call).return_();
    // h: throw 'x'
    let mut h = Asm::new(meta("h", 0, 0, p("function h"), line_end("function h")));
    h.b.set_statement_position(p("throw 'x'"));
    h.lda_str("x");
    h.b.throw();

    let mut s = Asm::script(src.len());
    let ctx = s.b.new_register();
    for (i, name) in ["f", "g", "h"].iter().enumerate() {
        let k =
            s.b.function_constant(ep9_bytecode::FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    try_print_message(&mut s, ctx, |s| {
        s.b.set_statement_position(p("g();"));
        s.lda_global("g");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    });
    // try { h() } catch (e) { print(e) }: a non-Error value.
    let (start, end, handler, done) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    s.lda_global("h");
    s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    s.print_acc();
    s.b.bind(done);
    if uncaught {
        s.b.set_statement_position(p("g();\n"));
        s.lda_global("g");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    }
    s.lda_str("done");
    s.b.return_();
    unit(src, vec![s.finish(), f.finish(), g.finish(), h.finish()])
}

#[test]
fn caught_across_frames() {
    let r = run_all(|| located(false));
    assert_eq!(ok(&r), "done");
    assert_eq!(r.output, ["boom", "x"]);
}

#[test]
fn uncaught_error_has_utf16_location_and_stack() {
    let r = run_all(|| located(true));
    let e = r.result.clone().unwrap_err();
    assert_eq!(e.message, "Error: boom");
    assert!(!e.terminated);
    let loc = e.location.clone().unwrap();
    assert_eq!(
        (loc.file.as_str(), loc.line, loc.column),
        ("test.js", 2, 16)
    );
    assert_eq!(e.to_string(), "test.js:2:16: Error: boom");
    let stack = e.stack.clone().unwrap();
    assert_eq!(
        stack,
        "Error: boom\n    at f (test.js:2:16)\n    at g (test.js:4:23)\n    at test.js:7:1"
    );
    assert_eq!(r.output, ["boom", "x"]);
}

#[test]
fn uncaught_non_error_reports_the_throw_site() {
    fn make() -> VerifiedUnit {
        let src = "let a = 1;\nthrow 'oops';\n";
        let mut s = Asm::script(src.len());
        s.b.set_statement_position(pos(src, "throw"));
        s.lda_str("oops");
        s.b.throw();
        unit(src, vec![s.finish()])
    }
    let r = run_all(make);
    let e = r.result.unwrap_err();
    assert_eq!(e.message, "oops");
    assert_eq!(e.stack, None);
    assert_eq!(e.to_string(), "test.js:2:1: oops");
}

#[test]
fn rethrow_and_nested_handlers() {
    fn make() -> VerifiedUnit {
        // try { try { throw new TypeError("in") } catch (e) { throw e } } catch (e) { print(e.message) }
        let mut s = Asm::script(0);
        let (ctx, e) = (s.b.new_register(), s.b.new_register());
        let args = RegisterList::new(s.a.get(1).local_index().unwrap(), 1);
        let labels: Vec<Label> = (0..7).map(|_| s.b.new_label()).collect();
        let [o_start, o_end, o_handler, i_start, i_end, i_handler, done] = labels[..] else {
            unreachable!()
        };
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(o_start, o_end, o_handler, ctx, HandlerKind::Caught);
        s.b.add_handler(i_start, i_end, i_handler, ctx, HandlerKind::Caught);
        s.b.bind(o_start);
        s.b.bind(i_start);
        s.lda_str("in");
        s.b.star(args.get(0));
        s.construct_global("TypeError", args);
        s.b.throw();
        s.b.bind(i_end);
        s.b.bind(i_handler);
        s.b.star(e).ldar(e).re_throw();
        s.b.bind(o_end);
        s.b.bind(o_handler);
        s.b.star(e);
        s.get_named(e, "name");
        s.print_acc();
        s.get_named(e, "message");
        s.print_acc();
        s.b.bind(done);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["TypeError", "in"]);
}

/// f1: function rec() { return rec(); }; the script catches the RangeError.
fn deep_recursion() -> VerifiedUnit {
    let mut f = Asm::new(meta("rec", 0, 0, 0, 0));
    f.lda_global("rec");
    f.b.star(f.f)
        .call_undefined_receiver0(f.f, f.call)
        .return_();
    let mut s = Asm::script(0);
    let ctx = s.b.new_register();
    let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("rec");
    try_print_message(&mut s, ctx, |s| {
        s.lda_global("rec");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    });
    // The isolate still works after the overflow.
    s.lda_str("after");
    s.b.return_();
    unit("", vec![s.finish(), f.finish()])
}

#[test]
fn vm_stack_overflow_is_a_catchable_range_error() {
    // Small VM stack: the cap, not the native stack, stops the recursion.
    for &d in Dispatch::ALL {
        let r = on_big_stack(move || {
            run_in(
                IsolateConfig {
                    vm_stack_slots: 4096,
                    ..config(d, false)
                },
                deep_recursion(),
            )
        });
        assert_eq!(ok(&r), "after", "{}", d.name());
        assert_eq!(
            r.output,
            ["Maximum call stack size exceeded"],
            "{}",
            d.name()
        );
    }
    // The default VM stack (1 M slots, about 80 k frames of `rec`) in the default dispatch.
    let r = run_in(config(Dispatch::DEFAULT, false), deep_recursion());
    assert_eq!(ok(&r), "after");
    assert_eq!(r.output, ["Maximum call stack size exceeded"]);
}

fn reenter(cx: &mut HostCx<'_>, _this: Value, args: Args<'_>, _nt: Value) -> JsResult<Value> {
    cx.call(args.get(0), Value::UNDEFINED, &[args.get(0)])
}

#[test]
fn native_stack_overflow_is_a_catchable_range_error() {
    // function r(f) { return reenter(f); } — JS -> host -> JS -> host ... until the native
    // stack limit (ep9_platform bounds) throws.
    fn make() -> VerifiedUnit {
        let mut f = Asm::new(meta("r", 1, 0, 0, 0));
        f.lda_global("reenter");
        f.b.star(f.f);
        f.b.call_undefined_receiver1(f.f, f.b.parameter(0), f.call)
            .return_();
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(s.a.get(1));
        try_print_message(&mut s, ctx, |s| {
            s.lda_global("reenter");
            s.b.star(s.f)
                .call_undefined_receiver1(s.f, s.a.get(1), s.call);
        });
        s.lda_str("after");
        s.b.return_();
        unit("", vec![s.finish(), f.finish()])
    }
    for &d in Dispatch::ALL {
        let r = std::thread::Builder::new()
            .stack_size(if cfg!(debug_assertions) {
                64 << 20
            } else {
                4 << 20
            })
            .spawn(move || {
                let mut iso = isolate(config(d, false));
                iso.register_host_function("reenter", 1, reenter);
                take_output();
                let r = iso.run_script(make(), "native.js").map(|v| iso.display(v));
                (r, take_output())
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(r.0.as_deref(), Ok("after"), "{}", d.name());
        assert_eq!(r.1, ["Maximum call stack size exceeded"], "{}", d.name());
    }
}

#[test]
fn type_errors_and_not_implemented_are_catchable() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, t) = (s.b.new_register(), s.b.new_register());
        let bin = s.bin;
        // undefined()
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_undefined()
                .star(s.f)
                .call_undefined_receiver0(s.f, s.call);
        });
        // undefined.x
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_undefined().star(t);
            s.get_named(t, "x");
        });
        // new print()
        try_print_message(&mut s, ctx, |s| {
            s.construct_global("print", RegisterList::EMPTY);
        });
        // /a/  (RegExp literals are not in v0)
        try_print_message(&mut s, ctx, |s| {
            let k = s.b.string_constant("a");
            let slot = s.b.feedback_slot(FeedbackSlotKind::Literal);
            s.b.create_reg_exp_literal(k, slot, 0);
        });
        // 2 ** 0.5  (needs ep9_math, H36)
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_smi(2).star(t);
            s.lda_num(0.5);
            s.b.exp(t, bin);
        });
        // 1n
        try_print_message(&mut s, ctx, |s| {
            let k =
                s.b.bigint_constant(ep9_bytecode::BigIntConst::new(false, &[1]));
            s.b.lda_constant(k);
        });
        // x is not defined
        try_print_message(&mut s, ctx, |s| {
            s.lda_global("x");
        });
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "undefined is not a function",
            "Cannot read properties of undefined (reading 'x')",
            "function print is not a constructor",
            "not implemented in ep9 v0: RegExp literals",
            "not implemented in ep9 v0: ** with a result that is not an exact integer (needs ep9_math::pow_js, H36)",
            "not implemented in ep9 v0: BigInt values",
            "x is not defined",
        ]
    );
}

#[test]
fn global_lexical_bindings_tdz_and_const() {
    fn make() -> VerifiedUnit {
        // let x; const c = 1;  with DeclareGlobals / InitializeGlobal.
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        let init = s.b.new_register_list(2);
        s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
        // x before its initialization: TDZ
        try_print_message(&mut s, ctx, |s| {
            s.lda_global("x");
        });
        for (name, v) in [("x", 5), ("c", 1)] {
            s.lda_str(name);
            s.b.star(init.get(0)).lda_smi(v).star(init.get(1));
            s.b.call_runtime(RuntimeId::InitializeGlobal, init, 2);
        }
        s.lda_global("x");
        s.print_acc();
        s.b.lda_smi(6);
        s.sta_global("x");
        s.lda_global("x");
        s.print_acc();
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_smi(2);
            s.sta_global("c");
        });
        s.lda_global("c");
        s.print_acc();
        // A lexical binding is not a property of the global object.
        s.lda_global("globalThis");
        s.b.star(s.a.get(1));
        s.get_named(s.a.get(1), "x");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit_with_scopes(
            "",
            vec![(
                ScopeKind::Script,
                None,
                vec![("x", VariableMode::Let), ("c", VariableMode::Const)],
            )],
            vec![s.finish()],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "Cannot access 'x' before initialization",
            "5",
            "6",
            "Assignment to constant variable.",
            "1",
            "undefined",
        ]
    );
}

#[test]
fn terminate_from_another_thread() {
    // for (;;) {} inside try/catch: the catch must not run.
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        try_print_message(&mut s, ctx, |s| {
            let head = s.b.bound_label();
            s.b.lda_zero().jump_loop(head, 0, s.loop_);
        });
        s.lda_str("not reached");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    // Debug builds have no tail calls: an endless loop under the sibling expansion would
    // exhaust the native stack before the 50 ms pass (the debug stack check stops it).
    let dispatches: &[Dispatch] = if cfg!(debug_assertions) {
        &[Dispatch::Match]
    } else {
        Dispatch::ALL
    };
    for &d in dispatches {
        let (r, out, after) = on_big_stack(move || {
            let mut iso = isolate(config(d, false));
            let h = iso.terminate_handle();
            let killer = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                h.terminate();
            });
            take_output();
            let r = iso.run_script(make(), "loop.js");
            killer.join().unwrap();
            let out = take_output();
            // The isolate keeps working.
            let mut s = Asm::script(0);
            s.lda_str("again");
            s.b.return_();
            let after = iso
                .run_script(unit("", vec![s.finish()]), "again.js")
                .map(|v| iso.display(v));
            (r.map(|_| ()), out, after)
        });
        let e = r.unwrap_err();
        assert!(e.terminated, "{}", d.name());
        assert!(out.is_empty(), "catch ran: {out:?}");
        assert_eq!(after.as_deref(), Ok("again"));
    }
}
