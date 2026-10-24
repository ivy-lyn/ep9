//! H20 rule R5: "a builtin creates objects in its callee's realm" (H20-Q2, H22-Q3; M3-P3
//! FREEZE-PREP task 2). Two realms in one isolate, made as test262's `$262.createRealm()`
//! makes them (`tools/test262/src/exec.rs` `create_realm`, M3-P1 REALMS: `Cx::create_realm`,
//! the host functions made in the new realm, then the `$262` prelude run in it).
//!
//! [`CASES`] is a function that one realm (the caller) makes and calls with the other realm's
//! global object `C` (the callee). Each case calls a builtin of `C` (from every `ep9_builtins`
//! module with a builtin that makes an object or throws, and from the VM table) and prints
//! whose prototype the result or the thrown error has: `callee` (R5 holds), `caller` or
//! `other`. The prototypes are read as properties (`C.Array.prototype`; the iterator
//! prototypes that [`PRELUDE`] stored while that realm was current), so the check itself
//! calls no builtin of `C`. The receiver and the arguments are the caller's objects, so a
//! builtin that took its realm from them would print `caller`. Then the other realm runs the
//! same cases the other way. Two control cases call the caller's own builtins and must print
//! `caller`. With the realm switch of `call::call_host` removed (a probe, reverted), the test
//! fails: 60 of the 61 cases of the first way print `caller` (`Function(..)` takes its
//! prototype from the constructor's `prototype` property, GetPrototypeFromConstructor), and
//! the controls of the second way print `callee` (the other realm's `eval` then compiles
//! `CASES` in the caller's realm).
//!
//! The expected lines are node v24.22.1's output for the same script, where
//! `$262.createRealm()` makes a `vm.createContext` realm with `print`, and runs [`PRELUDE`] in
//! it. The script runs under every dispatch expansion, with and without GC stress (a
//! collection before every allocation, then the heap verifier). It uses only the scoped
//! embedder forms (`Isolate::with_cx`, `Cx::run_script`, `Cx::evaluate_script`; H20-Q4).

use std::cell::RefCell;

use ep9_bytecode::{EvalContext, FunctionCtorKind};
use ep9_frontend::{CodegenError, CompileError, CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, CompileFailure, CompileResult, CompilerHooks, Cx, Dispatch, HostFn, Isolate,
    IsolateConfig, JsResult, Local, PropertyDescriptor,
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

/// The host functions of every realm (test262's are `print` and `__ep9_create_realm` too).
const HOST: [(&str, u32, HostFn); 2] =
    [("print", 1, print), ("__ep9_create_realm", 0, create_realm)];

/// The `$262` object of each realm (the part of test262's prelude that this test uses), and
/// the realm's iterator prototypes, read here while the realm is current: `CASES` reads them
/// as properties, so they do not depend on the realm switch under test.
const PRELUDE: &str = "var $262 = { global: globalThis, createRealm: __ep9_create_realm };\n\
     var __r5_protos = { array: Object.getPrototypeOf([][Symbol.iterator]()), \
     string: Object.getPrototypeOf(''[Symbol.iterator]()), \
     map: Object.getPrototypeOf(new Map().entries()), \
     set: Object.getPrototypeOf(new Set().values()) };";

/// `$262.createRealm()`: a new realm (`Cx::create_realm`, every builtin table) with [`HOST`]
/// made in it and [`PRELUDE`] run in it; returns that realm's `$262`.
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let global = cx.create_realm()?;
    cx.with_realm(global, |cx| {
        for (name, length, f) in HOST {
            let func = cx.new_builtin_closure(f, name, length, Local::undefined());
            let desc = PropertyDescriptor::data(func.into(), true, false, true);
            cx.define_property_or_throw(global, name, desc)?;
        }
        cx.evaluate_script(PRELUDE, "$262 prelude")?;
        cx.get(global, "$262")
    })
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

/// The cases: called as `CASES(C)` by the caller realm, with the callee realm's global `C`.
const CASES: &str = r#"(function (C) {
  var here = globalThis;
  function proto(name, v, ctor) {
    var p = Object.getPrototypeOf(v);
    print(name, p === C[ctor].prototype ? "callee" : p === here[ctor].prototype ? "caller" : "other");
  }
  function iter(name, v, key) {
    var p = Object.getPrototypeOf(v);
    print(name, p === C.__r5_protos[key] ? "callee" : p === here.__r5_protos[key] ? "caller" : "other");
  }
  function error(name, f, ctor) {
    try { f(); print(name, "no throw"); } catch (e) {
      print(name, e instanceof C[ctor] ? "callee" : e instanceof here[ctor] ? "caller" : "other");
    }
  }
  proto("Object()", C.Object(), "Object");
  proto("Object(1)", C.Object(1), "Number");
  proto("Object(symbol)", C.Object(Symbol()), "Symbol");
  proto("Object.keys", C.Object.keys({ a: 1 }), "Array");
  var entries = C.Object.entries({ a: 1 });
  proto("Object.entries", entries, "Array");
  proto("Object.entries[0]", entries[0], "Array");
  proto("Object.getOwnPropertyNames", C.Object.getOwnPropertyNames({}), "Array");
  proto("Object.getOwnPropertyDescriptor", C.Object.getOwnPropertyDescriptor({ a: 1 }, "a"), "Object");
  proto("Object.getOwnPropertyDescriptors", C.Object.getOwnPropertyDescriptors({ a: 1 }), "Object");
  proto("Object.fromEntries", C.Object.fromEntries([["a", 1]]), "Object");
  error("Object.defineProperty", function () { C.Object.defineProperty(1, "x", {}); }, "TypeError");
  proto("Function(..)", C.Function("return 1"), "Function");
  error("Function.prototype.toString", function () { C.Function.prototype.toString.call(1); }, "TypeError");
  error("Function(syntax)", function () { C.Function("("); }, "SyntaxError");
  proto("Array.prototype.map", C.Array.prototype.map.call([1], function (x) { return x; }), "Array");
  proto("Array.prototype.concat", C.Array.prototype.concat.call([1]), "Array");
  proto("Array.prototype.slice (array-like)", C.Array.prototype.slice.call({ length: 1, 0: 1 }), "Array");
  proto("Array.prototype.toSorted", C.Array.prototype.toSorted.call([2, 1]), "Array");
  proto("Array.prototype.flat", C.Array.prototype.flat.call([[1]]), "Array");
  proto("Array.of (no constructor this)", C.Array.of.call(undefined, 1), "Array");
  proto("Array.from (no constructor this)", C.Array.from.call(undefined, [1]), "Array");
  var ai = C.Array.prototype.values.call([1]);
  iter("Array.prototype.values", ai, "array");
  proto("%ArrayIteratorPrototype%.next", ai.next(), "Object");
  iter("Array.prototype.keys", C.Array.prototype.keys.call([1]), "array");
  error("Array.prototype.reduce", function () { C.Array.prototype.reduce.call([], function () {}); }, "TypeError");
  error("Array.prototype.push (not extensible)", function () { C.Array.prototype.push.call(Object.preventExtensions({ length: 0 }), 1); }, "TypeError");
  error("Array(-1)", function () { C.Array(-1); }, "RangeError");
  error("Number.prototype.toFixed", function () { C.Number.prototype.toFixed.call(1, 200); }, "RangeError");
  error("Number.prototype.toString", function () { C.Number.prototype.toString.call("x"); }, "TypeError");
  error("Boolean.prototype.valueOf", function () { C.Boolean.prototype.valueOf.call(1); }, "TypeError");
  proto("String.prototype.split", C.String.prototype.split.call("a,b", ","), "Array");
  error("String.prototype.repeat", function () { C.String.prototype.repeat.call("a", -1); }, "RangeError");
  var si = C.String.prototype[Symbol.iterator].call("ab");
  iter("String.prototype[@@iterator]", si, "string");
  proto("%StringIteratorPrototype%.next", si.next(), "Object");
  error("Symbol.prototype.toString", function () { C.Symbol.prototype.toString.call(1); }, "TypeError");
  error("Math.max", function () { C.Math.max(Symbol()); }, "TypeError");
  error("decodeURI", function () { C.decodeURI("%"); }, "URIError");
  error("Promise.prototype.then", function () { C.Promise.prototype.then.call(1); }, "TypeError");
  var resolveFn;
  new C.Promise(function (res) { resolveFn = res; });
  proto("Promise resolving function", resolveFn, "Function");
  proto("Reflect.ownKeys", C.Reflect.ownKeys({ a: 1 }), "Array");
  proto("Reflect.getOwnPropertyDescriptor", C.Reflect.getOwnPropertyDescriptor({ a: 1 }, "a"), "Object");
  error("Reflect.apply", function () { C.Reflect.apply(1); }, "TypeError");
  error("Date.prototype.toISOString", function () { C.Date.prototype.toISOString.call(new Date(NaN)); }, "RangeError");
  error("Date.prototype.getTime", function () { C.Date.prototype.getTime.call({}); }, "TypeError");
  var mi = C.Map.prototype.entries.call(new Map([[1, 2]]));
  iter("Map.prototype.entries", mi, "map");
  var step = mi.next();
  proto("%MapIteratorPrototype%.next", step, "Object");
  proto("%MapIteratorPrototype%.next value", step.value, "Array");
  iter("Set.prototype.values", C.Set.prototype.values.call(new Set([1])), "set");
  error("Map.prototype.get", function () { C.Map.prototype.get.call({}, 1); }, "TypeError");
  var grouped = C.Map.groupBy([1], function () { return "k"; });
  proto("Map.groupBy", grouped, "Map");
  proto("Map.groupBy group", grouped.get("k"), "Array");
  var parsed = C.JSON.parse("{\"a\": [1]}");
  proto("JSON.parse object", parsed, "Object");
  proto("JSON.parse array", parsed.a, "Array");
  error("JSON.parse", function () { C.JSON.parse("{"); }, "SyntaxError");
  var cyclic = {}; cyclic.self = cyclic;
  error("JSON.stringify (cyclic)", function () { C.JSON.stringify(cyclic); }, "TypeError");
  proto("Error()", C.Error("x"), "Error");
  error("Error.prototype.toString", function () { C.Error.prototype.toString.call(1); }, "TypeError");
  proto("eval", C.eval("[]"), "Array");
  proto("async function result", C.eval("(async function () {})")(), "Promise");
  proto("async generator next", C.eval("(async function* () {})")().next(), "Promise");
  proto("generator next", C.eval("(function* () { yield 1; })")().next(), "Object");
  proto("control: Object.keys of this realm", Object.keys({}), "Array");
  error("control: JSON.parse of this realm", function () { JSON.parse("{"); }, "SyntaxError");
})"#;

/// The script: two realms, then [`CASES`] each way.
fn script() -> String {
    format!(
        "var CASES = {};\n\
         var other = $262.createRealm().global;\n\
         print('two realms', other !== globalThis, other.Array !== Array);\n\
         eval(CASES)(other);\n\
         print('--- the other way');\n\
         other.eval(CASES)(globalThis);\n",
        js_string(CASES)
    )
}

/// `s` as a JS string literal.
fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An isolate with `ep9_builtins::TABLE`, [`HOOKS`], [`HOST`] and [`PRELUDE`]. `stress`: a
/// collection before every allocation, then the heap verifier.
fn isolate(dispatch: Dispatch, stress: bool) -> Isolate {
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
    for (name, length, f) in HOST {
        iso.register_host_function(name, length, f);
    }
    iso.with_cx(|cx| cx.evaluate_script(PRELUDE, "$262 prelude").map(|_| ()))
        .expect("the prelude runs");
    iso
}

/// Runs the sloppy script `src` (scoped forms only): the printed lines, after a microtask
/// checkpoint, or the uncaught exception as "Name: message". Then collects and verifies the
/// heap.
fn run_in(src: &str, dispatch: Dispatch, stress: bool) -> Result<Vec<String>, String> {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(|e| format!("compile error: {e:?}"))?;
    let mut iso = isolate(dispatch, stress);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso
        .with_cx(|cx| cx.run_script(unit, "r5.js").map(|_| ()))
        .and_then(|()| iso.perform_microtask_checkpoint());
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|()| out).map_err(|e| e.message)
}

/// Runs `f` on a thread with a 256 MiB stack: a debug build's sibling expansion makes a real
/// call per instruction.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("a thread")
        .join()
        .expect("no panic")
}

/// The case names, in [`CASES`] order, without the two control cases.
const NAMES: &[&str] = &[
    "Object()",
    "Object(1)",
    "Object(symbol)",
    "Object.keys",
    "Object.entries",
    "Object.entries[0]",
    "Object.getOwnPropertyNames",
    "Object.getOwnPropertyDescriptor",
    "Object.getOwnPropertyDescriptors",
    "Object.fromEntries",
    "Object.defineProperty",
    "Function(..)",
    "Function.prototype.toString",
    "Function(syntax)",
    "Array.prototype.map",
    "Array.prototype.concat",
    "Array.prototype.slice (array-like)",
    "Array.prototype.toSorted",
    "Array.prototype.flat",
    "Array.of (no constructor this)",
    "Array.from (no constructor this)",
    "Array.prototype.values",
    "%ArrayIteratorPrototype%.next",
    "Array.prototype.keys",
    "Array.prototype.reduce",
    "Array.prototype.push (not extensible)",
    "Array(-1)",
    "Number.prototype.toFixed",
    "Number.prototype.toString",
    "Boolean.prototype.valueOf",
    "String.prototype.split",
    "String.prototype.repeat",
    "String.prototype[@@iterator]",
    "%StringIteratorPrototype%.next",
    "Symbol.prototype.toString",
    "Math.max",
    "decodeURI",
    "Promise.prototype.then",
    "Promise resolving function",
    "Reflect.ownKeys",
    "Reflect.getOwnPropertyDescriptor",
    "Reflect.apply",
    "Date.prototype.toISOString",
    "Date.prototype.getTime",
    "Map.prototype.entries",
    "%MapIteratorPrototype%.next",
    "%MapIteratorPrototype%.next value",
    "Set.prototype.values",
    "Map.prototype.get",
    "Map.groupBy",
    "Map.groupBy group",
    "JSON.parse object",
    "JSON.parse array",
    "JSON.parse",
    "JSON.stringify (cyclic)",
    "Error()",
    "Error.prototype.toString",
    "eval",
    "async function result",
    "async generator next",
    "generator next",
];

/// node's lines: every case `callee`, the controls `caller`, both ways.
fn expected() -> Vec<String> {
    let mut want = vec!["two realms true true".to_string()];
    for way in 0..2 {
        if way == 1 {
            want.push("--- the other way".to_string());
        }
        want.extend(NAMES.iter().map(|n| format!("{n} callee")));
        want.push("control: Object.keys of this realm caller".to_string());
        want.push("control: JSON.parse of this realm caller".to_string());
    }
    want
}

#[test]
fn a_builtin_creates_objects_in_its_callee_realm() {
    let src = script();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let s = src.clone();
            let got = on_big_stack(move || run_in(&s, d, stress));
            assert_eq!(got, Ok(expected()), "{} stress={stress}", d.name());
        }
    }
}
