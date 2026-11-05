//! Primitive wrappers read without a throw (H20-Q5, M6-P3 H20Q). `Cx::primitive_data` gives
//! the primitive in a `[[NumberData]]`, `[[StringData]]`, `[[BooleanData]]` or (since
//! spec-temporal B4) `[[BigIntData]]` slot, else `None`; it runs no JS and never throws.
//! json.rs `primitive_data` is one call to it, so `JSON.stringify` (SerializeJSONProperty
//! step 4, the replacer list, `space`) reads the slot instead of calling
//! `this_primitive_value` and catching the TypeError (the M3-P1 B-JSON cost).
//!
//! The JSON lines are node v26.13.0's output for the same script (`print` = the arguments'
//! ToString joined by a space). Each script runs with and without GC stress (a collection
//! before every allocation, then the heap verifier).

mod common;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `JSON.stringify` of wrappers: the slot decides, not the prototype; ToNumber and ToString
/// run the wrapper's own `valueOf` and `toString` (step 4.a-c); a subclass instance has the
/// slot; a proxy of a wrapper has none and is serialized as an object (its traps: only
/// `get toJSON` and `ownKeys`); the replacer list and `space` forms; a throw from `valueOf`
/// comes out.
#[test]
fn json_stringify_reads_the_wrapper_slot() {
    assert_eq!(
        common::run_both(
            r#"
            print(JSON.stringify([new Number(3), new String("s"), new Boolean(false), Object(Symbol("q")), Object(true)]));
            print(JSON.stringify({ n: new Number(-0), s: new String(""), b: new Boolean(true) }));
            var n = new Number(5); n.toJSON = undefined; n.valueOf = function () { return 9; }; n.toString = function () { return "T"; };
            print(JSON.stringify(n), JSON.stringify([n]));
            var s = new String("abc"); s.toString = function () { return "X"; }; s.valueOf = function () { return "V"; };
            print(JSON.stringify(s), JSON.stringify({ k: s }));
            var b = new Boolean(false); b.valueOf = function () { return true; };
            print(JSON.stringify(b));
            class N extends Number {} class S extends String {} class B extends Boolean {}
            print(JSON.stringify([new N(7), new S("z"), new B(true)]));
            var fake = Object.create(Number.prototype); print(JSON.stringify(fake), JSON.stringify(Object.create(String.prototype)));
            var log = [];
            var p = new Proxy(new Number(4), { get(t, k, r) { log.push("get:" + String(k)); return Reflect.get(t, k, r); },
              ownKeys(t) { log.push("ownKeys"); return Reflect.ownKeys(t); },
              getOwnPropertyDescriptor(t, k) { log.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
              getPrototypeOf(t) { log.push("proto"); return Reflect.getPrototypeOf(t); } });
            print(JSON.stringify(p), log.join());
            log = []; print(JSON.stringify([p]), log.join());
            print(JSON.stringify({ a: 1, b: 2, c: 3 }, [new String("b"), new Number(1), "a", new String("b")]));
            var ks = new String("c"); ks.toString = function () { return "a"; };
            print(JSON.stringify({ a: 1, c: 3 }, [ks]));
            var kn = new Number(1); kn.toString = function () { return "a"; }; kn.valueOf = function () { return 7; };
            print(JSON.stringify({ a: 1, 1: 2 }, [kn]));
            print(JSON.stringify([1], null, new Number(2)), JSON.stringify([1], null, new String("--")));
            var sp = new Number(3); sp.valueOf = function () { return 1; }; sp.toString = function () { return "@@"; };
            print(JSON.stringify([1], null, sp));
            var ss = new String("ab"); ss.toString = function () { return "%%"; }; ss.valueOf = function () { return 5; };
            print(JSON.stringify([1], null, ss));
            print(JSON.stringify([1], null, new Boolean(true)), JSON.stringify([1], null, Object(Symbol())));
            var bad = new Number(1); bad.valueOf = function () { throw new Error("vo"); };
            try { JSON.stringify(bad); } catch (e) { print("throws", e.message); }
            try { JSON.stringify([1], null, bad); } catch (e) { print("throws", e.message); }
            var r = Proxy.revocable(new String("x"), {}); r.revoke();
            try { JSON.stringify(r.proxy); } catch (e) { print(e.name); }
            print(JSON.stringify(new Number(1), function (k, v) { return typeof v === "object" ? v + 1 : v; }));
            print(JSON.stringify({ x: new String("q") }, function (k, v) { return k === "x" ? new Boolean(true) : v; }));
            "#
        ),
        lines(&[
            r#"[3,"s",false,{},true]"#,
            r#"{"n":0,"s":"","b":true}"#,
            "9 [9]",
            r#""X" {"k":"X"}"#,
            "false",
            r#"[7,"z",true]"#,
            "{} {}",
            "{} get:toJSON,ownKeys",
            "[{}] get:toJSON,ownKeys",
            r#"{"b":2,"a":1}"#,
            r#"{"a":1}"#,
            r#"{"a":1}"#,
            "[\n  1\n] [\n--1\n]",
            "[\n 1\n]",
            "[\n%%1\n]",
            "[1] [1]",
            "throws vo",
            "throws vo",
            "TypeError",
            "2",
            r#"{"x":true}"#,
        ])
    );
}

/// `pd(v)`: `Cx::primitive_data(v)` as a value, or the string "none".
fn pd<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    match cx.primitive_data(args.get(0)) {
        Some(v) => Ok(v),
        None => Ok(cx.new_string("none")?.into()),
    }
}

/// Runs `src` with `print` and `pd`, then collects and verifies the heap.
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
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("print", 1, print);
    iso.register_host_function("pd", 1, pd);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
    r.map(|_| out).map_err(|e| e.message)
}

std::thread_local! {
    static OUT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
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

/// `Cx::primitive_data` on every kind of value: the three wrappers (also a subclass instance
/// and a wrapper from `Object(..)` and `Reflect.construct`) give their primitive, the wrapper's
/// own `valueOf` and `toString` are not run; a Symbol wrapper, a primitive, a plain object, an
/// object with a wrapper prototype, a function, an array, a proxy of a wrapper (no trap is
/// called) and a revoked proxy give "none". No case throws, so the script runs to its end.
#[test]
fn primitive_data_reads_the_slot_and_never_throws() {
    let src = r#"
        function show(v) { var r = pd(v); return typeof r + ":" + (typeof r === "symbol" ? r.description : String(r)); }
        var w = new Number(5); w.valueOf = function () { throw new Error("valueOf ran"); }; w.toString = w.valueOf;
        var s = new String("ab"); s.valueOf = w.valueOf; s.toString = w.valueOf;
        class N extends Number {}
        var log = [];
        var handler = {};
        ["get", "has", "ownKeys", "getOwnPropertyDescriptor", "getPrototypeOf", "apply"].forEach(function (k) {
            handler[k] = function () { log.push(k); return Reflect[k].apply(null, arguments); };
        });
        var p = new Proxy(new Number(1), handler);
        var r = Proxy.revocable(new String("x"), {}); r.revoke();
        print([w, s, new Boolean(false), new N(-0), Object(2.5), Object("q"), Reflect.construct(Boolean, [1])].map(show).join());
        print([Object(Symbol("d")), Symbol("e"), 1, "x", true, null, undefined].map(show).join());
        print([{}, Object.create(Number.prototype), Object.create(String.prototype), function () {}, [1], p, r.proxy].map(show).join());
        print(1 / pd(new Number(-0)), pd(new String("")).length, log.length);
        var bw = Object(10n ** 20n); bw.valueOf = w.valueOf; bw.toString = w.valueOf;
        print([bw, 1n, Object.create(BigInt.prototype)].map(show).join());
    "#;
    let expected = lines(&[
        "number:5,string:ab,boolean:false,number:0,number:2.5,string:q,boolean:true",
        "string:none,string:none,string:none,string:none,string:none,string:none,string:none",
        "string:none,string:none,string:none,string:none,string:none,string:none,string:none",
        "-Infinity 0 0",
        "bigint:100000000000000000000,string:none,string:none",
    ]);
    for stress in [false, true] {
        let got = common::on_big_stack(move || run(src, stress));
        assert_eq!(got, expected, "stress={stress}");
    }
}
