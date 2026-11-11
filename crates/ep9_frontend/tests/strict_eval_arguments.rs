//! PRE-1 regression tests: strict `eval`/`arguments` that is only read inside a destructuring
//! assignment target is not an early error (ECMA-262 §13.1.1 and AssignmentTargetType); a real
//! target or binding is.
//!
//! oxc_semantic 0.155.0 reported "Cannot assign to 'eval' in strict mode" for a read too, and
//! skipped its class `arguments` check (§15.7.1) for that read. From M4-P2 to M8-P0 ep9 dropped
//! those errors with a post-filter after oxc_semantic. oxc_semantic 0.157.0 reports only writes,
//! and on oxc 0.158.0 the filter dropped and replaced nothing on these 67 cases (STRICT-FILTER-REMOVE
//! task 1), so the filter is gone. These tests keep one check per case class, on each entry
//! point (scripts, modules, eval code, Function() bodies), for the next oxc bump. Upstream draft:
//! docs/upstream/oxc-strict-eval-default-init.md.
//!
//! `cargo test -p ep9_frontend --test strict_eval_arguments`

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{
    CodegenError, CompileError, CompileOptions, SourceOpts, Strictness, check_early_errors,
    compile, compile_eval, compile_function_ctor,
};

fn strict(src: &str) -> Result<(), CompileError> {
    check_early_errors(src, SourceOpts::script(Strictness::Strict))
}

fn assign_error(src: &str) -> String {
    match strict(src) {
        Err(CompileError::Syntax(e)) => e.first().message.clone(),
        other => panic!("{src}: expected a SyntaxError, got {other:?}"),
    }
}

/// The 6 PRE-1 forms (gc-front-fuzz §1.4) and more reads: node v24.22.1 accepts them all
/// (the run values are in the FE-FUZZ report). oxc_semantic 0.155.0 alone refused 14 of them.
const READS: &[&str] = &[
    r#"var v; [v = eval("1")] = [];"#,
    "var v; [v = eval] = [];",
    r#"var v; ({ v = eval("2") } = {});"#,
    r#"var v; ({ a: v = eval("3") } = {});"#,
    r#"var v; for ([v = eval("5")] of [[]]);"#,
    r#"var a = []; [a[eval("0")]] = [4];"#,
    "var v; [v = function(){ return eval }] = [];",
    "var v; [v = () => arguments] = [];",
    "function f(){ var v; [v = arguments] = []; return v }",
    r#"var v; ({ [eval("'k'")]: v } = { k: 7 });"#,
    r#"var o = {}; [o[eval("'p'")] = 8] = [];"#,
    r#"var v; [{ v = eval("9") }] = [{}];"#,
    "var v; ({ v = eval } = {});",
    "var v; [v = class { m(){ return eval } }] = [];",
    "var v; ({ v = arguments } = {});",
    "var v; [...[v = eval]] = [];",
];

/// Real targets and bindings: node v24.22.1 throws a SyntaxError for each (the for heads:
/// node v26.13.0).
const TARGETS: &[&str] = &[
    "[eval] = []",
    "({eval} = {})",
    "eval++",
    "--eval",
    "eval = 1",
    "eval += 1",
    "eval ??= 1",
    "(eval) = 1",
    "[(eval)] = []",
    "({a: eval} = {})",
    "({eval = 1} = {})",
    "[...eval] = []",
    "({...eval} = {})",
    "[arguments] = []",
    "var x = [arguments] = []",
    "var v; [v = (eval = 1)] = []",
    "var v; [v = eval++] = []",
    "var v; [v = eval] = []; arguments = 1",
    "function g(eval){}",
    "let eval = 1",
    "var v; [eval = v] = []",
    // The for-in/of head (the second gap of the upstream draft): oxc_semantic 0.155.0 accepted
    // these; 0.158.0 refuses them, as node v26.13.0 does.
    "for (eval of []);",
    "for (arguments in {});",
];

#[test]
fn reads_are_accepted() {
    for src in READS {
        assert_eq!(strict(src), Ok(()), "{src}");
    }
}

#[test]
fn targets_keep_their_error() {
    for src in TARGETS {
        let message = assign_error(src);
        assert!(
            message.starts_with("Cannot assign to '"),
            "{src}: {message}"
        );
    }
}

#[test]
fn sloppy_code_is_unchanged() {
    for src in READS
        .iter()
        .chain(&["[eval] = []", "eval++", "({eval} = {})"])
    {
        assert_eq!(
            check_early_errors(src, SourceOpts::script(Strictness::Sloppy)),
            Ok(()),
            "{src}"
        );
    }
}

#[test]
fn other_errors_stay() {
    // A read is not an error; the real error next to it stays.
    let src = "var v; [v = eval] = []; let let = 1;";
    assert!(strict(src).is_err());
    // A module is strict too.
    assert_eq!(
        check_early_errors("var v; [v = eval] = [];", SourceOpts::module()),
        Ok(())
    );
    assert!(check_early_errors("[eval] = []", SourceOpts::module()).is_err());
}

#[test]
fn eval_code_reads_and_targets() {
    let ctx = EvalContext {
        strict: true,
        ..EvalContext::default()
    };
    let options = CompileOptions::default();
    for src in READS {
        let r = compile_eval(src, &ctx, options);
        assert!(r.is_ok(), "{src}: {:?}", r.err());
    }
    for src in TARGETS {
        let r = compile_eval(src, &ctx, options);
        assert!(r.is_err(), "{src}");
    }
}

/// An `arguments` read in a class field initializer or a static block, inside a
/// destructuring target (ECMA-262 §15.7.1 ContainsArguments; arrows are not a boundary).
/// node v24.22.1: "SyntaxError: 'arguments' is not allowed in class field initializer or
/// static initialization block" for each.
const CLASS_ARGS: &[&str] = &[
    "class C{x=([v=arguments]=[])}",
    "class C{static{({[arguments]:v}={})}}",
    "class C{static{for([v=arguments] in {});}}",
    "class C{static{for([v=arguments] of []);}}",
    "class C{static x=([v=arguments]=[])}",
    "class C{x=({a:v=arguments}={})}",
    "class C{x=({v=arguments}={})}",
    "class C{x=[v=arguments]=[]}",
    "class C{x=([v=()=>arguments]=[])}",
    "class C{x=([v=()=>()=>arguments]=[])}",
    "class C{x=()=>{[v=arguments]=[]}}",
    "class C{x=([v={[arguments]:1}]=[])}",
    "class C{x=([v={[([w=arguments]=[])](){}}]=[])}",
    "class C{x=class{ y=([v=arguments]=[]) }}",
    "class C{static{ [v=arguments]=[] }}",
    "class C{static{ (()=>{[v=arguments]=[]}) }}",
    "class C{static{ class D{[([v=arguments]=[])]=1} }}",
    "class C{static{ class D{[([v=arguments]=[])](){}} }}",
];

/// The same reads behind an ordinary function or method boundary, or in a class element
/// key outside a static block: node v24.22.1 accepts each.
const CLASS_ARGS_OK: &[&str] = &[
    "class C{x=([v=function(){return arguments}]=[])}",
    "class C{x=([v=function(){return ()=>arguments}]=[])}",
    "class C{x=([v={m(){return arguments}}.m]=[])}",
    "class C{[([v=arguments]=[])]=1}",
    "class C{static [([v=arguments]=[])]=1}",
    "class C{[([v=arguments]=[])](){}}",
    "function f(){ class C{[([v=arguments]=[])]=1} }",
    "class C{m(){[v=arguments]=[]}}",
    "class C{x=class{ m(){[v=arguments]=[]} }}",
    "class C{static{ (function(){[v=arguments]=[]}) }}",
    "class C{x=([v=eval]=[])}",
    "class C{static{ [v=eval]=[] }}",
];

fn is_class_args_error(r: Result<(), CompileError>) -> bool {
    match r {
        Err(CompileError::Syntax(e)) => e.as_slice().iter().any(|e| {
            e.message == "'arguments' is not allowed in class field initializer"
                || e.message == "'arguments' is not allowed in static initialization block"
        }),
        _ => false,
    }
}

fn class_args_codegen_error<T>(r: &Result<T, CodegenError>) -> bool {
    match r {
        Err(CodegenError::Compile(e)) => is_class_args_error(Err(e.clone())),
        _ => false,
    }
}

#[test]
fn class_arguments_reads_are_errors_in_scripts() {
    for src in CLASS_ARGS {
        let src = format!("var v, w; {src}");
        for opts in [
            SourceOpts::script(Strictness::Strict),
            SourceOpts::script(Strictness::Sloppy),
            SourceOpts::module(),
        ] {
            let r = check_early_errors(&src, opts);
            assert!(is_class_args_error(r.clone()), "{src} {opts:?}: {r:?}");
            let c = compile(&src, opts, CompileOptions::default());
            assert!(
                class_args_codegen_error(&c),
                "{src} {opts:?}: {:?}",
                c.err()
            );
        }
    }
    for src in CLASS_ARGS_OK {
        let src = format!("var v, w; {src}");
        for opts in [
            SourceOpts::script(Strictness::Strict),
            SourceOpts::script(Strictness::Sloppy),
        ] {
            assert_eq!(check_early_errors(&src, opts), Ok(()), "{src} {opts:?}");
        }
    }
}

#[test]
fn class_arguments_reads_are_errors_in_eval_code() {
    let options = CompileOptions::default();
    for strict in [true, false] {
        let ctx = EvalContext {
            strict,
            ..EvalContext::default()
        };
        for src in CLASS_ARGS {
            let src = format!("var v, w; {src}");
            let r = compile_eval(&src, &ctx, options);
            assert!(
                class_args_codegen_error(&r),
                "{src} strict={strict}: {:?}",
                r.err()
            );
        }
        for src in CLASS_ARGS_OK {
            let src = format!("var v, w; {src}");
            let r = compile_eval(&src, &ctx, options);
            assert!(r.is_ok(), "{src} strict={strict}: {:?}", r.err());
        }
    }
}

#[test]
fn class_arguments_reads_are_errors_in_function_ctor_bodies() {
    let options = CompileOptions::default();
    for src in CLASS_ARGS {
        let body = format!("var v, w; {src}");
        let r = compile_function_ctor(FunctionCtorKind::Normal, "", &body, options);
        assert!(class_args_codegen_error(&r), "{body}: {:?}", r.err());
    }
    for src in CLASS_ARGS_OK {
        let body = format!("var v, w; {src}");
        let r = compile_function_ctor(FunctionCtorKind::Normal, "", &body, options);
        assert!(r.is_ok(), "{body}: {:?}", r.err());
    }
}
