//! Realms (H22-Q3, H20 R5; M3-P1 REALMS): JS-level tests with two or three realms in one
//! isolate. `createRealm()` makes a realm (`Cx::create_realm`) with `print` and `createRealm`
//! in it and returns its global object. Each script runs through BytecodeGen on isolates with
//! `ep9_builtins::TABLE` and this file's compiler hooks (`other.eval`, `other.Function`),
//! under every dispatch expansion, with and without GC stress (a collection before every
//! allocation, then the heap verifier).
//!
//! The expected lines are node v24.22.1's output for the same scripts, where `createRealm`
//! is `vm.runInContext('globalThis', vm.createContext({}))` with `print` and `createRealm`
//! copied in.

mod common;

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

/// The host functions of every realm.
const HOST: [(&str, u32, HostFn); 2] = [("print", 1, print), ("createRealm", 0, create_realm)];

/// `createRealm()`: a new realm with the host functions, made in it; returns its global.
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
        Ok(())
    })?;
    Ok(global.into())
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

/// Runs the sloppy script `src` on an isolate with the table, [`HOOKS`] and [`HOST`]: the
/// printed lines (after the microtask checkpoint), or the uncaught exception as "Name:
/// message". Then collects and verifies the heap.
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
    for (name, length, f) in HOST {
        iso.register_host_function(name, length, f);
    }
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.run_script(unit, "test.js");
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

/// [`run_in`] with the default dispatch, with and without GC stress; both must agree. For
/// scripts with long loops: a debug build's sibling expansion makes a real call per
/// instruction, so a long loop there runs out of native stack.
fn run_both(src: &'static str) -> Result<Vec<String>, String> {
    let a = common::on_big_stack(move || run_in(src, Dispatch::DEFAULT, false));
    let b = common::on_big_stack(move || run_in(src, Dispatch::DEFAULT, true));
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

/// Each value gets the prototype of the realm that made it: an object literal, an array
/// literal, a closure and its result, a TypeError thrown by bytecode and one thrown by a
/// builtin (H20 R5), `new other.Array()`, a builtin's result. The sloppy `this` of a call is
/// the callee realm's global (`other.Function("return this")()`, a sloppy function of the
/// other realm). `Reflect.construct(Array, [], nt)` where `nt.prototype` is not an object
/// takes `%Array.prototype%` of nt's realm (GetPrototypeFromConstructor, §10.1.14 step 4), as
/// does the receiver of a base constructor (OrdinaryCreateFromConstructor). A generator of the
/// other realm resumes there: its result objects are that realm's.
#[test]
fn values_get_their_realm_prototypes() {
    assert_eq!(
        run_every(
            r#"var other = createRealm();
var OP = other.Object.prototype;
print(other === globalThis, other.Object === Object, other.Array.prototype === Array.prototype);
print(Object.getPrototypeOf(other.eval('({})')) === OP, Object.getPrototypeOf(other.eval('({})')) === Object.prototype);
print(Object.getPrototypeOf(other.eval('[]')) === other.Array.prototype, other.eval('[]') instanceof Array, Array.isArray(other.eval('[]')));
var mk = other.eval('(function () { return function () { return [{}]; }; })')();
var made = mk();
print(Object.getPrototypeOf(mk) === other.Function.prototype, Object.getPrototypeOf(made) === other.Array.prototype, Object.getPrototypeOf(made[0]) === OP);
var e; try { other.eval('null.x'); } catch (x) { e = x; }
print(e instanceof other.TypeError, e instanceof TypeError, e.constructor === other.TypeError);
var e2; try { other.Object.defineProperty(1, 'x', {}); } catch (x) { e2 = x; }
print(e2 instanceof other.TypeError, e2 instanceof TypeError);
print(Object.getPrototypeOf(new other.Array()) === other.Array.prototype, Object.getPrototypeOf(other.Object.keys({ a: 1 })) === other.Array.prototype);
print(other.Function('return this')() === other, other.Function('return this')() === globalThis);
print(other.eval('(function () { return this; })')() === other, (function () { return this; })() === globalThis);
var nt = new other.Function();
nt.prototype = 1;
var a = Reflect.construct(Array, [], nt);
print(Object.getPrototypeOf(a) === other.Array.prototype, Array.isArray(a));
var o2 = Reflect.construct(function () {}, [], nt);
print(Object.getPrototypeOf(o2) === OP);
var g = other.eval('(function* () { yield {}; })');
var it = g(), step = it.next();
print(Object.getPrototypeOf(step) === OP, Object.getPrototypeOf(step.value) === OP);"#
        ),
        lines(&[
            "false false false",
            "true false",
            "true false true",
            "true true true",
            "true false true",
            "true false",
            "true true",
            "true false",
            "true true",
            "true true",
            "true",
            "true true",
        ])
    );
}

/// The same source text runs in two realms after IC warm-up (12 rounds of 20 object and array
/// literals; the feedback vector comes at the 8th call): each realm's objects get its own
/// prototypes. One call site calls functions of both realms in turn (T0's `call_fast` sends the cross-realm ones cold, `slow::call` runs
/// them in their realm). A third realm made from the second; a call chain that crosses three
/// realms; promise jobs of another realm's promise.
#[test]
fn ics_and_call_sites_across_realms() {
    assert_eq!(
        run_both(
            r#"var other = createRealm();
var src = '(function () { var r = []; for (var i = 0; i < 20; i++) r.push({ i: i }, [i]); return r; })';
var mkMain = eval(src), mkOther = other.eval(src);
var ok = true;
for (var n = 0; n < 12; n++) {
  var r1 = mkMain(), r2 = mkOther();
  for (var i = 0; i < r1.length; i += 2) {
    ok = ok && Object.getPrototypeOf(r1[i]) === Object.prototype && Object.getPrototypeOf(r1[i + 1]) === Array.prototype;
    ok = ok && Object.getPrototypeOf(r2[i]) === other.Object.prototype && Object.getPrototypeOf(r2[i + 1]) === other.Array.prototype;
  }
}
print('same code, two realms', ok);
function call(f) { return f(); }
var objOther = other.eval('(function () { return {}; })');
var objMain = function () { return {}; };
var mixed = true;
for (var j = 0; j < 60; j++) {
  var f = j % 3 ? objMain : objOther;
  var want = j % 3 ? Object.prototype : other.Object.prototype;
  mixed = mixed && Object.getPrototypeOf(call(f)) === want;
}
print('one call site, two realms', mixed);
var third = other.createRealm();
print(third === other, third.Object === other.Object, Object.getPrototypeOf(third.eval('[]')) === third.Array.prototype);
var depth = other.eval('(function (f) { return f(); })');
print(Object.getPrototypeOf(depth(function () { return third.eval('({})'); })) === third.Object.prototype);
var log = [];
other.Promise.resolve(1).then(function (v) { log.push('main then ' + v); print(log.join()); });
other.eval('Promise.resolve(2)').then(function (v) { print('other promise', v); });"#
        ),
        lines(&[
            "same code, two realms true",
            "one call site, two realms true",
            "false false true",
            "true",
            "main then 1",
            "other promise 2",
        ])
    );
}

/// A throw in another realm unwinds through the realm switch: the caller's realm is current
/// again in the `catch` (its literals and errors are its own), also after a throw from a
/// builtin of the other realm, a class constructor called without `new`, and a deep chain.
#[test]
fn a_throw_switches_back() {
    assert_eq!(
        run_every(
            r#"var other = createRealm();
function check(tag) { print(tag, Object.getPrototypeOf({}) === Object.prototype, Object.getPrototypeOf([]) === Array.prototype); }
try { other.eval('throw new Error("x")'); } catch (e) { check('eval throw'); print(e instanceof other.Error); }
try { other.JSON_missing.x; } catch (e) { check('main throw'); print(e instanceof TypeError); }
try { other.Object.defineProperty(1, 'x', {}); } catch (e) { check('builtin throw'); }
var C = other.eval('(class C {})');
try { C(); } catch (e) { check('class call'); print(e instanceof other.TypeError, e instanceof TypeError); }
var deep = other.eval('(function deep(n, f) { return n === 0 ? f() : deep(n - 1, f); })');
try { deep(50, function () { throw 7; }); } catch (e) { check('deep'); print(e); }
var c = new C();
print(Object.getPrototypeOf(c) === C.prototype, c instanceof other.Object, c instanceof Object);"#
        ),
        lines(&[
            "eval throw true true",
            "true",
            "main throw true true",
            "true",
            "builtin throw true true",
            "class call true true",
            "true false",
            "deep true true",
            "7",
            "true true false",
        ])
    );
}

/// Map, Set, WeakMap and WeakSet used from two realms (COLL-1): an object has one identity
/// hash for the whole isolate (the hash key is an isolate-wide private name, not a property of
/// a realm's `%Object.prototype%`). The methods of one realm find the keys that the other
/// realm's methods added, also after a rehash (40 keys grow a table past its first
/// capacities).
#[test]
fn collections_hash_keys_across_realms() {
    assert_eq!(
        run_every(
            r#"var other = createRealm();
var o = {}, p = {};
var m = new Map();
m.set(o, 1);
print('map', other.Map.prototype.get.call(m, o), other.Map.prototype.has.call(m, o));
other.Map.prototype.set.call(m, o, 2);
print('map size', m.size, m.get(o));
var om = new other.Map();
om.set(p, 'p');
print('other map', om.get(p), Map.prototype.get.call(om, p));
var s = new other.Set();
s.add(o);
print('set', s.has(o), Set.prototype.has.call(s, o), new Set([o]).has(o));
var keys = [];
for (var i = 0; i < 3; i++) keys.push({ i: i });
var wm = new WeakMap();
for (var i = 0; i < 3; i++) wm.set(keys[i], i);
for (var i = 0; i < 40; i++) { var k = { j: i }; keys.push(k); other.WeakMap.prototype.set.call(wm, k, i); }
var ok = true;
for (var i = 0; i < keys.length; i++) ok = ok && wm.get(keys[i]) === (i < 3 ? i : i - 3) && other.WeakMap.prototype.has.call(wm, keys[i]);
print('weakmap', ok);
var ws = new other.WeakSet();
for (var i = 0; i < keys.length; i++) WeakSet.prototype.add.call(ws, keys[i]);
var ok2 = true;
for (var i = 0; i < keys.length; i++) ok2 = ok2 && ws.has(keys[i]);
print('weakset', ok2);
var big = new Map();
for (var i = 0; i < 40; i++) { if (i % 2) big.set(keys[i], i); else other.Map.prototype.set.call(big, keys[i], i); }
var ok3 = big.size === 40;
for (var i = 0; i < 40; i++) ok3 = ok3 && big.get(keys[i]) === i && other.Map.prototype.get.call(big, keys[i]) === i;
print('rehash', ok3);
var od = other.eval('({})');
var m2 = new Map([[od, 'x']]);
print('other object key', other.eval('(function (m, k) { return m.get(k); })')(m2, od));"#
        ),
        lines(&[
            "map 1 true",
            "map size 1 2",
            "other map p p",
            "set true true true",
            "weakmap true",
            "weakset true",
            "rehash true",
            "other object key x",
        ])
    );
}
