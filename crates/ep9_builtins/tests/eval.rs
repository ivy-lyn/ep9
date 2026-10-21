//! JS-level tests of eval and the `Function` constructor (P3-EVAL-VM), run through BytecodeGen
//! on isolates with `ep9_builtins::TABLE` and this file's own compiler hooks (the
//! `common` isolates have none, so there `eval("..")` is an EvalError). The expected lines come
//! from node v24 (`print` = `console.log` of the joined strings) unless a comment says why ep9
//! differs. Every check runs with and without GC stress. The hand-built VM tests are in
//! `crates/ep9_vm/src/tests/eval.rs`.

mod common;

use std::cell::RefCell;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Isolate, IsolateConfig, JsResult, Local,
};

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

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

/// `evalScript(src)`: `Cx::evaluate_script` (a sloppy Script through the compile_script hook,
/// as `$262.evalScript` would use it).
fn eval_script<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let src = cx.to_rust_string(args.get(0))?;
    cx.evaluate_script(&src, "evalScript.js")
}

fn failure(e: CodegenError) -> CompileFailure {
    match e {
        CodegenError::Compile(CompileError::Syntax(errors)) => {
            CompileFailure::Syntax(errors.first().message.clone())
        }
        CodegenError::Compile(CompileError::Range(r)) => CompileFailure::Range(r.message),
        CodegenError::Unsupported(u) => CompileFailure::Unsupported(u.feature.to_string()),
        CodegenError::Limit(m) => CompileFailure::Range(m),
        CodegenError::Internal(m) => CompileFailure::Internal(m),
    }
}

fn hook_eval(source: &str, ctx: &EvalContext) -> CompileResult {
    ep9_frontend::compile_eval(source, ctx, CompileOptions::default()).map_err(failure)
}

/// The FNCTOR unit contract (plan-v5 D8) built here with `compile_eval` (indirect, sloppy) on
/// the joined text, so the VM side is tested before P3-FNCTOR's `compile_function_ctor` (which
/// adds the separate parameter and body checks) is merged.
fn hook_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    let text = format!("({} anonymous({params}\n) {{\n{body}\n}})", kind.prefix());
    ep9_frontend::compile_eval(&text, &EvalContext::default(), CompileOptions::default())
        .map_err(failure)
}

fn hook_script(source: &str) -> CompileResult {
    let opts = SourceOpts::script(Strictness::Sloppy);
    ep9_frontend::compile(source, opts, CompileOptions::default()).map_err(failure)
}

static HOOKS: CompilerHooks = CompilerHooks {
    compile_eval: hook_eval,
    compile_function: hook_function,
    compile_script: hook_script,
};

/// Runs the sloppy script `src` on an isolate with the table and [`HOOKS`]: the printed
/// lines, or the uncaught exception as "Name: message". Then collects and verifies the heap.
fn run(src: &str, stress: bool) -> Result<Vec<String>, String> {
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
        compiler: Some(&HOOKS),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("print", 1, print);
    iso.register_host_function("evalScript", 1, eval_script);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.run_script(unit, "test.js");
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// [`run`] with and without GC stress; both must agree.
fn run_both(src: &'static str) -> Result<Vec<String>, String> {
    let a = common::on_big_stack(move || run(src, false));
    let b = common::on_big_stack(move || run(src, true));
    assert_eq!(a, b, "GC stress changed the result of {src:?}");
    a
}

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Direct eval sees the caller's scope; indirect eval is global code.
#[test]
fn direct_and_indirect_eval() {
    assert_eq!(
        run_both(
            r#"var x = 'global';
function f() { var x = 'local'; return [eval('x'), (0, eval)('x'), eval?.('x')].join(); }
print(f());
print(eval('1 + 2'), eval(5), eval(), typeof eval, eval === globalThis.eval);
function g() { 'use strict'; return [eval('this'), (0, eval)('this') === globalThis].join(); }
print(g());
var o = { m() { return eval('this') === o; } };
print(o.m());
print(eval('var q1 = 1; q1 + 1'), (0, eval)('if (true) { 3 } else { 4 }'));"#
        ),
        lines(&[
            "local,global,global",
            "3 5 undefined function true",
            ",true",
            "true",
            "2 3"
        ])
    );
}

/// Sloppy direct eval puts its vars in the caller's variable environment (the function's
/// `.eval_ext` object); strict code (the caller's or the eval code's own) keeps them local.
#[test]
fn sloppy_and_strict_var() {
    assert_eq!(
        run_both(
            r#"function s() { eval('var a = 1'); return typeof a; }
function t() { 'use strict'; eval('var a = 1'); return typeof a; }
function u() { eval('"use strict"; var a = 1'); return typeof a; }
function w() { eval('function h() { return 7 }'); return h(); }
function d() { eval('var y = 2'); var r = [y, delete y, typeof y]; return r.join(); }
function p(a) { eval('var a = 5'); return a; }
function n() { var k = 1; eval('function k() { return 2 }'); return k(); }
print(s(), t(), u(), w(), d(), p(1), n());
print(typeof a, typeof h, typeof y);"#
        ),
        lines(&[
            "number undefined undefined 7 2,true,undefined 5 2",
            "undefined undefined undefined"
        ])
    );
}

/// The acceptance row of plan-v5 §6: a top-level sloppy `eval("var x")` makes a configurable
/// global property `x`, and no `.eval_ext` name is visible.
#[test]
fn top_level_eval_var_is_a_configurable_global() {
    assert_eq!(
        run_both(
            r#"eval("var x");
var names = Object.getOwnPropertyNames(globalThis);
print(names.indexOf('x') >= 0, names.indexOf('.eval_ext'));
print(true, delete x, typeof x);
(0, eval)('var ix = 4; function ifn() {}');
var dx = Object.getOwnPropertyDescriptor(globalThis, 'ix');
print(dx.configurable, dx.enumerable, dx.writable, delete ifn, typeof ifn);
function f() { eval('var inner = 1'); return Object.getOwnPropertyNames(globalThis).indexOf('inner'); }
print(f());"#
        ),
        lines(&[
            "true -1",
            "true true undefined",
            "true true true true undefined",
            "-1"
        ])
    );
}

/// EvalDeclarationInstantiation conflicts: a var over a lexical binding on the way is a
/// SyntaxError (the eval code does not run), a catch parameter is not a conflict (B.3.4), a
/// non-configurable global property refuses a function (TypeError), and a failed check leaves
/// no binding of the other names (the checks come first, §19.2.1.3).
#[test]
fn eval_declaration_conflicts() {
    assert_eq!(
        run_both(
            r#"var ran = false;
function e(f) { try { f(); return 'ok'; } catch (x) { return x.constructor.name; } }
print(e(function () { let a; eval('ran = true; var a'); }), ran);
print(e(function () { { let b; eval('var b'); } }));
print(e(function () { let c; { eval('function c() {}'); } }));
let top = 1;
print(e(function () { eval('var top'); }));
print(e(function () { (0, eval)('var top'); }));
try { throw 1; } catch (err) { eval('var err = 2'); print(err); }
print(e(function () { (0, eval)('function NaN() {}'); }), e(function () { (0, eval)('var NaN'); }));
print(e(function () { (0, eval)('function before1() {} function NaN() {} function after1() {}'); }));
print(typeof before1, typeof after1);
function lf() { let z; var r = e(function () { eval('var before2; function f2() {} var z'); }); return r + ' ' + typeof before2 + ' ' + typeof f2; }
print(lf());
function lh() { let z; var r; try { eval('var before4; function f4() {} var z'); r = 'ok'; } catch (x) { r = x.constructor.name; } return r + ' ' + typeof before4 + ' ' + typeof f4; }
print(lh());"#
        ),
        lines(&[
            "SyntaxError false",
            "SyntaxError",
            "SyntaxError",
            "ok",
            "SyntaxError",
            "2",
            "TypeError ok",
            "TypeError",
            // Spec (every name is checked before any binding is made); node v24 prints
            // "function undefined": V8 makes `before1` first.
            "undefined undefined",
            "ok undefined undefined",
            "SyntaxError undefined undefined",
        ])
    );
}

/// The EvalContext of a direct eval: `new.target` and `super.x` by the receiver function
/// (also through arrows and a nested eval), private names of the caller's classes, and the
/// strictness of the call site: class code is strict inside a sloppy function (M2-P2-L6).
#[test]
fn eval_context_from_the_call_site() {
    assert_eq!(
        run_both(
            r#"function e(f) { try { return String(f()); } catch (x) { return x.constructor.name; } }
function nt() { return eval('new.target'); }
print(e(nt), typeof new nt(), e(function () { return eval("(() => eval('new.target'))()"); }));
print(e(() => eval('new.target')), e(() => (0, eval)('new.target')));
class A { m() { return eval("eval('super.toString === Object.prototype.toString')"); } }
print(new A().m(), e(function () { return eval('super.x'); }));
function cs() { var r; class C { [r = eval('var q = 1; typeof q')]() {} } return r + ',' + typeof q; }
print(cs());
class P { #x = 1; m() { return eval('this.#y'); } }
print(e(() => new P().m()));
class F { x = eval('ran2 = 1; arguments'); }
var ran2 = 0;
print(e(() => new F()), ran2);"#
        ),
        lines(&[
            "undefined function undefined",
            "SyntaxError SyntaxError",
            "true SyntaxError",
            "number,undefined",
            "SyntaxError",
            "SyntaxError 0",
        ])
    );
}

/// L5 (M2-P2-L5-OPEN): a Dynamic `eval` that a `with` object holds and that is not %eval% is
/// called with the object as `this`; the object's %eval% is a direct eval.
#[test]
fn eval_in_with_gets_its_receiver() {
    assert_eq!(
        run_both(
            r#"var o = { eval: function () { 'use strict'; return this === o; } };
with (o) { print(eval('1'), eval?.('1')); }
function f() { var v = 'f'; with ({ eval: eval }) { return eval('v'); } }
print(f());"#
        ),
        lines(&["true true", "f"])
    );
}

/// The Function constructor and %GeneratorFunction% (CreateDynamicFunction): the closure is
/// over the global environment, its [[Prototype]] comes from new.target, and parameters are
/// joined with ",".
#[test]
fn function_constructor() {
    assert_eq!(
        run_both(
            r#"print(new Function('a', 'return a')(1) === 1, Function('a', 'b', 'return a + b')(2, 3));
var v = 'global';
function f() { var v = 'local'; return Function('return v')(); }
print(f(), Function()(), Function('a,b', 'c', 'return a + b + c')(1, 2, 3));
print(Function('') instanceof Function, Function('').name);
var GF = (function* () {}).constructor;
var g = GF('a', 'yield a; yield a + 1');
var it = g(5);
print(it.next().value, it.next().value, g instanceof GF, GF !== Function);
class Sub extends Function {}
var s = new Sub('return 9');
print(s(), s instanceof Sub);
try { Function('a', '}'); } catch (x) { print(x.constructor.name); }"#
        ),
        lines(&[
            "true 5",
            "global undefined 6",
            "true anonymous",
            "5 6 true true",
            "9 true",
            "SyntaxError",
        ])
    );
}

/// `VariableMode::FunctionName` through eval (M2-P2-OPEN-IN-EVAL): the own name of a named
/// function expression ignores a sloppy store and throws on a strict one.
#[test]
fn function_name_binding_in_eval() {
    assert_eq!(
        run_both(
            r#"var f = function g() { eval('g = 1'); return typeof g; };
print(f());
var h = function k() { 'use strict'; try { eval('k = 1'); return 'no'; } catch (e) { return e.constructor.name; } };
print(h());"#
        ),
        lines(&["function", "TypeError"])
    );
}

/// An Unsupported construct in eval code is the v0 not-implemented Error, never a SyntaxError
/// (D7, W4-P3-FALSE-PASS); a lone surrogate in the source is not implemented either (D8).
#[test]
fn unsupported_and_lone_surrogates_are_not_syntax_errors() {
    assert_eq!(
        run_both(
            r#"function e(f) { try { f(); return 'ok'; } catch (x) { return x.constructor.name + ': ' + x.message; } }
print(e(() => eval('async function af() {}')));
print(e(() => eval("'" + String.fromCharCode(0xD800) + "'")));"#
        ),
        lines(&[
            "Error: not implemented in ep9 v0: unsupported: async functions",
            "Error: not implemented in ep9 v0: source text with a lone surrogate",
        ])
    );
}

/// `Cx::evaluate_script` runs a sloppy Script through the compile_script hook: its global
/// declarations are script declarations (a var is not configurable), its errors are thrown to
/// the caller, and the result is what the entry function returns (undefined: a script unit
/// does not return its completion value).
#[test]
fn evaluate_script_runs_a_script() {
    assert_eq!(
        run_both(
            r#"print(evalScript('var es = 1; let el = 2; function ef() { return es + el; }'), ef());
var d = Object.getOwnPropertyDescriptor(globalThis, 'es');
print(d.configurable, delete es, typeof el);
try { evalScript('let el = 3'); } catch (e) { print(e.constructor.name); }
try { evalScript('(('); } catch (e) { print(e.constructor.name); }"#
        ),
        lines(&[
            "undefined 3",
            "false false number",
            "SyntaxError",
            "SyntaxError"
        ])
    );
}
