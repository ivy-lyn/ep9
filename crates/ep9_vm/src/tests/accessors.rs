//! Accessor properties (P3-ACC): getters and setters in `[[Get]]` and `[[Set]]` (with the
//! receiver as `this`), the runtime functions DefineAccessorInLiteral (12) and
//! DefineClassAccessor (16), `super` with a receiver, `[[DefineOwnProperty]]` with accessor
//! descriptors (ValidateAndApplyPropertyDescriptor), immutable ACCESSOR_PAIR cells, and the
//! runtime functions next to them (DefineClassMethod, InitializeGlobal, CopyDataProperties).
//!
//! Programs are hand-assembled. Each comment gives the JS a bytecode program stands for.

use ep9_bytecode::{
    FunctionFlags, FunctionId, FunctionKind, FunctionMeta, HandlerKind, Label, Register,
    RegisterList, RuntimeId, ScopeKind, VariableMode, VerifiedUnit,
};

use super::{
    Asm, config, isolate, meta, ok, on_big_stack, run_all, run_in, take_output, unit,
    unit_with_scopes,
};
use crate::runtime::Runtime;
use crate::{
    Args, Cx, Dispatch, HostFn, IntrinsicId, Isolate, JsResult, Local, PropertyDescriptor,
};

// ---- helpers ----------------------------------------------------------------------------------

/// Metadata of function `name` (child of the script) of `kind` with `params` parameters.
fn fn_meta(name: &str, kind: FunctionKind, params: u32) -> FunctionMeta {
    let mut m = meta(name, params, 0, 0, 0);
    m.kind = kind;
    m.flags = FunctionFlags::SIMPLE_PARAMETERS;
    m
}

/// [`fn_meta`] for an anonymous function (a computed key: the runtime names it).
fn anon_meta(kind: FunctionKind, params: u32) -> FunctionMeta {
    FunctionMeta {
        name: None,
        ..fn_meta("", kind, params)
    }
}

/// [`fn_meta`] for strict code.
fn strict_meta(name: &str, kind: FunctionKind, params: u32) -> FunctionMeta {
    let mut m = fn_meta(name, kind, params);
    m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    m
}

/// `acc = function #id` (a closure of unit function `id`).
fn closure(s: &mut Asm, id: u32) {
    let k = s.b.function_constant(FunctionId(id));
    s.b.create_closure(k, s.closure, 0);
}

/// `CallRuntime(id, [o, key, get, set])` with the closures of unit functions `get` and `set`
/// (0 = undefined): DefineAccessorInLiteral or DefineClassAccessor.
fn define_accessor(s: &mut Asm, id: RuntimeId, o: Register, key: &str, get: u32, set: u32) {
    let args = s.b.new_register_list(4);
    s.b.mov(o, args.get(0));
    s.lda_str(key);
    s.b.star(args.get(1));
    for (f, r) in [(get, args.get(2)), (set, args.get(3))] {
        if f == 0 {
            s.b.lda_undefined();
        } else {
            closure(s, f);
        }
        s.b.star(r);
    }
    s.b.call_runtime(id, args, 4);
}

/// `try { body } catch (e) { print(e.message) }` around what `body` emits.
fn try_print_message(s: &mut Asm, ctx: Register, body: impl FnOnce(&mut Asm)) {
    let (start, end, handler, done): (Label, Label, Label, Label) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    let e = s.a.get(2);
    s.b.star(e);
    s.get_named(e, "message");
    s.print_acc();
    s.b.bind(done);
}

/// Runs `f` with a builtin-ABI context of a fresh isolate (GC stress on or off), then checks
/// the heap.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let out = {
        let iso = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

/// A new host function object `name` for `f`.
fn host_fn<'s>(rt: &Runtime, name: &str, f: HostFn) -> Local<'s> {
    Local::from_bits(crate::realm::new_host_function(rt, f, name, 0, false))
}

/// The key atom of `name`.
fn atom(rt: &Runtime, name: &str) -> u64 {
    crate::atoms::intern_str(rt, name)
}

/// Takes the pending exception's message.
fn take_message(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let m = cx.get(e, "message").unwrap();
    cx.to_rust_string(m).unwrap()
}

// ---- [[Get]] and [[Set]] ----------------------------------------------------------------------

#[test]
fn literal_getters_and_setters_take_the_receiver_as_this() {
    // f1: get x() { return this.a + 1; }   f2: set x(v) { this.a = v; }
    // var o = {}; o.a = 1; DefineAccessorInLiteral(o, "x", f1, f2);
    // print(o.x); o.x = 5; print(o.a, o.x);
    // var c = {__proto__: o}; print(c.x); c.x = 10; print(c.x, o.x, c.a);
    fn make() -> VerifiedUnit {
        let mut g = Asm::new(fn_meta("get x", FunctionKind::Getter, 0));
        let this = g.b.receiver();
        g.get_named(this, "a");
        g.b.add_smi(1, g.bin).return_();
        let mut st = Asm::new(fn_meta("set x", FunctionKind::Setter, 1));
        let (this, v) = (st.b.receiver(), st.b.parameter(0));
        st.b.ldar(v);
        st.set_named(this, "a");
        st.b.lda_undefined().return_();

        let mut s = Asm::script(0);
        let (o, c) = (s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(o);
        s.b.lda_smi(1);
        s.set_named(o, "a");
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "x", 1, 2);
        s.get_named(o, "x");
        s.print_acc();
        s.b.lda_smi(5);
        s.set_named(o, "x");
        s.get_named(o, "a");
        s.print_acc();
        s.get_named(o, "x");
        s.print_acc();
        // c = {__proto__: o}
        let args = s.b.new_register_list(2);
        s.b.create_empty_object_literal().star(c);
        s.b.mov(c, args.get(0)).mov(o, args.get(1));
        s.b.call_runtime(RuntimeId::SetPrototypeInLiteral, args, 2);
        s.get_named(c, "x");
        s.print_acc();
        s.b.lda_smi(10);
        s.set_named(c, "x");
        s.get_named(c, "x");
        s.print_acc();
        s.get_named(o, "x");
        s.print_acc();
        s.get_named(c, "a");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish(), st.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["2", "5", "6", "6", "11", "6", "10"]);
}

#[test]
fn a_throwing_getter_unwinds() {
    // f1: get x() { throw new TypeError("from the getter"); }
    // f2: function call(o) { return o.x; }
    // try { o.x } catch (e) { print(e.message) }
    // try { call(o) } catch (e) { print(e.message) }   (one more JS frame between)
    // print(o.y)  (the isolate works after the unwind)
    // o.x         (uncaught: the run ends with the getter's error)
    fn make() -> VerifiedUnit {
        let mut g = Asm::new(fn_meta("get x", FunctionKind::Getter, 0));
        let args = g.b.new_register_list(1);
        g.lda_str("from the getter");
        g.b.star(args.get(0));
        g.construct_global("TypeError", args);
        g.b.throw();
        let mut c = Asm::new(fn_meta("call", FunctionKind::Normal, 1));
        let p = c.b.parameter(0);
        c.get_named(p, "x");
        c.b.return_();

        let mut s = Asm::script(0);
        let (o, ctx) = (s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(o);
        s.b.lda_smi(7);
        s.set_named(o, "y");
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "x", 1, 0);
        try_print_message(&mut s, ctx, |s| {
            s.get_named(o, "x");
        });
        closure(&mut s, 2);
        s.b.star(s.f);
        try_print_message(&mut s, ctx, |s| {
            s.b.call_undefined_receiver1(s.f, o, s.call);
        });
        s.get_named(o, "y");
        s.print_acc();
        s.get_named(o, "x");
        s.b.return_();
        unit("", vec![s.finish(), g.finish(), c.finish()])
    }
    let r = run_all(make);
    assert_eq!(r.output, ["from the getter", "from the getter", "7"]);
    let e = r.result.expect_err("the last o.x throws");
    assert_eq!(e.message, "TypeError: from the getter");
}

#[test]
fn strict_code_with_a_missing_setter_gets_a_type_error() {
    // f1: get x() { return 1; }
    // f2: function store(o) { "use strict"; o.x = 2; }
    // f3: function storeKeyed(o) { "use strict"; o["x"] = 2; }
    // o.x = 2 (sloppy: ignored); print(o.x);
    // try { store(o) } catch (e) { print(e.message) }; same for storeKeyed;
    // a child of o: try { store(child) } ... (the getter-only accessor is inherited)
    fn make() -> VerifiedUnit {
        let mut g = Asm::new(fn_meta("get x", FunctionKind::Getter, 0));
        g.b.lda_smi(1).return_();
        let mut st = Asm::new(strict_meta("store", FunctionKind::Normal, 1));
        let p = st.b.parameter(0);
        st.b.lda_smi(2);
        st.set_named(p, "x");
        st.b.lda_undefined().return_();
        let mut sk = Asm::new(strict_meta("storeKeyed", FunctionKind::Normal, 1));
        let (p, k) = (sk.b.parameter(0), sk.b.new_register());
        sk.lda_str("x");
        sk.b.star(k).lda_smi(2);
        sk.b.set_keyed_property(p, k, sk.set_keyed);
        sk.b.lda_undefined().return_();

        let mut s = Asm::script(0);
        let (o, ctx, child) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(o);
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "x", 1, 0);
        s.b.lda_smi(2);
        s.set_named(o, "x");
        s.get_named(o, "x");
        s.print_acc();
        let args = s.b.new_register_list(2);
        s.b.create_empty_object_literal().star(child);
        s.b.mov(child, args.get(0)).mov(o, args.get(1));
        s.b.call_runtime(RuntimeId::SetPrototypeInLiteral, args, 2);
        for (id, target) in [(2, o), (3, o), (2, child)] {
            closure(&mut s, id);
            s.b.star(s.f);
            try_print_message(&mut s, ctx, |s| {
                s.b.call_undefined_receiver1(s.f, target, s.call);
                s.lda_str("no error");
                s.print_acc();
            });
        }
        s.get_named(child, "x");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish(), st.finish(), sk.finish()])
    }
    let r = run_all(make);
    ok(&r);
    let msg = "Cannot set property x of #<Object> which has only a getter";
    assert_eq!(r.output, ["1", msg, msg, msg, "1"]);
}

#[test]
fn static_class_accessors_work() {
    // class C {
    //   static get s() { return this.tag; }   (f2)
    //   static set s(v) { this.tag = v; }     (f3)
    //   get i() { return this.n * 2; }        (f4)
    // }
    // C.tag = "C"; print(C.s); C.s = "set"; print(C.tag, C.s);
    // var c = new C(); c.n = 21; print(c.i); c.i = 1 (sloppy, no setter): print(c.i)
    fn make() -> VerifiedUnit {
        let mut ctor = Asm::new(fn_meta("C", FunctionKind::ClassConstructor, 0));
        ctor.b.lda_undefined().return_();
        let mut sg = Asm::new(strict_meta("get s", FunctionKind::Getter, 0));
        let this = sg.b.receiver();
        sg.get_named(this, "tag");
        sg.b.return_();
        let mut ss = Asm::new(strict_meta("set s", FunctionKind::Setter, 1));
        let (this, v) = (ss.b.receiver(), ss.b.parameter(0));
        ss.b.ldar(v);
        ss.set_named(this, "tag");
        ss.b.lda_undefined().return_();
        let mut ig = Asm::new(strict_meta("get i", FunctionKind::Getter, 0));
        let this = ig.b.receiver();
        ig.get_named(this, "n");
        ig.b.mul_smi(2, ig.bin).return_();

        let mut s = Asm::script(0);
        let def = s.b.new_register_list(2);
        let (cls, heritage) = (def.get(0), def.get(1));
        let (proto, c) = (s.b.new_register(), s.b.new_register());
        closure(&mut s, 1);
        s.b.star(cls).lda_the_hole().star(heritage);
        s.b.call_runtime(RuntimeId::DefineClass, def, 2).star(proto);
        define_accessor(&mut s, RuntimeId::DefineClassAccessor, cls, "s", 2, 0);
        define_accessor(&mut s, RuntimeId::DefineClassAccessor, cls, "s", 0, 3);
        define_accessor(&mut s, RuntimeId::DefineClassAccessor, proto, "i", 4, 0);
        s.b.ldar(cls);
        s.sta_global("C");
        s.lda_str("C");
        s.set_named(cls, "tag");
        s.get_named(cls, "s");
        s.print_acc();
        s.lda_str("set");
        s.set_named(cls, "s");
        s.get_named(cls, "tag");
        s.print_acc();
        s.get_named(cls, "s");
        s.print_acc();
        s.construct_global("C", RegisterList::EMPTY);
        s.b.star(c).lda_smi(21);
        s.set_named(c, "n");
        s.get_named(c, "i");
        s.print_acc();
        s.b.lda_smi(1);
        s.set_named(c, "i");
        s.get_named(c, "i");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit(
            "",
            vec![
                s.finish(),
                ctor.finish(),
                sg.finish(),
                ss.finish(),
                ig.finish(),
            ],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["C", "set", "set", "42", "42"]);
}

/// class B { get v() { return this.tag; } set v(x) { this.got = x; } }
/// class D extends B { m() { return <load>; } s(x) { super[k] = x; } }
/// var d = new D(); d.tag = "d"; print(d.m()); d.s(5); print(d.got, B.prototype.got)
///
/// `named`: `<load>` is `super.v` (opcode 0x22), else `super[k]` (LoadKeyedFromSuper).
fn super_unit(named: bool) -> VerifiedUnit {
    let mut bg = Asm::new(strict_meta("get v", FunctionKind::Getter, 0));
    let this = bg.b.receiver();
    bg.get_named(this, "tag");
    bg.b.return_();
    let mut bs = Asm::new(strict_meta("set v", FunctionKind::Setter, 1));
    let (this, x) = (bs.b.receiver(), bs.b.parameter(0));
    bs.b.ldar(x);
    bs.set_named(this, "got");
    bs.b.lda_undefined().return_();
    let mut b = Asm::new(fn_meta("B", FunctionKind::ClassConstructor, 0));
    b.b.lda_undefined().return_();
    // The derived constructor: constructor(...args) { super(...args); }, without arguments.
    let mut d = Asm::new(fn_meta("D", FunctionKind::DerivedConstructor, 0));
    let (this, sup) = (d.b.new_register(), d.b.new_register());
    d.b.lda_current_function().get_super_constructor(sup);
    d.b.throw_if_not_super_constructor(sup);
    d.b.lda_new_target()
        .construct(sup, RegisterList::EMPTY, 0, d.call);
    d.b.star(this).return_();
    // m(): the home object from the closure.
    let mut m = Asm::new(strict_meta("m", FunctionKind::Method, 0));
    let this = m.b.receiver();
    let args = m.b.new_register_list(3);
    m.b.lda_current_function().star(args.get(0));
    m.b.call_runtime(
        RuntimeId::GetHomeObject,
        RegisterList::new(args.get(0).local_index().unwrap(), 1),
        1,
    );
    if named {
        let k = m.b.string_constant("v");
        let slot = m.named;
        m.b.get_named_property_from_super(this, k, slot);
    } else {
        m.b.star(args.get(1)).mov(this, args.get(0));
        m.lda_str("v");
        m.b.star(args.get(2));
        m.b.call_runtime(RuntimeId::LoadKeyedFromSuper, args, 3);
    }
    m.b.return_();
    // s(x): super["v"] = x
    let mut st = Asm::new(strict_meta("s", FunctionKind::Method, 1));
    let (this, x) = (st.b.receiver(), st.b.parameter(0));
    let args = st.b.new_register_list(4);
    st.b.lda_current_function().star(args.get(0));
    st.b.call_runtime(
        RuntimeId::GetHomeObject,
        RegisterList::new(args.get(0).local_index().unwrap(), 1),
        1,
    );
    st.b.star(args.get(1))
        .mov(this, args.get(0))
        .mov(x, args.get(3));
    st.lda_str("v");
    st.b.star(args.get(2));
    st.b.call_runtime(RuntimeId::StoreToSuper, args, 4);
    st.b.lda_undefined().return_();

    let mut s = Asm::script(0);
    let def = s.b.new_register_list(2);
    let (cls, heritage) = (def.get(0), def.get(1));
    let (bproto, dproto, o) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    let margs = s.b.new_register_list(3);
    // class B
    closure(&mut s, 3);
    s.b.star(cls).lda_the_hole().star(heritage);
    s.b.call_runtime(RuntimeId::DefineClass, def, 2)
        .star(bproto);
    define_accessor(&mut s, RuntimeId::DefineClassAccessor, bproto, "v", 1, 2);
    s.b.ldar(cls).star(heritage);
    // class D extends B
    closure(&mut s, 4);
    s.b.star(cls);
    s.b.call_runtime(RuntimeId::DefineClass, def, 2)
        .star(dproto);
    for (name, id) in [("m", 5), ("s", 6)] {
        s.b.mov(dproto, margs.get(0));
        s.lda_str(name);
        s.b.star(margs.get(1));
        closure(&mut s, id);
        s.b.star(margs.get(2));
        s.b.call_runtime(RuntimeId::DefineClassMethod, margs, 3);
    }
    s.b.ldar(cls);
    s.sta_global("D");
    s.construct_global("D", RegisterList::EMPTY);
    s.b.star(o);
    s.lda_str("d");
    s.set_named(o, "tag");
    s.get_named(o, "m");
    s.b.star(s.f).call_property0(s.f, o, s.call);
    s.print_acc();
    s.b.lda_smi(5).star(s.a.get(0));
    s.get_named(o, "s");
    s.b.star(s.f).call_property1(s.f, o, s.a.get(0), s.call);
    s.get_named(o, "got");
    s.print_acc();
    s.get_named(bproto, "got");
    s.print_acc();
    s.b.lda_undefined().return_();
    unit(
        "",
        vec![
            s.finish(),
            bg.finish(),
            bs.finish(),
            b.finish(),
            d.finish(),
            m.finish(),
            st.finish(),
        ],
    )
}

#[test]
fn keyed_super_getters_and_setters_see_this() {
    let r = run_all(|| super_unit(false));
    ok(&r);
    assert_eq!(r.output, ["d", "5", "undefined"]);
}

#[test]
fn super_x_getters_see_this() {
    let r = run_all(|| super_unit(true));
    ok(&r);
    assert_eq!(r.output, ["d", "5", "undefined"]);
}

/// f1: get x() { n = n - 1; return n > 0 ? this.x : "bottom"; }  (global n = 10^6)
/// try { o.x } catch (e) { print(e.message) }; print(10^6 - n, n > 0); then o.y works.
fn getter_recursion() -> VerifiedUnit {
    let mut g = Asm::new(fn_meta("get x", FunctionKind::Getter, 0));
    let this = g.b.receiver();
    let bottom = g.b.new_label();
    // (Compare ops compute `register OP acc`.)
    g.b.lda_zero().star(g.f);
    g.lda_global("n");
    g.b.dec(g.bin);
    g.sta_global("n");
    g.b.test_less_than(g.f, g.cmp);
    g.b.jump_if_false(bottom);
    g.get_named(this, "x");
    g.b.return_();
    g.b.bind(bottom);
    g.lda_str("bottom");
    g.b.return_();

    let mut s = Asm::script(0);
    let (o, ctx, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    s.lda_num(1e6);
    s.sta_global("n");
    s.b.create_empty_object_literal().star(o);
    s.lda_str("alive");
    s.set_named(o, "y");
    define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "x", 1, 0);
    try_print_message(&mut s, ctx, |s| {
        s.get_named(o, "x");
        s.print_acc();
    });
    // The depth reached, and not the bottom.
    s.lda_num(1e6);
    s.b.star(t);
    s.lda_global("n");
    s.b.sub(t, s.bin);
    s.print_acc();
    s.b.lda_zero().star(t);
    s.lda_global("n");
    s.b.test_less_than(t, s.cmp);
    s.print_acc();
    s.get_named(o, "y");
    s.b.return_();
    unit("", vec![s.finish(), g.finish()])
}

#[test]
fn getter_recursion_a_million_deep_is_a_range_error() {
    // Every dispatch, with and without GC stress, on a thread with an ordinary native stack:
    // the native stack check (or the VM stack cap) stops the recursion with a catchable
    // RangeError; nothing crashes and the isolate keeps working.
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = std::thread::Builder::new()
                .stack_size(if cfg!(debug_assertions) {
                    64 << 20
                } else {
                    8 << 20
                })
                .spawn(move || run_in(config(d, stress), getter_recursion()))
                .unwrap()
                .join()
                .unwrap();
            let at = format!("{} stress={stress}", d.name());
            assert_eq!(ok(&r), "alive", "{at}");
            assert_eq!(r.output.len(), 3, "{at}");
            assert_eq!(r.output[0], "Maximum call stack size exceeded", "{at}");
            assert_eq!(r.output[2], "true", "{at}");
            // Thousands of nested getter calls in release (8 MiB: about 10 k on macOS
            // arm64); debug frames are much larger (match expansion: about 100 KiB a level).
            let depth: u32 = r.output[1]
                .parse()
                .unwrap_or_else(|_| panic!("{:?}", r.output));
            let min = if cfg!(debug_assertions) { 100 } else { 5_000 };
            assert!(depth > min, "{at}: depth {depth}");
        }
    }
}

// ---- runtime functions ------------------------------------------------------------------------

#[test]
fn accessor_functions_get_their_names() {
    // f1 (named by the front end "get x"), f2 (anonymous: a computed key), f3 (anonymous
    // setter) each return their own `name`:
    // o = {}; DefineAccessorInLiteral(o, "x", f1); DefineAccessorInLiteral(o, "k", f2);
    // DefineClassAccessor(o, "w", f2', f3)  -> "get x", "get k", "get w"; the setter of "w"
    // stores its name in o.setName.
    fn make() -> VerifiedUnit {
        let own_name = |m: FunctionMeta| {
            let mut f = Asm::new(m);
            let r = f.b.new_register();
            f.b.lda_current_function().star(r);
            f.get_named(r, "name");
            f.b.return_();
            f
        };
        let f1 = own_name(fn_meta("get x", FunctionKind::Getter, 0));
        let f2 = own_name(anon_meta(FunctionKind::Getter, 0));
        let mut f3 = Asm::new(anon_meta(FunctionKind::Setter, 1));
        let (this, r) = (f3.b.receiver(), f3.b.new_register());
        f3.b.lda_current_function().star(r);
        f3.get_named(r, "name");
        f3.set_named(this, "setName");
        f3.b.lda_undefined().return_();

        let mut s = Asm::script(0);
        let o = s.b.new_register();
        s.b.create_empty_object_literal().star(o);
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "x", 1, 0);
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, o, "k", 2, 0);
        define_accessor(&mut s, RuntimeId::DefineClassAccessor, o, "w", 2, 3);
        for k in ["x", "k", "w"] {
            s.get_named(o, k);
            s.print_acc();
        }
        s.b.lda_smi(0);
        s.set_named(o, "w");
        s.get_named(o, "setName");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), f1.finish(), f2.finish(), f3.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["get x", "get k", "get w", "set w"]);
}

#[test]
fn a_class_element_named_prototype_on_the_constructor_throws() {
    // class C {}; then DefineClassMethod(C, "prototype", f) and DefineClassAccessor(C,
    // "prototype", g): `static ['prototype']() {}` and `static get ['prototype']() {}`
    // (§10.2.8 DefinePropertyOrThrow on the non-configurable `prototype`).
    fn make() -> VerifiedUnit {
        let mut ctor = Asm::new(fn_meta("C", FunctionKind::ClassConstructor, 0));
        ctor.b.lda_undefined().return_();
        let mut f = Asm::new(anon_meta(FunctionKind::Method, 0));
        f.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let def = s.b.new_register_list(2);
        let (cls, heritage) = (def.get(0), def.get(1));
        let (ctx, p) = (s.b.new_register(), s.b.new_register());
        closure(&mut s, 1);
        s.b.star(cls).lda_the_hole().star(heritage);
        s.b.call_runtime(RuntimeId::DefineClass, def, 2).star(p);
        let margs = s.b.new_register_list(3);
        try_print_message(&mut s, ctx, |s| {
            s.b.mov(cls, margs.get(0));
            s.lda_str("prototype");
            s.b.star(margs.get(1));
            closure(s, 2);
            s.b.star(margs.get(2));
            s.b.call_runtime(RuntimeId::DefineClassMethod, margs, 3);
        });
        try_print_message(&mut s, ctx, |s| {
            define_accessor(s, RuntimeId::DefineClassAccessor, cls, "prototype", 2, 0);
        });
        // The prototype is unchanged.
        s.get_named(cls, "prototype");
        s.b.test_reference_equal(p);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), ctor.finish(), f.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "Cannot redefine property: prototype",
            "Cannot redefine property: prototype",
            "true"
        ]
    );
}

#[test]
fn copy_data_properties_runs_getters_once_in_key_order() {
    // f1: get g() { this.count = this.count + 1; return "G"; }  (on src, count on src)
    // src = {count: 0}; DefineAccessorInLiteral(src, "g", f1); src.h = 1;
    // t = {}; CopyDataProperties(t, src); print(t.g, t.h, src.count, t.count)
    // CopyDataPropertiesExcluding(src, "g") -> r; print(r.g, r.h, src.count)
    fn make() -> VerifiedUnit {
        let mut g = Asm::new(fn_meta("get g", FunctionKind::Getter, 0));
        let this = g.b.receiver();
        g.get_named(this, "count");
        g.b.add_smi(1, g.bin);
        g.set_named(this, "count");
        g.lda_str("G");
        g.b.return_();

        let mut s = Asm::script(0);
        let (src, t) = (s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(src);
        s.b.lda_zero();
        s.set_named(src, "count");
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, src, "g", 1, 0);
        s.b.lda_smi(1);
        s.set_named(src, "h");
        let args = s.b.new_register_list(2);
        s.b.create_empty_object_literal().star(t);
        s.b.mov(t, args.get(0)).mov(src, args.get(1));
        s.b.call_runtime(RuntimeId::CopyDataProperties, args, 2);
        for (o, k) in [(t, "g"), (t, "h"), (src, "count"), (t, "count")] {
            s.get_named(o, k);
            s.print_acc();
        }
        s.b.mov(src, args.get(0));
        s.lda_str("g");
        s.b.star(args.get(1));
        s.b.call_runtime(RuntimeId::CopyDataPropertiesExcluding, args, 2)
            .star(t);
        for (o, k) in [(t, "g"), (t, "h"), (src, "count")] {
            s.get_named(o, k);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["G", "1", "1", "0", "undefined", "1", "1"]);
}

#[test]
fn copy_data_properties_with_index_keys_compares_atoms_only() {
    // src = {}; src[1] = 0; src.bar = 2; t = {}; CopyDataProperties(t, src); print(t[1], t.bar)
    // r = CopyDataPropertiesExcluding(src, 1); print(r[1], r.bar)
    // r = CopyDataPropertiesExcluding("ab", 0); print(r[0], r[1])
    // The keys come from [[OwnPropertyKeys]] (an index key) and from a string's indices; the
    // excluded-key compares and the lookups must see atoms only (W4-KEY-EQ).
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (src, t, k) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(src);
        s.b.lda_smi(1).star(k).lda_zero();
        s.b.set_keyed_property(src, k, s.set_keyed);
        s.b.lda_smi(2);
        s.set_named(src, "bar");
        let args = s.b.new_register_list(2);
        s.b.create_empty_object_literal().star(t);
        s.b.mov(t, args.get(0)).mov(src, args.get(1));
        s.b.call_runtime(RuntimeId::CopyDataProperties, args, 2);
        let print_both = |s: &mut Asm, o: Register, i: i32, name: Option<&str>| {
            s.b.lda_smi(i).get_keyed_property(o, s.keyed);
            s.print_acc();
            match name {
                Some(n) => {
                    s.get_named(o, n);
                }
                None => {
                    s.b.lda_smi(i + 1).get_keyed_property(o, s.keyed);
                }
            }
            s.print_acc();
        };
        print_both(&mut s, t, 1, Some("bar"));
        s.b.mov(src, args.get(0)).lda_smi(1).star(args.get(1));
        s.b.call_runtime(RuntimeId::CopyDataPropertiesExcluding, args, 2)
            .star(t);
        print_both(&mut s, t, 1, Some("bar"));
        s.lda_str("ab");
        s.b.star(args.get(0)).lda_zero().star(args.get(1));
        s.b.call_runtime(RuntimeId::CopyDataPropertiesExcluding, args, 2)
            .star(t);
        print_both(&mut s, t, 0, None);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (output, compares) = on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                take_output();
                let r = iso.run_script(make(), "test.js").map(|v| iso.display(v));
                assert!(r.is_ok(), "{r:?}");
                let compares = iso.rt.atoms.borrow().non_atom_compares();
                (take_output(), compares)
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(
                output,
                ["0", "2", "undefined", "2", "undefined", "b"],
                "{label}"
            );
            if cfg!(debug_assertions) {
                assert_eq!(compares, 0, "W4-KEY-EQ, {label}");
            }
        }
    }
}

/// A script that declares `function f() { return 2; }` (DeclareGlobals, InitializeGlobal),
/// then prints `f()`.
fn declare_f() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let init = s.b.new_register_list(2);
    s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
    s.lda_str("f");
    s.b.star(init.get(0));
    closure(&mut s, 1);
    s.b.star(init.get(1));
    s.b.call_runtime(RuntimeId::InitializeGlobal, init, 2);
    s.lda_global("f");
    s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    s.print_acc();
    s.b.lda_undefined().return_();
    let mut f = Asm::new(fn_meta("f", FunctionKind::Normal, 0));
    f.b.lda_smi(2).return_();
    unit_with_scopes(
        "",
        vec![(ScopeKind::Script, None, vec![("f", VariableMode::Function)])],
        vec![s.finish(), f.finish()],
    )
}

fn getter_returning_1<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::int32(1))
}

#[test]
fn a_function_declaration_replaces_a_configurable_global_accessor() {
    // CreateGlobalFunctionBinding (§9.1.1.4.18): a configurable accessor `f` on the global
    // object becomes the data property {w, e, c: false} holding the function; a
    // non-configurable writable enumerable data property keeps its attributes.
    for stress in [false, true] {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        {
            let iso = &mut iso;
            let mut cx = Cx::new(&mut iso.cx);
            let g = cx.global();
            let get = host_fn(&iso.rt, "get f", getter_returning_1);
            let d = PropertyDescriptor {
                get: Some(get),
                set: Some(Local::undefined()),
                enumerable: Some(false),
                configurable: Some(true),
                ..PropertyDescriptor::default()
            };
            cx.define_property_or_throw(g, "f", d).unwrap();
        }
        take_output();
        iso.run_script(declare_f(), "a.js").unwrap();
        assert_eq!(take_output(), ["2"]);
        let attrs = iso
            .with_cx(|cx| {
                let g = cx.global();
                let d = cx.get_own_property(g, "f")?.unwrap();
                Ok((d.is_data(), d.writable, d.enumerable, d.configurable))
            })
            .unwrap();
        assert_eq!(attrs, (true, Some(true), Some(true), Some(false)));
        // A second script declares `f` again: the property is now non-configurable,
        // writable and enumerable, so only its value changes.
        iso.run_script(declare_f(), "b.js").unwrap();
        assert_eq!(take_output(), ["2"]);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    }
}

std::thread_local! {
    static GETTER_CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn counting_getter<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    GETTER_CALLS.with(|c| c.set(c.get() + 1));
    Ok(Local::int32(7))
}

#[test]
fn isolate_global_gives_none_for_a_global_accessor() {
    // `Isolate::global` runs no JS, so for an accessor of the global object it gives None and
    // calls no getter. It never hands out the internal ACCESSOR_PAIR cell. A data property
    // still reads, and [[Get]] through the ABI still runs the getter.
    for stress in [false, true] {
        GETTER_CALLS.with(|c| c.set(0));
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        {
            let iso = &mut iso;
            let mut cx = Cx::new(&mut iso.cx);
            let g = cx.global();
            let get = host_fn(&iso.rt, "get acc", counting_getter);
            let d = PropertyDescriptor {
                get: Some(get),
                enumerable: Some(true),
                configurable: Some(true),
                ..PropertyDescriptor::default()
            };
            cx.define_property_or_throw(g, "acc", d).unwrap();
            cx.set(g, "data", Local::int32(5), true).unwrap();
        }
        assert!(iso.global("acc").is_none(), "an accessor gave a value");
        assert_eq!(GETTER_CALLS.with(std::cell::Cell::get), 0);
        let data = iso.global("data").map(|v| iso.display(v));
        assert_eq!(data.as_deref(), Some("5"));
        let v = iso.with_cx(|cx| {
            let g = cx.global();
            cx.get(g, "acc").map(|v| v.as_int32())
        });
        assert_eq!(v, Ok(Some(7)));
        assert_eq!(GETTER_CALLS.with(std::cell::Cell::get), 1);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    }
}

// ---- [[DefineOwnProperty]] and [[GetOwnProperty]] through the builtin ABI ---------------------

fn h1<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> {
    Ok(Local::int32(1))
}

fn h2<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> {
    Ok(Local::int32(2))
}

/// The receiver a host accessor saw last, and the value its setter got.
fn record_this<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let g = cx.global();
    cx.set(g, "seenThis", this, true)?;
    cx.set(g, "seenArg", args.get(0), true)?;
    Ok(Local::int32(3))
}

/// An accessor descriptor with the given fields.
fn acc<'s>(
    get: Option<Local<'s>>,
    set: Option<Local<'s>>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
) -> PropertyDescriptor<'s> {
    PropertyDescriptor {
        get,
        set,
        enumerable,
        configurable,
        ..PropertyDescriptor::default()
    }
}

#[test]
fn validate_and_apply_accessor_descriptors() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let o = cx.new_object();
            let get1 = host_fn(rt, "g1", h1);
            let get2 = host_fn(rt, "g2", h2);
            let undef = Local::undefined();
            // A new non-configurable accessor: absent fields are undefined / false.
            assert!(
                cx.define_own_property(o, "x", acc(Some(get1), None, None, None))
                    .unwrap()
            );
            let d = cx.get_own_property(o, "x").unwrap().unwrap();
            assert!(d.is_accessor() && !d.is_data());
            assert_eq!(d.get.unwrap().bits(), get1.bits());
            assert_eq!(d.set.unwrap().bits(), undef.bits());
            assert_eq!((d.enumerable, d.configurable), (Some(false), Some(false)));
            assert_eq!(cx.get(o, "x").unwrap().as_int32(), Some(1));
            // §10.1.6.3 step 5 on a non-configurable accessor.
            let same = acc(Some(get1), Some(undef), Some(false), Some(false));
            assert!(cx.define_own_property(o, "x", same).unwrap());
            assert!(
                cx.define_own_property(o, "x", PropertyDescriptor::default())
                    .unwrap()
            );
            for bad in [
                acc(Some(get2), None, None, None),
                acc(None, Some(get2), None, None),
                acc(None, None, None, Some(true)),
                acc(None, None, Some(true), None),
                PropertyDescriptor::data(Local::int32(1), false, false, false),
                PropertyDescriptor {
                    writable: Some(false),
                    ..PropertyDescriptor::default()
                },
            ] {
                assert!(!cx.define_own_property(o, "x", bad).unwrap(), "{bad:?}");
            }
            assert_eq!(cx.get(o, "x").unwrap().as_int32(), Some(1));

            // A configurable accessor: a define of one half keeps the other.
            let c = Some(true);
            assert!(
                cx.define_own_property(o, "y", acc(Some(get1), None, c, c))
                    .unwrap()
            );
            let pair0 = crate::props::own_get(o.bits(), atom(rt, "y")).unwrap().0;
            assert!(
                cx.define_own_property(o, "y", acc(None, Some(get2), None, None))
                    .unwrap()
            );
            let d = cx.get_own_property(o, "y").unwrap().unwrap();
            assert_eq!(d.get.unwrap().bits(), get1.bits());
            assert_eq!(d.set.unwrap().bits(), get2.bits());
            assert_eq!((d.enumerable, d.configurable), (c, c));
            // Pairs are immutable: the define made a new pair; the old one is unchanged.
            let pair1 = crate::props::own_get(o.bits(), atom(rt, "y")).unwrap().0;
            assert_ne!(pair0, pair1);
            assert_eq!(crate::accessor::setter(pair0), undef.bits());
            assert_eq!(crate::accessor::setter(pair1), get2.bits());
            // Accessor to data (§10.1.6.3 step 6.b): writable defaults to false.
            let data = PropertyDescriptor {
                value: Some(Local::int32(9)),
                ..PropertyDescriptor::default()
            };
            assert!(cx.define_own_property(o, "y", data).unwrap());
            let d = cx.get_own_property(o, "y").unwrap().unwrap();
            assert!(d.is_data() && !d.is_accessor());
            assert_eq!(d.value.unwrap().as_int32(), Some(9));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(false), c, c)
            );
            // Data to accessor (step 6.a): the absent half is undefined.
            assert!(
                cx.define_own_property(o, "y", acc(None, Some(get1), Some(false), None))
                    .unwrap()
            );
            let d = cx.get_own_property(o, "y").unwrap().unwrap();
            assert_eq!(d.get.unwrap().bits(), undef.bits());
            assert_eq!(d.set.unwrap().bits(), get1.bits());
            assert_eq!((d.enumerable, d.configurable), (Some(false), c));
            assert!(cx.get(o, "y").unwrap().is_undefined());
            // Delete works on a configurable accessor.
            assert!(cx.delete(o, "y").unwrap());
            assert!(cx.get_own_property(o, "y").unwrap().is_none());

            // A non-extensible object takes no new accessor.
            let ne = cx.new_object();
            cx.prevent_extensions(ne).unwrap();
            assert!(
                !cx.define_own_property(ne, "x", acc(Some(get1), None, c, c))
                    .unwrap()
            );

            // Arrays: `length` rejects an accessor; an element accessor is "not implemented".
            let a = cx.new_array();
            assert!(
                !cx.define_own_property(a, "length", acc(Some(get1), None, None, None))
                    .unwrap()
            );
            assert!(
                cx.define_own_property(a, "0", acc(Some(get1), None, c, c))
                    .is_err()
            );
            assert_eq!(
                take_message(cx, rt),
                "not implemented in ep9 v0: accessor array elements"
            );
        });
    }
}

#[test]
fn getters_and_setters_receive_the_receiver_also_a_primitive() {
    // Getters and setters on %String.prototype% see the primitive string as `this`:
    // "abc".p (get) and "abc".p = 4 (set, sloppy and strict).
    fn make() -> VerifiedUnit {
        let mut st = Asm::new(strict_meta("store", FunctionKind::Normal, 1));
        let p = st.b.parameter(0);
        st.b.lda_smi(5);
        st.set_named(p, "p");
        st.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let str_ = s.b.new_register();
        s.lda_str("abc");
        s.b.star(str_);
        s.get_named(str_, "p");
        s.print_acc();
        s.lda_global("seenThis");
        s.print_acc();
        s.b.lda_smi(4);
        s.set_named(str_, "p");
        s.lda_global("seenArg");
        s.print_acc();
        closure(&mut s, 1);
        s.b.star(s.f).call_undefined_receiver1(s.f, str_, s.call);
        s.lda_global("seenArg");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), st.finish()])
    }
    fn setup(iso: &mut Isolate) {
        let mut cx = Cx::new(&mut iso.cx);
        let sp = cx.intrinsic(IntrinsicId::StringPrototype);
        let f = host_fn(&iso.rt, "p", record_this);
        let d = PropertyDescriptor {
            get: Some(f),
            set: Some(f),
            enumerable: Some(false),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        };
        cx.define_property_or_throw(sp, "p", d).unwrap();
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, out) = on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                setup(&mut iso);
                take_output();
                let r = iso.run_script(make(), "p.js").map(|v| iso.display(v));
                let out = take_output();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (r, out)
            });
            assert!(r.is_ok(), "{r:?}");
            assert_eq!(out, ["3", "abc", "4", "5"], "{} stress={stress}", d.name());
        }
    }
}

#[test]
fn replacing_an_accessor_of_a_prototype_bumps_the_epoch() {
    with_cx(false, |cx, rt| {
        let p = cx.new_object();
        let o = cx.new_object_with_proto(p);
        let get1 = host_fn(rt, "g1", h1);
        let get2 = host_fn(rt, "g2", h2);
        let d = |g| PropertyDescriptor {
            get: Some(g),
            enumerable: Some(true),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        };
        assert!(cx.define_own_property(p, "x", d(get1)).unwrap());
        let e0 = rt.shapes.proto_epoch.get();
        // Same attributes, a new pair: the stored value changes, and so does the epoch.
        assert!(cx.define_own_property(p, "x", d(get2)).unwrap());
        let e1 = rt.shapes.proto_epoch.get();
        assert_ne!(e0, e1);
        assert_eq!(cx.get(o, "x").unwrap().as_int32(), Some(2));
        // The same pair again: no change, no bump.
        assert!(cx.define_own_property(p, "x", d(get2)).unwrap());
        assert_eq!(rt.shapes.proto_epoch.get(), e1);
    });
}

#[test]
fn engine_error_text_runs_no_getters() {
    // f1: get name() { calls = calls + 1; throw 1; }  on %TypeError.prototype%
    // calls = 0; DefineAccessorInLiteral(TypeError.prototype, "name", f1);
    // var e = new TypeError("m");            (install_stack: the header reads no getter)
    // try { e.name } catch (x) {} ; print(calls)  -> 1 (JS reads run the getter)
    // print(e.stack)                         -> "Error: m" header (the accessor is absent)
    // throw { get constructor() { throw 2; } }   (the report reads no getter)
    fn make() -> VerifiedUnit {
        let mut g = Asm::new(fn_meta("get name", FunctionKind::Getter, 0));
        g.lda_global("calls");
        g.b.inc(g.bin);
        g.sta_global("calls");
        g.b.lda_smi(1).throw();
        let mut c = Asm::new(fn_meta("get constructor", FunctionKind::Getter, 0));
        c.b.lda_smi(2).throw();

        let mut s = Asm::script(0);
        let (p, e, ctx, o) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        s.b.lda_zero();
        s.sta_global("calls");
        s.lda_global("TypeError");
        s.b.star(p);
        s.get_named(p, "prototype");
        s.b.star(p);
        define_accessor(&mut s, RuntimeId::DefineAccessorInLiteral, p, "name", 1, 0);
        let args = s.b.new_register_list(1);
        s.lda_str("m");
        s.b.star(args.get(0));
        s.construct_global("TypeError", args);
        s.b.star(e);
        s.lda_global("calls");
        s.print_acc();
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
        s.b.bind(start);
        s.get_named(e, "name");
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        s.b.bind(done);
        s.lda_global("calls");
        s.print_acc();
        s.get_named(e, "stack");
        s.print_acc();
        s.b.create_empty_object_literal().star(o);
        define_accessor(
            &mut s,
            RuntimeId::DefineAccessorInLiteral,
            o,
            "constructor",
            2,
            0,
        );
        s.b.ldar(o).throw();
        unit("", vec![s.finish(), g.finish(), c.finish()])
    }
    let r = run_all(make);
    assert_eq!(r.output[..2], ["0", "1"]);
    assert!(r.output[2].starts_with("Error: m\n"), "{:?}", r.output[2]);
    let e = r.result.expect_err("the object is thrown");
    assert_eq!(e.constructor, None);
    // `run_in` ran `collect_garbage` and the heap verifier after the run; a pending
    // exception left behind would have failed the throw protocol checks in debug builds.
}
