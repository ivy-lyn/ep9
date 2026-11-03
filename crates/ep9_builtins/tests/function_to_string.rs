//! `Function.prototype.toString` (§20.2.3.5) with the source text (M6-P2 FN-SMALL): a function
//! whose unit embeds its source gives the text of its source range (`Cx::function_source_text`,
//! a copy made on each call) when that range is its whole `[[SourceText]]`; every other
//! callable gives the NativeFunction form. Expected texts are node's (the same script under
//! `node`, `print` -> `console.log`), except the NativeFunction lines: there node gives the
//! source text, which the ep9 frontend does not record yet for methods, accessors, classes
//! and the `Function` constructor (step 2 lets the host say no per function). Every JS check
//! runs with and without GC stress, on an isolate with compiler hooks (eval and the `Function`
//! constructor; the `common` isolates have none) as in `eval.rs`.

mod common;

use std::cell::RefCell;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Isolate, IsolateConfig, JsResult, Local,
};

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

/// A compile failure as the VM throws it (only its text is used here).
fn failure(e: CodegenError) -> CompileFailure {
    CompileFailure::Internal(format!("{e:?}"))
}

fn hook_eval(source: &str, ctx: &EvalContext) -> CompileResult {
    ep9_frontend::compile_eval(source, ctx, CompileOptions::default()).map_err(failure)
}

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

/// Runs the sloppy script `src` (compiled with `options`) on an isolate with the table, `print`
/// and [`HOOKS`]: the printed lines, or the uncaught exception as "Name: message". Then
/// collects and verifies the heap.
fn run(src: &str, options: CompileOptions, stress: bool) -> Result<Vec<String>, String> {
    let unit = ep9_frontend::compile(src, SourceOpts::script(Strictness::Sloppy), options)
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
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// [`run`] with `options`, with and without GC stress; both must agree.
fn run_both_with(src: &'static str, options: CompileOptions) -> Result<Vec<String>, String> {
    let a = common::on_big_stack(move || run(src, options, false));
    let b = common::on_big_stack(move || run(src, options, true));
    assert_eq!(a, b, "GC stress changed the result of {src:?}");
    a
}

/// [`run_both_with`] the default options (the unit embeds its source).
fn run_both(src: &'static str) -> Result<Vec<String>, String> {
    run_both_with(src, CompileOptions::default())
}

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Declarations and expressions of the four function kinds give their text from the first
/// token to the closing brace: comments, white space and line breaks kept, and nothing of the
/// text around them.
#[test]
fn function_declarations_and_expressions_give_their_source_text() {
    assert_eq!(
        run_both(
            "/* before */function /* a */ f /* b */ ( /* c */ x /* d */ , /* e */ y /* f */ ) /* g */ { /* h */ ; /* i */ ; /* j */ }/* after */
var g = /* before */function* /* a */ g /* b */ () { yield 1; }/* after */;
var af = async /* a */ function /* b */ () {};
var ag = async function* ag(a,
  b) {
  // a line comment
};
var anon = [function () {}][0];
print(f.toString());
print(g.toString());
print(af.toString());
print(JSON.stringify(Function.prototype.toString.call(ag)));
print(anon.toString(), '' + function named() { return 1 });"
        ),
        lines(&[
            "function /* a */ f /* b */ ( /* c */ x /* d */ , /* e */ y /* f */ ) /* g */ { /* h */ ; /* i */ ; /* j */ }",
            "function* /* a */ g /* b */ () { yield 1; }",
            "async /* a */ function /* b */ () {}",
            r#""async function* ag(a,\n  b) {\n  // a line comment\n}""#,
            "function () {} function named() { return 1 }",
        ])
    );
}

/// Arrow functions: one bare parameter, a parenthesized list (strings and nested brackets in
/// defaults), `async` of each form, a concise or a block body.
#[test]
fn arrow_functions_give_their_source_text() {
    assert_eq!(
        run_both(
            r#"var a = [
  x => x,
  ( /* a */ a /* b */ , /* c */ b /* d */ ) /* e */ => /* f */ 0,
  async x => x,
  async /* a */ ( /* b */ ) /* c */ => /* d */ 0,
  async(a) => { a },
  (a = ")", b = [1, (2)], {c} = {c: "}"}) => ({}),
  async => async,
];
for (var f of a) print(f.toString());"#
        ),
        lines(&[
            "x => x",
            "( /* a */ a /* b */ , /* c */ b /* d */ ) /* e */ => /* f */ 0",
            "async x => x",
            "async /* a */ ( /* b */ ) /* c */ => /* d */ 0",
            "async(a) => { a }",
            r#"(a = ")", b = [1, (2)], {c} = {c: "}"}) => ({})"#,
            "async => async",
        ])
    );
}

/// The text is the source as written: escapes stay escapes, non-ASCII text and a CR LF line
/// break are kept, and a function made by eval gives its text in the eval source.
#[test]
fn the_text_is_the_source_as_written() {
    assert_eq!(
        run_both(
            "function \\u0061(\\u{62}) { return '\u{1F600}\u{e9}'; }
var crlf = function (a,\r\n b) {\r\n};
var e = eval('(function e1(){ return 1 })');
var ea = eval('0, () => 2');
print(a.toString());
print(JSON.stringify(crlf.toString()));
print(e.toString(), ea.toString());
print(a.toString() === a.toString(), typeof a.toString());"
        ),
        lines(&[
            "function \\u0061(\\u{62}) { return '\u{1F600}\u{e9}'; }",
            r#""function (a,\r\n b) {\r\n}""#,
            "function e1(){ return 1 } () => 2",
            "true string",
        ])
    );
}

/// The text does not follow the `name` property: a redefined or deleted name changes nothing.
#[test]
fn the_text_does_not_follow_the_name_property() {
    assert_eq!(
        run_both(
            "function f(a) { return a; }
Object.defineProperty(f, 'name', {value: 'not an identifier'});
print(f.toString());
delete f.name;
print(f.toString());"
        ),
        lines(&["function f(a) { return a; }", "function f(a) { return a; }"])
    );
}

/// The kinds whose source range is not their whole `[[SourceText]]` today (a method, an
/// accessor, a class, the `Function` constructor's function) give the NativeFunction form,
/// as does an arrow whose parameters hold a regular expression, a division or a template
/// (no parse here tells them apart), and a function written in the exact shape of the
/// `Function` constructor's wrapper text. Bound functions, proxies and builtins as before.
#[test]
fn the_other_callables_give_the_native_function_form() {
    assert_eq!(
        run_both(
            "var o = { m() {}, *g() {}, async am() {}, get x() { return 1; }, set x(v) {} };
var d = Object.getOwnPropertyDescriptor(o, 'x');
print(o.m.toString(), o.g.toString(), o.am.toString());
print(d.get.toString(), d.set.toString());
class A { constructor(a) {} m() {} static s() {} }
class B {}
class C extends B {}
print(A.toString(), B.toString(), C.toString());
print(A.prototype.m.toString(), A.s.toString());
print(new Function('a', 'return a').toString(), new Function().toString());
var anonymous = function(a\n) {\nreturn a\n};
print(anonymous.toString());
var r = [(a = /[)]/) => a, (a = 4 / 2) => a, (a = `)`) => a];
print(r[0].toString(), r[1].toString(), r[2].toString());
function f() {}
print(f.bind().toString(), Function.prototype.toString.call(new Proxy(f, {})), Math.max.toString());"
        ),
        lines(&[
            "function m() { [native code] } function g() { [native code] } function am() { [native code] }",
            "function get x() { [native code] } function set x() { [native code] }",
            "function A() { [native code] } function B() { [native code] } function C() { [native code] }",
            "function m() { [native code] } function s() { [native code] }",
            "function anonymous() { [native code] } function anonymous() { [native code] }",
            "function anonymous() { [native code] }",
            "function () { [native code] } function () { [native code] } function () { [native code] }",
            "function () { [native code] } function () { [native code] } function max() { [native code] }",
        ])
    );
}

/// A value that is not callable is still a TypeError.
#[test]
fn a_value_that_is_not_callable_is_a_type_error() {
    for src in [
        "Function.prototype.toString.call(undefined)",
        "Function.prototype.toString.call({})",
        "Function.prototype.toString.call(class {}.prototype)",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

/// A unit compiled without its source (`embed_source: false`) has no text: its functions give
/// the NativeFunction form (HostHasSourceTextAvailable is false), with and without GC stress.
#[test]
fn a_unit_without_its_source_gives_the_native_function_form() {
    assert_eq!(
        run_both_with(
            "function f(a) { return a; }
print(f.toString(), (x => x).toString());",
            CompileOptions {
                embed_source: false,
                ..CompileOptions::default()
            },
        ),
        lines(&["function f() { [native code] } function () { [native code] }"])
    );
}
