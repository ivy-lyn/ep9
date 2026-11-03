//! Tests of eval and the `Function` constructor in the VM (P3-EVAL-VM): the compiler hook
//! contract with hand-built units (ep9_vm has no edge to the front end, not even as a
//! dev-dependency), runtime ids 7 and 27-28, the failure mapping (D7) and the isolate with no
//! hook (O2), and the host hook HostEnsureCanCompileStrings (`host.rs`, M6-P2 REALM-CLOSE).
//! Every case runs under every dispatch expansion, with and without GC stress. The
//! JS-level tests (the real front end: direct vs indirect, sloppy vs strict, `.eval_ext`,
//! `with`) are in `crates/ep9_builtins/tests/eval.rs`.

use ep9_bytecode::{
    CodeKind, EvalContext, FunctionCtorKind, FunctionId, FunctionMeta, HandlerKind, RegisterList,
    RuntimeId, ScopeKind, UnitBuilder, UnitOptions, VerifiedUnit,
};

use super::{Asm, Run, config, meta, on_big_stack, opts, run_in, unit_with_scopes};
use crate::{CompileFailure, CompileResult, CompilerHooks, Dispatch, IsolateConfig};

/// Options of a hand-built eval unit.
fn eval_opts() -> UnitOptions {
    UnitOptions {
        code_kind: CodeKind::Eval,
        ..opts()
    }
}

/// An eval unit of the functions `fns` (function 0 first).
fn eval_unit(fns: Vec<ep9_bytecode::FunctionCode>) -> VerifiedUnit {
    let mut u = UnitBuilder::new("", eval_opts());
    for (i, f) in fns.into_iter().enumerate() {
        u.set_function(FunctionId(i as u32), f);
    }
    u.finish().unwrap()
}

/// Function 0 of an eval unit.
fn eval_entry() -> Asm {
    Asm::new(FunctionMeta::entry(CodeKind::Eval, false, 0))
}

/// `return 42`.
fn unit_42() -> VerifiedUnit {
    let mut s = eval_entry();
    s.b.lda_smi(42).return_();
    eval_unit(vec![s.finish()])
}

/// Sloppy eval code `var v; function fn() { return 9 }`: DeclareEvalFunction("fn", closure),
/// DeclareEvalVar("v"), then `return 1`.
fn unit_decl() -> VerifiedUnit {
    let mut s = eval_entry();
    let args = s.b.new_register_list(2);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.b.star(args.get(1));
    s.lda_str("fn");
    s.b.star(args.get(0));
    s.b.call_runtime(RuntimeId::DeclareEvalFunction, args, 2);
    s.lda_str("v");
    s.b.star(args.get(0));
    s.b.call_runtime(
        RuntimeId::DeclareEvalVar,
        RegisterList::new(args.first().local_index().unwrap(), 1),
        1,
    );
    s.b.lda_smi(1).return_();
    let mut f = Asm::new(meta("fn", 0, 0, 0, 0));
    f.b.lda_smi(9).return_();
    eval_unit(vec![s.finish(), f.finish()])
}

/// The number of `var` names of [`unit_decl_many`].
const MANY_VARS: usize = 8;

/// Sloppy eval code `function fn() { return 9 } var v0, .., v7`: DeclareEvalFunction("fn",
/// closure), then DeclareEvalVar per var name, then `return 1`.
fn unit_decl_many() -> VerifiedUnit {
    let mut s = eval_entry();
    let args = s.b.new_register_list(2);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.b.star(args.get(1));
    s.lda_str("fn");
    s.b.star(args.get(0));
    s.b.call_runtime(RuntimeId::DeclareEvalFunction, args, 2);
    let one = RegisterList::new(args.first().local_index().unwrap(), 1);
    for i in 0..MANY_VARS {
        s.lda_str(&format!("v{i}"));
        s.b.star(args.get(0));
        s.b.call_runtime(RuntimeId::DeclareEvalVar, one, 1);
    }
    s.b.lda_smi(1).return_();
    let mut f = Asm::new(meta("fn", 0, 0, 0, 0));
    f.b.lda_smi(9).return_();
    eval_unit(vec![s.finish(), f.finish()])
}

/// The `Function("a", "return a")` unit of the FNCTOR contract (D8): function 0 returns a
/// closure of function 1, which returns its argument.
fn unit_identity_function() -> VerifiedUnit {
    let mut s = eval_entry();
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.b.return_();
    let mut f = Asm::new(meta("anonymous", 1, 0, 0, 0));
    let a = f.b.parameter(0);
    f.b.ldar(a).return_();
    eval_unit(vec![s.finish(), f.finish()])
}

/// The test hook: a source text names the unit or the failure it gives.
fn hook_eval(source: &str, ctx: &EvalContext) -> CompileResult {
    match source {
        "unit42" => Ok(unit_42()),
        "decl" => Ok(unit_decl()),
        "decl_many" => Ok(unit_decl_many()),
        // Direct eval at a sloppy script's top level: only `direct` is set.
        "direct?" if ctx.direct && !ctx.strict && !ctx.in_function => Ok(unit_42()),
        "syntax" => Err(CompileFailure::Syntax("bad token".into())),
        "range" => Err(CompileFailure::Range("too deep".into())),
        "unsupported" => Err(CompileFailure::Unsupported("a thing".into())),
        "internal" => Err(CompileFailure::Internal("verifier".into())),
        _ => Err(CompileFailure::Syntax(format!(
            "unknown test source {source:?}"
        ))),
    }
}

fn hook_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    match (kind, params, body) {
        (FunctionCtorKind::Normal, "a", "return a") => Ok(unit_identity_function()),
        (FunctionCtorKind::Normal, "a,b", "") => Err(CompileFailure::Syntax("params a,b".into())),
        _ => Err(CompileFailure::Unsupported(format!(
            "{kind:?} {params:?} {body:?}"
        ))),
    }
}

fn hook_script(_source: &str) -> CompileResult {
    Err(CompileFailure::Unsupported("scripts".into()))
}

static HOOKS: CompilerHooks = CompilerHooks {
    compile_eval: hook_eval,
    compile_function: hook_function,
    compile_script: hook_script,
};

/// Runs the unit from `make` under every dispatch expansion, with and without GC stress, on
/// isolates with `hooks`; all runs must agree.
fn run_every(make: fn() -> VerifiedUnit, hooks: Option<&'static CompilerHooks>) -> Run {
    let mut first: Option<(String, Run)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = on_big_stack(move || {
                let c = IsolateConfig {
                    compiler: hooks,
                    ..config(d, stress)
                };
                run_in(c, make())
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// `print(eval(<acc>))` through the global `eval` (an indirect call), inside
/// `try { ... } catch (e) { print(e) }`.
fn print_indirect(s: &mut Asm, source: &str) {
    s.lda_str(source);
    let a0 = s.a.get(0);
    s.b.star(a0);
    s.lda_global("eval");
    let f = s.f;
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.print_acc();
}

/// `print(eval(source))` as a direct eval: `ResolvePossiblyDirectEval(%eval%, source,
/// <current function>, scope 0, 0)`, then a call of its result with the source.
fn print_direct(s: &mut Asm, source: &str) {
    let r = s.b.new_register_list(5);
    let callee = s.b.new_register();
    s.lda_global("eval");
    s.b.star(r.get(0));
    s.lda_str(source);
    s.b.star(r.get(1));
    s.b.lda_current_function().star(r.get(2));
    s.b.lda_smi(0).star(r.get(3));
    s.b.lda_smi(0).star(r.get(4));
    s.b.call_runtime(RuntimeId::ResolvePossiblyDirectEval, r, 5);
    s.b.star(callee);
    s.b.call_undefined_receiver1(callee, r.get(1), s.call);
    s.print_acc();
}

/// Wraps `body` in `try { body } catch (e) { print(e) }` (ToString of the error: "Name:
/// message").
fn try_print(s: &mut Asm, body: impl FnOnce(&mut Asm)) {
    let (start, end, handler, done) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    let ctx = s.b.new_register();
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    s.print_acc();
    s.b.bind(done);
}

/// A script with a script scope 0 (the direct eval's scope operand).
fn script_unit(s: Asm) -> VerifiedUnit {
    unit_with_scopes(
        "",
        vec![(ScopeKind::Script, None, vec![])],
        vec![s.finish()],
    )
}

/// O2: with no compiler hook, `eval(string)` (indirect and direct) and `Function(...)` throw
/// an EvalError; `eval` of a value that is not a string returns it without the hook.
#[test]
fn no_compiler_hook_is_an_eval_error() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        // eval(5) -> 5 (PerformEval step 2 comes before HostEnsureCanCompileStrings).
        s.b.lda_smi(5).star(s.a.get(0));
        s.lda_global("eval");
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver1(f, s.a.get(0), s.call);
        s.print_acc();
        try_print(&mut s, |s| print_indirect(s, "unit42"));
        try_print(&mut s, |s| print_direct(s, "unit42"));
        try_print(&mut s, |s| {
            let (f, args) = (s.f, s.a);
            s.lda_global("Function");
            s.b.star(f);
            s.b.call_undefined_receiver0(f, s.call);
            let _ = args;
            s.print_acc();
        });
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, None);
    let e = "EvalError: code generation from strings is not available (no compiler hook)";
    assert_eq!(r.output, ["5", e, e, e]);
}

/// D7: Syntax -> SyntaxError, Range -> RangeError, Unsupported -> the v0 not-implemented
/// Error "unsupported: <f>" (never a SyntaxError), Internal -> an internal error.
#[test]
fn compile_failures_map_to_errors() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        for src in ["syntax", "range", "unsupported", "internal"] {
            try_print(&mut s, |s| print_indirect(s, src));
        }
        try_print(&mut s, |s| print_direct(s, "unsupported"));
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, Some(&HOOKS));
    assert_eq!(
        r.output,
        [
            "SyntaxError: bad token",
            "RangeError: too deep",
            "Error: not implemented in ep9 v0: unsupported: a thing",
            "Error: ep9 internal error: verifier",
            "Error: not implemented in ep9 v0: unsupported: a thing",
        ]
    );
}

/// Indirect and direct eval run the hook's unit (function 0's return value is the result);
/// the direct one gets an EvalContext with `direct` set; `eval(5)` is 5 and `eval()` is
/// undefined.
#[test]
fn eval_runs_the_hook_unit() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        print_indirect(&mut s, "unit42");
        print_direct(&mut s, "direct?");
        // eval() with no argument: undefined.
        s.lda_global("eval");
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver0(f, s.call);
        s.print_acc();
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, Some(&HOOKS));
    assert_eq!(r.output, ["42", "42", "undefined"]);
}

/// Ids 27-28 at the global level (indirect eval code): CreateGlobalFunctionBinding and
/// CreateGlobalVarBinding with D = true, so both are configurable (`delete` gives true).
#[test]
fn declare_eval_ids_make_configurable_globals() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let g = s.b.new_register();
        print_indirect(&mut s, "decl");
        // fn() -> 9, v -> undefined
        s.lda_global("fn");
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver0(f, s.call);
        s.print_acc();
        s.lda_global("v");
        s.print_acc();
        // delete globalThis.v, delete globalThis.fn
        for name in ["v", "fn"] {
            s.lda_global("globalThis");
            s.b.star(g);
            s.lda_str(name);
            s.b.delete_property_sloppy(g);
            s.print_acc();
        }
        // A second run of the same code finds no binding to keep and makes them again.
        print_indirect(&mut s, "decl");
        s.lda_global("v");
        s.print_acc();
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, Some(&HOOKS));
    assert_eq!(
        r.output,
        ["1", "9", "undefined", "true", "true", "1", "undefined"]
    );
}

/// A run of sloppy eval code decodes its prologue once, however many names it declares: the
/// first DeclareEval* call of the run checks every name (EvalDeclarationInstantiation), the
/// later calls find that they are not the first without decoding function 0 again (P3 review:
/// one decode per call made a run with D names cost D times the function length). Two runs,
/// each a new unit: two decodes.
#[test]
fn eval_prologue_is_decoded_once_per_run() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        print_indirect(&mut s, "decl_many");
        print_indirect(&mut s, "decl_many");
        s.lda_global("fn");
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver0(f, s.call);
        s.print_acc();
        s.lda_global("v7");
        s.print_acc();
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, decodes) = on_big_stack(move || {
                crate::interp::slow::PROLOGUE_DECODES.with(|n| n.set(0));
                let c = IsolateConfig {
                    compiler: Some(&HOOKS),
                    ..config(d, stress)
                };
                let r = run_in(c, make());
                (r, crate::interp::slow::PROLOGUE_DECODES.with(|n| n.get()))
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.output, ["1", "1", "9", "undefined"], "{label}");
            assert_eq!(decodes, 2, "{label}: 2 runs of {} names", MANY_VARS + 1);
        }
    }
}

/// The Function constructor runs function 0 of the hook's unit (the FNCTOR contract, D8) and
/// returns the closure it gives: `Function("a", "return a")(7)` and `new Function(...)`. The
/// parameters are joined with ","; the body is the last argument ("" when absent).
#[test]
fn function_constructor_runs_function_zero() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let args = s.b.new_register_list(2);
        let g = s.b.new_register();
        let f = s.f;
        s.lda_str("a");
        s.b.star(args.get(0));
        s.lda_str("return a");
        s.b.star(args.get(1));
        s.lda_global("Function");
        s.b.star(f);
        s.b.call_undefined_receiver2(f, args.get(0), args.get(1), s.call);
        s.b.star(g);
        s.b.lda_smi(7).star(args.get(0));
        s.b.call_undefined_receiver1(g, args.get(0), s.call);
        s.print_acc();
        // new Function("a", "return a")(8)
        s.lda_str("a");
        s.b.star(args.get(0));
        s.lda_global("Function");
        s.b.star(f);
        s.b.construct(f, args, 2, s.call);
        s.b.star(g);
        s.b.lda_smi(8).star(args.get(0));
        s.b.call_undefined_receiver1(g, args.get(0), s.call);
        s.print_acc();
        // Function("a", "b", ""): params "a,b", body "".
        try_print(&mut s, |s| {
            let f = s.f;
            s.lda_str("a");
            s.b.star(s.a.get(0));
            s.lda_str("b");
            s.b.star(s.a.get(1));
            s.lda_str("");
            s.b.star(s.a.get(2));
            s.lda_global("Function");
            s.b.star(f);
            s.b.call_undefined_receiver(f, s.a, 3, s.call);
            s.print_acc();
        });
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, Some(&HOOKS));
    assert_eq!(r.output, ["7", "8", "SyntaxError: params a,b"]);
}

/// ResolvePossiblyDirectEval gives the callee back when it is not %eval% or the source is
/// not a string (the call then goes on as written).
#[test]
fn resolve_gives_the_callee_back() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let r = s.b.new_register_list(5);
        for (callee, src) in [("print", true), ("eval", false)] {
            s.lda_global(callee);
            s.b.star(r.get(0));
            if src {
                s.lda_str("unit42");
            } else {
                s.b.lda_smi(3);
            }
            s.b.star(r.get(1));
            s.b.lda_current_function().star(r.get(2));
            s.b.lda_smi(0).star(r.get(3));
            s.b.lda_smi(0).star(r.get(4));
            s.b.call_runtime(RuntimeId::ResolvePossiblyDirectEval, r, 5);
            s.b.test_equal_strict(r.get(0), s.cmp);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let r = run_every(make, Some(&HOOKS));
    assert_eq!(r.output, ["true", "true"]);
}

// ---- HostEnsureCanCompileStrings (§19.2.1.2; M6-P2 REALM-CLOSE, V9-A12) ------------------------

thread_local! {
    /// The calls of [`record`] on this thread: "<parameters> <body> direct=<d> realm=<tag>".
    static SEEN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn take_seen() -> Vec<String> {
    SEEN.with_borrow_mut(std::mem::take)
}

/// The test HostEnsureCanCompileStrings hook: it reads the global `realmTag` of the realm it
/// runs in (ABI code in the hook, then a collection), records the call, and refuses three
/// bodies: "deny" with an EvalError, "deny-silent" with an `Err` and no pending exception,
/// "throw-value" by throwing 7.
fn record(
    cx: &mut crate::Cx<'_>,
    parameters: &[String],
    body: &str,
    direct: bool,
) -> crate::JsResult<()> {
    let g = cx.global();
    let tag = cx.get(g, "realmTag")?;
    let tag = cx.display(tag);
    cx.collect_garbage();
    SEEN.with_borrow_mut(|s| {
        s.push(format!(
            "{parameters:?} {body:?} direct={direct} realm={tag}"
        ))
    });
    match body {
        "deny" => Err(cx.throw_error(crate::ErrorKind::EvalError, "denied")),
        "deny-silent" => Err(ep9_value::Throw),
        "throw-value" => Err(cx.throw(crate::Local::int32(7))),
        _ => Ok(()),
    }
}

/// A hook that refuses every text.
fn deny_all(
    cx: &mut crate::Cx<'_>,
    _parameters: &[String],
    body: &str,
    _direct: bool,
) -> crate::JsResult<()> {
    Err(cx.throw_error(crate::ErrorKind::EvalError, &format!("no: {body}")))
}

/// `print(Function(p1, .., pn, body))` inside try/catch: the call's result (a function) or
/// the error.
fn print_function(s: &mut Asm, strings: &[&str]) {
    try_print(s, |s| {
        let f = s.f;
        for (i, t) in strings.iter().enumerate() {
            s.lda_str(t);
            s.b.star(s.a.get(i as u32));
        }
        s.lda_global("Function");
        s.b.star(f);
        let n = strings.len() as u32;
        let list = match n {
            0 => RegisterList::EMPTY,
            _ => RegisterList::new(s.a.first().local_index().unwrap(), n),
        };
        s.b.call_undefined_receiver(f, list, n, s.call);
        s.b.lda_smi(1);
        s.print_acc();
    });
}

/// Runs the unit from `make` under every dispatch, with and without GC stress, on an isolate
/// with [`HOOKS`], the global `realmTag` = "one" and the compile-strings hook `hook`; all runs
/// must agree. Returns the run and the [`record`] calls.
fn run_hooked(
    make: fn() -> VerifiedUnit,
    hook: Option<crate::host::CompileStringsHook>,
) -> (Run, Vec<String>) {
    let mut first: Option<(String, (Run, Vec<String>))> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = on_big_stack(move || {
                let mut iso = super::isolate(IsolateConfig {
                    compiler: Some(&HOOKS),
                    ..config(d, stress)
                });
                iso.set_compile_strings_hook(hook);
                iso.with_cx(|cx| {
                    let (g, one) = (cx.global(), cx.new_string("one")?);
                    cx.set(g, "realmTag", one, true)
                })
                .unwrap();
                super::take_output();
                take_seen();
                let result = iso.with_cx_checkpoint(|cx| {
                    cx.run_script(make(), "test.js").map(|v| cx.display(v))
                });
                let output = super::take_output();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (Run { result, output }, take_seen())
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// The hook runs at the one compile point of code from strings, with the spec's arguments:
/// indirect eval (« », x, false), direct eval (« », x, true) and the Function constructor
/// (the ToString of each parameter, the body, false); `eval(5)` asks nothing. It runs before
/// the compiler hook: a refused text never reaches it ([`hook_eval`] would give a
/// SyntaxError for "deny"). A refusal is the hook's throw (an EvalError, or any value); an
/// `Err` with no pending exception is the VM's EvalError.
#[test]
fn the_compile_strings_hook_runs_before_each_compile_and_can_refuse() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        print_indirect(&mut s, "unit42");
        print_direct(&mut s, "direct?");
        s.b.lda_smi(5).star(s.a.get(0));
        s.lda_global("eval");
        let f = s.f;
        s.b.star(f);
        s.b.call_undefined_receiver1(f, s.a.get(0), s.call);
        s.print_acc();
        print_function(&mut s, &["a", "return a"]);
        for src in ["deny", "deny-silent", "throw-value"] {
            try_print(&mut s, |s| print_indirect(s, src));
        }
        try_print(&mut s, |s| print_direct(s, "deny"));
        print_function(&mut s, &["a", "b", "deny"]);
        print_function(&mut s, &[]);
        s.b.lda_undefined().return_();
        script_unit(s)
    }
    let (r, seen) = run_hooked(make, Some(record));
    assert_eq!(r.result.as_deref(), Ok("undefined"));
    let silent = "EvalError: code generation from strings disallowed for this realm";
    assert_eq!(
        r.output,
        [
            "42",
            "42",
            "5",
            "1",
            "EvalError: denied",
            silent,
            "7",
            "EvalError: denied",
            "EvalError: denied",
            "Error: not implemented in ep9 v0: unsupported: Normal \"\" \"\"",
        ]
    );
    assert_eq!(
        seen,
        [
            "[] \"unit42\" direct=false realm=one",
            "[] \"direct?\" direct=true realm=one",
            "[\"a\"] \"return a\" direct=false realm=one",
            "[] \"deny\" direct=false realm=one",
            "[] \"deny-silent\" direct=false realm=one",
            "[] \"throw-value\" direct=false realm=one",
            "[] \"deny\" direct=true realm=one",
            "[\"a\", \"b\"] \"deny\" direct=false realm=one",
            "[] \"\" direct=false realm=one",
        ]
    );
    // With no hook, the same code compiles each text (the default host allows).
    let (r, seen) = run_hooked(make, None);
    assert_eq!(&r.output[..4], ["42", "42", "5", "1"]);
    assert_eq!(r.output[4], "SyntaxError: unknown test source \"deny\"");
    assert!(seen.is_empty());
}

/// The hook runs in the callee realm (§19.2.1.2 calleeRealm): `%eval%` and `Function` of a
/// second realm, called from the first, give the hook the second realm. `Cx::evaluate_script`
/// (a host's own script) asks no hook, and neither does `eval` of a value that is not a
/// string.
#[test]
fn the_compile_strings_hook_runs_in_the_callee_realm() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, seen) = on_big_stack(move || {
                let mut iso = super::isolate(IsolateConfig {
                    compiler: Some(&HOOKS),
                    ..config(d, stress)
                });
                iso.set_compile_strings_hook(Some(record));
                take_seen();
                let r = iso.with_cx(|cx| {
                    let (g1, one) = (cx.global(), cx.new_string("one")?);
                    cx.set(g1, "realmTag", one, true)?;
                    let g2 = cx.create_realm()?;
                    let two = cx.new_string("two")?;
                    cx.set(g2, "realmTag", two, true)?;
                    let (eval1, eval2) = (cx.get(g1, "eval")?, cx.get(g2, "eval")?);
                    let function2 = cx.get(g2, "Function")?;
                    let text = cx.new_string("unit42")?;
                    let a = cx.call(eval2, g1, &[text.into()])?;
                    let b = cx.call(eval1, g1, &[text.into()])?;
                    let (p, body) = (cx.new_string("a")?, cx.new_string("return a")?);
                    let id = cx.call(function2, g1, &[p.into(), body.into()])?;
                    let c = cx.call(id, g1, &[crate::Local::int32(3)])?;
                    let script = cx.evaluate_script("unit42", "host.js").map(|_| ());
                    let script = script.is_err() && cx.inner().rt().take_pending() != 0;
                    let five = cx.call(eval2, g1, &[crate::Local::int32(5)])?;
                    cx.collect_garbage();
                    Ok([a, b, c, five].map(|v| cx.display(v)).join(" ") + &format!(" {script}"))
                });
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (r, take_seen())
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.as_deref(), Ok("42 42 3 5 true"), "{label}");
            assert_eq!(
                seen,
                [
                    "[] \"unit42\" direct=false realm=two",
                    "[] \"unit42\" direct=false realm=one",
                    "[\"a\"] \"return a\" direct=false realm=two",
                ],
                "{label}"
            );
        }
    }
}

/// The hook is per isolate (H22-Q6: several isolates on one thread, one after another and
/// at the same time): an isolate without a hook compiles what another isolate's hook refuses;
/// `None` removes a hook; the table drops the entry of a dropped isolate at the next set.
#[test]
fn the_compile_strings_hook_is_per_isolate() {
    fn eval_text(iso: &mut crate::Isolate, text: &str) -> Result<String, String> {
        iso.with_cx(|cx| {
            let g = cx.global();
            let eval = cx.get(g, "eval")?;
            let t = cx.new_string(text)?;
            let v = cx.call(eval, g, &[t.into()])?;
            Ok(cx.display(v))
        })
        .map_err(|e| e.message)
    }
    let with_hooks = || IsolateConfig {
        compiler: Some(&HOOKS),
        ..IsolateConfig::default()
    };
    let mut a = crate::Isolate::new(with_hooks()).unwrap();
    let mut b = crate::Isolate::new(with_hooks()).unwrap();
    a.set_compile_strings_hook(Some(deny_all));
    assert_eq!(
        eval_text(&mut a, "unit42"),
        Err("EvalError: no: unit42".into())
    );
    assert_eq!(eval_text(&mut b, "unit42"), Ok("42".into()));
    b.set_compile_strings_hook(Some(deny_all));
    b.set_compile_strings_hook(None);
    assert_eq!(eval_text(&mut b, "unit42"), Ok("42".into()));
    assert_eq!(crate::host::hook_count(), 1, "a's entry");
    drop(a);
    let mut c = crate::Isolate::new(with_hooks()).unwrap();
    c.set_compile_strings_hook(Some(deny_all));
    assert_eq!(crate::host::hook_count(), 1, "c's entry; a's went with a");
    assert_eq!(
        eval_text(&mut c, "unit42"),
        Err("EvalError: no: unit42".into())
    );
    assert_eq!(eval_text(&mut b, "unit42"), Ok("42".into()));
    c.set_compile_strings_hook(None);
    assert_eq!(crate::host::hook_count(), 0);
}
