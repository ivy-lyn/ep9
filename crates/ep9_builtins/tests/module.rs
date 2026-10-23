//! Modules (M3-P2 MODULE, §16.2): JS-level tests of `Cx::run_module` (the loader API) with
//! an in-memory module graph. Each graph runs on isolates with `ep9_builtins::TABLE`, under
//! every dispatch expansion, with and without GC stress (a collection before every
//! allocation, then the heap verifier); all runs must agree.
//!
//! The expected lines are node v24.22.1's output for the same files (`.mjs`, with `print`
//! as `console.log` of the arguments joined by a space), except where a test says so.

mod common;

use std::cell::RefCell;

use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, CompileFailure, Cx, Dispatch, Isolate, IsolateConfig, JsResult, Local};

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

/// A module graph: (name, source); a specifier `./name` names the file `name`.
type Files = &'static [(&'static str, &'static str)];

fn source(files: Files, name: &str) -> Option<&'static str> {
    files.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

fn compile(src: &str) -> Result<ep9_bytecode::VerifiedUnit, CompileFailure> {
    ep9_frontend::compile(src, SourceOpts::module(), CompileOptions::default()).map_err(failure)
}

/// Runs each entry of `entries` (file names of `files`) in turn with `Cx::run_module` on one
/// isolate, a microtask checkpoint after each: the printed lines, and after each entry that
/// throws, a line "uncaught Name: message". Then collects and verifies the heap.
fn run_in(files: Files, entries: &[&str], dispatch: Dispatch, stress: bool) -> Vec<String> {
    let mut iso = Isolate::new(IsolateConfig {
        heap: HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..HeapConfig::default()
        },
        dispatch,
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("print", 1, print);
    OUT.with(|o| o.borrow_mut().clear());
    for entry in entries {
        let unit = compile(source(files, entry).expect("an entry file")).expect("compiles");
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
        let r = iso.with_cx(|cx| {
            cx.run_module(unit, entry, &mut resolve, &mut load)
                .map(|_| ())
        });
        let c = iso.perform_microtask_checkpoint();
        if let Err(e) = r.and(c) {
            OUT.with(|o| o.borrow_mut().push(format!("uncaught {}", e.message)));
        }
    }
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    out
}

/// [`run_in`] under every dispatch expansion, with and without GC stress; all runs agree.
fn run_every(files: Files, entries: &'static [&'static str]) -> Vec<String> {
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run_in(files, entries, d, stress));
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
/// (ParseModule in ES2025 and test262's ambiguous-export-bindings tests), so two modules that
/// re-export it agree. Each failing graph throws before any module runs. node v24.22.1 prints
/// the same lines, except the first: it still finds `nsx` ambiguous (the older rule).
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
