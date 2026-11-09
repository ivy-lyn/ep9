//! M7-P4 SPEC-BUILTINS4: JSON modules (ES2025, `import ... with { type: "json" }`) and text
//! modules (the import text proposal, `type: "text"`), through the text hook of
//! `Cx::run_module_with_text` and `Cx::with_module_text_host`. Each graph runs in memory on
//! isolates with `ep9_builtins::TABLE`, under every dispatch expansion, with and without GC
//! stress (a collection before every allocation, then the heap verifier); all runs must
//! agree.
//!
//! The expected lines are node v26.13.0's output for the same files (`.mjs`, with
//! `--experimental-import-text` for the text modules, and `print` as `console.log` of the
//! arguments joined by a space), except where a test says so.

mod common;

use std::cell::{Cell, RefCell};

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

/// `createRealm()`: the global object of a new realm (`Cx::create_realm`).
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.create_realm()?.into())
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

fn compile(src: &str) -> CompileResult {
    ep9_frontend::compile(src, SourceOpts::module(), CompileOptions::default()).map_err(failure)
}

fn compile_script(src: &str) -> CompileResult {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(failure)
}

/// `./name` names the file `name` of `files`; another name is node's error text.
fn resolve_in(files: Files, referrer: &str, spec: &str) -> Result<String, String> {
    let name = spec.strip_prefix("./").unwrap_or(spec);
    match source(files, name) {
        Some(_) => Ok(name.to_string()),
        None => Err(format!(
            "Cannot find module '{spec}' imported from {referrer}"
        )),
    }
}

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
    iso.register_host_function("createRealm", 0, create_realm);
    OUT.with(|o| o.borrow_mut().clear());
    iso
}

/// How [`run_in`] gives the text hook.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Host {
    /// `Cx::run_module_with_text`: the text hook is in place.
    Text,
    /// `Cx::run_module`: no text hook.
    NoText,
}

/// Runs each entry of `entries` (module file names of `files`) in turn on one isolate, a
/// microtask checkpoint after each: the printed lines, and after each entry that throws, a
/// line "uncaught <constructor>". Then one line "text hook: <calls>" (the (name, type) pairs
/// that the text hook got, in order), a collection and the heap verifier.
fn run_in(
    files: Files,
    entries: &[&str],
    host: Host,
    dispatch: Dispatch,
    stress: bool,
) -> Vec<String> {
    let mut iso = new_isolate(dispatch, stress);
    let calls = RefCell::new(Vec::new());
    for entry in entries {
        let unit = compile(source(files, entry).expect("an entry file")).expect("compiles");
        let mut resolve = |referrer: &str, spec: &str| resolve_in(files, referrer, spec);
        let mut load = |name: &str| compile(source(files, name).expect("a resolved file"));
        let mut text = |name: &str, ty: &str| {
            calls.borrow_mut().push(format!("{name} {ty}"));
            Ok(source(files, name).expect("a resolved file").to_string())
        };
        let r = iso.with_cx(|cx| {
            match host {
                Host::Text => {
                    cx.run_module_with_text(unit, entry, &mut resolve, &mut load, &mut text)
                }
                Host::NoText => cx.run_module(unit, entry, &mut resolve, &mut load),
            }
            .map(|_| ())
        });
        let c = iso.perform_microtask_checkpoint();
        if let Err(e) = r.and(c) {
            let name = e.constructor.clone().unwrap_or_else(|| "none".into());
            OUT.with(|o| {
                o.borrow_mut()
                    .push(format!("uncaught {name}: {}", e.message))
            });
        }
    }
    OUT.with(|o| {
        o.borrow_mut()
            .push(format!("text hook: {}", calls.borrow().join(", ")))
    });
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    out
}

/// [`run_in`] under every dispatch expansion, with and without GC stress; all runs agree.
fn run_every(files: Files, entries: &'static [&'static str], host: Host) -> Vec<String> {
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = common::on_big_stack(move || run_in(files, entries, host, d, stress));
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

/// The JSON and text files of the graphs below. `d.json` has a `__proto__` key (an own
/// property in JSON.parse, unlike an object literal) and a U+2028 escape.
const DATA: &[(&str, &str)] = &[
    (
        "d.json",
        r#"{"a": [1, 2], "s": "x y", "__proto__": {"p": 1}, "n": null}"#,
    ),
    ("a.json", r#" [1, "two", {}, []] "#),
    ("n.json", "-1.5e2"),
    ("t.txt", "line1\nline2 \u{e9}"),
    ("e.json", r#"{"k": true}"#),
];

/// JSON modules and text modules: the default export is the JSON.parse value (ordinary,
/// extensible objects of the realm, a `__proto__` own key) or the text; one module per name
/// and type, so every import of `d.json` as JSON gets the same object, also `import()` and
/// another module's import; a file is a JavaScript module and a text module at once
/// (`main.js` imports itself as text); the namespace has only `default`. A change of
/// `JSON.parse` does not change how a JSON module parses. The text hook runs once per name
/// and type.
#[test]
fn json_and_text_modules_give_their_default_export() {
    const FILES: Files = &[
        (
            "main.js",
            r#"import obj from './d.json' with { type: 'json' };
import { default as obj2 } from './d.json' with { type: 'json' };
import * as ns from './d.json' with { type: 'json' };
import arr from './a.json' with { type: 'json' };
import num from './n.json' with { type: 'json' };
import txt from './t.txt' with { type: 'text' };
import self from './main.js' with { type: 'text' };
import './side.js';
print(obj === obj2, ns.default === obj, Object.keys(ns).join(), Object.prototype.toString.call(ns));
print(Object.getPrototypeOf(obj) === Object.prototype, Object.isExtensible(obj), Object.hasOwn(obj, '__proto__'), JSON.stringify(obj));
print(Array.isArray(arr), Object.getPrototypeOf(arr) === Array.prototype, arr.length, num, typeof num);
print(JSON.stringify(txt), typeof self, self.startsWith('import obj'));
print(globalThis.fromSide === obj);
JSON.parse = () => 42;
const d = await import('./d.json', { with: { type: 'json' } });
print(d.default === obj, d === ns);
const t = await import('./t.txt', { with: { type: 'text' } });
print(t.default === txt);
const e = await import('./e.json', { with: { type: 'json' } });
print(JSON.stringify(e.default));
"#,
        ),
        (
            "side.js",
            "import v from './d.json' with { type: 'json' };\nglobalThis.fromSide = v;\n",
        ),
        DATA[0],
        DATA[1],
        DATA[2],
        DATA[3],
        DATA[4],
    ];
    assert_eq!(
        run_every(FILES, &["main.js"], Host::Text),
        lines(&[
            "true true default [object Module]",
            "true true true {\"a\":[1,2],\"s\":\"x\u{2028}y\",\"__proto__\":{\"p\":1},\"n\":null}",
            "true true 4 -150 number",
            "\"line1\\nline2 \u{e9}\" string true",
            "true",
            "true true",
            "true",
            "{\"k\":true}",
            // Not node output: the calls of the text hook.
            "text hook: d.json json, a.json json, n.json json, t.txt text, main.js text, e.json json",
        ])
    );
}

/// Load errors (node: the same error constructors). Text that is not JSON is a SyntaxError of
/// the load, before any module code runs, and leaves no module: the next run asks the host
/// again. A named import of a JSON module is a SyntaxError of Link (it exports only
/// `default`).
#[test]
fn bad_json_and_named_imports_throw_syntax_errors() {
    const FILES: Files = &[
        (
            "bad.js",
            "print('not run');\nimport v from './bad.json' with { type: 'json' };\n",
        ),
        (
            "named.js",
            "print('not run');\nimport { a } from './d.json' with { type: 'json' };\n",
        ),
        ("bad.json", "{\n  notJson: 0\n}"),
        DATA[0],
    ];
    let out = run_every(FILES, &["bad.js", "bad.js", "named.js"], Host::Text);
    let names: Vec<String> = out
        .iter()
        .map(|l| l.split(':').next().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        names,
        lines(&[
            "uncaught SyntaxError",
            "uncaught SyntaxError",
            "uncaught SyntaxError",
            "text hook"
        ]),
        "{out:?}"
    );
    assert!(
        out[2].contains("does not provide an export named 'a'"),
        "{out:?}"
    );
    assert_eq!(
        out[3], "text hook: bad.json json, bad.json json, d.json json",
        "a failed load is asked again"
    );
}

/// With no text hook (`Cx::run_module`), the old answers stay: `type: "json"` is the v0
/// not-implemented Error, `type: "text"` a TypeError; the hook is never asked.
#[test]
fn without_a_text_hook_json_and_text_are_refused() {
    const FILES: Files = &[
        ("j.js", "import v from './d.json' with { type: 'json' };\n"),
        ("t.js", "import v from './t.txt' with { type: 'text' };\n"),
        (
            "dyn.js",
            "import('./d.json', { with: { type: 'json' } }).then(null, e => print(e.constructor.name, e.message));\n",
        ),
        DATA[0],
        DATA[3],
    ];
    assert_eq!(
        run_every(FILES, &["j.js", "t.js", "dyn.js"], Host::NoText),
        lines(&[
            "uncaught Error: Error: not implemented in ep9 v0: JSON modules",
            "uncaught TypeError: TypeError: Import attribute type \"text\" is not supported",
            "Error not implemented in ep9 v0: JSON modules",
            "text hook: ",
        ])
    );
}

/// A JSON module belongs to the realm of its referrer (the module map that holds it), and
/// its objects get that realm's prototypes, also when code of another realm runs the
/// `import()`. Not node output (node has no `createRealm`): the referrer of a `Function`
/// made by main.js is main.js (`ep9_builtins/tests/module.rs`, the realm case).
#[test]
fn a_json_module_is_made_in_the_realm_of_its_module_map() {
    const FILES: Files = &[
        (
            "main.js",
            r#"const other = createRealm();
const m = await new other.Function("return import('./d.json', { with: { type: 'json' } })")();
print(Object.getPrototypeOf(m.default) === Object.prototype, Object.getPrototypeOf(m.default.a) === Array.prototype);
print(Object.getPrototypeOf(m.default) === other.Object.prototype);
const mine = await import('./d.json', { with: { type: 'json' } });
print(mine.default === m.default);
"#,
        ),
        DATA[0],
    ];
    assert_eq!(
        run_every(FILES, &["main.js"], Host::Text),
        lines(&["true true", "false", "true", "text hook: d.json json"])
    );
}

/// `import()` of a JSON or text module in a script: `Cx::with_module_text_host` is the
/// module host of the script run, and of the jobs of a checkpoint inside it. Outside the
/// call (`Isolate::with_module_host`, no text hook) the old answer stays.
#[test]
fn a_script_imports_json_and_text_through_the_text_host() {
    const FILES: Files = &[DATA[0], DATA[3]];
    const SCRIPT: &str = r#"
import('./d.json', { with: { type: 'json' } }).then(m => print('json', m.default.a.length));
Promise.resolve().then(() => import('./t.txt', { with: { type: 'text' } }))
    .then(m => print('text', m.default.length));
"#;
    let mut first: Option<Vec<String>> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let out = common::on_big_stack(move || {
                let mut iso = new_isolate(d, stress);
                let mut resolve = |referrer: &str, spec: &str| resolve_in(FILES, referrer, spec);
                let mut load = |name: &str| compile(source(FILES, name).expect("a file"));
                let mut text =
                    |name: &str, _ty: &str| Ok(source(FILES, name).expect("a file").to_string());
                let unit = compile_script(SCRIPT).expect("compiles");
                let r = iso.with_cx(|cx| {
                    cx.with_module_text_host(&mut resolve, &mut load, &mut text, |cx| {
                        cx.run_script(unit, "s.js")?;
                        cx.perform_microtask_checkpoint()
                    })
                });
                r.expect("no throw");
                // Outside: the Isolate host has no text hook.
                let mut resolve = |referrer: &str, spec: &str| resolve_in(FILES, referrer, spec);
                let mut load = |name: &str| compile(source(FILES, name).expect("a file"));
                let unit = compile_script(
                    "import('./d.json', { with: { type: 'json' } }).then(null, e => print(e.message));",
                )
                .expect("compiles");
                iso.with_module_host(&mut resolve, &mut load, |iso| {
                    iso.with_cx_checkpoint(|cx| cx.run_script(unit, "s2.js").map(|_| ()))
                })
                .expect("no throw");
                let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
                iso.collect_garbage();
                iso.verify_heap().expect("a valid heap");
                out
            });
            match &first {
                None => first = Some(out),
                Some(f) => assert_eq!(&out, f, "{} stress={stress}", d.name()),
            }
        }
    }
    assert_eq!(
        first.expect("a run"),
        lines(&[
            "json 2",
            "text 13",
            "not implemented in ep9 v0: JSON modules"
        ])
    );
}

/// A text hook error is an Error with its text (the load fails; no record).
#[test]
fn a_text_hook_error_is_thrown() {
    let out = common::on_big_stack(|| {
        let mut iso = new_isolate(Dispatch::DEFAULT, false);
        let asked = Cell::new(0);
        let unit = compile("import v from './x.json' with { type: 'json' };").expect("compiles");
        let r = iso.with_cx(|cx| {
            cx.run_module_with_text(
                unit,
                "main.js",
                &mut |_, s| Ok(s.trim_start_matches("./").to_string()),
                &mut |name| Err(CompileFailure::Internal(format!("no load of {name}"))),
                &mut |name, ty| {
                    asked.set(asked.get() + 1);
                    Err(format!("no {ty} file {name}"))
                },
            )
            .map(|_| ())
        });
        let e = r.expect_err("throws");
        (e.constructor, e.message, asked.get())
    });
    assert_eq!(
        out,
        (
            Some("Error".to_string()),
            "Error: no json file x.json".to_string(),
            1
        )
    );
}
