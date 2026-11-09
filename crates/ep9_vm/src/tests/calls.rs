//! Calls, recursion, argument padding, `this`, construct, classes, closures, host functions.

use ep9_bytecode::{
    FeedbackSlotKind, FunctionCode, FunctionFlags, FunctionId, FunctionKind, Register,
    RegisterList, RuntimeId, ScopeId, ScopeKind, VariableMode, VerifiedUnit,
};

use super::{Asm, isolate, meta, ok, run_all, take_output, unit, unit_with_scopes};
use crate::{Args, Cx, Dispatch, ErrorKind, JsResult, Local};

#[test]
fn recursion_fib() {
    fn make() -> VerifiedUnit {
        // function fib(n) { if (n < 2) return n; return fib(n - 1) + fib(n - 2); }
        let mut f = Asm::new(meta("fib", 1, 0, 0, 0));
        let t = f.b.new_register();
        let n = f.b.parameter(0);
        let rec = f.b.new_label();
        f.b.lda_smi(2).test_less_than(n, f.cmp).jump_if_false(rec);
        f.b.ldar(n).return_();
        f.b.bind(rec);
        for (d, last) in [(1, false), (2, true)] {
            f.lda_global("fib");
            f.b.star(f.f).ldar(n).sub_smi(d, f.bin).star(f.a.get(0));
            f.b.call_undefined_receiver1(f.f, f.a.get(0), f.call);
            if last {
                f.b.add(t, f.bin).return_();
            } else {
                f.b.star(t);
            }
        }
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("fib");
        s.lda_global("fib");
        s.b.star(s.f).lda_smi(20).star(s.a.get(0));
        s.b.call_undefined_receiver1(s.f, s.a.get(0), s.call)
            .return_();
        unit("", vec![s.finish(), f.finish()])
    }
    assert_eq!(ok(&run_all(make)), "6765");
}

#[test]
fn arguments_receiver_and_this() {
    fn make() -> VerifiedUnit {
        // f1: function kind(a, b) { return typeof b; }
        let mut kind = Asm::new(meta("kind", 2, 0, 0, 0));
        let ts = kind.b.feedback_slot(FeedbackSlotKind::TypeOf);
        kind.b.ldar(kind.b.parameter(1)).type_of(ts).return_();
        // f2: function self_() { return this; }   (sloppy: undefined this -> globalThis)
        let mut sloppy = Asm::new(meta("self_", 0, 0, 0, 0));
        sloppy.b.ldar(sloppy.b.receiver()).return_();
        // f3: function strictSelf() { "use strict"; return this; }
        let mut m = meta("strictSelf", 0, 0, 0, 0);
        m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut strict = Asm::new(m);
        strict.b.ldar(strict.b.receiver()).return_();
        // f4: function count() { return arguments.length; }
        let mut count = Asm::new(meta("count", 0, 0, 0, 0));
        let r = count.b.new_register();
        count.b.create_unmapped_arguments().star(r);
        count.get_named(r, "length");
        count.b.return_();
        // f5: function rest(a, ...r) { return r.length + ":" + r[1]; }
        let mut rest = Asm::new(meta("rest", 1, 0, 0, 0));
        let (r, t) = (rest.b.new_register(), rest.b.new_register());
        rest.b.create_rest_parameter().star(r);
        rest.get_named(r, "length");
        rest.b.star(t);
        rest.lda_str(":");
        rest.b
            .add(t, rest.bin)
            .star(t)
            .lda_smi(1)
            .get_keyed_property(r, rest.keyed)
            .add(t, rest.bin);
        rest.b.return_();

        let mut s = Asm::script(0);
        let (o, t) = (s.b.new_register(), s.b.new_register());
        for (i, name) in ["kind", "self_", "strictSelf", "count", "rest"]
            .iter()
            .enumerate()
        {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        let args = s.b.new_register_list(3);
        // kind(1) and kind(1, "x", 3): a missing argument is undefined; extras are ignored.
        s.lda_global("kind");
        s.b.star(s.f).lda_smi(1).star(args.get(0));
        s.b.call_undefined_receiver(
            s.f,
            RegisterList::new(args.first().local_index().unwrap(), 1),
            1,
            s.call,
        );
        s.print_acc();
        s.lda_global("kind");
        s.b.star(s.f).lda_smi(1).star(args.get(0));
        s.lda_str("x");
        s.b.star(args.get(1)).lda_smi(3).star(args.get(2));
        s.b.call_undefined_receiver(s.f, args, 3, s.call);
        s.print_acc();
        // self_() === globalThis; strictSelf() === undefined
        s.lda_global("globalThis");
        s.b.star(t);
        s.lda_global("self_");
        s.b.star(s.f)
            .call_undefined_receiver0(s.f, s.call)
            .test_reference_equal(t);
        s.print_acc();
        s.lda_global("strictSelf");
        s.b.star(s.f)
            .call_undefined_receiver0(s.f, s.call)
            .test_undefined();
        s.print_acc();
        // o = {}; o.m = self_; o.m() === o
        s.b.create_empty_object_literal().star(o);
        s.lda_global("self_");
        s.set_named(o, "m");
        s.get_named(o, "m");
        s.b.star(s.f)
            .call_property0(s.f, o, s.call)
            .test_reference_equal(o);
        s.print_acc();
        // count(1, 2, 3); rest(1, 2, 3)
        for name in ["count", "rest"] {
            s.lda_global(name);
            s.b.star(s.f);
            for i in 0..3 {
                s.b.lda_smi(i + 1).star(args.get(i as u32));
            }
            s.b.call_undefined_receiver(s.f, args, 3, s.call);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit(
            "",
            vec![
                s.finish(),
                kind.finish(),
                sloppy.finish(),
                strict.finish(),
                count.finish(),
                rest.finish(),
            ],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        ["undefined", "string", "true", "true", "true", "3", "2:3"]
    );
}

#[test]
fn construct_classes_and_super() {
    fn make() -> VerifiedUnit {
        // f1: function Point(x, y) { this.x = x; this.y = y; }
        let mut point = Asm::new(meta("Point", 2, 0, 0, 0));
        let this = point.b.receiver();
        point.b.ldar(point.b.parameter(0));
        point.set_named(this, "x");
        point.b.ldar(point.b.parameter(1));
        point.set_named(this, "y");
        point.b.lda_undefined().return_();
        // f2: function Wrap() { return {w: 1}; }  (an object result replaces `this`)
        let mut wrap = Asm::new(meta("Wrap", 0, 0, 0, 0));
        let o = wrap.b.new_register();
        wrap.b.create_empty_object_literal().star(o).lda_smi(1);
        wrap.set_named(o, "w");
        wrap.b.ldar(o).return_();
        // f3: class A { constructor() { this.v = 7; } }
        let mut ma = meta("A", 0, 0, 0, 0);
        ma.kind = FunctionKind::ClassConstructor;
        ma.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut a = Asm::new(ma);
        let this = a.b.receiver();
        a.b.lda_smi(7);
        a.set_named(this, "v");
        a.b.lda_undefined().return_();
        // f4: class B extends A { constructor() { super(); this.w = this.v + 1; } }
        let mut mb = meta("B", 0, 0, 0, 0);
        mb.kind = FunctionKind::DerivedConstructor;
        mb.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut b = Asm::new(mb);
        let (this, sup) = (b.b.new_register(), b.b.new_register());
        b.b.lda_the_hole().star(this);
        b.b.lda_current_function().get_super_constructor(sup);
        b.b.throw_if_not_super_constructor(sup);
        b.b.lda_new_target()
            .construct(sup, RegisterList::EMPTY, 0, b.call);
        b.b.star(this);
        b.get_named(this, "v");
        b.b.add_smi(1, b.bin);
        b.set_named(this, "w");
        b.b.ldar(this).throw_super_not_called_if_hole().return_();

        let mut s = Asm::script(0);
        let (p, cls, heritage, t) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let args = s.b.new_register_list(2);
        for (i, name) in ["Point", "Wrap"].iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        // DefineClass(A, hole); DefineClass(B, A)
        let def = RegisterList::new(cls.local_index().unwrap(), 2);
        assert_eq!(heritage.local_index(), Some(cls.local_index().unwrap() + 1));
        let ka = s.b.function_constant(FunctionId(3));
        s.b.create_closure(ka, s.closure, 0)
            .star(cls)
            .lda_the_hole()
            .star(heritage);
        s.b.call_runtime(RuntimeId::DefineClass, def, 2);
        s.b.ldar(cls);
        s.sta_global("A");
        let kb = s.b.function_constant(FunctionId(4));
        s.b.create_closure(kb, s.closure, 0).star(cls);
        s.lda_global("A");
        s.b.star(heritage)
            .call_runtime(RuntimeId::DefineClass, def, 2);
        s.b.ldar(cls);
        s.sta_global("B");
        // p = new Point(1, 2); print(p.x + p.y); print(p instanceof Point)
        s.b.lda_smi(1)
            .star(args.get(0))
            .lda_smi(2)
            .star(args.get(1));
        s.construct_global("Point", args);
        s.b.star(p);
        s.get_named(p, "x");
        s.b.star(t);
        s.get_named(p, "y");
        s.b.add(t, s.bin);
        s.print_acc();
        let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
        s.lda_global("Point");
        s.b.test_instance_of(p, is);
        s.print_acc();
        // new Wrap().w
        s.construct_global("Wrap", RegisterList::EMPTY);
        s.b.star(p);
        s.get_named(p, "w");
        s.print_acc();
        // new A().v; A.prototype.constructor === A
        s.construct_global("A", RegisterList::EMPTY);
        s.b.star(p);
        s.get_named(p, "v");
        s.print_acc();
        s.lda_global("A");
        s.b.star(t);
        s.get_named(t, "prototype");
        s.b.star(p);
        s.get_named(p, "constructor");
        s.b.test_reference_equal(t);
        s.print_acc();
        // b = new B(); print(b.v, b.w); b instanceof A
        s.construct_global("B", RegisterList::EMPTY);
        s.b.star(p);
        s.get_named(p, "w");
        s.print_acc();
        s.lda_global("A");
        s.b.test_instance_of(p, is);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit(
            "",
            vec![
                s.finish(),
                point.finish(),
                wrap.finish(),
                a.finish(),
                b.finish(),
            ],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["3", "true", "1", "7", "true", "8", "true"]);
}

#[test]
fn closures_and_contexts() {
    fn make() -> VerifiedUnit {
        // f1: function makeCounter() { let n = 0; return function () { { let tmp; n = n + 1; } return n; }; }
        let mut mk = Asm::new(meta("makeCounter", 0, 0, 0, 0));
        let ctx = mk.b.new_register();
        let ks = mk.b.scope_constant(ScopeId(0));
        mk.b.create_function_context(ks, 1).push_context(ctx);
        mk.b.lda_zero().sta_current_context_slot(0);
        let kf = mk.b.function_constant(FunctionId(2));
        mk.b.create_closure(kf, mk.closure, 0).return_();
        // f2: the counter: a block context inside, the counter one hop out.
        let mut inc = Asm::new(meta("", 0, 1, 0, 0));
        let (save, cur) = (inc.b.new_register(), inc.b.new_register());
        let kb = inc.b.scope_constant(ScopeId(1));
        inc.b.create_block_context(kb).push_context(save);
        inc.b.lda_current_context().star(cur);
        inc.b
            .lda_context_slot(cur, 0, 1)
            .add_smi(1, inc.bin)
            .sta_context_slot(cur, 0, 1);
        inc.b.pop_context(save);
        inc.b.lda_current_context_slot(0).return_();

        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("makeCounter");
        for name in ["c1", "c2"] {
            s.lda_global("makeCounter");
            s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
            s.sta_global(name);
        }
        for name in ["c1", "c1", "c1", "c2"] {
            s.lda_global(name);
            s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit_with_scopes(
            "",
            vec![
                (ScopeKind::Function, None, vec![("n", VariableMode::Let)]),
                (ScopeKind::Block, Some(0), vec![("tmp", VariableMode::Let)]),
            ],
            vec![s.finish(), mk.finish(), inc.finish()],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["1", "2", "3", "1"]);
}

fn call_twice<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let f = args.get(0);
    cx.call(f, Local::undefined(), &[])?;
    cx.call(f, Local::undefined(), &[])
}

fn fail<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_error(ErrorKind::TypeError, "host says no"))
}

fn add_host<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let a = cx.to_number(args.get(0))?;
    let b = cx.to_number(args.get(1))?;
    Ok(Local::number(a + b))
}

#[test]
fn host_functions_and_reentry() {
    let make = || {
        // f1: function bump() { count = count + 1; return count; }
        let mut bump = Asm::new(meta("bump", 0, 0, 0, 0));
        bump.lda_global("count");
        bump.b.inc(bump.bin);
        bump.sta_global("count");
        bump.b.return_();

        let mut s = Asm::script(0);
        let (ctx, t) = (s.b.new_register(), s.b.new_register());
        s.b.lda_zero();
        s.sta_global("count");
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(s.a.get(1));
        // print(callTwice(bump)); print(count)
        s.lda_global("callTwice");
        s.b.star(s.f)
            .call_undefined_receiver1(s.f, s.a.get(1), s.call);
        s.print_acc();
        s.lda_global("count");
        s.print_acc();
        // print(add(2, "3")), add.length
        let args = RegisterList::new(s.a.get(1).local_index().unwrap(), 2);
        s.lda_global("add");
        s.b.star(s.f).lda_smi(2).star(args.get(0));
        s.lda_str("3");
        s.b.star(args.get(1))
            .call_undefined_receiver(s.f, args, 2, s.call);
        s.print_acc();
        s.lda_global("add");
        s.b.star(t);
        s.get_named(t, "length");
        s.print_acc();
        // try { fail(); } catch (e) { print(e.message); print(e instanceof TypeError); }
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
        s.b.bind(start);
        s.lda_global("fail");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        s.b.star(t);
        s.get_named(t, "message");
        s.print_acc();
        let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
        s.lda_global("TypeError");
        s.b.test_instance_of(t, is);
        s.print_acc();
        s.b.bind(done);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), bump.finish()])
    };
    for &d in Dispatch::ALL {
        let r = super::on_big_stack(move || {
            let mut iso = isolate(super::config(d, false));
            iso.register_host_function("callTwice", 1, call_twice);
            iso.register_host_function("fail", 0, fail);
            iso.register_host_function("add", 2, add_host);
            take_output();
            let r = iso
                .with_cx_checkpoint(|cx| cx.run_script(make(), "host.js").map(|v| cx.display(v)));
            (r, take_output())
        });
        assert!(r.0.is_ok(), "{:?}", r.0);
        assert_eq!(
            r.1,
            ["2", "2", "5", "2", "host says no", "true"],
            "{}",
            d.name()
        );
    }
}

fn construct_host<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let f = args.get(0);
    Ok(cx.construct(f, &[], f)?.value())
}

/// `try { body } catch (e) { print(e.message); }`
fn try_print_message(s: &mut Asm, ctx: Register, t: Register, body: impl FnOnce(&mut Asm)) {
    let (start, end, handler, done) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    s.b.star(t);
    s.get_named(t, "message");
    s.print_acc();
    s.b.bind(done);
}

/// A class constructor called from Rust without new.target (ToPrimitive, a host function,
/// the embedder) throws a TypeError before its body runs (§10.2.1 step 2), like the `Call*`
/// opcodes. Construct from Rust still runs it.
#[test]
fn class_constructor_needs_new_on_rust_calls() {
    let make = || {
        // f1: class C { constructor() { ran = ran + 1; return 7; } }
        let mut mc = meta("C", 0, 0, 0, 0);
        mc.kind = FunctionKind::ClassConstructor;
        mc.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
        let mut c = Asm::new(mc);
        c.lda_global("ran");
        c.b.inc(c.bin);
        c.sta_global("ran");
        c.b.lda_smi(7).return_();

        let mut s = Asm::script(0);
        let (ctx, t, o, cls, heritage) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        s.b.lda_zero();
        s.sta_global("ran");
        // C = DefineClass(C, hole)
        let def = RegisterList::new(cls.local_index().unwrap(), 2);
        assert_eq!(heritage.local_index(), Some(cls.local_index().unwrap() + 1));
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0)
            .star(cls)
            .lda_the_hole()
            .star(heritage);
        s.b.call_runtime(RuntimeId::DefineClass, def, 2);
        s.b.ldar(cls);
        s.sta_global("C");
        // try { print({ valueOf: C } + 1); } catch (e) { print(e.message); }  (ToPrimitive)
        try_print_message(&mut s, ctx, t, |s| {
            s.b.create_empty_object_literal().star(o);
            s.lda_global("C");
            s.set_named(o, "valueOf");
            s.b.lda_smi(1).add(o, s.bin);
            s.print_acc();
        });
        // try { print(callTwice(C)); } catch (e) { print(e.message); }  (host function)
        try_print_message(&mut s, ctx, t, |s| {
            s.lda_global("C");
            s.b.star(s.a.get(1));
            s.lda_global("callTwice");
            s.b.star(s.f)
                .call_undefined_receiver1(s.f, s.a.get(1), s.call);
            s.print_acc();
        });
        // print(ran); print(construct(C) instanceof C); print(ran)
        s.lda_global("ran");
        s.print_acc();
        s.lda_global("C");
        s.b.star(s.a.get(1));
        s.lda_global("construct");
        s.b.star(s.f)
            .call_undefined_receiver1(s.f, s.a.get(1), s.call)
            .star(t);
        let is = s.b.feedback_slot(FeedbackSlotKind::InstanceOf);
        s.lda_global("C");
        s.b.test_instance_of(t, is);
        s.print_acc();
        s.lda_global("ran");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), c.finish()])
    };
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = super::on_big_stack(move || {
                let mut iso = isolate(super::config(d, stress));
                iso.register_host_function("callTwice", 1, call_twice);
                iso.register_host_function("construct", 1, construct_host);
                take_output();
                let r = iso.with_cx_checkpoint(|cx| {
                    cx.run_script(make(), "class.js").map(|v| cx.display(v))
                });
                // The embedder: Cx::call(C).
                let called = iso.with_cx_checkpoint(|cx| {
                    let c = cx.global_binding("C")?.expect("C");
                    cx.call(c, Local::undefined(), &[]).map(|v| cx.display(v))
                });
                let ran = iso
                    .with_cx(|cx| Ok(cx.global_binding("ran")?.map(|v| cx.display(v))))
                    .unwrap()
                    .expect("ran");
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (r, take_output(), called.map_err(|e| e.message), ran)
            });
            let label = format!("{} stress={stress}", d.name());
            assert!(r.0.is_ok(), "{label}: {:?}", r.0);
            let msg = "Class constructor C cannot be invoked without 'new'";
            assert_eq!(r.1, [msg, msg, "0", "true", "1"], "{label}");
            assert_eq!(r.2, Err(format!("TypeError: {msg}")), "{label}");
            assert_eq!(r.3, "1", "{label}");
        }
    }
}

/// FRAME 2 (H14-Q1): `LdaNewTarget` reads fp-3 with no construct-bit check. A call gives
/// undefined, a `[[Construct]]` gives new.target, and a frame that `call::push_resume_frame`
/// pushed (construct bit clear, fp-3 = the generator object) gives that object.
#[test]
fn lda_new_target_reads_the_resumed_frame_slot() {
    fn make() -> VerifiedUnit {
        // function nt() { return new.target; }
        let mut f = Asm::new(meta("nt", 0, 0, 0, 0));
        f.b.lda_new_target().return_();
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("nt");
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), f.finish()])
    }
    /// Runs `f` (a bytecode function) in a resumed frame whose fp-3 holds `generator`.
    fn run_resumed(iso: &mut crate::Isolate, f: u64, generator: u64) -> u64 {
        use crate::frame::{UNDEFINED, int32_of, slot};
        let cx = &mut *iso.cx;
        let rt = cx.rt();
        let fi = crate::object::function_info(f);
        let caller = rt.stack().fp();
        let base = crate::call::top_of(cx, caller);
        // SAFETY: `base` is the first free slot above the top frame (none: the stack base);
        // the limit is the stack end; no arguments.
        let fp = unsafe {
            crate::call::push_resume_frame(
                base,
                cx.stack_limit,
                fi,
                f,
                UNDEFINED,
                0,
                |_| UNDEFINED,
                caller,
                crate::interp::EXIT_STUB.as_ptr(),
                crate::object::function_context(f),
                generator,
            )
        }
        .unwrap();
        // SAFETY: header slots of the frame just pushed.
        let argc = unsafe { *fp.offset(slot::ARGC) };
        assert_eq!(
            int32_of(argc) as u32 & crate::frame::CONSTRUCT_BIT,
            0,
            "construct bit clear"
        );
        rt.stack().set_fp(fp);
        rt.stack().clear_scratch();
        // SAFETY: `fp` is a frame for `fi` that push_resume_frame just wrote; verified code.
        let r = unsafe { crate::interp::run(cx, fi.code, fp) };
        rt.stack().set_fp(caller);
        r
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            super::on_big_stack(move || {
                let mut iso = isolate(super::config(d, stress));
                iso.with_cx_checkpoint(|cx| cx.run_script(make(), "nt.js").map(|_| ()))
                    .unwrap();
                // `nt` stays a global binding, so its bits stay valid (the heap does not move).
                let fb = iso.global_raw("nt").unwrap().to_bits();
                // A call: undefined.
                let r = iso
                    .with_cx_checkpoint(|cx| {
                        let f: Local = Local::from_bits(fb);
                        cx.call(f, Local::undefined(), &[])
                            .map(|v| v.is_undefined())
                    })
                    .unwrap();
                assert!(r, "{} stress={stress}", d.name());
                // A resumed frame: the object in fp-3 (here a plain object for the generator).
                let g = crate::object::new_plain_object(&iso.rt);
                let r = run_resumed(&mut iso, fb, g);
                assert_eq!(r, g, "{} stress={stress}", d.name());
                // [[Construct]]: new.target (`nt` itself; an object result is kept).
                let r = crate::call::construct_with(&mut iso.cx, fb, 0, |_| 0, fb).unwrap();

                assert_eq!(r, fb, "{} stress={stress}", d.name());
            });
        }
    }
}

/// `function f(o) { return o.a; }` (sloppy), stored in the global `f`.
fn reads_a_of_its_argument() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f");
    s.b.lda_undefined().return_();
    let mut f = Asm::new(meta("f", 1, 0, 0, 0));
    let o = f.b.parameter(0);
    f.get_named(o, "a");
    f.b.return_();
    unit("", vec![s.finish(), f.finish()])
}

/// A new object `{a: 7}` whose only copy is in a heap `Vec`, which the GC does not scan.
/// `inline(never)`: its frame is gone before the call under test.
#[inline(never)]
fn an_object_only_in_a_vec(iso: &crate::Isolate) -> Vec<crate::Value> {
    let rt = &*iso.rt;
    let key = crate::atoms::intern_str(rt, "a");
    let o = crate::object::new_plain_object(rt);
    crate::props::own_define(
        rt,
        o,
        key,
        crate::frame::int32_bits(7),
        crate::object::attr::DEFAULT,
    );
    vec![crate::runtime::val(o)]
}

/// Plain Rust with no isolate call: zeroes a part of the native stack below the caller, so no
/// old copy of a Value stays there for the conservative scan to find.
#[inline(never)]
fn clear_stack() {
    let mut a = [1u64; 16384];
    for x in &mut a {
        // SAFETY: a valid, aligned element of the local array.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
    std::hint::black_box(&a);
}

/// The R2 rule (M2-P5-FIX-R2) for a Rust caller of `call::call_with` (the spike-native
/// `call_from_rust`, the tests' `call_raw`; the embedder `Isolate::call` had it until M3-P3):
/// the arguments may live only in memory the GC does not scan, so every GC point of the call
/// comes after the callee frame copied them. The wrap of a primitive `this` for a sloppy
/// callee (H14-Q2) is such a GC point. Under GC stress, `f(o)` with `this` = 1, true and
/// undefined reads `o.a` = 7, and the heap verifies.
#[test]
fn a_rust_call_copies_the_arguments_before_the_this_wrap() {
    for this in [
        crate::Value::int32(1),
        crate::Value::TRUE,
        crate::Value::UNDEFINED,
    ] {
        let r = super::on_big_stack(move || {
            let mut iso = isolate(super::config(Dispatch::DEFAULT, true));
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(reads_a_of_its_argument(), "r2.js")
                    .map(|_| ())
            })
            .unwrap();
            // `f` stays a global binding, so its Value stays valid.
            let f = iso.global_raw("f").unwrap();
            let args = an_object_only_in_a_vec(&iso);
            clear_stack();
            let r = iso.call_raw(f, this, &args).map(|v| v.as_int32());
            drop(args);
            iso.verify_heap().unwrap();
            r.map_err(|e| e.message)
        });
        assert_eq!(r, Ok(Some(7)), "this = {this:?}");
    }
}

/// A host function for the frame tests below; never called.
fn never_called<'s>(
    _: &mut Cx<'s>,
    _: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    unreachable!("the frame tests only read the counts of this info")
}

/// PERF3 item 2 (M5-P2): the x86-64 T0 call fast path's frame (`call::call_frame_fp`, then
/// `call::fill_missing_args`, `call::write_call_args` and `call::write_call_header`) is the
/// frame `call::push_frame` writes, slot for slot, and both refuse the same sizes: no missing
/// argument, missing arguments, no register, more arguments than parameters, 0 to 4
/// arguments, a frame one slot too big, one that just fits and one with room left. The
/// arguments go in by the list copy (`LIST`, 0 to 4) and by the fixed-count stores (`!LIST`,
/// 0 to 2), and before or after the header (M6-P1 CBC: the forms with a receiver operand
/// write them before the later callee tests).
#[test]
fn the_fast_frame_write_equals_push_frame() {
    use crate::call::{
        call_frame_fp, fill_missing_args, push_frame, write_call_args, write_call_header,
    };
    use crate::frame::{HEADER, UNDEFINED, int32_bits};
    use crate::unit::FunctionInfo;
    const MARK: u64 = 0x7FF8_0000_DEAD_BEEF;
    let (callee, this, context) = (int32_bits(1), int32_bits(2), int32_bits(3));
    let context_of = || context;
    let caller_fp = core::ptr::without_provenance_mut::<u64>(0x1000);
    let return_pc = core::ptr::without_provenance::<u8>(0x2000);
    let arg = |i: u32| int32_bits(100 + i as i32);
    for (params, regs) in [(0, 0), (0, 4), (1, 0), (3, 2), (5, 7)] {
        let mut fi = FunctionInfo::host(never_called, "f", 0, false);
        fi.parameter_count = params;
        fi.register_count = regs;
        for (argc, list, early) in (0..=4u32)
            .flat_map(|n| {
                [
                    (n, true, false),
                    (n, true, true),
                    (n, false, false),
                    (n, false, true),
                ]
            })
            .filter(|&(n, list, _)| list || n <= 2)
        {
            let size = argc.max(params) as usize + 1 + HEADER + regs as usize;
            for room in [size - 1, size, size + 3] {
                let (mut a, mut b) = (vec![MARK; 64], vec![MARK; 64]);
                // SAFETY: base + room lies inside each 64-slot buffer (room <= 25).
                let pa = unsafe {
                    let base = a.as_mut_ptr().add(4);
                    push_frame(
                        base,
                        base.add(room),
                        &fi,
                        callee,
                        this,
                        argc,
                        arg,
                        caller_fp,
                        return_pc,
                        context,
                        UNDEFINED,
                    )
                };
                // SAFETY: as above; `fp` is `call_frame_fp` of the same base and limit.
                let pb = unsafe {
                    let base = b.as_mut_ptr().add(4);
                    call_frame_fp(base, base.add(room), &fi, argc).inspect(|&fp| {
                        fill_missing_args(base, fp, argc);
                        let args = || {
                            if list {
                                write_call_args::<true>(fp, this, argc, arg)
                            } else {
                                write_call_args::<false>(fp, this, argc, arg)
                            }
                        };
                        if early {
                            args();
                        }
                        write_call_header(fp, &fi, callee, argc, caller_fp, return_pc, context_of);
                        if !early {
                            args();
                        }
                    })
                };
                let case = format!(
                    "params {params} regs {regs} argc {argc} room {room} list {list} early {early}"
                );
                assert_eq!(
                    pa.map(|p| p.addr() - a.as_ptr().addr()),
                    pb.map(|p| p.addr() - b.as_ptr().addr()),
                    "{case}"
                );
                assert_eq!(pa.is_some(), room >= size, "{case}");
                assert_eq!(a, b, "{case}");
            }
        }
    }
}

/// PERF3 item 2: `call::bound_with_global` after `call::try_bind_this(.., undefined)` gives
/// what `call::try_bind_this(.., global)` gives, for a strict and a sloppy callee and each
/// receiver that is not a cell (a cell receiver comes back as it is either way, and never as
/// undefined).
#[test]
fn bound_with_global_after_the_receiver_test_is_try_bind_this() {
    use crate::call::{bound_with_global, try_bind_this};
    use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits};
    use crate::function_info::fi_flags;
    use crate::unit::FunctionInfo;
    let global = int32_bits(77);
    let receivers = [
        UNDEFINED,
        NULL,
        HOLE,
        TRUE,
        FALSE,
        int32_bits(0),
        int32_bits(-5),
        ep9_value::Value::number(1.5).to_bits(),
    ];
    for strict in [false, true] {
        let mut fi = FunctionInfo::host(never_called, "f", 0, false);
        if !strict {
            fi.flags &= !fi_flags::STRICT;
        }
        assert_eq!(fi.is_strict(), strict);
        for this in receivers {
            let want = try_bind_this(&fi, this, global);
            let got =
                try_bind_this(&fi, this, UNDEFINED).map(|b| bound_with_global(&fi, b, global));
            assert_eq!(got, want, "strict {strict} receiver {this:#x}");
        }
    }
}

// ---- the decode-free host entry and the forwarded calls (M7-P0 CALLHOST) --------------------

/// `echo(...)`: "<typeof this>:<args joined by ,>" (a host callee for the entry shapes).
fn echo<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    let t = if this.is_undefined() {
        "undefined".to_string()
    } else {
        cx.to_rust_string(this)?
    };
    let s = format!("{t}:{}", parts.join(","));
    Ok(cx.new_string(&s)?.value())
}

/// A host function in the form of `Function.prototype.call` (IsCallable, then the VM steps).
fn fcall<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    if !this.is_callable() {
        return Err(cx.throw_type_error("fcall: not callable"));
    }
    cx.forward_call(args)
}

/// A host function in the form of `Function.prototype.apply`.
fn fapply<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    if !this.is_callable() {
        return Err(cx.throw_type_error("fapply: not callable"));
    }
    cx.forward_apply(args)
}

/// `fcall` and `fapply` as the definitions of a builtin table, installed as globals: a
/// forwarder records only a function made from a table definition (M7-P0 JIT reviewer F1).
static FORWARDERS: crate::BuiltinTable = crate::BuiltinTable::new(
    "forwarders",
    &[crate::BuiltinModule {
        name: "forwarders",
        block: 104,
        defs: &[
            crate::BuiltinDef::method("fcall", 1, fcall),
            crate::BuiltinDef::method("fapply", 2, fapply),
        ],
        intrinsics: &[],
        props: &[crate::ObjectProps {
            target: crate::IntrinsicId::GlobalObject,
            props: &[
                crate::PropDef::method(crate::PropKey::Str("fcall"), 0),
                crate::PropDef::method(crate::PropKey::Str("fapply"), 1),
            ],
        }],
    }],
);

/// The 9 fixed-operand call shapes.
#[derive(Clone, Copy, Debug)]
enum Shape {
    CallProperty0,
    CallProperty1,
    CallProperty2,
    CallProperty,
    CallAnyReceiver,
    CallUndefinedReceiver0,
    CallUndefinedReceiver1,
    CallUndefinedReceiver2,
    CallUndefinedReceiver,
}

/// Emits `acc = callee(..)` in `shape` with the receiver "R" (property shapes) and the
/// arguments 1..=n (n = 0, 1, 2 for the numbered shapes, 3 for the list shapes).
fn emit_shape(s: &mut Asm, shape: Shape, callee: &str, l: RegisterList) {
    let property = matches!(
        shape,
        Shape::CallProperty0
            | Shape::CallProperty1
            | Shape::CallProperty2
            | Shape::CallProperty
            | Shape::CallAnyReceiver
    );
    let n = match shape {
        Shape::CallProperty0 | Shape::CallUndefinedReceiver0 => 0,
        Shape::CallProperty1 | Shape::CallUndefinedReceiver1 => 1,
        Shape::CallProperty2 | Shape::CallUndefinedReceiver2 => 2,
        _ => 3,
    };
    let mut k = 0;
    if property {
        s.lda_str("R");
        s.b.star(l.get(0));
        k = 1;
    }
    for i in 0..n {
        s.b.lda_smi(i as i32 + 1).star(l.get(k + i));
    }
    s.lda_global(callee);
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    let list = RegisterList::new(l.get(0).local_index().unwrap(), k + n);
    match shape {
        Shape::CallProperty0 => s.b.call_property0(f, l.get(0), slot),
        Shape::CallProperty1 => s.b.call_property1(f, l.get(0), l.get(1), slot),
        Shape::CallProperty2 => s.b.call_property2(f, l.get(0), l.get(1), l.get(2), slot),
        Shape::CallProperty => s.b.call_property(f, list, k + n, slot),
        Shape::CallAnyReceiver => s.b.call_any_receiver(f, list, k + n, slot),
        Shape::CallUndefinedReceiver0 => s.b.call_undefined_receiver0(f, slot),
        Shape::CallUndefinedReceiver1 => s.b.call_undefined_receiver1(f, l.get(0), slot),
        Shape::CallUndefinedReceiver2 => s.b.call_undefined_receiver2(f, l.get(0), l.get(1), slot),
        Shape::CallUndefinedReceiver => s.b.call_undefined_receiver(f, list, n, slot),
    };
}

/// Runs `make` with `echo`, `sp`, `fcall` and `fapply` registered, the human fast paths `on`
/// or off: (result, output, host entries, forwarded calls, the forwarded calls of the T0 call
/// fast path).
fn run_host(
    make: fn() -> VerifiedUnit,
    d: Dispatch,
    stress: bool,
    on: bool,
) -> (Result<String, String>, Vec<String>, u64, u64, u64) {
    use crate::call::{FORWARDED_CALLS, HOST_ENTRY_HITS, T0_FORWARDED_CALLS};
    super::on_big_stack(move || {
        let mut c = super::config(d, stress);
        c.builtins = Some(&FORWARDERS);
        let mut iso = isolate(c);
        iso.register_host_function("echo", 0, echo);
        iso.register_host_function("sp", 0, sp);
        take_output();
        HOST_ENTRY_HITS.with(|h| h.set(0));
        FORWARDED_CALLS.with(|h| h.set(0));
        T0_FORWARDED_CALLS.with(|h| h.set(0));
        let r = iso.with_cx_checkpoint(|cx| {
            cx.set_human_fast_paths(on);
            cx.run_script(make(), "host.js").map(|v| cx.display(v))
        });
        let out = take_output();
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        // The switch is process-wide and keyed by the isolate id: turn it on again, so a dropped
        // isolate leaves no id behind (else every later host call takes the lock; M7-P0 JIT
        // reviewer F5).
        let _ = iso.with_cx(|cx| {
            cx.set_human_fast_paths(true);
            Ok(())
        });
        (
            r.map_err(|e| e.message),
            out,
            HOST_ENTRY_HITS.with(|h| h.get()),
            FORWARDED_CALLS.with(|h| h.get()),
            T0_FORWARDED_CALLS.with(|h| h.get()),
        )
    })
}

#[test]
fn a_host_callee_takes_the_decode_free_entry_in_each_fixed_shape() {
    const SHAPES: [Shape; 9] = [
        Shape::CallProperty0,
        Shape::CallProperty1,
        Shape::CallProperty2,
        Shape::CallProperty,
        Shape::CallAnyReceiver,
        Shape::CallUndefinedReceiver0,
        Shape::CallUndefinedReceiver1,
        Shape::CallUndefinedReceiver2,
        Shape::CallUndefinedReceiver,
    ];
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(4);
        for shape in SHAPES {
            emit_shape(&mut s, shape, "echo", l);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let want = [
        "R:",
        "R:1",
        "R:1,2",
        "R:1,2,3",
        "R:1,2,3",
        "undefined:",
        "undefined:1",
        "undefined:1,2",
        "undefined:1,2,3",
    ];
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run_host(make, d, stress, true);
            let off = run_host(make, d, stress, false);
            assert!(on.0.is_ok(), "{:?}", on.0);
            assert_eq!(on.1, want, "{} stress={stress}", d.name());
            assert_eq!((on.0, on.1), (off.0, off.1), "{}", d.name());
            // `print` is a host callee too: 9 calls of echo, 9 of print.
            assert_eq!((on.2, off.2), (18, 0), "{} stress={stress}", d.name());
        }
    }
}

#[test]
fn a_forwarded_call_pushes_the_bytecode_callee_in_the_same_run() {
    fn make() -> VerifiedUnit {
        // f1: function add3(a, b, c) { return (c + (b + (a + ""))) + typeof this; }
        let mut add3 = Asm::new(meta("add3", 3, 0, 0, 0));
        let t = add3.b.new_register();
        let ts = add3.b.feedback_slot(FeedbackSlotKind::TypeOf);
        add3.lda_str("");
        for i in 0..3 {
            let p = add3.b.parameter(i);
            add3.b.add(p, add3.bin);
        }
        let receiver = add3.b.receiver();
        add3.b.star(t).ldar(receiver).type_of(ts).add(t, add3.bin);
        add3.b.return_();
        // f2: function viaArgs(a, b, c) { return fapply.call-form(add3, [undefined, arguments]); }
        // i.e. add3.fapply(undefined, arguments) with fapply as the callee and add3 its `this`
        let mut via = Asm::new(meta("viaArgs", 3, 0, 0, 0));
        let l = via.b.new_register_list(3);
        via.lda_global("add3");
        via.b.star(l.get(0)).lda_undefined().star(l.get(1));
        via.b.create_unmapped_arguments().star(l.get(2));
        via.lda_global("fapply");
        via.b.star(via.f);
        via.b
            .call_property2(via.f, l.get(0), l.get(1), l.get(2), via.call)
            .return_();

        let mut s = Asm::script(0);
        let l = s.b.new_register_list(5);
        for (id, name) in [(1, "add3"), (2, "viaArgs")] {
            let k = s.b.function_constant(FunctionId(id));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        for _ in 0..3 {
            // add3.fcall("x", 1, 2, 3): fcall with this = add3 (the list shape)
            s.lda_global("add3");
            s.b.star(l.get(0));
            s.lda_str("x");
            s.b.star(l.get(1));
            for i in 0..3 {
                s.b.lda_smi(i + 1).star(l.get(2 + i as u32));
            }
            s.lda_global("fcall");
            s.b.star(s.f).call_property(s.f, l, 5, s.call);
            s.print_acc();
            // viaArgs(4, 5, 6)
            s.lda_global("viaArgs");
            s.b.star(s.f);
            let args = RegisterList::new(l.get(0).local_index().unwrap(), 3);
            for i in 0..3 {
                s.b.lda_smi(i + 4).star(args.get(i as u32));
            }
            s.b.call_undefined_receiver(s.f, args, 3, s.call);
            s.print_acc();
            // add3.fapply(null, undefined): 0 arguments, this = the global object
            s.lda_global("add3");
            s.b.star(l.get(0))
                .lda_null()
                .star(l.get(1))
                .lda_undefined()
                .star(l.get(2));
            s.lda_global("fapply");
            s.b.star(s.f)
                .call_property2(s.f, l.get(0), l.get(1), l.get(2), s.call);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), add3.finish(), via.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run_host(make, d, stress, true);
            let off = run_host(make, d, stress, false);
            assert!(on.0.is_ok(), "{:?}", on.0);
            let want = [
                "321object",
                "654object",
                "undefinedundefinedundefinedobject",
            ]
            .repeat(3);
            assert_eq!(on.1, want, "{} stress={stress}", d.name());
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{}", d.name());
            // The first call of each builtin records it (the host function runs): fcall in
            // round 1, fapply at the first viaArgs. Forwarded: fcall in rounds 2 and 3, and
            // fapply(null, undefined) in all 3 rounds. An unmapped arguments object of this
            // realm never has the cached shape (the VM table makes no %ThrowTypeError%,
            // `arguments::fits`), so viaArgs always takes the builtin's
            // CreateListFromArrayLike. Off: none.
            assert_eq!((on.3, off.3), (5, 0), "{} stress={stress}", d.name());
        }
    }
}

/// `function add3(a, b, c) { return (c + (b + (a + ""))) + typeof this; }`.
fn add3_code() -> FunctionCode {
    let mut add3 = Asm::new(meta("add3", 3, 0, 0, 0));
    let t = add3.b.new_register();
    let ts = add3.b.feedback_slot(FeedbackSlotKind::TypeOf);
    add3.lda_str("");
    for i in 0..3 {
        let p = add3.b.parameter(i);
        add3.b.add(p, add3.bin);
    }
    let receiver = add3.b.receiver();
    add3.b.star(t).ldar(receiver).type_of(ts).add(t, add3.bin);
    add3.b.return_();
    add3.finish()
}

#[test]
fn the_t0_call_fast_path_pushes_a_forwarded_call_in_place() {
    /// The rounds: past the call record window of the isolate start (`profile::CALL_WINDOW`
    /// budget units: each of these calls goes cold), the first call of each builtin and the
    /// feedback threshold of add3 (its vector).
    const ROUNDS: usize = 40;
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(5);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("add3");
        for _ in 0..ROUNDS {
            // add3.fcall("x", 1, 2, 3): the list shape (CallProperty)
            s.lda_global("add3");
            s.b.star(l.get(0));
            s.lda_str("x");
            s.b.star(l.get(1));
            for i in 0..3 {
                s.b.lda_smi(i + 1).star(l.get(2 + i as u32));
            }
            s.lda_global("fcall");
            s.b.star(s.f).call_property(s.f, l, 5, s.call);
            s.print_acc();
            // add3.fcall(null, 7): CallProperty2, a missing argument
            s.lda_global("add3");
            s.b.star(l.get(0))
                .lda_null()
                .star(l.get(1))
                .lda_smi(7)
                .star(l.get(2));
            s.lda_global("fcall");
            s.b.star(s.f)
                .call_property2(s.f, l.get(0), l.get(1), l.get(2), s.call);
            s.print_acc();
            // add3.fapply(null, undefined): 0 arguments, this = the global object
            s.lda_global("add3");
            s.b.star(l.get(0))
                .lda_null()
                .star(l.get(1))
                .lda_undefined()
                .star(l.get(2));
            s.lda_global("fapply");
            s.b.star(s.f)
                .call_property2(s.f, l.get(0), l.get(1), l.get(2), s.call);
            s.print_acc();
            // add3.fcall(): CallProperty0, this undefined (the global object)
            s.lda_global("add3");
            s.b.star(l.get(0));
            s.lda_global("fcall");
            s.b.star(s.f).call_property0(s.f, l.get(0), s.call);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), add3_code()])
    }
    let want = [
        "321object",
        "undefinedundefined7object",
        "undefinedundefinedundefinedobject",
        "undefinedundefinedundefinedobject",
    ]
    .repeat(ROUNDS);
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run_host(make, d, stress, true);
            let off = run_host(make, d, stress, false);
            assert!(on.0.is_ok(), "{:?}", on.0);
            assert_eq!(on.1, want, "{} stress={stress}", d.name());
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{}", d.name());
            // The T0 call fast path takes most forwards (not the first call of each builtin,
            // not the call that makes add3's vector, not a call in the profile window); the
            // cold path takes the rest. Off: none.
            // The x86-64 form of the fast path does not forward (`call::forward_frame`).
            let (all, t0) = (on.3, on.4);
            assert!(
                all > ROUNDS as u64 && all <= 4 * ROUNDS as u64,
                "{}: {all}",
                d.name()
            );
            if cfg!(all(not(feature = "spike-native"), target_arch = "x86_64")) {
                assert_eq!(t0, 0, "{} stress={stress}", d.name());
            } else {
                assert!(
                    t0 > ROUNDS as u64 && t0 < all,
                    "{} stress={stress}: {t0} of {all}",
                    d.name()
                );
            }
            assert_eq!((off.3, off.4), (0, 0), "{} stress={stress}", d.name());
        }
    }
}

/// `sp()`: the native stack pointer, as a number.
fn sp<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::number(ep9_platform::stack_pointer() as f64))
}

/// The call handlers that may forward keep the sibling dispatch flat: each one ends in a tail
/// jump, so a run of forwarded calls leaves the native stack where it was (M7-P4 diag N3).
///
/// ```text
/// handler ──fast──> call::forward_frame (out of line in an unoptimized build only)
///         ──tail jump──> the next handler          (no native frame stays behind)
/// ```
///
/// At c66480e3 `forward_frame` was out of line in every build with debug assertions, also an
/// optimized one (Spike C's test build: opt-level 2 for ep9_vm). Its reader closure held the
/// address of the handler's `St`, so `St` escaped and every `Call*` handler of the sibling
/// expansion lost its tail jump: one native frame per call stayed until the dispatch run
/// ended, and the Octane crypto warm-up of Spike C threw "Maximum call stack size exceeded".
///
/// The run measures the native stack pointer (`sp()`) at the ends of 3 windows, after a
/// warm-up past the call record window and the feedback threshold of add3:
/// d0 = 2 calls of sp() in a row (what a call handler leaves behind), d1 = sp(), 50 register
/// moves, sp() (what other handlers leave behind), d2 = sp(), the 4 forwarded call shapes of
/// `the_t0_call_fast_path_pushes_a_forwarded_call_in_place` ROUNDS times, sp(). A build with
/// no tail calls at all (opt-level 0: d1 > d0) has nothing to check and returns. Else
/// d2 = d0: the forwarded calls left nothing behind.
#[test]
fn forwarding_call_handlers_keep_the_sibling_dispatch_flat() {
    const ROUNDS: usize = 40;
    /// The 4 call shapes of the T0 forward test (no print: a print is a call too).
    fn shapes(s: &mut Asm, l: RegisterList) {
        // add3.fcall("x", 1, 2, 3): CallProperty
        s.lda_global("add3");
        s.b.star(l.get(0));
        s.lda_str("x");
        s.b.star(l.get(1));
        for i in 0..3 {
            s.b.lda_smi(i + 1).star(l.get(2 + i as u32));
        }
        s.lda_global("fcall");
        s.b.star(s.f).call_property(s.f, l, 5, s.call);
        // add3.fcall(null, 7): CallProperty2
        s.lda_global("add3");
        s.b.star(l.get(0))
            .lda_null()
            .star(l.get(1))
            .lda_smi(7)
            .star(l.get(2));
        s.lda_global("fcall");
        s.b.star(s.f)
            .call_property2(s.f, l.get(0), l.get(1), l.get(2), s.call);
        // add3.fapply(null, undefined): CallProperty2, the apply form
        s.lda_global("add3");
        s.b.star(l.get(0))
            .lda_null()
            .star(l.get(1))
            .lda_undefined()
            .star(l.get(2));
        s.lda_global("fapply");
        s.b.star(s.f)
            .call_property2(s.f, l.get(0), l.get(1), l.get(2), s.call);
        // add3.fcall(): CallProperty0
        s.lda_global("add3");
        s.b.star(l.get(0));
        s.lda_global("fcall");
        s.b.star(s.f).call_property0(s.f, l.get(0), s.call);
    }
    /// acc = sp().
    fn call_sp(s: &mut Asm) {
        s.lda_global("sp");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    }
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(5);
        let (t, d0, d1, d2) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("add3");
        for _ in 0..ROUNDS {
            shapes(&mut s, l);
        }
        // d0: sp(), sp()
        call_sp(&mut s);
        s.b.star(t);
        call_sp(&mut s);
        s.b.sub(t, s.bin).star(d0);
        // d1: sp(), 50 register moves, sp()
        call_sp(&mut s);
        s.b.star(t);
        for i in 0..25 {
            s.b.lda_smi(i).star(l.get(0));
        }
        call_sp(&mut s);
        s.b.sub(t, s.bin).star(d1);
        // d2: sp(), ROUNDS x the 4 forwarded shapes, sp()
        call_sp(&mut s);
        s.b.star(t);
        for _ in 0..ROUNDS {
            shapes(&mut s, l);
        }
        call_sp(&mut s);
        s.b.sub(t, s.bin).star(d2);
        for d in [d0, d1, d2] {
            s.b.ldar(d);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), add3_code()])
    }
    let (r, out, _, all, t0) = run_host(make, Dispatch::Sibling, false, true);
    assert_eq!(r, Ok("undefined".to_string()));
    let [d0, d1, d2] = [0, 1, 2].map(|i| out[i].parse::<f64>().expect("a number"));
    // The forwards ran: in both windows of rounds; on a64 the T0 call fast path took most.
    assert!(all > ROUNDS as u64, "{all}");
    if cfg!(all(not(feature = "spike-native"), target_arch = "x86_64")) {
        assert_eq!(t0, 0);
    } else {
        assert!(t0 > ROUNDS as u64, "{t0} of {all}");
    }
    if d1 != d0 {
        eprintln!(
            "skip: this build has no tail calls (d0 {d0}, d1 {d1}: opt-level 0), so the \
             sibling dispatch grows anyway"
        );
        return;
    }
    assert_eq!(
        d2,
        d0,
        "the forwarded calls left {} bytes of native stack behind ({ROUNDS} rounds of 4 \
         calls; d0 {d0}): a Call* handler of the sibling expansion has no tail jump",
        d2 - d0
    );
}
