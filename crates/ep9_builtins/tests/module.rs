//! Modules (M3-P2 MODULE, §16.2): JS-level tests of `Cx::run_module` (the loader API) with
//! an in-memory module graph. Each graph runs on isolates with `ep9_builtins::TABLE`, under
//! every dispatch expansion, with and without GC stress (a collection before every
//! allocation, then the heap verifier); all runs must agree.
//!
//! The expected lines are node v24.22.1's output for the same files (`.mjs`, with `print`
//! as `console.log` of the arguments joined by a space), except where a test says so.

mod common;

use std::cell::RefCell;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Dispatch, Isolate, IsolateConfig,
    JsResult, Local, TerminateHandle,
};

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TERM: RefCell<Option<TerminateHandle>> = const { RefCell::new(None) };
}

/// `stop()`: terminates the running code at its next interrupt poll (`TerminateHandle`).
fn stop<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    TERM.with(|t| t.borrow().as_ref().expect("a handle").terminate());
    Ok(Local::undefined())
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

fn hook_function(kind: FunctionCtorKind, params: &str, body: &str) -> CompileResult {
    ep9_frontend::compile_function_ctor(kind, params, body, CompileOptions::default())
        .map_err(failure)
}

fn hook_script(source: &str) -> CompileResult {
    let opts = SourceOpts::script(Strictness::Sloppy);
    ep9_frontend::compile(source, opts, CompileOptions::default()).map_err(failure)
}

/// The compiler hooks (`eval`, `Function`, scripts), as the CLI has them.
static HOOKS: CompilerHooks = CompilerHooks {
    compile_eval: hook_eval,
    compile_function: hook_function,
    compile_script: hook_script,
};

/// A module graph: (name, source); a specifier `./name` names the file `name`.
type Files = &'static [(&'static str, &'static str)];

fn source(files: Files, name: &str) -> Option<&'static str> {
    files.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

fn compile(src: &str) -> Result<ep9_bytecode::VerifiedUnit, CompileFailure> {
    ep9_frontend::compile(src, SourceOpts::module(), CompileOptions::default()).map_err(failure)
}

/// The name of the file of `files` that `spec` names in the module or script `referrer`:
/// `./` and `../` are relative to the directory of `referrer` (as the CLI resolves paths); a
/// name that `files` does not have is the error of node's message.
fn resolve_in(files: Files, referrer: &str, spec: &str) -> Result<String, String> {
    let mut parts: Vec<&str> = referrer.split('/').collect();
    parts.pop();
    for p in spec.split('/') {
        match p {
            "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    let name = parts.join("/");
    match source(files, &name) {
        Some(_) => Ok(name),
        None => Err(format!(
            "Cannot find module '{spec}' imported from {referrer}"
        )),
    }
}

/// `createRealm()`: the global object of a new realm (`Cx::create_realm`).
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.create_realm()?.into())
}

/// An isolate with the table, [`HOOKS`], `print`, `stop` and `createRealm`; OUT is empty.
fn new_isolate(dispatch: Dispatch, stress: bool) -> Isolate {
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
    iso.register_host_function("stop", 0, stop);
    iso.register_host_function("createRealm", 0, create_realm);
    TERM.with(|t| *t.borrow_mut() = Some(iso.terminate_handle()));
    OUT.with(|o| o.borrow_mut().clear());
    iso
}

/// Runs each entry of `entries` (file names of `files`) in turn with `Cx::run_module` on one
/// isolate, a microtask checkpoint after each: the printed lines, and after each entry that
/// throws, a line "uncaught Name: message". Then collects and verifies the heap.
fn run_in(files: Files, entries: &[&str], dispatch: Dispatch, stress: bool) -> Vec<String> {
    run_in_with(files, entries, dispatch, stress, false)
}

/// [`run_in`]; with `host`, the runs and their checkpoints are inside
/// `Isolate::with_module_host` with the same hooks (as the CLI runs a module).
fn run_in_with(
    files: Files,
    entries: &[&str],
    dispatch: Dispatch,
    stress: bool,
    host: bool,
) -> Vec<String> {
    let mut iso = new_isolate(dispatch, stress);
    let run = |iso: &mut Isolate| {
        for entry in entries {
            let unit = compile(source(files, entry).expect("an entry file")).expect("compiles");
            let mut resolve = |referrer: &str, spec: &str| resolve_in(files, referrer, spec);
            let mut load = |name: &str| compile(source(files, name).expect("a resolved file"));
            let r = iso.with_cx(|cx| {
                cx.run_module(unit, entry, &mut resolve, &mut load)
                    .map(|_| ())
            });
            let c = iso.perform_microtask_checkpoint();
            if let Err(e) = r.and(c) {
                OUT.with(|o| o.borrow_mut().push(format!("uncaught {}", e.message)));
            }
        }
    };
    if host {
        let mut resolve = |referrer: &str, spec: &str| resolve_in(files, referrer, spec);
        let mut load = |name: &str| compile(source(files, name).expect("a resolved file"));
        iso.with_module_host(&mut resolve, &mut load, run);
    } else {
        run(&mut iso);
    }
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    out
}

/// [`run_in`] under every dispatch expansion, with and without GC stress; all runs agree.
fn run_every(files: Files, entries: &'static [&'static str]) -> Vec<String> {
    run_every_with(files, entries, false)
}

/// [`run_in_with`] under every dispatch expansion, with and without GC stress; all runs
/// agree.
fn run_every_with(files: Files, entries: &'static [&'static str], host: bool) -> Vec<String> {
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run_in_with(files, entries, d, stress, host));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.expect("at least one dispatch").1
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Live bindings: an import sees later stores of the exporting module; an import cannot be
/// assigned (TypeError); a function export is hoisted in its module; `this` is undefined at
/// a module's top level, and module code is strict.
#[test]
fn imports_are_live_bindings() {
    const FILES: Files = &[
        (
            "main.js",
            r#"import { count, inc, early } from './counter.js';
import * as ns from './counter.js';
print(count, early());
inc(); inc();
print(count, ns.count);
try { count = 1; } catch (e) { print(e.constructor.name); }
print(this === undefined, (function () { return this; })() === undefined);
"#,
        ),
        (
            "counter.js",
            r#"print('counter', typeof inc, v);
export let count = 0;
export function inc() { count++; }
export function early() { return v; }
export var v = 'v';
"#,
        ),
    ];
    assert_eq!(
        run_every(FILES, &["main.js"]),
        lines(&[
            "counter function undefined",
            "0 v",
            "2 2",
            "TypeError",
            "true true"
        ])
    );
}

/// A module namespace object (§10.4.6): sorted string keys, then @@toStringTag ("Module",
/// not writable, enumerable or configurable); null prototype; not extensible; exports are
/// {w, e, not c}; `[[Set]]` and `[[Delete]]` of an export fail; `[[DefineOwnProperty]]`
/// accepts only no change. node v24.22.1 prints the same lines.
#[test]
fn namespace_objects_follow_10_4_6() {
    const FILES: Files = &[
        (
            "main.js",
            r#"'use strict';
import * as ns from './lib.js';
print(Reflect.ownKeys(ns).map(String).join());
print(ns[Symbol.toStringTag], Object.prototype.toString.call(ns));
var t = Object.getOwnPropertyDescriptor(ns, Symbol.toStringTag);
print(t.writable, t.enumerable, t.configurable);
print(Object.getPrototypeOf(ns), Object.isExtensible(ns), Object.isFrozen(ns), Object.isSealed(ns));
var d = Object.getOwnPropertyDescriptor(ns, 'b');
print(d.value, d.writable, d.enumerable, d.configurable);
print(Reflect.set(ns, 'b', 1), Reflect.set(ns, 'zz', 1), Reflect.deleteProperty(ns, 'b'), Reflect.deleteProperty(ns, 'zz'));
try { ns.b = 2; } catch (e) { print('set', e.constructor.name); }
try { delete ns.b; } catch (e) { print('delete', e.constructor.name); }
print(Reflect.defineProperty(ns, 'b', { value: 'B' }), Reflect.defineProperty(ns, 'b', { value: 1 }));
print(Reflect.defineProperty(ns, 'b', { configurable: true }), Reflect.defineProperty(ns, 'zz', {}));
print('b' in ns, 'zz' in ns, Object.keys(ns).join(), JSON.stringify(ns));
print(Reflect.setPrototypeOf(ns, null), Reflect.setPrototypeOf(ns, {}), Reflect.preventExtensions(ns));
var keys = []; for (var k in ns) keys.push(k); print(keys.join());
print(ns.default, ns.inner === ns.inner, ns.inner.x);
"#,
        ),
        (
            "lib.js",
            r#"export var b = 'B', a = 'A';
export { b as 'c d' };
export default 'D';
export * as inner from './inner.js';
"#,
        ),
        ("inner.js", "export const x = 'X';\n"),
    ];
    assert_eq!(
        run_every(FILES, &["main.js"]),
        lines(&[
            "a,b,c d,default,inner,Symbol(Symbol.toStringTag)",
            "Module [object Module]",
            "false false false",
            "null false false true",
            "B true true false",
            "false false false true",
            "set TypeError",
            "delete TypeError",
            "true false",
            "false false",
            "true false a,b,c d,default,inner {\"a\":\"A\",\"b\":\"B\",\"c d\":\"B\",\"default\":\"D\",\"inner\":{\"x\":\"X\"}}",
            "true false true",
            "a,b,c d,default,inner",
            "D true X",
        ])
    );
}

/// A namespace on a prototype chain: `[[Set]]` of the namespace answers false (§10.4.6.9), so
/// a store through an object that inherits from it fails (M3-P0-CXVM2-MODULE: the spec, JSC
/// and Brimstone). node v24.22.1 and QuickJS print `true 1` here.
#[test]
fn a_namespace_prototype_refuses_set() {
    const FILES: Files = &[
        (
            "main.js",
            r#"import * as ns from './lib.js';
var o = Object.create(ns);
print(Reflect.set(o, 'x', 1), o.x);
try { o.y = 1; } catch (e) { print(e.constructor.name); }
"#,
        ),
        ("lib.js", "export const k = 1;\n"),
    ];
    assert_eq!(
        run_every(FILES, &["main.js"]),
        lines(&["false undefined", "TypeError"])
    );
}

/// GC stress over a graph of 3 modules with a cycle (a -> b -> c -> a): evaluation order
/// (dependencies first, each body once), a binding in its TDZ seen from the cycle
/// (ReferenceError through the namespace and through an import), a call of a function export
/// of a module that has not run yet (the function exists from InitializeEnvironment on; its
/// body throws the ReferenceError for `a`), and the live values after the cycle ran. node
/// v24.22.1 prints the same lines.
#[test]
fn a_three_module_cycle_under_gc_stress() {
    const FILES: Files = &[
        (
            "a.js",
            r#"import { b, bf } from './b.js';
import * as C from './c.js';
print('a runs', b, bf(), C.c);
export const a = 'A';
export function af() { return 'af:' + a; }
"#,
        ),
        (
            "b.js",
            r#"import { c, cf } from './c.js';
print('b runs', c, cf());
export let b = 'B';
export function bf() { return 'bf:' + b + c; }
"#,
        ),
        (
            "c.js",
            r#"import { a, af } from './a.js';
import * as A from './a.js';
print('c runs', Reflect.ownKeys(A).map(String).join());
try { a; } catch (e) { print('import', e.constructor.name); }
try { A.a; } catch (e) { print('namespace', e.constructor.name); }
try { af(); } catch (e) { print('function', e.constructor.name, e.message); }
export const c = 'C';
export function cf() { return 'cf'; }
export function later() { return a + af(); }
globalThis.later = later;
"#,
        ),
    ];
    let out = run_every(FILES, &["a.js"]);
    let mut want = lines(&[
        "c runs a,af,Symbol(Symbol.toStringTag)",
        "import ReferenceError",
        "namespace ReferenceError",
        "function ReferenceError Cannot access 'a' before initialization",
        "b runs C cf",
        "a runs B bf:BC C",
    ]);
    assert_eq!(out, want);
    // The same graph, then a second entry that reads the cycle's live values.
    const AFTER: Files = &[
        FILES[0],
        FILES[1],
        FILES[2],
        ("after.js", "import './a.js';\nprint(later());\n"),
    ];
    want.push("Aaf:A".to_string());
    assert_eq!(run_every(AFTER, &["a.js", "after.js"]), want);
}

/// InitializeEnvironment makes a module's functions (the default one too) and its `var`
/// exports (undefined) before any module of the graph runs (§16.2.1.6.4 steps 21-24; function
/// 0's first part, M3-P2-MODULE-F0): `b.js` runs first and sees them, while a `let` is still
/// in its TDZ. `this` is undefined, also in an arrow; `import.meta` is one object per module
/// (§13.3.12.1). node v24.22.1 prints the same lines.
#[test]
fn declarations_exist_before_the_body_runs() {
    const FILES: Files = &[
        (
            "a.js",
            r#"import { readA, metaB } from './b.js';
export var v = 'V';
export let l = 'L';
export function fa() { return 'fa'; }
export default function () { return 'def'; }
const arrowThis = () => this;
function own() { return import.meta; }
print('a runs', readA(), arrowThis(), this);
print(typeof import.meta, 'x' in import.meta);
import.meta.x = 1;
print(own() === import.meta, own().x, metaB() === import.meta, metaB().x);
"#,
        ),
        (
            "b.js",
            r#"import { v, fa, l } from './a.js';
import * as A from './a.js';
import def from './a.js';
print('b sees', v, fa(), typeof def, def.name, def());
try { l; } catch (e) { print('l', e.constructor.name, e.message); }
try { A.l; } catch (e) { print('A.l', e.constructor.name, e.message); }
print(A.v, A.fa(), A.default());
export function readA() { return v + l; }
export function metaB() { return import.meta; }
"#,
        ),
    ];
    assert_eq!(
        run_every(FILES, &["a.js"]),
        lines(&[
            "b sees undefined fa function default def",
            "l ReferenceError Cannot access 'l' before initialization",
            "A.l ReferenceError Cannot access 'l' before initialization",
            "undefined fa def",
            "a runs VL undefined undefined",
            "object false",
            "true 1 false undefined",
        ])
    );
}

/// ResolveExport (§16.2.1.6.3): `export *` skips `default` and a name two stars give from two
/// bindings (ambiguous: absent from the namespace, a SyntaxError when imported by name); the
/// same binding through two stars is not ambiguous; a local export wins over a star; a
/// re-export of an imported namespace is an indirect export of that module's namespace
/// (ParseModule step 10 in the ES2027 draft, and test262's ambiguous-export-bindings
/// namespace-unambiguous tests), so two modules that re-export it agree. Each failing graph
/// throws before any module runs. node v24.22.1 prints the same lines, except the first: it
/// still finds `nsx` ambiguous (the ES2025 rule, a SyntaxError at link).
#[test]
fn resolve_export_with_stars_and_ambiguity() {
    const FILES: Files = &[
        (
            "main.js",
            r#"import * as ns from './stars.js';
import { same, own, nsx } from './stars.js';
print(Reflect.ownKeys(ns).map(String).join(), same, own, nsx.v);
"#,
        ),
        (
            "stars.js",
            "export * from './x.js';\nexport * from './y.js';\nexport const own = 'own';\n",
        ),
        (
            "x.js",
            "export const dup = 1, own = 'x';\nexport { s as same } from './s.js';\nexport default 1;\nimport * as nsx from './s.js';\nexport { nsx };\n",
        ),
        (
            "y.js",
            "export const dup = 2;\nexport { s as same } from './s.js';\nimport * as nsx from './s.js';\nexport { nsx };\n",
        ),
        ("s.js", "export const s = 'S', v = 'V';\n"),
        (
            "ambiguous.js",
            "print('never');\nimport { dup } from './stars.js';\n",
        ),
        (
            "missing.js",
            "print('never');\nimport { nope } from './x.js';\n",
        ),
        (
            "indirect.js",
            "print('never');\nexport { nope } from './x.js';\n",
        ),
        ("dflt.js", "print('never');\nimport d from './stars.js';\n"),
    ];
    assert_eq!(
        run_every(
            FILES,
            &[
                "main.js",
                "ambiguous.js",
                "missing.js",
                "indirect.js",
                "dflt.js"
            ]
        ),
        lines(&[
            "nsx,own,same,Symbol(Symbol.toStringTag) S own V",
            "uncaught SyntaxError: The requested module './stars.js' contains conflicting star exports for name 'dup'",
            "uncaught SyntaxError: The requested module './x.js' does not provide an export named 'nope'",
            "uncaught SyntaxError: The requested module './x.js' does not provide an export named 'nope'",
            "uncaught SyntaxError: The requested module './stars.js' does not provide an export named 'default'",
        ])
    );
}

/// Evaluation (§16.2.1.5.3): a diamond runs each body once, dependencies first in request
/// order. A module that throws keeps its error (`[[EvaluationError]]`): a later import of it,
/// or of a module that depends on it, throws the same error object, and its body does not run
/// again. A module that a former run evaluated does not run again. (node runs one entry per
/// process; for each single entry it prints the same lines.)
#[test]
fn evaluation_order_and_cached_errors() {
    const FILES: Files = &[
        (
            "main.js",
            "import './l.js';\nimport './r.js';\nprint('main');\n",
        ),
        ("l.js", "import './base.js';\nprint('l');\n"),
        ("r.js", "import './base.js';\nprint('r');\n"),
        ("base.js", "print('base');\n"),
        (
            "bad.js",
            "globalThis.n = (globalThis.n || 0) + 1;\nprint('bad runs', n);\nglobalThis.err = new RangeError('boom');\nthrow err;\n",
        ),
        ("uses-bad.js", "import './bad.js';\nprint('never');\n"),
        (
            "check.js",
            "import './main.js';\nprint('check', globalThis.n, err instanceof RangeError);\n",
        ),
    ];
    assert_eq!(
        run_every(
            FILES,
            &["main.js", "bad.js", "uses-bad.js", "bad.js", "check.js"]
        ),
        lines(&[
            "base",
            "l",
            "r",
            "main",
            "bad runs 1",
            "uncaught RangeError: boom",
            "uncaught RangeError: boom",
            "uncaught RangeError: boom",
            "check 1 true",
        ])
    );
}

/// Load errors: a missing module is an Error with the host's text; an early error in a
/// dependency is a SyntaxError; an unsupported import attribute key is a SyntaxError
/// (AllImportAttributesSupported; node v24.22.1 throws a TypeError). No module of the graph
/// runs.
#[test]
fn load_errors_run_no_module() {
    const FILES: Files = &[
        ("missing.js", "print('never');\nimport './nope.js';\n"),
        ("syntax.js", "print('never');\nimport './bad-syntax.js';\n"),
        ("bad-syntax.js", "print('never');\nlet x; let x;\n"),
        (
            "attr.js",
            "print('never');\nimport './ok.js' with { foo: 'bar' };\n",
        ),
        ("ok.js", "print('never');\n"),
    ];
    let out = run_every(FILES, &["missing.js", "syntax.js", "attr.js"]);
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(
        out[0],
        "uncaught Error: Cannot find module './nope.js' imported from missing.js"
    );
    assert!(out[1].starts_with("uncaught SyntaxError: "), "{out:?}");
    assert_eq!(
        out[2],
        "uncaught SyntaxError: Import attribute \"foo\" is not supported"
    );
}

/// A failed load leaves records in the map that are still `new` (a module whose requests did
/// not all load), and modules whose dependencies did not load. A later entry that reaches
/// them loads the rest again (§16.2.1.5.1.1 step 2: every `new` module is walked; a module
/// becomes `unlinked` only when the whole graph loaded). This host gives the same result for
/// each load, so the failure repeats with the same error; for a host that gives a valid
/// module later, see [`a_failed_load_is_retried`]. node runs one entry per process; for each
/// single entry it prints the same lines, with its own SyntaxError text.
#[test]
fn a_failed_load_strands_no_record() {
    const FILES: Files = &[
        ("a.js", "import './b.js';\nprint('a');\n"),
        ("b.js", "import './c.js';\nprint('b');\n"),
        ("c.js", "let x = 1; let x = 2;\n"),
        ("d.js", "import './b.js';\nprint('d');\n"),
        (
            "x.js",
            "import './b3.js';\nimport './missing.js';\nprint('x');\n",
        ),
        ("b3.js", "import './c3.js';\nprint('b3');\n"),
        ("c3.js", "print('c3');\n"),
        ("e.js", "import './b3.js';\nprint('e');\n"),
        ("p.js", "import './q.js';\nprint('p');\n"),
        ("q.js", "import './c.js';\nprint('q');\n"),
    ];
    let syntax = "uncaught SyntaxError: Identifier `x` has already been declared";
    assert_eq!(
        run_every(
            FILES,
            &["a.js", "d.js", "a.js", "x.js", "e.js", "p.js", "p.js"]
        ),
        lines(&[
            syntax,
            syntax,
            syntax,
            "uncaught Error: Cannot find module './missing.js' imported from x.js",
            "c3",
            "b3",
            "e",
            syntax,
            syntax,
        ])
    );
}

/// A failed load keeps no failed completion (H22-Q3, R3-5, RV4-3): the host fails the first
/// load of `dep.js` (an early error), then gives a valid module. The first run of `main.js`
/// throws and runs no body. The second run calls `resolve` and `load` again only for the
/// request that has no module (`dep.js`; `b.js` keeps its record) and runs every body. A
/// third run calls no hook and runs nothing. (node v24.22.1 runs b, dep and main for a
/// dynamic `import()` of main.mjs after dep.mjs is fixed.)
#[test]
fn a_failed_load_is_retried() {
    const MAIN: &str = "import './b.js';\nimport './dep.js';\nprint('main');\n";
    let run = |dispatch: Dispatch, stress: bool| -> Vec<String> {
        let mut iso = new_isolate(dispatch, stress);
        let log = RefCell::new(Vec::new());
        let dep_loads = core::cell::Cell::new(0);
        for round in 0..3 {
            log.borrow_mut().push(format!("round {round}"));
            let mut resolve = |referrer: &str, spec: &str| -> Result<String, String> {
                log.borrow_mut().push(format!("resolve {referrer} {spec}"));
                Ok(spec.strip_prefix("./").unwrap_or(spec).to_string())
            };
            let mut load = |name: &str| {
                log.borrow_mut().push(format!("load {name}"));
                match name {
                    "b.js" => compile("print('b');\n"),
                    "dep.js" => {
                        dep_loads.set(dep_loads.get() + 1);
                        if dep_loads.get() == 1 {
                            compile("let x = 1; let x = 2;\n")
                        } else {
                            compile("print('dep');\n")
                        }
                    }
                    _ => panic!("no file {name}"),
                }
            };
            let unit = compile(MAIN).expect("compiles");
            let r = iso.with_cx(|cx| {
                cx.run_module(unit, "main.js", &mut resolve, &mut load)
                    .map(|_| ())
            });
            let c = iso.perform_microtask_checkpoint();
            let printed = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
            let mut log = log.borrow_mut();
            log.extend(printed);
            if let Err(e) = r.and(c) {
                let name = e.message.split(':').next().unwrap_or("").to_string();
                log.push(format!("uncaught {name}"));
            }
        }
        iso.collect_garbage();
        iso.verify_heap().expect("a valid heap");
        log.into_inner()
    };
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run(d, stress));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    assert_eq!(
        first.expect("at least one dispatch").1,
        lines(&[
            "round 0",
            "resolve main.js ./b.js",
            "load b.js",
            "resolve main.js ./dep.js",
            "load dep.js",
            "uncaught SyntaxError",
            "round 1",
            "resolve main.js ./dep.js",
            "load dep.js",
            "b",
            "dep",
            "main",
            "round 2",
        ])
    );
}

/// A termination during Evaluate (H23: the isolate is used again after it) leaves no module
/// that a later run takes as evaluated: the terminated module and each module that imports it
/// throw, and no body runs on the incomplete module. (node has no termination of a module
/// that the same process imports again; the error text is ep9's.)
#[test]
fn a_terminated_evaluation_is_not_a_success() {
    const FILES: Files = &[
        (
            "half.js",
            "globalThis.started = true;\nstop();\nfor (;;) {}\nglobalThis.finished = true;\n",
        ),
        (
            "dep.js",
            "import './half.js';\nprint('dep runs', globalThis.started, globalThis.finished);\n",
        ),
        ("top.js", "import './dep.js';\nprint('top runs');\n"),
    ];
    let aborted = "uncaught Error: the evaluation of module half.js was terminated";
    assert_eq!(
        run_every(FILES, &["half.js", "half.js", "dep.js", "top.js"]),
        lines(&["uncaught execution terminated", aborted, aborted, aborted])
    );
}

/// ResolveExport on a shallow graph with a deep resolution: `a` stars `b`, and `b` re-exports
/// `n{i}` as `a`'s `n{i+1}`, around a cycle. The walk goes 2 levels per name, so it needs far
/// more native stack than the graph depth. On a small stack it is a RangeError, not a native
/// stack overflow. (node v24.22.1 crashes with SIGSEGV on this graph.)
#[test]
fn a_deep_resolve_export_is_a_range_error() {
    const N: usize = 6000;
    let b: String = (0..N)
        .map(|i| format!("export {{ n{} as n{i} }} from './a.js';\n", (i + 1) % N))
        .collect();
    let files: Files = Box::leak(Box::new([
        ("a.js", "export * from './b.js';\n"),
        ("b.js", Box::leak(b.into_boxed_str()) as &str),
        (
            "main.js",
            "import { n0 } from './a.js';\nprint('never', n0);\n",
        ),
        (
            "ns.js",
            "import * as ns from './a.js';\nprint('never', ns);\n",
        ),
    ]));
    let out = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || run_in(files, &["main.js", "ns.js"], Dispatch::ALL[0], false))
        .expect("a thread")
        .join()
        .expect("no panic");
    let range = "uncaught RangeError: Maximum call stack size exceeded";
    assert_eq!(out, lines(&[range, range]));
}

/// Direct eval in module code sees the module's cells (§19.2.1.1 PerformEval: the eval code's
/// outer environment ends at the module Environment Record): imports (named, namespace,
/// a hoisted function), exported `let`/`var` bindings, `typeof`, a call, the TDZ, and a store
/// to an import (TypeError). node v24.22.1 prints the same lines, except the store to an
/// exported binding: ep9 throws its v0 not-implemented Error there, because the module records
/// (ABI 5) do not say which exported binding is `const`.
#[test]
fn direct_eval_sees_module_cells() {
    const FILES: Files = &[
        (
            "main.js",
            r#"import { v, w, fx } from './x.js';
import * as ns from './x.js';
export let y = 'y1';
export var z = 'z1';
export const k = 'k1';
function t(s) { try { print(s, String(eval(s))); } catch (e) { print(s, 'threw', e.name, e.message); } }
t('v'); t('w'); t('fx()'); t('ns.v'); t('y'); t('z'); t('k'); t('typeof v'); t('typeof nope');
t('late'); t('typeof late');
t('v = 1'); t('ns = 1');
t('y = "y2", y');
print('after', y);
(function () { var v = 'inner'; t2(); function t2() { print('inner', eval('v + w')); } })();
print('arrow', (() => eval('fx() + k'))());
export let late = 'late';
"#,
        ),
        (
            "x.js",
            "export let v = 'xv';
export var w = 'xw';
export function fx() { return 'fx'; }
",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["main.js"]),
        lines(&[
            "v xv",
            "w xw",
            "fx() fx",
            "ns.v xv",
            "y y1",
            "z z1",
            "k k1",
            "typeof v string",
            "typeof nope undefined",
            "late threw ReferenceError Cannot access 'late' before initialization",
            "typeof late threw ReferenceError Cannot access 'late' before initialization",
            "v = 1 threw TypeError Assignment to constant variable.",
            "ns = 1 threw TypeError Assignment to constant variable.",
            "y = \"y2\", y threw Error not implemented in ep9 v0: assignment to an exported module binding in direct eval code",
            "after y1",
            "inner innerxw",
            "arrow fxk1",
        ])
    );
}

// ---- M3-P3 IMPORT-TLA ----------------------------------------------------------------------

/// Top-level await (§16.2.1.5.3, the async steps): the importer waits for its async
/// dependency; a sync sibling and its job run while the dependency awaits; `run_module` runs
/// checkpoints until the entry settles. Node v24.22.1 prints the same lines.
#[test]
fn top_level_await_orders_like_node() {
    const FILES: Files = &[
        (
            "a.js",
            "import { b } from './b.js';\nimport './c.js';\nprint('a', b);",
        ),
        (
            "b.js",
            "print('b start');\nawait 0;\nprint('b mid');\nawait Promise.resolve();\nexport const b = 'B';\nprint('b end');",
        ),
        (
            "c.js",
            "print('c');\nPromise.resolve().then(() => print('c job'));",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["a.js"]),
        lines(&["b start", "c", "b mid", "c job", "b end", "a B"])
    );
}

/// `import()` in module code (§13.3.10.1): a namespace, a namespace with a `then` export (a
/// thenable), a load error and a cached evaluation error (rejections), a non-object options
/// value (TypeError) and a throwing ToString (its error), as node v24.22.1 gives them. A
/// rejected top-level await is the error of the run.
#[test]
fn import_calls_settle_like_node() {
    const FILES: Files = &[
        (
            "d.js",
            "const ns = await import('./b.js');\nprint('d', ns.b, Object.keys(ns).join());\nconst t = await import('./t.js');\nprint('t', typeof t, t.then === undefined);\ntry { await import('./missing.js'); } catch (e) { print('missing', e.constructor.name); }\ntry { await import('./e.js'); } catch (e) { print('e1', e.message); }\ntry { await import('./e.js'); } catch (e) { print('e2', e.message); }\nimport('./b.js', 1).catch(e => print('opt', e.constructor.name));\nimport({ toString() { throw new RangeError('spec'); } }).catch(e => print('spec', e.constructor.name));",
        ),
        ("b.js", "await 0;\nexport const b = 'B';"),
        ("t.js", "export function then(r) { r(42); }"),
        ("e.js", "await 0;\nthrow new Error('boom');"),
    ];
    assert_eq!(
        run_every(FILES, &["d.js", "e.js"]),
        lines(&[
            "d B b",
            "t number true",
            "missing Error",
            "e1 boom",
            "e2 boom",
            "opt TypeError",
            "spec RangeError",
            "uncaught Error: boom",
        ])
    );
}

// ---- M3-P3 integration fixer (the review findings TLA-1 to TLA-3) ---------------------------

/// `import()` of a module whose async evaluation finished and that was not the root of an
/// earlier Evaluate (so it has no `[[TopLevelCapability]]`): Evaluate (§16.2.1.5.3 step
/// 10.c) fulfills the new promise because the module is `evaluated`, also when its
/// `[[AsyncEvaluationOrder]]` is done. `d.js` has no TLA but waits for `c.js`; `a2.js` has
/// TLA. Node v24.22.1 prints the same lines.
#[test]
fn import_of_an_evaluated_async_module_settles() {
    const FILES: Files = &[
        ("c.js", "print('c start');\nawait null;\nprint('c end');"),
        ("d.js", "import './c.js';\nprint('d');"),
        (
            "m2.js",
            "import './d.js';\nconst ns = await import('./d.js');\nprint('after import d', Object.keys(ns).length);",
        ),
        ("a2.js", "await 0;\nexport const x = 1;"),
        (
            "m3.js",
            "import { x } from './a2.js';\nawait 0;\nconst ns = await import('./a2.js');\nprint('import a2 ok', ns.x, x);",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["m2.js", "m3.js"]),
        lines(&[
            "c start",
            "c end",
            "d",
            "after import d 0",
            "import a2 ok 1 1",
        ])
    );
}

/// `import()` with an import attribute key other than `type` rejects with a TypeError
/// (EvaluateImportCall: AllImportAttributesSupported is false), also next to a `type` key.
/// A static import with that key is a SyntaxError ([`load_errors_run_no_module`]). Node
/// v24.22.1 gives the same errors; its message adds " in <the module URL>".
#[test]
fn an_import_call_with_an_unsupported_key_is_a_type_error() {
    const FILES: Files = &[
        ("ok.js", "print('never');\nexport const k = 1;"),
        (
            "m4.js",
            "const show = e => print(e.constructor.name, e.message);\nawait import('./ok.js', { with: { foo: 'bar' } }).then(() => print('ok?'), show);\nawait import('./ok.js', { with: { type: 'css', foo: 'bar' } }).then(() => print('ok?'), show);\nawait import('./ok.js', { with: { type: 'css' } }).then(() => print('ok?'), e => print(e.constructor.name));",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["m4.js"]),
        lines(&[
            "TypeError Import attribute \"foo\" with value \"bar\" is not supported",
            "TypeError Import attribute \"foo\" with value \"bar\" is not supported",
            "TypeError",
        ])
    );
}

/// A termination after a top-level `await` (in a job: the checkpoint of `run_module`, or the
/// host's checkpoint after it) also leaves no module that a later run waits for or takes as
/// evaluated (the M3-P2 ABORTED rule of [`a_terminated_evaluation_is_not_a_success`]): a
/// later run of the module or of an importer throws the "was terminated" Error, and a later
/// `import()` of it rejects with that Error. (node has no termination of a module that the
/// same process imports again; the error text is ep9's.)
#[test]
fn a_termination_after_a_top_level_await_is_not_a_success() {
    const FILES: Files = &[
        (
            "half.js",
            "globalThis.started = true;\nawait 0;\nstop();\nfor (;;) {}\nglobalThis.finished = true;\n",
        ),
        (
            "dep.js",
            "import './half.js';\nprint('dep runs', globalThis.started, globalThis.finished);\n",
        ),
        (
            "dyn.js",
            "import('./half.js').then(() => print('dyn ok'), e => print('dyn err', e.message));\nawait 0; await 0; await 0; await 0;\nprint('dyn end');\n",
        ),
        // The termination in the host's checkpoint after the run: `spawn.js` runs to its end
        // and `half2.js` evaluates in a job of `import()`.
        ("half2.js", "await 0;\nstop();\nfor (;;) {}\n"),
        ("spawn.js", "import('./half2.js');\nprint('spawn');\n"),
        ("dep2.js", "import './half2.js';\nprint('dep2 runs');\n"),
    ];
    let aborted = "the evaluation of module half.js was terminated";
    assert_eq!(
        run_every(
            FILES,
            &[
                "half.js", "half.js", "dep.js", "dyn.js", "spawn.js", "dep2.js"
            ]
        ),
        lines(&[
            "uncaught execution terminated",
            &format!("uncaught Error: {aborted}"),
            &format!("uncaught Error: {aborted}"),
            &format!("dyn err {aborted}"),
            "dyn end",
            "spawn",
            "uncaught execution terminated",
            "uncaught Error: the evaluation of module half2.js was terminated",
        ])
    );
}

/// A module body that waits for a promise outside the job queue goes on after the
/// termination that made its module aborted (as any async function does), but the module
/// stays aborted: its late settlement changes no record, and a later importer throws the
/// "was terminated" Error.
#[test]
fn an_aborted_async_module_stays_aborted() {
    const FILES: Files = &[
        (
            "half3.js",
            "await new Promise(r => { globalThis.resume = r; });\nprint('half3 resumed');\n",
        ),
        (
            "e.js",
            "import('./half3.js');\nawait 0;\nstop();\nfor (;;) {}\n",
        ),
        ("r.js", "globalThis.resume();\nprint('r');\n"),
        ("dep3.js", "import './half3.js';\nprint('dep3 runs');\n"),
    ];
    assert_eq!(
        run_every(FILES, &["e.js", "r.js", "dep3.js"]),
        lines(&[
            "uncaught execution terminated",
            "r",
            "half3 resumed",
            "uncaught Error: the evaluation of module half3.js was terminated",
        ])
    );
}

/// The isolate's list of async modules drops the finished ones when it is full, and keeps
/// the pending ones: `never.js` waits forever from the second entry on, the list is full at
/// the run of `l2.js` (4 entries), and the termination in `t.js` still makes `never.js`
/// aborted, so `dep4.js` throws the "was terminated" Error (not the "never settled" one).
#[test]
fn the_async_module_list_keeps_the_pending_modules() {
    const FILES: Files = &[
        ("w.js", "import('./never.js');\nawait 0;\n"),
        (
            "never.js",
            "await new Promise(() => {});\nprint('never');\n",
        ),
        ("l0.js", "await 0;\n"),
        ("l1.js", "await 0;\n"),
        ("l2.js", "await 0;\n"),
        ("l3.js", "await 0;\n"),
        ("t.js", "await 0;\nstop();\nfor (;;) {}\n"),
        ("dep4.js", "import './never.js';\nprint('dep4 runs');\n"),
    ];
    assert_eq!(
        run_every(
            FILES,
            &[
                "w.js", "l0.js", "l1.js", "l2.js", "l3.js", "t.js", "dep4.js"
            ]
        ),
        lines(&[
            "uncaught execution terminated",
            "uncaught Error: the evaluation of module never.js was terminated",
        ])
    );
}

// ---- M3-P3 EMBED-MIGRATE step 2: the module host of scripts (`Isolate::with_module_host`) ---

/// Runs the script `script` (named `main.js`) on one isolate inside
/// `Isolate::with_module_host` with the hooks of [`run_in`] over `files`, a microtask
/// checkpoint after it; then, after the host is gone, the script `after` with no host. The
/// printed lines, and "uncaught Name: message" after a run that throws. Then collects and
/// verifies the heap.
fn run_script_in(
    files: Files,
    script: &str,
    after: &str,
    dispatch: Dispatch,
    stress: bool,
) -> Vec<String> {
    let mut iso = new_isolate(dispatch, stress);
    let mut resolve = |referrer: &str, spec: &str| -> Result<String, String> {
        let name = spec.strip_prefix("./").unwrap_or(spec);
        match source(files, name) {
            Some(_) => Ok(name.to_string()),
            None => Err(format!(
                "Cannot find module '{spec}' imported from {referrer}"
            )),
        }
    };
    let mut load = |name: &str| compile(source(files, name).expect("a resolved file"));
    let unit = hook_script(script).expect("compiles");
    let r = iso.with_module_host(&mut resolve, &mut load, |iso| {
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|_| ()))
    });
    if let Err(e) = r {
        OUT.with(|o| o.borrow_mut().push(format!("uncaught {}", e.message)));
    }
    let unit = hook_script(after).expect("compiles");
    if let Err(e) = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "after.js").map(|_| ())) {
        OUT.with(|o| o.borrow_mut().push(format!("uncaught {}", e.message)));
    }
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    out
}

/// `import()` in a script and in a promise job after the script (the host's checkpoint) uses
/// the hooks of `Isolate::with_module_host`; a module that two imports name is loaded and
/// evaluated once, and a load error rejects. The first five lines are node v24.22.1's output
/// for the same script (as a CommonJS file, with `.mjs` modules). After the host is gone,
/// `import()` throws the v0 not-implemented Error (ep9's own rule, IT-NOHOST).
#[test]
fn import_calls_in_scripts_and_jobs_use_the_module_host() {
    const FILES: Files = &[
        ("b.js", "print('b');\nexport const b = 'B';"),
        (
            "c.js",
            "import { b } from './b.js';\nexport const c = 'C' + b;",
        ),
    ];
    const SCRIPT: &str = "print('sync');\nimport('./b.js').then(ns => {\n  print('script', ns.b);\n  return import('./c.js');\n}).then(ns => {\n  print('job', ns.c);\n  return import('./missing.js');\n}).catch(e => print('missing', e.constructor.name));";
    const AFTER: &str = "try { import('./b.js'); } catch (e) { print('no host', e.message); }";
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run_script_in(FILES, SCRIPT, AFTER, d, stress));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    assert_eq!(
        first.expect("at least one dispatch").1,
        lines(&[
            "sync",
            "b",
            "script B",
            "job CB",
            "missing Error",
            "no host not implemented in ep9 v0: DynamicImport",
        ])
    );
}

// ---- M3-P3 fix round 2 (the Codex findings C-TLA2 to C-TLA6) -------------------------------

/// `import()` in a job of a module run (C-TLA2): in a promise job after a synchronous entry,
/// and in a module that `import()` evaluates in a job. `Cx::run_module` ends with a microtask
/// checkpoint while its hooks are the module host, so both load through them, also with no
/// `Isolate::with_module_host` around the run (before the fix they threw the v0
/// "DynamicImport" Error after the run). Inside `with_module_host` (as the CLI runs) the
/// output is the same. Node v24.22.1 prints the same lines for each entry.
#[test]
fn import_calls_in_jobs_of_a_module_run_use_its_hooks() {
    const FILES: Files = &[
        ("b.js", "print('b');\nexport const b = 'B';"),
        ("c.js", "print('c');\nexport const c = 'C';"),
        (
            "s.js",
            "Promise.resolve().then(() => import('./b.js')).then(ns => print('deferred', ns.b), e => print('err', e.message));\nprint('s');",
        ),
        (
            "n1.js",
            "print('n1');\nimport('./c.js').then(m => print('n1 got', m.c), e => print('n1 err', e.message));\nexport const v = 'V';",
        ),
        (
            "n.js",
            "import('./n1.js').then(ns => print('n', ns.v), e => print('n err', e.message));\nprint('n sync');",
        ),
    ];
    let expected = lines(&[
        "s",
        "b",
        "deferred B",
        "n sync",
        "n1",
        "c",
        "n V",
        "n1 got C",
    ]);
    assert_eq!(run_every_with(FILES, &["s.js", "n.js"], false), expected);
    assert_eq!(run_every_with(FILES, &["s.js", "n.js"], true), expected);
}

/// `import()` from a function of another realm (C-TLA3): the function made by
/// `new other.Function(...)` in `r.js` has `r.js` as its ScriptOrModule, so its import of
/// `./dep.js` gives the module that `r.js` loaded (HostLoadImportedModule, §16.2.1.10: one
/// referrer and specifier give one module; the same for a function that `other.eval` makes),
/// and the promise is of the other realm
/// (EvaluateImportCall: %Promise% of the current realm). A module that such a call loads
/// first (`fresh.js`) is in `r.js`'s realm: its objects and functions are of that realm,
/// and `r.js`'s own import of it later gives the same namespace. Before the fix, ep9 made a
/// second record of each module in the other realm's map (`same false false`). The expected
/// lines follow the spec (node's `vm` realms have no import() of the main module map).
#[test]
fn an_import_call_from_another_realm_uses_the_referrer_module_map() {
    const FILES: Files = &[
        ("dep.js", "export const o = {};"),
        (
            "fresh.js",
            "export const arr = [];\nexport function fn() {}\nexport function g() { return globalThis; }",
        ),
        (
            "r.js",
            "import * as a from './dep.js';\nconst other = createRealm();\nconst p = new other.Function(\"return import('./dep.js')\")();\nprint('promise realm', p instanceof other.Promise, p instanceof Promise);\nconst ns = await p;\nprint('same', ns === a, ns.o === a.o);\nprint('eval same', (await other.eval(\"(function () { return import('./dep.js'); })\")()) === a);\nconst f = await new other.Function(\"return import('./fresh.js')\")();\nprint('fresh realm', Object.getPrototypeOf(f.arr) === Array.prototype, Object.getPrototypeOf(f.fn) === Function.prototype, f.g() === globalThis);\nprint('fresh same', (await import('./fresh.js')) === f);",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["r.js"]),
        lines(&[
            "promise realm true false",
            "same true true",
            "eval same true",
            "fresh realm true true true",
            "fresh same true",
        ])
    );
}

/// A function made by eval code or by `Function()` keeps the ScriptOrModule of the code that
/// made it (C-TLA4; OrdinaryFunctionCreate §10.2.3, PerformEval §19.2.1.1): called from
/// `main.js`, its `import('./dep.js')` still resolves from `sub/creator.js` (indirect eval,
/// `Function()`, `Function()` in indirect eval code, direct eval). Indirect eval code in
/// `main.js` resolves from `main.js`. Before the fix, ep9 took the caller (`main.js`) for
/// the first three and loaded `dep.js` (`root`). Node v24.22.1 prints the same lines.
#[test]
fn functions_made_by_eval_code_import_from_their_maker() {
    const FILES: Files = &[
        ("dep.js", "export const where = 'root';"),
        ("sub/dep.js", "export const where = 'sub';"),
        (
            "sub/creator.js",
            "export function make() { return (0, eval)(\"(function () { return import('./dep.js'); })\"); }\nexport function makeFn() { return new Function(\"return import('./dep.js')\"); }\nexport function makeNested() { return (0, eval)(\"new Function(\\\"return import('./dep.js')\\\")\"); }\nexport function makeDirect() { return eval(\"(function () { return import('./dep.js'); })\"); }",
        ),
        (
            "main.js",
            "import { make, makeFn, makeNested, makeDirect } from './sub/creator.js';\nconst show = (tag) => [ns => print(tag, ns.where), e => print(tag, 'err', e.message)];\nawait make()().then(...show('indirect eval'));\nawait makeFn()().then(...show('Function'));\nawait makeNested()().then(...show('Function in eval code'));\nawait makeDirect()().then(...show('direct eval'));\nawait (0, eval)(\"import('./dep.js')\").then(...show('main eval'));",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["main.js"]),
        lines(&[
            "indirect eval sub",
            "Function sub",
            "Function in eval code sub",
            "direct eval sub",
            "main eval root",
        ])
    );
}

/// `import()` attributes (C-TLA6; EvaluateImportCall step 10, ES2025): EnumerableOwnProperties
/// of the `with` object runs every getter first (a throw is the rejection), and only then
/// the value check (a non-string value is a TypeError). Before the fix, ep9 checked each
/// value as it read it: case `a` skipped the getter and rejected with a TypeError, case `b`
/// did not run `get z`. Node v24.22.1 prints the same lines.
#[test]
fn import_attributes_run_every_getter_before_the_checks() {
    const FILES: Files = &[
        ("dep.js", "export const d = 1;"),
        (
            "at.js",
            "const show = (tag) => [() => print(tag, 'ok'), e => print(tag, typeof e === 'object' ? e.constructor.name : e)];\nawait import('./dep.js', { with: { type: 1, get later() { print('getter'); throw 42; } } }).then(...show('a'));\nawait import('./dep.js', { with: { get type() { print('get type'); return 1; }, get z() { print('get z'); return 'x'; } } }).then(...show('b'));\nawait import('./dep.js', { with: { foo: 1, get z() { print('get z2'); return 'x'; } } }).then(...show('c'));\nconst w = { get a() { delete this.b; return 'x'; }, b: 1 };\nawait import('./dep.js', { with: w }).then(...show('d'));",
        ),
    ];
    assert_eq!(
        run_every(FILES, &["at.js"]),
        lines(&[
            "getter",
            "a 42",
            "get type",
            "get z",
            "b TypeError",
            "get z2",
            "c TypeError",
            "d TypeError",
        ])
    );
}

/// The module host of `Isolate::with_module_host` belongs to its isolate only
/// (EM2-HOST-ADDR): code in the call can put a new isolate in its place and drop the old
/// one, and an isolate made after that can get the old one's address (the test makes 8, so
/// the allocator is likely to give one of them that address). None has a host: `import()`
/// throws the v0 not-implemented Error. Before the fix (hosts keyed by the `Runtime`
/// address), a later isolate used the first one's hooks (`got B`).
#[test]
fn a_dropped_isolate_leaves_no_module_host() {
    const FILES: Files = &[("b.js", "export const b = 'B';")];
    fn script(tag: &str) -> String {
        format!(
            "try {{ import('./b.js').then(ns => print('{tag} got', ns.b), e => print('{tag} err', e.message)); }} catch (e) {{ print('{tag} no host', e.message); }}"
        )
    }
    let mut iso = new_isolate(Dispatch::ALL[0], false);
    let mut resolve = |referrer: &str, spec: &str| resolve_in(FILES, referrer, spec);
    let mut load = |name: &str| compile(source(FILES, name).expect("a resolved file"));
    let run = |iso: &mut Isolate, tag: &str| {
        let unit = hook_script(&script(tag)).expect("compiles");
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|_| ()))
            .expect("no throw");
        OUT.with(|o| core::mem::take(&mut *o.borrow_mut()))
    };
    let out = iso.with_module_host(&mut resolve, &mut load, |iso| {
        let old = std::mem::replace(iso, new_isolate(Dispatch::ALL[0], false));
        drop(old);
        let mut out = run(iso, "second");
        let mut later: Vec<Isolate> = Vec::new();
        for i in 0..8 {
            let mut next = new_isolate(Dispatch::ALL[0], false);
            out.extend(run(&mut next, &format!("later {i}")));
            later.push(next);
        }
        out
    });
    let no_host = "not implemented in ep9 v0: DynamicImport";
    let mut expected = vec![format!("second no host {no_host}")];
    expected.extend((0..8).map(|i| format!("later {i} no host {no_host}")));
    assert_eq!(out, expected);
}

thread_local! {
    /// The module graph of `runModule` ([`run_module_from_js`]).
    static GRAPH: std::cell::Cell<Files> = const { std::cell::Cell::new(&[]) };
}

/// `runModule(name)`: `Cx::run_module` of the file `name` of [`GRAPH`], called from JS (JS
/// frames are on the stack).
fn run_module_from_js<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let files = GRAPH.get();
    let name = cx.to_rust_string(args.get(0))?;
    let unit = compile(source(files, &name).expect("a file")).expect("compiles");
    let mut resolve = |referrer: &str, spec: &str| resolve_in(files, referrer, spec);
    let mut load = |name: &str| compile(source(files, name).expect("a resolved file"));
    cx.run_module(unit, &name, &mut resolve, &mut load)?;
    Ok(Local::undefined())
}

/// `Cx::run_module` from a host function that JS calls (RV-P3F2-1): with JS on the stack,
/// the run has no microtask checkpoint (a checkpoint needs an empty JS stack). A module with
/// no top-level `await` runs and the call returns; the jobs that it queued run in the
/// caller's checkpoint, after the jobs queued before them. A top-level `await` that is
/// still pending after Evaluate throws the checkpoint's Error, and the module goes on in the
/// caller's checkpoint (the behaviour before C-TLA2). Before the fix, the call of `p.js`
/// threw that Error after the module body ran (`caught a microtask checkpoint ...`).
#[test]
fn run_module_with_js_on_the_stack_has_no_checkpoint() {
    const FILES: Files = &[
        (
            "p.js",
            "Promise.resolve().then(() => print('p job'));\nprint('p body');",
        ),
        (
            "t.js",
            "print('t start');\nawait 0;\nprint('t after await');",
        ),
    ];
    const SCRIPT: &str = "Promise.resolve().then(() => print('script job'));\ntry { runModule('p.js'); print('p returned'); } catch (e) { print('caught', e.message); }\ntry { runModule('t.js'); print('t returned'); } catch (e) { print('t caught', e.message); }\nprint('script end');";
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || {
                let mut iso = new_isolate(d, stress);
                iso.register_host_function("runModule", 1, run_module_from_js);
                GRAPH.set(FILES);
                let unit = hook_script(SCRIPT).expect("compiles");
                let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|_| ()));
                if let Err(e) = r {
                    OUT.with(|o| o.borrow_mut().push(format!("uncaught {}", e.message)));
                }
                let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
                iso.collect_garbage();
                iso.verify_heap().expect("a valid heap");
                out
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    assert_eq!(
        first.expect("at least one dispatch").1,
        lines(&[
            "p body",
            "p returned",
            "t start",
            "t caught a microtask checkpoint needs an empty JS stack",
            "script end",
            "script job",
            "p job",
            "t after await",
        ])
    );
}

/// A function made by `Function()` in a promise job has a null ScriptOrModule (no script or
/// module code runs in the job). GetActiveScriptOrModule (§9.4.1) skips a null component, so
/// its `import('./dep.js')` called from `sub/main.js` resolves from `sub/main.js` (C2-M3).
/// Before the fix, ep9 stopped at that frame with an empty referrer and loaded the root
/// `dep.js` (the CLI threw "... imported from "); 2178586 printed `sub`. Node v24.22.1
/// throws ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING (V8 gives the function no referrer), so the
/// expected line follows the spec.
#[test]
fn a_null_script_or_module_is_skipped_for_the_caller() {
    const FILES: Files = &[
        ("dep.js", "export const where = 'root';"),
        ("sub/dep.js", "export const where = 'sub';"),
        (
            "sub/main.js",
            "const f = await Promise.resolve(\"return import('./dep.js')\").then(Function);\nprint((await f()).where);",
        ),
    ];
    assert_eq!(run_every(FILES, &["sub/main.js"]), lines(&["sub"]));
}

/// The module graph of [`a_function_keeps_its_referrer_realm_alive`]: `maker.js` runs in a
/// new realm and imports `dep.js`, which counts its evaluations in `sink.evals` (`sink` is
/// the main global).
const KEEP_FILES: Files = &[
    (
        "maker.js",
        "import './dep.js';\nsink.f = new sink.Function(\"return import('./dep.js')\");",
    ),
    (
        "dep.js",
        "sink.evals = (sink.evals | 0) + 1;\nexport function g() { return globalThis; }",
    ),
];

/// [`KEEP_FILES`] with a `sink.f` whose body drops `sink.f` (its only reference) first, then
/// allocates (under GC stress, collections run while its frame is the only holder of the
/// realm), then imports `dep.js`.
const FRAME_FILES: Files = &[
    (
        "maker.js",
        "import './dep.js';\nsink.f = new sink.Function(\"delete sink.f; let a = []; for (let i = 0; i < 50; i++) a.push({ i }); return import('./dep.js')\");",
    ),
    (
        "dep.js",
        "sink.evals = (sink.evals | 0) + 1;\nexport function g() { return globalThis; }",
    ),
];

/// Runs `maker.js` of `files` ([`KEEP_FILES`], [`FRAME_FILES`]) in a new realm and drops
/// every reference to that realm but the one of `sink.f` (its ScriptOrModule `maker.js`).
#[inline(never)]
fn make_cross_realm_importer(iso: &mut Isolate, files: Files) {
    iso.with_cx(|cx| {
        let sink = cx.global();
        let other = cx.create_realm()?;
        cx.set(other, "sink", sink, true)?;
        cx.with_realm(other, |cx| {
            let unit = compile(source(files, "maker.js").expect("a file")).expect("compiles");
            let mut resolve = |referrer: &str, spec: &str| resolve_in(files, referrer, spec);
            let mut load = |name: &str| compile(source(files, name).expect("a file"));
            cx.run_module(unit, "maker.js", &mut resolve, &mut load)
                .map(|_| ())
        })
    })
    .expect("no throw");
}

/// A function made by eval code keeps its ScriptOrModule, and so the realm of that module,
/// alive (C2-M1): `sink.f` is a main-realm function made by `maker.js` of another realm, so
/// its `import('./dep.js')` gives the `dep.js` of that realm's module map (`g()` returns
/// that realm's global), also after the realm has no other reference and collections ran.
/// `dep.js` loads and evaluates once. Before the fix, the realm was held only by a weak
/// handle: after a collection the import used the main realm's map, and a second `dep.js`
/// loaded and ran (`after true`, `evals 2`). The realm is held by an edge that the GC traces
/// from each function of the eval code (C3-M1): without that edge this test fails the same
/// way.
#[test]
fn a_function_keeps_its_referrer_realm_alive() {
    const SCRIPT: &str = "f().then(ns => print(tag, ns.g() === globalThis, sink.evals));";
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || {
                let mut iso = new_isolate(d, stress);
                iso.with_cx(|cx| {
                    let g = cx.global();
                    cx.set(g, "sink", g, true)
                })
                .expect("no throw");
                make_cross_realm_importer(&mut iso, KEEP_FILES);
                let mut resolve =
                    |referrer: &str, spec: &str| resolve_in(KEEP_FILES, referrer, spec);
                let mut loads = 0;
                let mut load = |name: &str| {
                    loads += 1;
                    compile(source(KEEP_FILES, name).expect("a file"))
                };
                iso.with_module_host(&mut resolve, &mut load, |iso| {
                    for tag in ["before", "after"] {
                        let src = format!("var tag = '{tag}';\n{SCRIPT}");
                        let unit = hook_script(&src).expect("compiles");
                        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|_| ()))
                            .expect("no throw");
                        for _ in 0..3 {
                            iso.collect_garbage();
                        }
                    }
                });
                let mut out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
                out.push(format!("loads {loads}"));
                iso.verify_heap().expect("a valid heap");
                out
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    assert_eq!(
        first.expect("at least one dispatch").1,
        lines(&["before false 1", "after false 1", "loads 0"])
    );
}

/// A running frame of eval code keeps its referrer realm alive when no reference to its
/// function is left (C3-M1): `sink.f` of [`FRAME_FILES`] deletes `sink.f`, allocates (GC
/// stress collects there), then imports `dep.js`. The import gives the `dep.js` of the
/// maker's realm, loaded and evaluated once. The frame's callee slot (a VM stack root) marks
/// the function, and the function marks the realm.
#[test]
fn a_running_frame_keeps_its_referrer_realm_alive() {
    const SCRIPT: &str =
        "sink.f().then(ns => print(ns.g() === globalThis, sink.evals, typeof sink.f));";
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || {
                let mut iso = new_isolate(d, stress);
                iso.with_cx(|cx| {
                    let g = cx.global();
                    cx.set(g, "sink", g, true)
                })
                .expect("no throw");
                make_cross_realm_importer(&mut iso, FRAME_FILES);
                let mut resolve =
                    |referrer: &str, spec: &str| resolve_in(FRAME_FILES, referrer, spec);
                let mut loads = 0;
                let mut load = |name: &str| {
                    loads += 1;
                    compile(source(FRAME_FILES, name).expect("a file"))
                };
                iso.with_module_host(&mut resolve, &mut load, |iso| {
                    let unit = hook_script(SCRIPT).expect("compiles");
                    iso.with_cx_checkpoint(|cx| cx.run_script(unit, "main.js").map(|_| ()))
                        .expect("no throw");
                });
                let mut out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
                out.push(format!("loads {loads}"));
                iso.verify_heap().expect("a valid heap");
                out
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    assert_eq!(
        first.expect("at least one dispatch").1,
        lines(&["false 1 undefined", "loads 0"])
    );
}

/// Deletes `sink.f` of [`make_cross_realm_importer`]: its realm then has no reference.
#[inline(never)]
fn drop_cross_realm_importer(iso: &mut Isolate) {
    iso.with_cx(|cx| cx.evaluate_script("delete sink.f;", "drop.js").map(|_| ()))
        .expect("no throw");
}

/// A referrer realm dies with the last function of the eval code (C3-M1): after the last
/// reference to `sink.f` goes, a collection frees the maker's realm, its globals and its
/// module map. Each round makes a new realm with [`KEEP_FILES`], drops `sink.f` and
/// collects; the marked bytes stay at the value of the first round (the slack allows one
/// realm that a stale word of the conservative stack scan keeps). With a strong handle per
/// unit (f5149b1) each round kept its realm: about 80 KB more per round.
#[test]
fn a_referrer_realm_dies_with_its_last_function() {
    const ROUNDS: usize = 6;
    const SLACK: usize = 40_000;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let marked = common::on_big_stack(move || {
                let mut iso = new_isolate(d, stress);
                iso.with_cx(|cx| {
                    let g = cx.global();
                    cx.set(g, "sink", g, true)
                })
                .expect("no throw");
                let mut marked = Vec::new();
                for _ in 0..ROUNDS {
                    make_cross_realm_importer(&mut iso, KEEP_FILES);
                    drop_cross_realm_importer(&mut iso);
                    for _ in 0..3 {
                        iso.collect_garbage();
                    }
                    marked.push(iso.gc_stats().last_marked_bytes);
                }
                iso.verify_heap().expect("a valid heap");
                marked
            });
            let label = format!("{} stress={stress}", d.name());
            assert!(
                marked[ROUNDS - 1] <= marked[0] + SLACK,
                "{label}: dead realms stay marked: {marked:?}"
            );
        }
    }
}
