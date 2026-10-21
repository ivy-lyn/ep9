//! `Function.prototype.call`, `apply` and `bind` (§20.2.3), and the bound function exotic
//! objects that `bind` makes (§10.4.1): their `[[Call]]` and `[[Construct]]` from bytecode
//! (`slow::call`) and from Rust (`call::call_with`, `call::construct_with`). Every JS check runs
//! with and without GC stress (`common::run_both`).

mod common;

use common::run_both;
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::Local;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn the_three_methods_are_on_function_prototype() {
    assert_eq!(
        run_both(
            "var p = Function.prototype;
             print(p.call.name, p.call.length, p.apply.name, p.apply.length, p.bind.name, p.bind.length);
             var d = Object.getOwnPropertyDescriptor(p, 'call');
             print(d.writable, d.enumerable, d.configurable, typeof p.call);
             print(Object.getOwnPropertyNames(p).join());"
        ),
        lines(&[
            "call 1 apply 2 bind 1",
            "true false true function",
            "length,name,constructor,apply,bind,call,toString",
        ])
    );
}

#[test]
fn call_passes_this_and_forwards_the_rest() {
    assert_eq!(
        run_both(
            "'use strict';
             function f(a, b, c) { return [this, a, b, c, arguments.length].join(); }
             print(f.call('t', 1, 2, 3));
             print(f.call(undefined), f.call());
             print(Function.prototype.call.call(f, 'x', 9));
             var o = {m: function () { return this === o; }};
             print(o.m.call(o), Array.prototype.join.call([1, 2], '+'));"
        ),
        lines(&["t,1,2,3,3", ",,,,0 ,,,,0", "x,9,,,1", "true 1+2"])
    );
}

#[test]
fn call_and_apply_on_a_non_callable_throw_a_type_error() {
    for src in [
        "Function.prototype.call.call({})",
        "Function.prototype.call.call(1)",
        "Function.prototype.apply.call({}, null, [])",
        "Function.prototype.bind.call('f')",
        "var f = Function.prototype.call; f()",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

#[test]
fn apply_reads_an_array_like() {
    assert_eq!(
        run_both(
            "'use strict';
             function f() { return [this, arguments.length, Array.prototype.join.call(arguments, '/')].join(); }
             print(f.apply('t', [1, 2, 3]));
             print(f.apply('t', {length: 2, 0: 'a', 1: 'b', 2: 'c'}));
             print(f.apply('t', {length: 2.9, 1: 'z'}));
             print(f.apply('t', null), f.apply('t', undefined), f.apply('t'));
             print(f.apply('t', {}), f.apply('t', {length: -5}), f.apply('t', 'ignored' && []));"
        ),
        lines(&[
            "t,3,1/2/3",
            "t,2,a/b",
            "t,2,/z",
            "t,0, t,0, t,0,",
            "t,0, t,0, t,0,",
        ])
    );
    let r = run_both("(function () {}).apply(null, 1)");
    assert_eq!(
        r,
        Err("TypeError: CreateListFromArrayLike called on non-object".to_string())
    );
}

#[test]
fn apply_with_many_arguments_and_a_huge_length() {
    assert_eq!(
        run_both(
            "var a = []; for (var i = 0; i < 1000; i++) a.push(i);
             function count() { return arguments.length + ':' + arguments[999]; }
             print(count.apply(null, a));"
        ),
        lines(&["1000:999"])
    );
    // CreateListFromArrayLike stops at the VM stack limit (2^20 slots) with a RangeError.
    // Without GC stress: under stress every element read would collect.
    let r = common::on_big_stack(|| {
        common::run("(function () {}).apply(null, {length: 4294967295})", false)
    });
    assert!(
        matches!(&r, Err(m) if m.starts_with("RangeError: ")),
        "{r:?}"
    );
}

#[test]
fn bind_sets_length_and_name() {
    assert_eq!(
        run_both(
            "function f(a, b, c) {}
             var b = f.bind(null, 1);
             print(b.name, b.length, typeof b, Object.getOwnPropertyNames(b).join());
             var d = Object.getOwnPropertyDescriptor(b, 'length');
             print(d.value, d.writable, d.enumerable, d.configurable);
             d = Object.getOwnPropertyDescriptor(b, 'name');
             print(d.value, d.writable, d.enumerable, d.configurable);
             print(f.bind(null, 1, 2, 3, 4).length, f.bind().length, b.bind().name);
             print('prototype' in b, b.hasOwnProperty('prototype'), Object.prototype.toString.call(b));"
        ),
        lines(&[
            "bound f 2 function length,name",
            "2 false false true",
            "bound f false false true",
            "0 3 bound bound f",
            "false false [object Function]",
        ])
    );
}

#[test]
fn bind_length_follows_the_target_length_steps() {
    assert_eq!(
        run_both(
            "function f(a, b) {}
             var h = function () {};
             delete f.length;
             print(f.bind().length);
             Object.defineProperty(h, 'length', {value: Infinity});
             print(h.bind(null, 1).length);
             Object.defineProperty(h, 'length', {value: -Infinity});
             print(h.bind().length);
             Object.defineProperty(h, 'length', {value: '3'});
             print(h.bind().length);
             Object.defineProperty(h, 'length', {value: 2.7});
             print(h.bind().length, h.bind(0, 0, 0).length);
             Object.defineProperty(h, 'length', {value: -0});
             print(1 / h.bind().length);
             Object.defineProperty(h, 'name', {value: 12});
             print('[' + h.bind().name + ']');"
        ),
        lines(&["0", "Infinity", "0", "0", "2 0", "Infinity", "[bound ]"])
    );
}

#[test]
fn a_bound_function_calls_its_target() {
    assert_eq!(
        run_both(
            "'use strict';
             function f(a, b, c) { return [typeof this, this, a, b, c, arguments.length, new.target === undefined].join(); }
             var b = f.bind('T', 1);
             print(b(2, 3));
             print(b.call('ignored', 2), b.apply('ignored', [2, 3, 4]));
             var o = {m: b}; print(o.m(5));
             var bb = b.bind('U', 2);
             print(bb(3), bb.bind(null, 3)());
             print(Array.prototype.join.bind([1, 2])('-'));
             var pushTo = Function.prototype.call.bind(Array.prototype.push);
             var a = []; pushTo(a, 1, 2); print(a.join(), a.length);"
        ),
        lines(&[
            "string,T,1,2,3,3,true",
            "string,T,1,2,,2,true string,T,1,2,3,4,true",
            "string,T,1,5,,2,true",
            "string,T,1,2,3,3,true string,T,1,2,3,3,true",
            "1-2",
            "1,2 2",
        ])
    );
}

#[test]
fn a_sloppy_target_gets_the_global_object_for_a_null_bound_this() {
    assert_eq!(
        run_both(
            "var g = this;
             function f() { return this === g; }
             print(f.bind(null)(), f.bind(undefined)(), f.bind(1)() === false);"
        ),
        lines(&["true true true"])
    );
}

#[test]
fn new_on_a_bound_function_constructs_its_target() {
    assert_eq!(
        run_both(
            "function C(x, y) { this.x = x; this.y = y; this.nt = new.target === C; }
             C.prototype.sum = function () { return this.x + this.y; };
             var BC = C.bind({ignored: true}, 10);
             var c = new BC(5);
             print(c.x, c.y, c.nt, c.sum(), c instanceof C, c.ignored);
             var BBC = BC.bind(null, 20);
             var cc = new BBC();
             print(cc.x, cc.y, cc.nt, cc instanceof C);
             class K { constructor(v) { this.v = v; } get() { return this.v; } }
             var BK = K.bind(null, 7);
             print(new BK().get(), new BK(8).v);"
        ),
        lines(&["10 5 true 15 true undefined", "10 20 true true", "7 7"])
    );
}

#[test]
fn a_subclass_of_a_bound_function_keeps_its_new_target() {
    assert_eq!(
        run_both(
            "function P(a, b) { this.a = a; this.b = b; this.t = new.target; }
             var BP = P.bind(null, 1);
             BP.prototype = P.prototype;
             class D extends BP { constructor() { super(2); } }
             var d = new D();
             print(d.a, d.b, d.t === D, d instanceof D, d instanceof P);"
        ),
        lines(&["1 2 true true true"])
    );
    // A bound function has no `prototype`, so `extends` of it throws (§15.7.14 step 8.g).
    let r = run_both("function P() {} class D extends P.bind() {}");
    assert!(
        matches!(&r, Err(m) if m.starts_with("TypeError: ")),
        "{r:?}"
    );
}

#[test]
fn constructor_checks_follow_the_target() {
    for (src, msg) in [
        ("var b = (() => 1).bind(); new b()", "is not a constructor"),
        (
            "var b = Function.prototype.call.bind(); new b()",
            "is not a constructor",
        ),
        (
            "class K {} var b = K.bind(); b()",
            "cannot be invoked without 'new'",
        ),
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ") && m.contains(msg)),
            "{src}: {r:?}"
        );
    }
    // A bound arrow function is callable.
    assert_eq!(
        run_both("var b = ((x) => x * 2).bind(null, 21); print(b(), typeof b)"),
        lines(&["42 function"])
    );
}

#[test]
fn a_long_bind_chain_needs_no_rust_recursion() {
    // 100000 levels on an 8 MiB stack: the chain walks are loops (`bound::unwrap`,
    // `bound::chain_arg`, `bound::is_constructor`), so the depth costs no native stack.
    // Deleting each `name` keeps the names short ("bound " + the inherited "").
    let r = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            common::run(
                "function f() { return arguments.length + ':' + Array.prototype.join.call(arguments, ''); }
                 function K(x) { this.x = x; }
                 var b = f, bk = K;
                 for (var i = 0; i < 100000; i++) {
                     b = b.bind(null); delete b.name;
                     bk = bk.bind(null); delete bk.name;
                 }
                 var c = b.bind(null, 'a').bind(null, 'b').bind(null, 'c');
                 print(b(1, 2), c('d'), new bk(3).x, new bk(4) instanceof K, b.name);",
                false,
            )
        })
        .expect("a thread")
        .join()
        .expect("no stack overflow");
    // The last `name` was deleted: `b.name` is %Function.prototype%'s "".
    assert_eq!(r, lines(&["2:12 4:abcd 3 true "]));
}

#[test]
fn bound_arguments_survive_gc_points_before_the_frame_push() {
    // The bound arguments are new objects held only by the bound function. In `new BC()`
    // OrdinaryCreateFromConstructor allocates (a GC point under stress) after the chain is
    // walked and before the callee frame is written.
    assert_eq!(
        run_both(
            "function C(a, b, c) { this.s = a.v + b.v + c.v; }
             function mk(v) { return {v: v}; }
             var BC = C.bind(null, mk('x'), mk('y'));
             var f = function (a, b) { return a.v + b.v + arguments.length; };
             var bf = f.bind(null, mk('p')).bind(null, mk('q'));
             var out = [];
             for (var i = 0; i < 3; i++) { out.push(new BC(mk(i)).s, bf(mk('r')), bf.call(null), bf.apply(null, [])); }
             print(out.join());"
        ),
        lines(&["xy0,pq3,pq2,pq2,xy1,pq3,pq2,pq2,xy2,pq3,pq2,pq2"])
    );
}

/// `Cx::construct` and `Cx::call` from Rust reach a bound function's `[[Construct]]` and
/// `[[Call]]` (`call::construct_with`, `call::call_with`), with new.target rules of §10.4.1.2.
#[test]
fn rust_calls_reach_the_bound_internal_methods() {
    for stress in [false, true] {
        let r = common::on_big_stack(move || {
            let unit = ep9_frontend::compile(
                "function C(a, b) { this.a = a; this.b = b; this.nt = new.target; }
                 var BC = C.bind(null, 1);
                 function Other() {}
                 Other.prototype.tag = 'other';
                 function add(a, b) { return a + b; }
                 var badd = add.bind(null, 40);",
                SourceOpts::script(Strictness::Sloppy),
                CompileOptions::default(),
            )
            .expect("compiles");
            let mut iso = common::isolate(stress);
            iso.run_script(unit, "rust.js").map_err(|e| e.message)?;
            iso.with_cx(|cx| {
                let g = cx.global();
                let (c, bc, other, badd) = (
                    cx.get(g, "C")?,
                    cx.get(g, "BC")?,
                    cx.get(g, "Other")?,
                    cx.get(g, "badd")?,
                );
                // new.target == F: the target C.
                let o = cx.construct(bc, &[Local::int32(2)], bc)?;
                let a = cx.get(o, "a")?;
                let b = cx.get(o, "b")?;
                let nt = cx.get(o, "nt")?;
                assert_eq!((a.as_int32(), b.as_int32()), (Some(1), Some(2)));
                assert!(nt.is_same_bits(&c));
                // new.target != F: kept, and it gives the prototype.
                let o = cx.construct(bc, &[], other)?;
                let nt = cx.get(o, "nt")?;
                assert!(nt.is_same_bits(&other));
                let tag = cx.get(o, "tag")?;
                assert_eq!(cx.to_rust_string(tag)?, "other");
                // [[Call]].
                let r = cx.call(badd, Local::undefined(), &[Local::int32(2)])?;
                assert_eq!(r.as_int32(), Some(42));
                Ok(())
            })
            .map_err(|e| e.message)?;
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            Ok::<(), String>(())
        });
        assert_eq!(r, Ok(()), "stress {stress}");
    }
}

/// v0 Function.prototype.toString: the NativeFunction form for every callable (no source text
/// is kept), with the `name` when it fits the syntax; a TypeError for anything else.
#[test]
fn to_string_gives_the_native_function_form() {
    assert_eq!(
        run_both(
            "function f(a) { return a; }
             var anon = [function () {}][0];
             var o = {m: function () {}};
             print(f.toString());
             print(('' + anon) === 'function () { [native code] }', Function.prototype.toString.call(o.m));
             print(Function.prototype.toString.call(Function.prototype.call), Array.prototype.pop.toString());
             print(f.bind().toString(), Function.prototype.toString.length, Function.prototype.toString.name);
             class C {} print(C.toString(), (() => 1).toString());
             Object.defineProperty(f, 'name', {value: 'not an identifier'});
             print(f.toString());
             Object.defineProperty(f, 'name', {value: 'get x'});
             print(f.toString());"
        ),
        lines(&[
            "function f() { [native code] }",
            "true function m() { [native code] }",
            "function call() { [native code] } function pop() { [native code] }",
            "function () { [native code] } 0 toString",
            "function C() { [native code] } function () { [native code] }",
            "function () { [native code] }",
            "function get x() { [native code] }",
        ])
    );
    for src in [
        "Function.prototype.toString.call(undefined)",
        "Function.prototype.toString.call(null)",
        "Function.prototype.toString.call({})",
        "Function.prototype.toString.call('f')",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

/// A builtin's NativeFunction text names it by its `[[InitialName]]` (§20.2.3.5 step 3), not by
/// its `name` property: redefining or deleting `name` changes nothing. Table and VM builtins,
/// a table getter ("get description", §10.2.9 step 5.b), symbol-named builtins and an
/// embedder's host function. A bound function has no `[[InitialName]]`. Expected lines are
/// node 24's, except the last one: node has no host `print` (d8 prints the same line).
#[test]
fn to_string_names_a_builtin_by_its_initial_name() {
    assert_eq!(
        run_both(
            "var pop = Array.prototype.pop;
             Object.defineProperty(pop, 'name', {value: 'changed'});
             print(pop.name, pop.toString());
             var io = Array.prototype.indexOf;
             delete io.name;
             print(io.hasOwnProperty('name'), io.toString());
             Object.defineProperty(Array.prototype.join, 'name', {value: 'not an identifier'});
             print(Array.prototype.join.toString());
             var desc = Object.getOwnPropertyDescriptor(Symbol.prototype, 'description').get;
             Object.defineProperty(desc, 'name', {value: 'x'});
             print(desc.toString());
             print(Symbol.prototype[Symbol.toPrimitive].toString(), String.prototype[Symbol.iterator].toString());
             print(Function.prototype.toString(), Function.prototype.toString.call(Array));
             function f() {}
             print(f.bind().toString(), pop.bind().toString());
             delete print.name;
             print(print.toString());"
        ),
        lines(&[
            "changed function pop() { [native code] }",
            "false function indexOf() { [native code] }",
            "function join() { [native code] }",
            "function get description() { [native code] }",
            "function [Symbol.toPrimitive]() { [native code] } function [Symbol.iterator]() { [native code] }",
            "function () { [native code] } function Array() { [native code] }",
            "function () { [native code] } function () { [native code] }",
            "function print() { [native code] }",
        ])
    );
}
