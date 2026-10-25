//! JS-level tests of eval and the `Function` constructor (P3-EVAL-VM), run through BytecodeGen
//! on isolates with `ep9_builtins::TABLE` and this file's own compiler hooks (the
//! `common` isolates have none, so there `eval("..")` is an EvalError). The expected lines come
//! from node v24 (`print` = `console.log` of the joined strings) unless a comment says why ep9
//! differs. Every check runs with and without GC stress (`run_every`: also under every dispatch
//! expansion). The hand-built VM tests are in `crates/ep9_vm/src/tests/eval.rs`.

mod common;

use std::cell::RefCell;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Dispatch, Isolate, IsolateConfig,
    JsResult, Local,
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

/// P3-FNCTOR's `compile_function_ctor`: the unit of the FNCTOR contract (plan-v5 D8), whose
/// function 0 returns the new closure.
fn hook_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    ep9_frontend::compile_function_ctor(kind, params, body, CompileOptions::default())
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
    run_in(src, Dispatch::DEFAULT, stress)
}

/// [`run`] with the dispatch expansion `dispatch`.
fn run_in(src: &str, dispatch: Dispatch, stress: bool) -> Result<Vec<String>, String> {
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
        dispatch,
        builtins: Some(&ep9_builtins::TABLE),
        compiler: Some(&HOOKS),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("print", 1, print);
    iso.register_host_function("evalScript", 1, eval_script);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
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

/// [`run_in`] under every dispatch expansion, with and without GC stress; all runs must agree.
fn run_every(src: &'static str) -> Result<Vec<String>, String> {
    let mut first: Option<(String, Result<Vec<String>, String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run_in(src, d, stress));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0} for {src:?}"),
            }
        }
    }
    first.expect("at least one dispatch").1
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

/// A direct eval in eval code gets the receiver flags its eval unit was compiled with
/// (PerformEval step 8: GetThisEnvironment is the same function), so a class field
/// initializer keeps `arguments` an early error through nested eval, an arrow and a static
/// field: SyntaxError before the eval code runs (`ran` stays 0). A method or a plain function
/// in between keeps its own flags. Every dispatch, with and without GC stress.
#[test]
fn nested_eval_keeps_the_receiver_flags() {
    assert_eq!(
        run_every(
            r#"function e(f) { try { return String(f()); } catch (x) { return x.constructor.name; } }
var ran = 0;
class C1 { x = eval("eval('ran = 1; arguments')"); }
print(e(() => new C1()), ran);
class C2 { x = eval("(() => eval('ran = 2; arguments'))()"); }
print(e(() => new C2()), ran);
print(e(() => { class C3 { static x = eval("eval('ran = 3; arguments')"); } }), ran);
class C4 { x = eval("eval(\"eval('ran = 4; arguments')\")"); }
print(e(() => new C4()), ran);
class C5 { x = eval("eval('typeof new.target + typeof this.constructor + (super.toString === Object.prototype.toString)')"); }
print(new C5().x);
class C6 { m() { return eval("eval('ran = 6; arguments.length')"); } }
print(new C6().m(1, 2), ran);
class C7 { x = eval("(function () { return eval('arguments.length'); })(1, 2, 3)"); }
print(new C7().x);
function g() { return eval("eval('typeof new.target')"); }
print(g(), new g() instanceof g, e(() => eval("eval('new.target')")));"#
        ),
        lines(&[
            "SyntaxError 0",
            "SyntaxError 0",
            "SyntaxError 0",
            "SyntaxError 0",
            "undefinedfunctiontrue",
            "2 6",
            "3",
            "undefined true SyntaxError",
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
try { Function('a', '}'); } catch (x) { print(x.constructor.name); }
function e(f) { try { f(); return 'ok'; } catch (x) { return x.constructor.name; } }
print(e(() => Function('/*', '*/){')), e(() => Function('', '}); (function(){')));
print(e(() => Function('a = 1', '"use strict"')), e(() => Function('a, a', '"use strict"')));
print(Function('return typeof anonymous')(), Function('return this === globalThis')());
print(Function("'use strict'; return this")(), Function('a, b', '').length, typeof Function('').prototype);
var nf = Function('this.p = 1'); var no = new nf();
print(no.p, no instanceof nf, nf.prototype.constructor === nf);"#
        ),
        lines(&[
            "true 5",
            "global undefined 6",
            "true anonymous",
            "5 6 true true",
            "9 true",
            "SyntaxError",
            "SyntaxError SyntaxError",
            "SyntaxError SyntaxError",
            "undefined true",
            "undefined 2 object",
            "1 true true",
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
print(e(() => eval('class A { accessor x = 1 }')));
print(e(() => eval("'" + String.fromCharCode(0xD800) + "'")));"#
        ),
        lines(&[
            "Error: not implemented in ep9 v0: unsupported: class auto-accessors",
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

/// X-ARGS2 (M3-P2 fix round 2): Annex B.3.2.3 in eval code inside the parameter list of a
/// sloppy function with parameter expressions (ABI 5 `ScopeFlags::PARAMETER_ENV`). The
/// parameters are in their own environment between the eval code and the variable
/// environment (FunctionDeclarationInstantiation step 20), so a block function of the eval
/// code with a parameter's name gets no var binding (bindingExists) and the parameter keeps
/// its value. A name that is not a parameter (`zz`), the function's own name, an eval in an
/// arrow of the parameter list and an eval in the body still get one. With and without GC
/// stress. Node v24.22.1 prints the same lines, except `h`: V8 keeps the arguments object
/// binding out of the parameter environment and prints "function function"; the spec (step
/// 22: `env` is the parameter environment) gives "object object".
#[test]
fn annex_b_eval_in_parameters_keeps_the_parameters() {
    assert_eq!(
        run_both(
            r#"function f(af = 1, b = eval('{ function af() { return 9; } }')) { return typeof af + ' ' + af; }
print(f());
function g(af = 1, b = eval('{ function af() {} } typeof af')) { return b + ' ' + typeof af; }
print(g());
function q({ af } = { af: 1 }, b = eval('{ function af() {} } typeof af')) { return b + ' ' + typeof af; }
print(q());
function p(arguments = 1, b = eval('{ function arguments() {} } typeof arguments')) { return b + ' ' + typeof arguments; }
print(p());
function h(a = 1, b = eval('{ function arguments() {} } typeof arguments')) { return b + ' ' + typeof arguments; }
print(h());
function k(a = 1, b = eval('{ function zz() {} } typeof zz')) { return b + ' ' + typeof zz; }
print(k());
function r(x = 1, b = eval('{ function r() {} } typeof r')) { return b + ' ' + typeof r; }
print(r());
function t(x = 1, b = eval('{ function x() {} } typeof x'), x2 = x) { return b + ' ' + typeof x + ' ' + typeof x2; }
print(t());
function v(af = 1, b = () => eval('{ function af() {} } typeof af')) { return b() + ' ' + typeof af; }
print(v());
function n(af = 1, b = eval("eval('{ function af() {} } typeof af')")) { return b + ' ' + typeof af; }
print(n());
function w(af = 1, b = eval('{ function af() {} } { function bf() {} } typeof af + typeof bf')) { return b + ' ' + typeof af + ' ' + typeof bf; }
print(w());
function m(a, b = 2) { eval('{ function a() {} }'); return typeof a; }
print(m(1));"#
        ),
        lines(&[
            "number 1",
            "number number",
            "number number",
            "number number",
            "object object",
            "function function",
            "function function",
            "number number number",
            "function number",
            "number number",
            "numberfunction number function",
            "function",
        ])
    );
}

/// X-ARGS3 (M3-P2 fix round 2): `StoreEvalAnnexBFunction` (id 47) at the global level when a
/// Script run inside the eval code made a lexical binding of the name after id 46 made the
/// var binding (`delete` removed it first). A `const` is an immutable strict binding, so
/// SetMutableBinding throws a TypeError (§9.1.1.1.5 steps 2 and 5.b; node v24.22.1 prints the
/// same lines when the script runs at the global level); a `let` takes the function.
#[test]
fn annex_b_eval_store_to_a_late_global_const_throws() {
    assert_eq!(
        run_both(
            r#"function lateConst() { evalScript('const af = 1'); }
function lateLet() { evalScript('let al = 1'); }
try { eval('delete af; lateConst(); { function af() {} } print("after")'); print('no error'); } catch (e) { print(e.constructor.name, e.message); }
print(typeof af, af, JSON.stringify(Object.getOwnPropertyDescriptor(globalThis, 'af')));
eval('delete al; lateLet(); { function al() {} }');
print(typeof al, JSON.stringify(Object.getOwnPropertyDescriptor(globalThis, 'al')));"#
        ),
        lines(&[
            "TypeError Assignment to constant variable.",
            "number 1 undefined",
            "function undefined",
        ])
    );
}

/// M3-P2-EVAL-CATCH (R3-1): B.3.4 skips the Environment Record of any catch clause, simple or
/// destructured, at runtime (EvalDeclarationInstantiation steps 3.d.i.2.a.i and
/// 13.b.ii.4.a.i.i); the rule for a simple catch parameter is only the static early error of
/// the catch Block. So a block function of a direct eval in a destructured catch gets a var
/// binding (B.3.2.3), at the function level and at the global level, and `var f` in such an
/// eval is no SyntaxError. ep9 keeps the spec. node v24 (V8) skips only a simple catch
/// parameter, so it prints other text for the 5 lines below that have a "spec" comment; the
/// other lines are the same. Under every dispatch expansion, with and without GC stress.
#[test]
fn annex_b_eval_in_a_destructured_catch_follows_the_spec() {
    assert_eq!(
        run_every(
            r#"function g() { try { throw { f: 1 }; } catch ({ f }) { eval('{ function f() {} }'); } return typeof f; }
function g2() { var inner; try { throw [1]; } catch ([f]) { eval('{ function f() {} }'); inner = typeof f; } return inner + ' ' + typeof f; }
function h() { try { throw 1; } catch (f) { eval('{ function f() {} }'); } return typeof f; }
function v() { var inner; try { throw { f: 1 }; } catch ({ f }) { eval('var f = 2'); inner = f; } return inner + ' ' + f; }
function vs() { var inner; try { throw 1; } catch (f) { eval('var f = 2'); inner = f; } return inner + ' ' + f; }
print('g', g());
print('g2', g2());
print('h', h());
try { print('v', v()); } catch (e) { print('v', e.constructor.name); }
print('vs', vs());
try { throw { gf: 1 }; } catch ({ gf }) { eval('{ function gf() {} }'); }
print('global destr', typeof gf);
try { throw [1]; } catch ([ga]) { eval('{ function ga() {} }'); }
print('global array', typeof ga);
try { throw 1; } catch (hf) { eval('{ function hf() {} }'); }
print('global simple', typeof hf);"#
        ),
        lines(&[
            // spec; node v24: "g undefined"
            "g function",
            // spec; node v24: "g2 number undefined"
            "g2 number function",
            "h function",
            // spec; node v24: "v SyntaxError"
            "v 2 undefined",
            "vs 2 undefined",
            // spec; node v24: "global destr undefined"
            "global destr function",
            // spec; node v24: "global array undefined"
            "global array function",
            "global simple function",
        ])
    );
}

/// D3d (M3-P4; M3-P2-OPEN item 3): EvalDeclarationInstantiation step 3.d passes the parameter
/// environment of a sloppy function with parameter expressions (FunctionDeclarationInstantiation
/// step 20; ABI 5 `ScopeFlags::PARAMETER_ENV`) before it reaches the variable environment. So a
/// var or function name of eval code in the parameter list that a parameter holds is a
/// SyntaxError, checked before any binding is made (`zz` stays undeclared) and before the body
/// runs. A var name that no parameter holds, an eval in the body, a destructured parameter
/// with a body eval, an eval in an arrow of the parameter list and the name of a function
/// expression are still allowed. Node v24.22.1 prints the same lines, except the
/// `arguments` rows: V8 keeps the arguments object out of the parameter environment and
/// prints "undefined"; the spec (step 22: `env` is the parameter environment) and the
/// test262 files `language/eval-code/direct/*-declare-arguments*.js` give a SyntaxError.
#[test]
fn eval_var_in_parameters_conflicts_with_a_parameter() {
    assert_eq!(
        run_every(
            r#"function t(label, f) { try { print(label, String(f())); } catch (e) { print(label, e.constructor.name, e.message); } }
t('a', () => { function f(a = eval('var a = 1')) { return a; } return f(); });
t('b', () => { function f(a, b = eval('var a = 2')) { return a; } return f(5); });
t('c', () => { function f(a = eval('function a() {}')) { return a; } return f(); });
t('d', () => { function f(a = eval('var b'), b) { return b; } return f(); });
t('j', () => { var f = (a = eval('var a = 8')) => a; return f(); });
t('k', () => { function f(a, b = eval("eval('var a = 9')")) { return a; } return f(1); });
t('q', () => { function f(a = 1, b = eval('try {} catch (a) { var a }')) { return a; } return f(); });
t('u', () => { var n = 0; function f(a = eval('var zz = 1; var a')) { n++; } try { f(); } catch (e) {} return typeof zz + ' ' + n; });
t('args', () => { function f(p = eval('var arguments')) {} return f(); });
t('args-let', () => { function f(p = eval('var arguments')) { let arguments; } return f(); });
t('f', () => { function f(a = eval('var z = 3')) { return z; } return f(); });
t('g', () => { function f(a = 1) { eval('var a = 4'); return a; } return f(); });
t('h', () => { function f({a}) { eval('var a = 6'); { let z; } return a; } return f({a: 1}); });
t('i', () => { function f(a, b = () => eval('var a = 7')) { b(); return a; } return f(1); });
t('m', () => { var g = function f(a = eval('var f = 11')) { return f; }; return g(); });
t('arrow-args', () => { var g = (p = eval("var arguments = 'param'")) => arguments; return g(); });"#
        ),
        lines(&[
            "a SyntaxError Identifier 'a' has already been declared",
            "b SyntaxError Identifier 'a' has already been declared",
            "c SyntaxError Identifier 'a' has already been declared",
            "d SyntaxError Identifier 'b' has already been declared",
            "j SyntaxError Identifier 'a' has already been declared",
            "k SyntaxError Identifier 'a' has already been declared",
            "q SyntaxError Identifier 'a' has already been declared",
            "u undefined 0",
            // spec and test262; node v24: "args undefined"
            "args SyntaxError Identifier 'arguments' has already been declared",
            // spec and test262; node v24: "args-let undefined"
            "args-let SyntaxError Identifier 'arguments' has already been declared",
            "f 3",
            "g 4",
            "h 6",
            "i 1",
            "m 11",
            "arrow-args param",
        ])
    );
}

/// E6 (M3-P4; M3-P2-OPEN item 2): `super()` in direct eval code in a derived constructor
/// (PerformEval step 10.b.iv inDerivedConstructor), also through arrows and nested eval: it calls
/// the parent constructor with the caller's `new.target`, binds the caller's `this` (a second
/// binding is a ReferenceError, after the parent constructor ran), and runs the class field
/// initializer once. `super.x`, `new.target` and `this` in eval code keep working. Outside a
/// derived constructor `super()` stays a SyntaxError. Node v24.22.1 prints the same lines
/// (its SyntaxError and TDZ messages differ, so only the error names are printed).
#[test]
fn super_call_in_eval_code() {
    assert_eq!(
        run_every(
            r#"function t(label, f) { try { print(label, String(f())); } catch (e) { print(label, e.constructor.name); } }
class A { constructor(...v) { this.v = v.join(','); } m() { return 'A.m'; } }
t('call', () => { class B extends A { constructor() { eval('super(1)'); } } return new B().v; });
t('call-ret', () => { class B extends A { constructor() { var r = eval('super(2)'); this.same = r === this; } } var b = new B(); return b.v + ' ' + b.same; });
t('call-arrow', () => { class B extends A { constructor() { var f = () => eval('super(3)'); f(); } } return new B().v; });
t('call-nested', () => { class B extends A { constructor() { eval("eval('super(5)')"); } } return new B().v; });
t('arrow-in-eval', () => { class B extends A { constructor() { var f = eval('() => super(10)'); f(); } } return new B().v; });
t('twice', () => { class B extends A { constructor() { super(1); eval('super(6)'); } } return new B().v; });
t('twice2', () => { class B extends A { constructor() { eval('super(7)'); super(1); } } return new B().v; });
t('this-before', () => { class B extends A { constructor() { eval('this'); super(1); } } return new B().v; });
t('this-after', () => { class B extends A { constructor() { eval('super(8)'); this.w = eval('this.v'); } } return new B().w; });
t('no-super', () => { class B extends A { constructor() { eval('1'); } } return new B().v; });
t('fields', () => { var n = 0; class B extends A { f = ++n; #p = 5; g() { return this.#p; } constructor() { try { eval('super(9)'); eval('super(9)'); } catch (e) {} } } var b = new B(); return n + ' ' + b.f + ' ' + b.g(); });
t('newtarget', () => { class B extends A { constructor() { eval('super(new.target === B ? 10 : 0)'); } } return new B().v; });
t('reflect', () => { class B extends A { constructor() { eval('super(11)'); } } class X {} var o = Reflect.construct(B, [], X); return o.v + ' ' + (Object.getPrototypeOf(o) === X.prototype); });
t('spread', () => { class B extends A { constructor() { eval('super(...[1, 2], 3)'); } } return new B().v; });
t('escaped', () => { class B extends A { constructor() { super(1); this.f = () => eval('super(2)'); } } var b = new B(); try { b.f(); } catch (e) { return b.v + ' ' + e.constructor.name; } });
t('not-ctor', () => { class B extends A { constructor() { eval('super(1)'); } } Object.setPrototypeOf(B, Object.prototype); return new B(); });
t('super-prop', () => { class B extends A { constructor() { eval('super(13)'); this.r = eval('super.m()'); } } return new B().r; });
t('in-method', () => { class B extends A { m() { eval('super()'); } } return new B().m(); });
t('in-base', () => { class B { constructor() { eval('super()'); } } return new B(); });
t('in-field', () => { class B extends A { x = eval('super()'); } return new B(); });
t('in-eval-fn', () => { class B extends A { constructor() { eval('function q() { super(); }'); } } return new B(); });"#
        ),
        lines(&[
            "call 1",
            "call-ret 2 true",
            "call-arrow 3",
            "call-nested 5",
            "arrow-in-eval 10",
            "twice ReferenceError",
            "twice2 ReferenceError",
            "this-before ReferenceError",
            "this-after 8",
            "no-super ReferenceError",
            "fields 1 1 5",
            "newtarget 10",
            "reflect 11 true",
            "spread 1,2,3",
            "escaped 1 ReferenceError",
            "not-ctor TypeError",
            "super-prop A.m",
            "in-method SyntaxError",
            "in-base SyntaxError",
            "in-field SyntaxError",
            "in-eval-fn SyntaxError",
        ])
    );
}

/// E9 (M3-P4; M3-P2-OPEN item 1): a private name of the caller's classes in direct eval code
/// (PerformEval: the PrivateIdentifier names of the enclosing class bodies; §15.7
/// AllPrivateIdentifiersValid): fields, methods, getters, setters, static ones, `#x in o`,
/// through arrows, nested eval, field initializers and static blocks, with an inner class
/// shadowing an outer name, and with both brands in one class (the brand of the right kind
/// is checked). A name no enclosing class declares is a SyntaxError before the code runs. In a
/// computed key, before the class defines its private method, the brand check throws the
/// TypeError (`before-def`). Node v24.22.1 prints the same lines.
#[test]
fn caller_private_names_in_eval_code() {
    assert_eq!(
        run_every(
            r#"function t(label, f) { try { print(label, String(f())); } catch (e) { print(label, e.constructor.name); } }
t('field', () => { class C { #x = 1; m() { return eval('this.#x'); } } return new C().m(); });
t('field-set', () => { class C { #x = 1; m() { eval('this.#x = 5'); return this.#x; } } return new C().m(); });
t('field-inc', () => { class C { #x = 1; m() { eval('this.#x++; this.#x += 10'); return this.#x; } } return new C().m(); });
t('field-other', () => { class C { #x = 1; m(o) { return eval('o.#x'); } } return new C().m({}); });
t('field-set-other', () => { class C { #x = 1; m(o) { return eval('o.#x = 2'); } } return new C().m({}); });
t('method', () => { class C { #m() { return 'm' + this.v; } v = 2; f() { return eval('this.#m()'); } } return new C().f(); });
t('method-get', () => { class C { #m() {} f() { return eval('this.#m') === eval('this.#m'); } } return new C().f(); });
t('method-other', () => { class C { #m() {} f(o) { return eval('o.#m'); } } return new C().f({}); });
t('method-assign', () => { class C { #m() {} f() { eval('this.#m = 1'); } } return new C().f(); });
t('getter', () => { class C { get #g() { return 'g' + this.v; } v = 3; f() { return eval('this.#g'); } } return new C().f(); });
t('getter-set', () => { class C { get #g() { return 1; } f() { eval('this.#g = 2'); } } return new C().f(); });
t('setter', () => { class C { set #s(v) { this.v = v; } f() { eval('this.#s = 4'); return this.v; } } return new C().f(); });
t('setter-get', () => { class C { set #s(v) {} f() { return eval('this.#s'); } } return new C().f(); });
t('accessor-both', () => { class C { #v = 1; get #a() { return this.#v; } set #a(x) { this.#v = x; } f() { eval('this.#a += 5'); return eval('this.#a'); } } return new C().f(); });
t('static-field', () => { class C { static #x = 7; static f() { return eval('C.#x'); } } return C.f(); });
t('static-method', () => { class C { static #m() { return 8; } static f() { return eval('this.#m()'); } } return C.f(); });
t('static-method-wrong', () => { class C { static #m() { return 8; } static f() { return eval('this.#m()'); } } return C.f.call({}); });
t('static-getter', () => { class C { static get #g() { return 9; } static f() { return eval('this.#g'); } } return C.f(); });
t('static-setter', () => { class C { static set #s(v) { C.v = v; } static f() { eval('this.#s = 10'); return C.v; } } return C.f(); });
t('both-brands-instance-on-ctor', () => { class C { #m() { return 1; } static #s() { return 2; } static f() { return eval('this.#m()'); } } return C.f(); });
t('both-brands-static-on-inst', () => { class C { #m() { return 1; } static #s() { return 2; } f() { return eval('this.#s()'); } } return new C().f(); });
t('both-brands-ok', () => { class C { #m() { return 1; } static #s() { return 2; } f() { return eval('this.#m() + C.#s()'); } } return new C().f(); });
t('in-field', () => { class C { #x; f(o) { return eval('#x in o'); } } var c = new C(); return c.f(c) + ' ' + c.f({}); });
t('in-method', () => { class C { #m() {} f(o) { return eval('#m in o'); } } var c = new C(); return c.f(c) + ' ' + c.f({}); });
t('in-static', () => { class C { static #m() {} static f(o) { return eval('#m in o'); } } return C.f(C) + ' ' + C.f({}); });
t('arrow', () => { class C { #x = 11; f() { return (() => eval('this.#x'))(); } } return new C().f(); });
t('eval-arrow', () => { class C { #x = 12; f() { return eval('(() => this.#x)()'); } } return new C().f(); });
t('nested-eval', () => { class C { #x = 13; f() { return eval("eval('this.#x')"); } } return new C().f(); });
t('initializer', () => { class C { #x = 14; y = eval('this.#x'); } return new C().y; });
t('static-block', () => { class C { static #x = 15; static { C.r = eval('C.#x'); } } return C.r; });
t('shadow', () => { class O { #x = 'outer'; f() { class I { #x = 'inner'; g() { return eval('this.#x'); } } return new I().g(); } } return new O().f(); });
t('outer-from-inner', () => { class O { #x = 'outer'; f() { var self = this; class I { #y = 1; g() { return eval('self.#x'); } } return new I().g(); } } return new O().f(); });
t('outer-method-from-inner', () => { class O { #m() { return 'om'; } f() { var self = this; class I { #n() {} g() { return eval('self.#m()'); } } return new I().g(); } } return new O().f(); });
t('eval-class', () => { class C { #x = 16; f() { return eval('class D { #x = 17; g(o) { return o.#x; } }; new D().g(this) + new D().g(new D())'); } } return new C().f(); });
t('eval-class-caller', () => { class C { #x = 18; f() { return eval('class D { g(o) { return o.#x; } }; new D().g(this)'); } } return new C().f(); });
t('optional', () => { class C { #x = 19; f(o) { return eval('o?.#x'); } } return new C().f(null) + ' ' + new C().f(new C()); });
t('call-optional', () => { class C { #m() { return 20; } f(o) { return eval('o?.#m()'); } } return new C().f(undefined) + ' ' + new C().f(new C()); });
t('destructure', () => { class C { #x = 1; f() { eval('[this.#x] = [21]'); return this.#x; } } return new C().f(); });
t('not-declared', () => { class C { f() { return eval('this.#zz'); } } return new C().f(); });
t('not-declared-ran', () => { var ran = false; class C { f() { eval('ran = true; this.#zz'); } } try { new C().f(); } catch (e) { return e.constructor.name + ' ' + ran; } });
t('fn-in-eval', () => { class C { #x = 22; f() { return eval('(function(o) { return o.#x; })')(this); } } return new C().f(); });
t('computed-key', () => { class C { #x = 23; static [eval('"k"')] = 1; } return Object.keys(C).join(); });
t('heritage', () => { class C { #x = 1; static f() { class D extends (eval('1'), Object) {} return 1; } } return C.f(); });
t('before-def', () => { class C { [eval('try { this.#m = 1; "none" } catch (e) { e.constructor.name }')] = 1; #m() {} } return Object.keys(new C()).join(); });
t('before-def-get', () => { class C { static [eval('try { this.#g; "none" } catch (e) { e.constructor.name }')] = 1; get #g() { return 1; } } return Object.keys(C).join(); });"#
        ),
        lines(&[
            "field 1",
            "field-set 5",
            "field-inc 12",
            "field-other TypeError",
            "field-set-other TypeError",
            "method m2",
            "method-get true",
            "method-other TypeError",
            "method-assign TypeError",
            "getter g3",
            "getter-set TypeError",
            "setter 4",
            "setter-get TypeError",
            "accessor-both 6",
            "static-field 7",
            "static-method 8",
            "static-method-wrong TypeError",
            "static-getter 9",
            "static-setter 10",
            "both-brands-instance-on-ctor TypeError",
            "both-brands-static-on-inst TypeError",
            "both-brands-ok 3",
            "in-field true false",
            "in-method true false",
            "in-static true false",
            "arrow 11",
            "eval-arrow 12",
            "nested-eval 13",
            "initializer 14",
            "static-block 15",
            "shadow inner",
            "outer-from-inner outer",
            "outer-method-from-inner om",
            "eval-class TypeError",
            "eval-class-caller 18",
            "optional undefined 19",
            "call-optional undefined 20",
            "destructure 21",
            "not-declared SyntaxError",
            "not-declared-ran SyntaxError false",
            "fn-in-eval 22",
            "computed-key k",
            "heritage 1",
            "before-def TypeError",
            "before-def-get TypeError",
        ])
    );
}

/// CX2-E9-1 (M3-P4 fix round 2): a class heritage runs with the outer PrivateEnvironment
/// (ECMA-262 §15.7.14 step 8.b), computed keys with the class's own (step 12, before
/// ClassElementEvaluation). So a `#x` of eval code in a heritage, or of a class heritage in eval
/// code, is the outer class's `#x`, never the inner class's, and a slot of the inner class that
/// holds undefined never becomes an ordinary property key (`heritage-{field,set}-other`: before
/// the fix the store made a property "undefined"). `local-*`: the class is in the eval code;
/// `site-*`: the eval is in the caller's heritage; `key-*`: computed keys; `same-*`: a field
/// and a method of the same name. Node v24.22.1 prints the same lines, except where eval code
/// runs in a heritage (`site-*` and `local-nested-eval`): there V8 resolves `#x` to the inner
/// class, against step 8.b (a TypeError, or false for `#x in o`), and for `site-only-inner` it
/// throws that TypeError at run time, not the SyntaxError of PerformEval step 10.b.
#[test]
fn caller_private_names_and_the_heritage() {
    assert_eq!(
        run_every(
            r#"function t(label, f) { try { print(label, String(f())); } catch (e) { print(label, e.constructor.name); } }
t('local-set-other', () => { class C { #x; f(o) { return eval('class D extends (o.#x = 9, Object) { #x() {} }; o.undefined'); } } return new C().f({}); });
t('local-set', () => { class C { #x = 1; f(o) { return eval('class D extends (o.#x = 9, Object) { #x() {} }; o.#x'); } } var c = new C(); return c.f(c); });
t('local-get', () => { class C { #x = 5; f(o) { return eval('var v; class D extends (v = o.#x, Object) { #x() {} }; v'); } } var c = new C(); return c.f(c); });
t('local-in', () => { class C { #x = 5; f(o) { return eval('var v; class D extends (v = #x in o, Object) { #x() {} }; v'); } } var c = new C(); return c.f(c) + ' ' + c.f({}); });
t('local-method', () => { class C { #x() { return 'C.m'; } f(o) { return eval('var v; class D extends (v = o.#x(), Object) { #x = 1; }; v'); } } var c = new C(); return c.f(c); });
t('local-method-other', () => { class C { #x() { return 'C.m'; } f(o) { return eval('var v; class D extends (v = o.#x(), Object) { #x = 1; }; v'); } } return new C().f({}); });
t('local-arrow', () => { class C { #x = 7; f(o) { return eval('var g; class D extends (g = () => o.#x, Object) { #x() {} }; g()'); } } var c = new C(); return c.f(c) + ' ' + (() => { try { return c.f({}); } catch (e) { return e.constructor.name; } })(); });
t('local-getter', () => { class C { get #x() { return 'get'; } f(o) { return eval('var v; class D extends (v = o.#x, Object) { #x() {} }; v'); } } var c = new C(); return c.f(c); });
t('local-static', () => { class C { static #x() { return 'sm'; } static f(o) { return eval('var v; class D extends (v = o.#x(), Object) { static #x() { return "D"; } }; v'); } } return C.f(C); });
t('local-nested', () => { class C { #x = 11; f(o) { return eval('var v; class E extends (class D extends (v = o.#x, Object) { #x; }) { #x() {} }; v'); } } var c = new C(); return c.f(c); });
t('local-body', () => { class C { #x = 1; f(o) { return eval('class E extends (class D { #x = 12; m(p) { return p.#x; } }) { #x() {} }; new E().m(new E())'); } } return new C().f(new C()); });
t('local-only-inner', () => { class C { #y; f(o) { return eval('class D extends (o.#x, Object) { #x; }'); } } return new C().f({}); });
t('local-nested-eval', () => { class C { #x = 31; f(o) { return eval('var v; class D extends (v = eval("o.#x"), Object) { #x() {} }; v'); } } var c = new C(); return c.f(c) + ' ' + (() => { try { return c.f({}); } catch (e) { return e.constructor.name; } })(); });
t('local-class-in-heritage', () => { class C { #x = 45; f(o) { return eval('var v; class D extends (v = (class E { g() { return o.#x; } }), Object) { #x() {} }; new v().g()'); } } var c = new C(); return c.f(c); });
t('site-set-other', () => { class C { #x; f(o) { class D extends (eval('o.#x = 9'), Object) { #x() {} } return o.undefined; } } return new C().f({}); });
t('site-set', () => { class C { #x = 1; f(o) { class D extends (eval('o.#x = 9'), Object) { #x() {} } return o.#x; } } var c = new C(); return c.f(c); });
t('site-get', () => { class C { #x = 3; f(o) { var v; class D extends (v = eval('o.#x'), Object) { #x() {} } return v; } } var c = new C(); return c.f(c); });
t('site-in', () => { class C { #x = 3; f(o) { var v; class D extends (v = eval('#x in o'), Object) { #x() {} } return v; } } var c = new C(); return c.f(c) + ' ' + c.f({}); });
t('site-only-inner', () => { class C { f(o) { class D extends (eval('o.#x'), Object) { #x; } } } return new C().f({}); });
t('site-method', () => { class C { #x() { return 'Cm'; } f(o) { var v; class D extends (v = eval('o.#x()'), Object) { #x = 1; } return v; } } var c = new C(); return c.f(c); });
t('site-arrow', () => { class C { #x = 7; f(o) { var g; class D extends (g = () => eval('o.#x'), Object) { #x() {} } return g(); } } var c = new C(); return c.f(c) + ' ' + (() => { try { return c.f({}); } catch (e) { return e.constructor.name; } })(); });
t('site-nested-eval', () => { class C { #x = 9; f(o) { var v; class D extends (v = eval('eval("o.#x")'), Object) { #x() {} } return v; } } var c = new C(); return c.f(c); });
t('site-nested-class', () => { class C { #x = 10; f(o) { var v; class E extends (class D extends (v = eval('o.#x'), Object) { #x; }) { #x() {} } return v; } } var c = new C(); return c.f(c); });
t('site-static', () => { class C { static #x() { return 'Cs'; } static f(o) { var v; class D extends (v = eval('o.#x()'), Object) { static #x() { return 'Ds'; } } return v; } } return C.f(C); });
t('site-accessor', () => { class C { #v = 0; set #x(a) { this.#v = a; } get #x() { return this.#v; } f(o) { class D extends (eval('o.#x = 13'), Object) { #x() {} } return o.#x; } } var c = new C(); return c.f(c); });
t('site-middle-class', () => { class C { #x = 1; m() { class F { #y = 'F'; g(o) { var v; class E extends (v = eval('o.#y'), Object) { #y() {} } return v; } } return new F().g(new F()); } } return new C().m(); });
t('site-and-key', () => { class C { #x = 'C'; f(o) { var h, k; class D extends (h = eval('o.#x'), Object) { #x = 'D'; [k = eval('#x in o')] = 1; } return h + ' ' + k; } } var c = new C(); return c.f(c); });
t('site-tdz', () => { class C { #x = 1; f() { class D extends (eval('D'), Object) { #x() {} } } } return new C().f(); });
t('key-own', () => { class C { #x = 1; f(o) { var r; class D { #x = 2; [r = eval('#x in o')] = 0; } return r; } } var c = new C(); return c.f(c); });
t('key-own-other', () => { class C { #x = 1; f(o) { var r; class D { #x() {} [r = (() => { try { return eval('o.#x'); } catch (e) { return e.constructor.name; } })()] = 0; } return r; } } var c = new C(); return c.f(c); });
t('key-local-own', () => { class C { #x = 1; f(o) { return eval('var r; class D { #x = 2; [r = #x in o] = 0; }; r'); } } var c = new C(); return c.f(c); });
t('key-local-own-later', () => eval('var g; class D { static #m() { return "m6"; } static [(g = () => D.#m(), "k")] = 1; }; g()'));
t('same-inner-field', () => { class C { #x() { return 'Cm'; } f() { class D { #x = 'Df'; g(p) { return eval('p.#x'); } } return new D().g(new D()); } } return new C().f(); });
t('same-inner-method', () => { class C { #x = 'Cf'; f() { class D { #x() { return 'Dm'; } g(p) { return eval('p.#x()'); } } return new D().g(new D()); } } return new C().f(); });
t('same-inner-method-other', () => { class C { #x = 'Cf'; f() { var c = this; class D { #x() { return 'Dm'; } g(p) { return eval('p.#x'); } } return new D().g(c); } } return new C().f(); });"#
        ),
        lines(&[
            "local-set-other TypeError",
            "local-set 9",
            "local-get 5",
            "local-in true false",
            "local-method C.m",
            "local-method-other TypeError",
            "local-arrow 7 TypeError",
            "local-getter get",
            "local-static sm",
            "local-nested 11",
            "local-body 12",
            "local-only-inner SyntaxError",
            "local-nested-eval 31 TypeError",
            "local-class-in-heritage 45",
            "site-set-other TypeError",
            "site-set 9",
            "site-get 3",
            "site-in true false",
            "site-only-inner SyntaxError",
            "site-method Cm",
            "site-arrow 7 TypeError",
            "site-nested-eval 9",
            "site-nested-class 10",
            "site-static Cs",
            "site-accessor 13",
            "site-middle-class F",
            "site-and-key C false",
            "site-tdz ReferenceError",
            "key-own false",
            "key-own-other TypeError",
            "key-local-own false",
            "key-local-own-later m6",
            "same-inner-field Df",
            "same-inner-method Dm",
            "same-inner-method-other TypeError",
        ])
    );
}
