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
                assert_fill_eq(&a, &b, pa, regs, &case);
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

/// Taken for the whole body of [`run_host`], so no two `run_host` runs overlap (M7-P4 fixer
/// r2, reviewer R1 and J3). The human fast path switch is process-wide (`call::ANY_OFF`): while
/// one test runs with the switch off, the T0 call fast path of every other thread goes cold,
/// and a test that counts T0 forwards sees too few.
static RUN_HOST: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `make` with `echo`, `sp`, `targs`, `fcall` and `fapply` registered, the human fast
/// paths `on` or off: (result, output, host entries, forwarded calls, the forwarded calls of
/// the T0 call fast path). Holds [`RUN_HOST`] until it returns.
fn run_host(
    make: fn() -> VerifiedUnit,
    d: Dispatch,
    stress: bool,
    on: bool,
) -> (Result<String, String>, Vec<String>, u64, u64, u64) {
    use crate::call::{FORWARDED_CALLS, HOST_ENTRY_HITS, T0_FORWARDED_CALLS};
    // A test that failed with the lock held poisons it; the next test must still run.
    let _one = RUN_HOST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    super::on_big_stack(move || {
        let mut c = super::config(d, stress);
        c.builtins = Some(&FORWARDERS);
        let mut iso = isolate(c);
        iso.register_host_function("echo", 0, echo);
        iso.register_host_function("sp", 0, sp);
        iso.register_host_function("targs", 0, targs);
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
            // The T0 call path takes most forwards (not the first call of each builtin, not
            // the call that makes add3's vector; on a64 also not a call in the profile
            // window); `slow::call` takes the rest. Off: none.
            // x86-64 (not spike-native): from the cold body of the call (`forward_or_call`,
            // M8-P0 X86-CALL-FWD); before it, every x86-64 forward took `slow::call` (t0 0).
            let (all, t0) = (on.3, on.4);
            assert!(
                all > ROUNDS as u64 && all <= 4 * ROUNDS as u64,
                "{}: {all}",
                d.name()
            );
            assert!(
                t0 > ROUNDS as u64 && t0 < all,
                "{} stress={stress}: {t0} of {all}",
                d.name()
            );
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
/// x86-64: handler ──tail jump──> <op>_cold ──> forward_or_call ──> forward_frame
///         <op>_cold ──tail jump──> the next handler
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
    // The forwards ran: in both windows of rounds; the T0 call path took most (x86-64: in
    // the cold handler of the call, which ends in a tail jump too; M8-P0 X86-CALL-FWD).
    assert!(all > ROUNDS as u64, "{all}");
    assert!(t0 > ROUNDS as u64, "{t0} of {all}");
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

/// `targs(...)`: "<function|undefined|other>:<args joined by ,>", the kind of `this` and the
/// arguments: a host callee that is not a forwarder (M8-P0 X86-CALL-FWD).
fn targs<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    let t = if this.is_callable() {
        "function"
    } else if this.is_undefined() {
        "undefined"
    } else {
        "other"
    };
    let s = format!("{t}:{}", parts.join(","));
    Ok(cx.new_string(&s)?.value())
}

/// The arms of the x86-64 cold call body that do not forward (M8-P0 X86-CALL-FWD,
/// `interp::ops::call_receiver_cold` and `forward_or_call`) give the result of the cold path,
/// with the human fast paths on and off, on every target:
///
/// ```text
/// targs with this = add3 (a FUNCTION), 5 receiver shapes ──> forward_or_call: forward_frame
///   refuses (not apply or call) ──> slow::call ──> host_entry runs targs
/// targs() as CallProperty with count 0 (a list form, no receiver) ──> call_record_cold
/// add3.fcall(null, 7) in registers above 127 (Wide) ──> forward_or_call: a prefixed call
///   ──> slow::call (host_entry refuses a prefixed call) ──> the builtin runs add3
/// ```
///
/// On x86-64 (not spike-native) none of these calls forwards. The other targets may forward
/// the prefixed call in `call_fast` (the prefixed step runs the fast body).
#[test]
fn the_cold_call_arms_that_do_not_forward_give_the_cold_path_result() {
    const ROUNDS: usize = 12;
    const SHAPES: [Shape; 5] = [
        Shape::CallProperty0,
        Shape::CallProperty1,
        Shape::CallProperty2,
        Shape::CallProperty,
        Shape::CallAnyReceiver,
    ];
    fn script() -> FunctionCode {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(4);
        // 200 registers, so that the operands of the list after them need the Wide prefix
        // (one-byte register operands end at 127).
        let _pad = s.b.new_register_list(200);
        let w = s.b.new_register_list(3);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("add3");
        for _ in 0..ROUNDS {
            for shape in SHAPES {
                let n: u32 = match shape {
                    Shape::CallProperty0 => 0,
                    Shape::CallProperty1 => 1,
                    Shape::CallProperty2 => 2,
                    _ => 3,
                };
                s.lda_global("add3");
                s.b.star(l.get(0));
                for i in 1..=n {
                    s.b.lda_smi(i as i32).star(l.get(i));
                }
                s.lda_global("targs");
                s.b.star(s.f);
                let (f, slot) = (s.f, s.call);
                let list = RegisterList::new(l.get(0).local_index().unwrap(), n + 1);
                match shape {
                    Shape::CallProperty0 => s.b.call_property0(f, l.get(0), slot),
                    Shape::CallProperty1 => s.b.call_property1(f, l.get(0), l.get(1), slot),
                    Shape::CallProperty2 => {
                        s.b.call_property2(f, l.get(0), l.get(1), l.get(2), slot)
                    }
                    Shape::CallProperty => s.b.call_property(f, list, n + 1, slot),
                    _ => s.b.call_any_receiver(f, list, n + 1, slot),
                };
                s.print_acc();
            }
            // targs() as CallProperty with count 0 (an empty list): no receiver.
            let none = RegisterList::new(l.get(0).local_index().unwrap(), 0);
            s.lda_global("targs");
            s.b.star(s.f).call_property(s.f, none, 0, s.call);
            s.print_acc();
            // add3.fcall(null, 7) with Wide register operands.
            s.lda_global("add3");
            s.b.star(w.get(0))
                .lda_null()
                .star(w.get(1))
                .lda_smi(7)
                .star(w.get(2));
            s.lda_global("fcall");
            s.b.star(s.f)
                .call_property2(s.f, w.get(0), w.get(1), w.get(2), s.call);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        s.finish()
    }
    fn make() -> VerifiedUnit {
        unit("", vec![script(), add3_code()])
    }
    // The fcall of `w` is a prefixed instruction.
    let code = script();
    let wide = ep9_bytecode::Instructions::new(code.bytecode())
        .map(|i| i.expect("valid bytecode"))
        .filter(|i| {
            i.opcode == ep9_bytecode::Opcode::CallProperty2
                && i.scale == ep9_bytecode::OperandScale::Wide
        })
        .count();
    assert_eq!(wide, ROUNDS, "one Wide CallProperty2 per round");
    let want = [
        "function:",
        "function:1",
        "function:1,2",
        "function:1,2,3",
        "function:1,2,3",
        "undefined:",
        "undefinedundefined7object",
    ]
    .repeat(ROUNDS);
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run_host(make, d, stress, true);
            let off = run_host(make, d, stress, false);
            assert!(on.0.is_ok(), "{:?}", on.0);
            assert_eq!(on.1, want, "{} stress={stress}", d.name());
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{}", d.name());
            // The decode-free host entry: the 6 targs calls and the 7 prints of each round
            // (never the prefixed fcall). Off: none.
            let hits = 13 * ROUNDS as u64;
            assert_eq!((on.2, off.2), (hits, 0), "{} stress={stress}", d.name());
            if cfg!(all(not(feature = "spike-native"), target_arch = "x86_64")) {
                assert_eq!((on.3, on.4), (0, 0), "{} stress={stress}", d.name());
            }
            assert_eq!((off.3, off.4), (0, 0), "{} stress={stress}", d.name());
        }
    }
}

// ---- the decode-free host construct (M8-P2 TS-HOSTCALL) -------------------------------------

/// `hk(...)`, a host constructor: "<new|call>:<self|other>:<this>:<args joined by ,>" (whether
/// it was constructed, whether new.target is the callee, and the kind of `this`).
fn hk<'s>(cx: &mut Cx<'s>, this: Local<'s>, args: Args<'s>, nt: Local<'s>) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    let how = if args.is_construct_call() {
        "new"
    } else {
        "call"
    };
    let who = if nt.is_same_bits(&args.callee()) {
        "self"
    } else if nt.is_undefined() {
        "none"
    } else {
        "other"
    };
    let t = if this.is_undefined() { "u" } else { "x" };
    let s = format!("{how}:{who}:{t}:{}", parts.join(","));
    Ok(cx.new_string(&s)?.value())
}

/// `hthrow(...)`, a host constructor that throws a RangeError with its argument count.
fn hthrow<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let msg = format!("hthrow {}", args.len());
    Err(cx.throw_range_error(&msg))
}

/// `hk`, `hk2` (the same function, another constructor) and `hthrow` as constructors of a
/// builtin table, installed as globals.
static HOST_CTORS: crate::BuiltinTable = crate::BuiltinTable::new(
    "host-ctors",
    &[crate::BuiltinModule {
        name: "host-ctors",
        block: 104,
        defs: &[
            crate::BuiltinDef::constructor("hk", 0, hk),
            crate::BuiltinDef::constructor("hk2", 0, hk),
            crate::BuiltinDef::constructor("hthrow", 0, hthrow),
        ],
        intrinsics: &[],
        props: &[crate::ObjectProps {
            target: crate::IntrinsicId::GlobalObject,
            props: &[
                crate::PropDef::method(crate::PropKey::Str("hk"), 0),
                crate::PropDef::method(crate::PropKey::Str("hk2"), 1),
                crate::PropDef::method(crate::PropKey::Str("hthrow"), 2),
            ],
        }],
    }],
);

/// An unprefixed `Construct` of a host constructor takes the decode-free host entry
/// (`interp/slow/call.rs` `host_construct`, M8-P2 TS-HOSTCALL), with 0, 1, 3 and 9 arguments
/// (no `ArgBuf` for any count), new.target from the accumulator when it is a constructor and
/// the callee when it is not, a host constructor that throws, and a host function that is not
/// a constructor (the TypeError of `do_call`). The human fast paths on and off give the same
/// output; on, the entry counts each construct of a host constructor and each `print`; off,
/// none. Before M8-P2 a `Construct` never took the entry (the count was the prints only).
#[test]
fn a_host_construct_takes_the_decode_free_entry() {
    /// The constructs that take the entry (the throwing one included) and the prints.
    const CONSTRUCTS: u64 = 7;
    const PRINTS: u64 = 8;
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, t) = (s.b.new_register(), s.b.new_register());
        let l = s.b.new_register_list(9);
        for i in 0..9 {
            s.b.lda_smi(i + 1).star(l.get(i as u32));
        }
        let list = |n: u32| RegisterList::new(l.get(0).local_index().unwrap(), n);
        // new hk() .. new hk(1, .., 9): acc = hk, so new.target = hk.
        for n in [0, 1, 3, 9] {
            s.construct_global("hk", list(n));
            s.print_acc();
        }
        // new.target = hk2 (a constructor in the accumulator), then 7 (not one: the callee).
        s.lda_global("hk");
        s.b.star(s.f);
        s.lda_global("hk2");
        s.b.construct(s.f, list(1), 1, s.call);
        s.print_acc();
        s.lda_global("hk");
        s.b.star(s.f).lda_smi(7).construct(s.f, list(2), 2, s.call);
        s.print_acc();
        // A host constructor that throws, and a host function that is not a constructor.
        try_print_message(&mut s, ctx, t, |s| {
            s.construct_global("hthrow", list(3));
        });
        try_print_message(&mut s, ctx, t, |s| {
            s.construct_global("echo", list(1));
        });
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    fn run(d: Dispatch, stress: bool, on: bool) -> (Result<String, String>, Vec<String>, u64) {
        use crate::call::HOST_ENTRY_HITS;
        // The switch is process-wide: no `run_host` run at the same time.
        let _one = RUN_HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        super::on_big_stack(move || {
            let mut c = super::config(d, stress);
            c.builtins = Some(&HOST_CTORS);
            let mut iso = isolate(c);
            iso.register_host_function("echo", 0, echo);
            take_output();
            HOST_ENTRY_HITS.with(|h| h.set(0));
            let r = iso.with_cx_checkpoint(|cx| {
                cx.set_human_fast_paths(on);
                cx.run_script(make(), "host.js").map(|v| cx.display(v))
            });
            let out = take_output();
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            let _ = iso.with_cx(|cx| {
                cx.set_human_fast_paths(true);
                Ok(())
            });
            (
                r.map_err(|e| e.message),
                out,
                HOST_ENTRY_HITS.with(|h| h.get()),
            )
        })
    }
    let want = [
        "new:self:u:",
        "new:self:u:1",
        "new:self:u:1,2,3",
        "new:self:u:1,2,3,4,5,6,7,8,9",
        "new:other:u:1",
        "new:self:u:1,2",
        "hthrow 3",
        "function echo is not a constructor",
    ];
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run(d, stress, true);
            let off = run(d, stress, false);
            assert!(on.0.is_ok(), "{:?}", on.0);
            assert_eq!(on.1, want, "{} stress={stress}", d.name());
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{}", d.name());
            assert_eq!(
                (on.2, off.2),
                (CONSTRUCTS + PRINTS, 0),
                "{} stress={stress}",
                d.name()
            );
        }
    }
}

/// `realm_of()`: the global object of the realm the host function runs in.
fn realm_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.global().into())
}

/// JIT review M8-P2 A1 (H20 R5 on the TS-HOSTCALL path): bytecode of the first realm calls
/// and constructs host functions of a second realm through the decode-free entry
/// (`host_entry`, `host_construct` -> `call::call_host_from_bytecode`): `f()` (a host function
/// made in the second realm) gives the second realm's global, `new A(2)` (the second realm's
/// Array) has the second realm's `Array.prototype`, `twice(f)` (a first-realm host function
/// that calls `f` from Rust; ep9_vm has no `Array.prototype.map`) gives the second global too,
/// and after these `mine()` (a first-realm host function) gives the first global again. The
/// human fast paths on and off give the same values; on, the entry counts the 4 host calls of
/// the bytecode (`f`, `new A`, `twice`, `mine`);
/// off, none. Before this test no test made bytecode call or construct a host function of
/// another realm through the entry (realm.rs calls from Rust; the FASTDIFF case has one realm).
#[test]
fn a_host_callee_of_another_realm_runs_in_its_realm_through_the_entry() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(1);
        // r1 = f()
        s.lda_global("f");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.sta_global("r1");
        // r2 = new A(2)
        s.b.lda_smi(2).star(l.get(0));
        s.construct_global("A", l);
        s.sta_global("r2");
        // r3 = twice(f): a first-realm host function calls `f` from Rust (twice).
        s.lda_global("f");
        s.b.star(s.a.get(0));
        s.lda_global("twice");
        s.b.star(s.f)
            .call_undefined_receiver1(s.f, s.a.get(0), s.call);
        s.sta_global("r3");
        // r4 = mine()
        s.lda_global("mine");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.sta_global("r4");
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    /// (r1 is the second global, r2's prototype is the second Array.prototype and not the
    /// first, r3 is the second global, r4 is the first global, the first realm is current
    /// after the run), the host entries.
    fn run(d: Dispatch, stress: bool, on: bool) -> ([bool; 6], u64) {
        use crate::call::HOST_ENTRY_HITS;
        let _one = RUN_HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        super::on_big_stack(move || {
            let mut iso = isolate(super::config(d, stress));
            iso.register_host_function("mine", 0, realm_of);
            iso.register_host_function("twice", 1, call_twice);
            HOST_ENTRY_HITS.with(|h| h.set(0));
            let first = iso.cx.realm;
            let r = iso.with_cx_checkpoint(|cx| {
                cx.set_human_fast_paths(on);
                let g1 = cx.global();
                let g2 = cx.create_realm()?;
                let f = cx.with_realm(g2, |cx| {
                    Ok(cx.new_builtin_closure(realm_of, "f", 0, Local::undefined()))
                })?;
                let a2 = cx.get(g2, "Array")?;
                cx.set(g1, "A", a2, true)?;
                cx.set(g1, "f", f, true)?;
                cx.set(g1, "g2", g2, true)?;
                cx.run_script(make(), "realms.js")?;
                let g1 = cx.global();
                let g2 = cx.get(g1, "g2")?;
                let a2 = cx.get_v(g2, "Array")?;
                let p2 = cx.get_v(a2, "prototype")?;
                let a1 = cx.get(g1, "Array")?;
                let p1 = cx.get_v(a1, "prototype")?;
                let r1 = cx.get(g1, "r1")?;
                let r2 = cx.get(g1, "r2")?;
                let r2p = crate::object::proto_of(r2.bits());
                let r3 = cx.get(g1, "r3")?;
                let r4 = cx.get(g1, "r4")?;
                Ok([
                    r1.bits() == g2.bits(),
                    r2p == p2.bits(),
                    r2p != p1.bits(),
                    r3.bits() == g2.bits(),
                    r4.bits() == g1.bits(),
                    cx.inner().realm == first,
                ])
            });
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            let _ = iso.with_cx(|cx| {
                cx.set_human_fast_paths(true);
                Ok(())
            });
            (r.unwrap(), HOST_ENTRY_HITS.with(|h| h.get()))
        })
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let on = run(d, stress, true);
            let off = run(d, stress, false);
            assert_eq!(on.0, [true; 6], "{label}");
            assert_eq!(on.0, off.0, "{label}");
            assert_eq!((on.1, off.1), (4, 0), "{label}");
        }
    }
}

// ---- the host call of CallProperty0-2 (M9 OXC-W2-HOSTCALL, `slow::call_property`) ---------

/// Sets the budget of `iso` below the call record window of the isolate start
/// (`profile::CALL_WINDOW`), as `tests/profile.rs`
/// `calls_outside_the_window_are_not_recorded_until_a_poll` does. A host call spends no
/// budget unit (H23), so each call of a short script runs in that window. On x86-64 (not
/// spike-native) the cold body of `CallProperty0`-`2` (`interp/ops.rs` `call_property_cold`)
/// sends a call in the window to the record and `slow::call`, not to `slow::call_property`.
fn start_below_the_call_window(iso: &mut crate::Isolate) {
    iso.cx.budget = crate::interp::BUDGET / 2;
}

/// The budget after a run that [`start_below_the_call_window`] started is still in
/// `1..=BUDGET / 2`: no poll ran (a poll sets the budget to `BUDGET` or more), so the budget
/// was below the call record window at each call of the run.
fn assert_below_the_call_window(iso: &crate::Isolate, label: &str) {
    use crate::interp::BUDGET;
    use crate::profile::CALL_WINDOW;
    const { assert!(BUDGET / 2 <= BUDGET - CALL_WINDOW) };
    let b = iso.cx.budget;
    assert!((1..=BUDGET / 2).contains(&b), "{label}: budget {b}");
}

/// The bytecode offset that the caller frame of this host call saved (H14 header slot 7),
/// which `Runtime::enter` writes before a host call. An error made before the host frame
/// exists (the RangeError of a full VM or native stack) takes the caller's position in its
/// stack from this slot (`error::capture`: the top frame's `saved_offset`).
fn caller_offset<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // The host frame is the top frame (`call_host_from_bytecode`, `call_host`).
    let fp = cx.inner().rt().stack().fp();
    let off = crate::error::FrameRef(fp).caller().saved_offset();
    Ok(Local::int32(off as i32))
}

/// Runs `make` with `echo`, `hthrow`, `mine` and `at` ([`caller_offset`]) registered, the
/// budget below the call record window ([`start_below_the_call_window`]) and the human fast
/// paths `on` or off: (result, output, host entries, the host calls that
/// `slow::call_property` took). Holds [`RUN_HOST`] until it returns.
fn run_property(
    make: fn() -> VerifiedUnit,
    d: Dispatch,
    stress: bool,
    on: bool,
) -> (Result<String, String>, Vec<String>, u64, u64) {
    use crate::call::{HOST_ENTRY_HITS, HOST_PROPERTY_HITS};
    let _one = RUN_HOST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    super::on_big_stack(move || {
        let mut iso = isolate(super::config(d, stress));
        iso.register_host_function("echo", 0, echo);
        iso.register_host_function("hthrow", 0, hthrow);
        iso.register_host_function("mine", 0, realm_of);
        iso.register_host_function("at", 0, caller_offset);
        take_output();
        HOST_ENTRY_HITS.with(|h| h.set(0));
        HOST_PROPERTY_HITS.with(|h| h.set(0));
        start_below_the_call_window(&mut iso);
        let r = iso.with_cx_checkpoint(|cx| {
            cx.set_human_fast_paths(on);
            cx.run_script(make(), "property.js").map(|v| cx.display(v))
        });
        assert_below_the_call_window(&iso, &format!("{} stress={stress} on={on}", d.name()));
        let out = take_output();
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        let _ = iso.with_cx(|cx| {
            cx.set_human_fast_paths(true);
            Ok(())
        });
        (
            r.map_err(|e| e.message),
            out,
            HOST_ENTRY_HITS.with(|h| h.get()),
            HOST_PROPERTY_HITS.with(|h| h.get()),
        )
    })
}

/// A host callee of `CallProperty0`, `CallProperty1` and `CallProperty2` with a receiver that
/// is not a FUNCTION (a string, an int32, an object) takes `slow::call_property`: the same
/// receiver, arguments and result as the general path (the fast paths off), and one hit per
/// call. The calls it refuses take `slow::call` with the same result: a `Wide` call (an
/// argument register above 127), a FUNCTION receiver (the forward test of `host_entry`, which
/// counts it), and a bytecode callee (no host entry at all).
#[test]
fn a_host_method_call_takes_the_fixed_property_path_in_each_arity() {
    fn make() -> VerifiedUnit {
        // f1: function id(a) { return a; }, a bytecode callee
        let mut id = Asm::new(meta("id", 1, 0, 0, 0));
        let p = id.b.parameter(0);
        id.b.ldar(p).return_();
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(4);
        for shape in [
            Shape::CallProperty0,
            Shape::CallProperty1,
            Shape::CallProperty2,
        ] {
            emit_shape(&mut s, shape, "echo", l);
            s.print_acc();
        }
        let (f, slot) = (s.f, s.call);
        // 7 .echo(1): an int32 receiver.
        s.b.lda_smi(7).star(l.get(0)).lda_smi(1).star(l.get(1));
        s.lda_global("echo");
        s.b.star(f).call_property1(f, l.get(0), l.get(1), slot);
        s.print_acc();
        // globalThis.echo(2, 3): an object receiver.
        s.lda_global("globalThis");
        s.b.star(l.get(0))
            .lda_smi(2)
            .star(l.get(1))
            .lda_smi(3)
            .star(l.get(2));
        s.lda_global("echo");
        s.b.star(f)
            .call_property2(f, l.get(0), l.get(1), l.get(2), slot);
        s.print_acc();
        // "R".echo(r150): a Wide call.
        let big = s.b.new_register_list(160);
        s.lda_str("R");
        s.b.star(l.get(0)).lda_smi(5).star(big.get(150));
        s.lda_global("echo");
        s.b.star(f).call_property1(f, l.get(0), big.get(150), slot);
        s.print_acc();
        // id.echo(): a FUNCTION receiver; print the result's length (echo prints the source).
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(l.get(0));
        s.lda_global("echo");
        s.b.star(f).call_property0(f, l.get(0), slot);
        s.b.star(l.get(1));
        s.get_named(l.get(1), "length");
        s.print_acc();
        // "R".id(9): a bytecode callee.
        s.lda_str("R");
        s.b.star(l.get(0)).lda_smi(9).star(l.get(1));
        s.b.create_closure(k, s.closure, 0).star(f);
        s.b.call_property1(f, l.get(0), l.get(1), slot);
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), id.finish()])
    }
    let want = ["R:", "R:1", "R:1,2", "7:1", "[object Object]:2,3", "R:5"];
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let on = run_property(make, d, stress, true);
            let off = run_property(make, d, stress, false);
            assert!(on.0.is_ok(), "{label}: {:?}", on.0);
            assert_eq!(on.1[..want.len()], want, "{label}");
            assert_eq!(on.1.len(), want.len() + 2, "{label}");
            assert_eq!(on.1[want.len() + 1], "9", "{label}");
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{label}");
            // On: 5 property calls take the path; host entries: those 5, the FUNCTION
            // receiver call and 8 prints. Off: none.
            assert_eq!((on.3, off.3), (5, 0), "{label}");
            assert_eq!((on.2, off.2), (5 + 1 + 8, 0), "{label}");
        }
    }
}

/// A host callee of the fixed property path that throws: the exception unwinds from the call
/// (a handler in the same function catches it), with the same message and stack as the
/// general path, and the script goes on after it. A host function of another realm runs in
/// its realm on this path (H20 R5), and the caller's realm is current again after it.
#[test]
fn a_host_method_call_throws_and_switches_realms_as_the_general_path() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, t) = (s.b.new_register(), s.b.new_register());
        let l = s.b.new_register_list(3);
        let (f, slot) = (s.f, s.call);
        // try { "R".hthrow(1) } catch (e) { print(e.message) }, then e.stack.
        try_print_message(&mut s, ctx, t, |s| {
            s.lda_str("R");
            s.b.star(l.get(0)).lda_smi(1).star(l.get(1));
            s.lda_global("hthrow");
            s.b.star(f).call_property1(f, l.get(0), l.get(1), slot);
        });
        s.get_named(t, "stack");
        s.print_acc();
        // r1 = "R".f(), a host function of the second realm; r2 = "R".mine().
        for (callee, out) in [("f", "r1"), ("mine", "r2")] {
            s.lda_str("R");
            s.b.star(l.get(0));
            s.lda_global(callee);
            s.b.star(f).call_property0(f, l.get(0), slot);
            s.sta_global(out);
        }
        s.lda_str("after");
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    /// (output, [r1 is the second global, r2 the first, the first realm is current], hits).
    fn run(d: Dispatch, stress: bool, on: bool) -> (Vec<String>, [bool; 3], u64) {
        use crate::call::HOST_PROPERTY_HITS;
        let _one = RUN_HOST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        super::on_big_stack(move || {
            let mut iso = isolate(super::config(d, stress));
            iso.register_host_function("hthrow", 0, hthrow);
            iso.register_host_function("mine", 0, realm_of);
            take_output();
            HOST_PROPERTY_HITS.with(|h| h.set(0));
            let first = iso.cx.realm;
            start_below_the_call_window(&mut iso);
            let r = iso.with_cx_checkpoint(|cx| {
                cx.set_human_fast_paths(on);
                let g1 = cx.global();
                let g2 = cx.create_realm()?;
                let f = cx.with_realm(g2, |cx| {
                    Ok(cx.new_builtin_closure(realm_of, "f", 0, Local::undefined()))
                })?;
                cx.set(g1, "f", f, true)?;
                cx.run_script(make(), "property-realm.js")?;
                let g1 = cx.global();
                let r1 = cx.get(g1, "r1")?;
                let r2 = cx.get(g1, "r2")?;
                Ok([
                    r1.bits() == g2.bits(),
                    r2.bits() == g1.bits(),
                    cx.inner().realm == first,
                ])
            });
            assert_below_the_call_window(&iso, &format!("{} stress={stress} on={on}", d.name()));
            let out = take_output();
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            let _ = iso.with_cx(|cx| {
                cx.set_human_fast_paths(true);
                Ok(())
            });
            (out, r.unwrap(), HOST_PROPERTY_HITS.with(|h| h.get()))
        })
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let on = run(d, stress, true);
            let off = run(d, stress, false);
            assert_eq!(on.0.len(), 3, "{label}: {:?}", on.0);
            assert_eq!(
                (on.0[0].as_str(), on.0[2].as_str()),
                ("hthrow 1", "after"),
                "{label}"
            );
            assert!(
                on.0[1].starts_with("RangeError: hthrow 1"),
                "{label}: {}",
                on.0[1]
            );
            assert_eq!(on.0, off.0, "{label}");
            assert_eq!(on.1, [true; 3], "{label}");
            assert_eq!(on.1, off.1, "{label}");
            // On: hthrow, f and mine take the path. Off: none.
            assert_eq!((on.2, off.2), (3, 0), "{label}");
        }
    }
}

/// The fixed property path saves the caller's bytecode offset before the host call, as the
/// general path does (`Runtime::enter`): `"R".at()`, `"R".at(1)` and `"R".at(1, 2)`
/// ([`caller_offset`]) give the offsets of their 3 call instructions, the same with the fast
/// paths on and off. The test fails when `call_property` does not call `enter`: then the slot
/// holds the offset of an earlier slow path of the frame (the `LdaGlobal` of `at`) or 0.
#[test]
fn a_host_method_call_saves_the_caller_offset_as_the_general_path() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(3);
        for shape in [
            Shape::CallProperty0,
            Shape::CallProperty1,
            Shape::CallProperty2,
        ] {
            emit_shape(&mut s, shape, "at", l);
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let on = run_property(make, d, stress, true);
            let off = run_property(make, d, stress, false);
            assert!(on.0.is_ok(), "{label}: {:?}", on.0);
            assert_eq!((&on.0, &on.1), (&off.0, &off.1), "{label}");
            let offs: Vec<u32> = on.1.iter().map(|o| o.parse().unwrap()).collect();
            assert_eq!(offs.len(), 3, "{label}: {offs:?}");
            assert!(
                0 < offs[0] && offs[0] < offs[1] && offs[1] < offs[2],
                "{label}: {offs:?}"
            );
            assert_eq!((on.3, off.3), (3, 0), "{label}");
        }
    }
}

// ---- the x86-64 register fill of a call frame (M10-P3 LEVER-13, CALL-FILL) ----------------
//
// On x86-64 (not `spike-native`), `call::write_call_header` writes the registers of a new frame
// 2 slots per pass, so for an odd register count it writes undefined to the slot at the frame
// top too (`VmStack::GUARD` is 1 there, else 0). The frame tests read `VmStack::GUARD`, not the
// target, so they check the round-up wherever the fill has it; the bytecode tests run the T0
// call fast path where the target has it (x86-64) and the frame push of the other paths.

/// The buffer compare of the fast-frame tests: `fast` (the writes of the fast path) is `push`
/// (the writes of `call::push_frame`) slot for slot, but for the slot at the frame top: with
/// the round-up (`VmStack::GUARD` 1) and an odd `regs`, `fast` has undefined there, else that
/// slot is not written. `fp` is the frame in `push` (`None`: no frame; the buffers are equal).
fn assert_fill_eq(push: &[u64], fast: &[u64], fp: Option<*mut u64>, regs: u32, case: &str) {
    use crate::frame::{UNDEFINED, VmStack};
    // Not a constant for the compare (it is 0 on most targets).
    let guard = std::hint::black_box(VmStack::GUARD);
    assert!(guard <= 1, "the fill writes at most 1 slot past the top");
    let mut want = push.to_vec();
    if let Some(fp) = fp {
        let top = (fp.addr() - push.as_ptr().addr()) / 8 + regs as usize;
        if guard == 1 && regs % 2 == 1 {
            want[top] = UNDEFINED;
        }
    }
    assert_eq!(fast, &want[..], "{case}");
}

/// Writes the frame of `fi` with `argc` arguments at slot 4 of two `len`-slot buffers full of
/// a mark, with the limit `room` slots above the base: by `call::push_frame` (the first buffer)
/// and by the steps of the fast path (the second), as `the_fast_frame_write_equals_push_frame`
/// does. Returns each buffer and its frame.
fn write_both(
    fi: &crate::unit::FunctionInfo,
    argc: u32,
    list: bool,
    early: bool,
    room: usize,
    len: usize,
) -> (Vec<u64>, Option<*mut u64>, Vec<u64>, Option<*mut u64>) {
    use crate::call::{
        call_frame_fp, fill_missing_args, push_frame, write_call_args, write_call_header,
    };
    use crate::frame::{UNDEFINED, int32_bits};
    const MARK: u64 = 0x7FF8_0000_DEAD_BEEF;
    let (callee, this, context) = (int32_bits(1), int32_bits(2), int32_bits(3));
    let caller_fp = core::ptr::without_provenance_mut::<u64>(0x1000);
    let return_pc = core::ptr::without_provenance::<u8>(0x2000);
    let arg = |i: u32| int32_bits(100 + i as i32);
    let (mut a, mut b) = (vec![MARK; len], vec![MARK; len]);
    assert!(room + 4 < len, "room for the round-up slot");
    // SAFETY: base + room + 1 lies inside each buffer (checked above).
    let pa = unsafe {
        let base = a.as_mut_ptr().add(4);
        push_frame(
            base,
            base.add(room),
            fi,
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
        call_frame_fp(base, base.add(room), fi, argc).inspect(|&fp| {
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
            write_call_header(fp, fi, callee, argc, caller_fp, return_pc, || context);
            if !early {
                args();
            }
        })
    };
    (a, pa, b, pb)
}

/// LEVER-13: frames with 0, 1, 7, 8, 9 and 17 registers (even and odd counts, below, at and
/// above 8) from the fast path equal the frames of `call::push_frame`, for 0 to 3 arguments
/// and 0 or 2 parameters, the list and fixed-count argument stores, before and after the
/// header, and a limit one slot too low, at the frame top, and 1 and 3 slots above it.
#[test]
fn fast_frames_of_0_1_7_8_9_and_17_registers_equal_push_frame() {
    use crate::frame::HEADER;
    use crate::unit::FunctionInfo;
    for regs in [0u32, 1, 7, 8, 9, 17] {
        for params in [0u32, 2] {
            let mut fi = FunctionInfo::host(never_called, "f", 0, false);
            fi.parameter_count = params;
            fi.register_count = regs;
            for argc in 0..=3u32 {
                for (list, early) in [(true, false), (true, true), (false, false), (false, true)] {
                    if !list && argc > 2 {
                        continue;
                    }
                    let size = argc.max(params) as usize + 1 + HEADER + regs as usize;
                    for room in [size - 1, size, size + 1, size + 3] {
                        let (a, pa, b, pb) = write_both(&fi, argc, list, early, room, 64);
                        let case = format!(
                            "regs {regs} params {params} argc {argc} room {room} list {list} \
                             early {early}"
                        );
                        assert_eq!(
                            pa.map(|p| p.addr() - a.as_ptr().addr()),
                            pb.map(|p| p.addr() - b.as_ptr().addr()),
                            "{case}"
                        );
                        assert_eq!(pa.is_some(), room >= size, "{case}");
                        assert_fill_eq(&a, &b, pa, regs, &case);
                    }
                }
            }
        }
    }
}

/// LEVER-13: a frame that ends at the end of a real `VmStack` (the stack limit of an isolate,
/// `isolate.rs` `stack_limit: rt.stack().end()`). `end()` stays `base + slots`; the fast path
/// and `call::push_frame` take the frame and refuse one slot more alike; with the round-up the
/// store past the top lands in the guard slot past `end()` (allocated, no frame owns it), else
/// that slot is not written.
#[test]
fn a_fast_frame_that_ends_at_the_stack_end_puts_the_round_up_in_the_guard_slot() {
    use crate::call::{
        call_frame_fp, fill_missing_args, push_frame, write_call_args, write_call_header,
    };
    use crate::frame::{HEADER, UNDEFINED, VmStack, int32_bits};
    use crate::unit::FunctionInfo;
    const SLOTS: usize = 64;
    let (callee, this, context) = (int32_bits(1), int32_bits(2), int32_bits(3));
    let caller_fp = core::ptr::without_provenance_mut::<u64>(0x1000);
    let return_pc = core::ptr::without_provenance::<u8>(0x2000);
    let arg = |i: u32| int32_bits(100 + i as i32);
    for regs in [0u32, 1, 7, 8, 9, 17] {
        for argc in 0..=2u32 {
            let mut fi = FunctionInfo::host(never_called, "f", 0, false);
            fi.parameter_count = 1;
            fi.register_count = regs;
            let case = format!("regs {regs} argc {argc}");
            let size = argc.max(1) as usize + 1 + HEADER + regs as usize;
            let (pushed, fast) = (VmStack::new(SLOTS), VmStack::new(SLOTS));
            for s in [&pushed, &fast] {
                assert_eq!(s.end().addr() - s.base().addr(), SLOTS * 8, "{case}");
            }
            // One slot too big: both refuse, and nothing is written.
            // SAFETY: the base lies inside the stack; the limit is its end.
            unsafe {
                let base = pushed.end().sub(size - 1);
                let p = push_frame(
                    base,
                    pushed.end(),
                    &fi,
                    callee,
                    this,
                    argc,
                    arg,
                    caller_fp,
                    return_pc,
                    context,
                    UNDEFINED,
                );
                assert!(p.is_none(), "{case}");
                let base = fast.end().sub(size - 1);
                assert!(
                    call_frame_fp(base, fast.end(), &fi, argc).is_none(),
                    "{case}"
                );
            }
            // SAFETY: the frame `[base, end)` lies inside the stack; the limit is its end.
            let (pa, pb) = unsafe {
                let base = pushed.end().sub(size);
                let pa = push_frame(
                    base,
                    pushed.end(),
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
                .expect(&case);
                let base = fast.end().sub(size);
                let pb = call_frame_fp(base, fast.end(), &fi, argc).expect(&case);
                fill_missing_args(base, pb, argc);
                write_call_args::<false>(pb, this, argc, arg);
                write_call_header(pb, &fi, callee, argc, caller_fp, return_pc, || context);
                (pa, pb)
            };
            assert_eq!(pa.wrapping_add(regs as usize), pushed.end(), "{case}");
            assert_eq!(pb.wrapping_add(regs as usize), fast.end(), "{case}");
            // SAFETY: `[base, end)` is the stack memory (zeroed, then written).
            let (a, b) = unsafe {
                (
                    core::slice::from_raw_parts(pushed.base(), SLOTS).to_vec(),
                    core::slice::from_raw_parts(fast.base(), SLOTS).to_vec(),
                )
            };
            assert_eq!(a, b, "{case}: the slots below the end");
            if std::hint::black_box(VmStack::GUARD) > 0 {
                // SAFETY: `VmStack::new` allocates `GUARD` slots past `end()`.
                let (ga, gb) = unsafe { (*pushed.end(), *fast.end()) };
                assert_eq!(ga, 0, "{case}: push_frame writes no guard slot");
                let want = if regs % 2 == 1 { UNDEFINED } else { 0 };
                assert_eq!(gb, want, "{case}: the guard slot");
            }
        }
    }
}

/// `function name(a, b) { "use strict"; .. }` with exactly `regs` registers: returns -1 if
/// every register reads undefined at entry, else the index of the first one that does not.
/// Strict, so a primitive receiver (`emit_shape` gives "R") does not send the call cold
/// (`call::try_bind_this`): every call form reaches the fast frame write.
fn undefined_reader(name: &str, regs: u32) -> FunctionCode {
    let mut m = meta(name, 2, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut b = ep9_bytecode::FunctionBuilder::new(m);
    let r: Vec<Register> = (0..regs).map(|_| b.new_register()).collect();
    assert_eq!(b.register_count(), regs);
    let bad: Vec<ep9_bytecode::Label> = r.iter().map(|_| b.new_label()).collect();
    for (&reg, &l) in r.iter().zip(&bad) {
        b.ldar(reg).jump_if_not_undefined(l);
    }
    b.lda_smi(-1).return_();
    for (i, &l) in bad.iter().enumerate() {
        b.bind(l);
        b.lda_smi(i as i32).return_();
    }
    b.finish().unwrap()
}

/// `function dirty() { r0 = .. = r23 = 42 }`: leaves non-undefined Values in the 24 slots
/// above its frame base, where the next frame of the same call site lies.
fn dirty() -> FunctionCode {
    let mut b = ep9_bytecode::FunctionBuilder::new(meta("dirty", 0, 0, 0, 0));
    let r: Vec<Register> = (0..24).map(|_| b.new_register()).collect();
    b.lda_smi(42);
    for reg in r {
        b.star(reg);
    }
    b.lda_undefined().return_();
    b.finish().unwrap()
}

/// The register counts of the `undefined_reader` functions `u<n>`.
const READER_REGS: [u32; 6] = [0, 1, 7, 8, 9, 17];

/// The script part that defines `dirty` (function 1) and `u0` .. `u17` (functions 2..=7).
fn define_readers(s: &mut Asm) {
    let names = ["dirty"].into_iter().chain(READER_REGS.map(reader_name));
    for (i, name) in names.enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
}

fn reader_name(regs: u32) -> &'static str {
    match regs {
        0 => "u0",
        1 => "u1",
        7 => "u7",
        8 => "u8",
        9 => "u9",
        _ => "u17",
    }
}

/// Functions 1..=7 of the units below: `dirty` and the readers.
fn reader_fns() -> Vec<FunctionCode> {
    let mut fns = vec![dirty()];
    fns.extend(READER_REGS.map(|n| undefined_reader(reader_name(n), n)));
    fns
}

/// `dirty()` from the script, with the frame at the base of the next call.
fn call_dirty(s: &mut Asm) {
    s.lda_global("dirty");
    s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
}

/// `peek(k)`: the slot `k` above the top of the caller frame (the base of the frame that the
/// caller pushed last): 1 if it holds undefined, 0 if it holds 42 (from `dirty`), else -1. It
/// reads the slot first, before any other step of the host call.
fn peek<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    use crate::frame::{UNDEFINED, int32_bits, native_ptr, slot};
    use crate::runtime::frame_top;
    let stack = cx.inner().rt().stack();
    assert!(
        stack.scratch_top().is_null(),
        "no scratch top at the host entry"
    );
    let k = args.get(0).as_int32().expect("peek(k): an int32 k") as usize;
    // SAFETY: the host frame is live; its caller slot was written at its push.
    let caller = unsafe { native_ptr::<u64>(*stack.fp().offset(slot::CALLER_FP)) };
    let at = frame_top(caller).wrapping_add(k);
    assert!(
        at.addr() < stack.end().addr(),
        "slot {k} lies inside the stack"
    );
    // SAFETY: `at` lies in the stack, below `end()` (checked above).
    let bits = unsafe { *at };
    Ok(Local::int32(if bits == UNDEFINED {
        1
    } else if bits == int32_bits(42) {
        0
    } else {
        -1
    }))
}

/// LEVER-13 (Codex focus 1): every register of a new frame reads undefined, for 0, 1, 7, 8, 9
/// and 17 registers, in each of the 9 call forms (0 to 3 arguments, 2 parameters: missing and
/// extra arguments), each time after `dirty` left 42 in the slots of the new frame; under
/// every dispatch expansion, with and without GC stress. After each call, `peek` reads the
/// slot at the top of the callee frame: undefined (1) shows that the call wrote the frame by
/// `fill_registers` with the round-up (`VmStack::GUARD` 1, an odd count), 42 (0) that it did
/// not write there. Each reader is called 8 times first, so the threshold-8 cold call
/// (`count_call`) is used up before the shapes.
#[test]
fn every_register_of_a_new_frame_reads_undefined_after_a_dirty_frame() {
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
    fn argc(shape: Shape) -> u32 {
        match shape {
            Shape::CallProperty0 | Shape::CallUndefinedReceiver0 => 0,
            Shape::CallProperty1 | Shape::CallUndefinedReceiver1 => 1,
            Shape::CallProperty2 | Shape::CallUndefinedReceiver2 => 2,
            _ => 3,
        }
    }
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(4);
        let (p, save) = (s.b.new_register(), s.b.new_register());
        define_readers(&mut s);
        s.lda_global("peek");
        s.b.star(p);
        for regs in READER_REGS {
            for _ in 0..8 {
                s.lda_global(reader_name(regs));
                s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
            }
            for shape in SHAPES {
                // The slot at the callee frame top, from the base of the callee frame. It
                // lies in the 24 slots that `dirty` wrote, above the frame of `peek`.
                let k = argc(shape).max(2) + 1 + crate::frame::HEADER as u32 + regs;
                for _ in 0..2 {
                    call_dirty(&mut s);
                    emit_shape(&mut s, shape, reader_name(regs), l);
                    s.b.star(save);
                    s.b.lda_smi(k as i32).star(s.a.get(0));
                    s.b.call_undefined_receiver1(p, s.a.get(0), s.call);
                    s.print_acc();
                    s.b.ldar(save);
                    s.print_acc();
                }
            }
        }
        s.b.lda_undefined().return_();
        let mut fns = vec![s.finish()];
        fns.extend(reader_fns());
        unit("", fns)
    }
    let want: Vec<&str> = READER_REGS
        .iter()
        .flat_map(|&regs| {
            let top = if std::hint::black_box(crate::frame::VmStack::GUARD) == 1 && regs % 2 == 1 {
                "1"
            } else {
                "0"
            };
            std::iter::repeat_n([top, "-1"], SHAPES.len() * 2).flatten()
        })
        .collect();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let (r, output) = super::on_big_stack(move || {
                let mut iso = isolate(super::config(d, stress));
                iso.register_host_function("peek", 1, peek);
                take_output();
                let r = iso.with_cx_checkpoint(|cx| {
                    cx.run_script(make(), "fill.js").map(|v| cx.display(v))
                });
                let output = take_output();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (r.map_err(|e| e.message), output)
            });
            assert_eq!(r.as_deref(), Ok("undefined"), "{label}");
            assert_eq!(output, want, "{label}");
        }
    }
}

/// LEVER-13 (Codex focus 2): a throw out of fast-called frames with odd register counts, and
/// a generator that makes fast calls and is resumed above a dirty frame: the catch, the
/// resumed registers and every new frame after them read right. `thrower` (5 registers) calls
/// `u7`, then throws "boom"; `middle` (5 registers) calls `thrower` with no handler, so the
/// throw unwinds 2 frames. `gen` (9 registers): `x = u7(); yield x + 10; return x + u7();`.
#[test]
fn a_throw_and_a_generator_resume_after_fast_calls_with_the_round_up() {
    fn odd(name: &str) -> Asm {
        let mut f = Asm::new(meta(name, 0, 0, 0, 0));
        f.b.new_register();
        assert_eq!(f.b.register_count(), 5);
        f
    }
    fn make() -> VerifiedUnit {
        // function thrower() { u7(); throw "boom"; }
        let mut t = odd("thrower");
        t.lda_global("u7");
        t.b.star(t.f).call_undefined_receiver0(t.f, t.call);
        t.lda_str("boom");
        t.b.throw();
        // function middle() { return thrower(); }
        let mut m = odd("middle");
        m.lda_global("thrower");
        m.b.star(m.f)
            .call_undefined_receiver0(m.f, m.call)
            .return_();
        // function* gen() { const x = u7(); yield x + 10; return x + u7(); } (the Gen form of
        // tests/generator.rs, with no resume-mode switch: the script calls only `next`).
        let mut gm = meta("gen", 0, 0, 0, 0);
        gm.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::GENERATOR;
        let mut g = Asm::new(gm);
        let (gr, x) = (g.b.new_register(), g.b.new_register());
        let tl = g.b.new_register_list(2);
        g.b.new_register();
        let save = RegisterList::new(0, g.b.register_count());
        assert_eq!(save.len(), 9);
        let (table, resumes) = g.b.jump_table(2);
        let this = g.b.receiver();
        g.b.lda_new_target().star(gr);
        g.b.switch_on_generator_state(gr, table);
        g.b.lda_current_function().star(tl.get(0));
        g.b.ldar(this).star(tl.get(1));
        g.b.call_runtime(RuntimeId::CreateGeneratorObject, tl, 2);
        g.b.star(gr);
        g.b.suspend_generator(gr, save, save.len(), 0);
        g.b.bind(resumes[0]);
        g.b.resume_generator(gr, save, save.len());
        g.lda_global("u7");
        g.b.star(g.f).call_undefined_receiver0(g.f, g.call).star(x);
        g.b.add_smi(10, g.bin);
        g.b.star(tl.get(0)).lda_false().star(tl.get(1));
        g.b.call_runtime(RuntimeId::CreateIterResultObject, tl, 2);
        g.b.suspend_generator(gr, save, save.len(), 1);
        g.b.bind(resumes[1]);
        g.b.resume_generator(gr, save, save.len());
        g.lda_global("u7");
        g.b.star(g.f)
            .call_undefined_receiver0(g.f, g.call)
            .add(x, g.bin)
            .return_();
        // The script.
        let mut s = Asm::script(0);
        let (ctx, e, it, res) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        define_readers(&mut s);
        for (id, name) in [(8, "thrower"), (9, "middle"), (10, "gen")] {
            let k = s.b.function_constant(FunctionId(id));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        // try { thrower() } catch (e) { print(e) }, then try { middle() } catch (e) { print(e) },
        // each followed by `print(u7())` at the same base.
        for callee in ["thrower", "middle"] {
            for _ in 0..2 {
                let (start, end, handler, done) = (
                    s.b.new_label(),
                    s.b.new_label(),
                    s.b.new_label(),
                    s.b.new_label(),
                );
                s.b.lda_current_context().star(ctx);
                s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
                s.b.bind(start);
                call_dirty(&mut s);
                s.lda_global(callee);
                s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
                s.b.bind(end);
                s.b.jump(done);
                s.b.bind(handler);
                s.b.star(e).ldar(e);
                s.print_acc();
                s.b.bind(done);
                call_dirty(&mut s);
                s.lda_global("u7");
                s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
                s.print_acc();
            }
        }
        // it = gen(); then `it.next()` twice after `dirty()`: print value and done.
        s.lda_global("gen");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call).star(it);
        for _ in 0..2 {
            call_dirty(&mut s);
            s.get_named(it, "next");
            s.b.star(s.f).call_property0(s.f, it, s.call).star(res);
            s.get_named(res, "value");
            s.print_acc();
            s.get_named(res, "done");
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        let mut fns = vec![s.finish()];
        fns.extend(reader_fns());
        fns.extend([t.finish(), m.finish(), g.finish()]);
        unit("", fns)
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "boom", "-1", "boom", "-1", "boom", "-1", "boom", "-1", "9", "false", "-2", "true"
        ]
    );
}

/// `scanRange()`: checks the GC root range (`Runtime::trace`: `[base, max(top, scratch top))`)
/// from a host frame that a fast-called frame with an odd register count pushed: no scratch
/// top, the range ends at the host frame top inside `[base, end()]`, the host frame starts at
/// the caller's top (so the round-up slot of the caller is a host-frame slot that the host
/// frame push wrote), and every slot in the range holds a valid Value. Returns undefined.
fn scan_range<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    use crate::frame::{CONSTRUCT_BIT, HEADER, int32_of, native_ptr, slot};
    use crate::runtime::frame_top;
    let rt = cx.inner().rt();
    let stack = rt.stack();
    let host = stack.fp();
    assert!(
        stack.scratch_top().is_null(),
        "bytecode runs with no scratch top"
    );
    // SAFETY: `host` is the live host frame; its header slots were written at its push.
    let (caller, argc, fi) = unsafe {
        (
            native_ptr::<u64>(*host.offset(slot::CALLER_FP)),
            int32_of(*host.offset(slot::ARGC)) as u32 & !CONSTRUCT_BIT,
            &*native_ptr::<crate::unit::FunctionInfo>(*host.offset(slot::FUNCTION_INFO)),
        )
    };
    let n = argc.max(fi.parameter_count) as usize;
    let host_base = host.wrapping_sub(n + 1 + HEADER);
    assert_eq!(
        host_base,
        frame_top(caller),
        "the host frame starts at the caller's top"
    );
    // SAFETY: the caller is a live bytecode frame below the host frame.
    let caller_regs = unsafe {
        (*native_ptr::<crate::unit::FunctionInfo>(*caller.offset(slot::FUNCTION_INFO)))
            .register_count
    };
    assert_eq!(caller_regs % 2, 1, "the caller has an odd register count");
    let (base, end) = (stack.base(), frame_top(host));
    assert!(
        end.addr() <= stack.end().addr(),
        "the range ends inside the stack"
    );
    let n = (end.addr() - base.addr()) / 8;
    for i in 0..n {
        // SAFETY: `[base, end)` is the live part of the stack.
        let bits = unsafe { *base.add(i) };
        assert!(
            ep9_value::Value::is_valid_bits(bits),
            "slot {i} of {n}: {bits:#x}"
        );
    }
    Ok(Local::undefined())
}

/// LEVER-13 (Codex focus 3): the GC root range after fast calls with the round-up. `scanner`
/// (5 registers) makes an object (a GC point: a collection under stress), calls `u7` (the
/// round-up past its top), then `scanRange()`; the script calls it twice after `dirty()`.
/// Every dispatch expansion, with and without GC stress; the heap verifies after the run.
#[test]
fn the_gc_root_range_after_a_fast_call_with_the_round_up() {
    fn make() -> VerifiedUnit {
        let mut f = Asm::new(meta("scanner", 0, 0, 0, 0));
        let o = f.b.new_register();
        assert_eq!(f.b.register_count(), 5);
        f.b.create_empty_object_literal().star(o);
        f.lda_global("u7");
        f.b.star(f.f).call_undefined_receiver0(f.f, f.call);
        f.lda_global("scanRange");
        f.b.star(f.f).call_undefined_receiver0(f.f, f.call);
        f.b.create_empty_object_literal().star(o);
        f.b.ldar(o).return_();
        let mut s = Asm::script(0);
        define_readers(&mut s);
        let k = s.b.function_constant(FunctionId(8));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("scanner");
        for _ in 0..2 {
            call_dirty(&mut s);
            s.lda_global("scanner");
            s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        }
        s.lda_str("ok");
        s.b.return_();
        let mut fns = vec![s.finish()];
        fns.extend(reader_fns());
        fns.push(f.finish());
        unit("", fns)
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let r = super::on_big_stack(move || {
                let mut iso = isolate(super::config(d, stress));
                iso.register_host_function("scanRange", 0, scan_range);
                let r = iso.with_cx_checkpoint(|cx| {
                    cx.run_script(make(), "scan.js").map(|v| cx.display(v))
                });
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                r.map_err(|e| e.message)
            });
            assert_eq!(r.as_deref(), Ok("ok"), "{label}");
        }
    }
}

/// LEVER-13 (Codex focus 4, H14-Q4): the RangeError comes at the same depth. `rec(n)` (1
/// parameter, 5 registers: frames of 15 slots) stores `depth = n` and calls `rec(n + 1)`; the
/// script (a 14-slot frame at the stack base) catches the error and returns its name and
/// `depth`. Over 16 VM stack sizes in a row (one frame size), the error is a RangeError and
/// the depth is `(slots - 14) / 15 - 1`: it grows by 1 at 1034 slots, where the last frame
/// ends at the stack end (on x86-64 the round-up store goes to the guard slot). Every dispatch
/// expansion, with and without GC stress, gives the same depths.
#[test]
fn the_overflow_depth_steps_once_per_frame_size_of_stack() {
    const FRAME: usize = 1 + 1 + crate::frame::HEADER + 5;
    // The script frame: no arguments, the receiver, the header and 5 registers.
    const SCRIPT: usize = 1 + crate::frame::HEADER + 5;
    fn make() -> VerifiedUnit {
        let mut f = Asm::new(meta("rec", 1, 0, 0, 0));
        f.b.new_register();
        assert_eq!(f.b.register_count(), 5);
        let n = f.b.parameter(0);
        f.b.ldar(n);
        f.sta_global("depth");
        f.lda_global("rec");
        f.b.star(f.f).ldar(n).add_smi(1, f.bin).star(f.a.get(0));
        f.b.call_undefined_receiver1(f.f, f.a.get(0), f.call)
            .return_();
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("rec");
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, ep9_bytecode::HandlerKind::Caught);
        s.b.bind(start);
        s.lda_global("rec");
        s.b.star(s.f).lda_smi(0).star(s.a.get(0));
        s.b.call_undefined_receiver1(s.f, s.a.get(0), s.call);
        s.b.bind(end);
        s.b.jump(done);
        // catch (e) { return e.name + depth } (in the free `a` registers: no new register).
        s.b.bind(handler);
        s.b.star(s.a.get(1));
        s.get_named(s.a.get(1), "name");
        s.b.star(s.a.get(2));
        s.lda_global("depth");
        s.b.add(s.a.get(2), s.bin).return_();
        // No throw: the depth only, which the parse below refuses.
        s.b.bind(done);
        s.lda_global("depth");
        s.b.return_();
        assert_eq!(s.b.register_count(), 5);
        unit("", vec![s.finish(), f.finish()])
    }
    let mut first: Option<(String, Vec<u32>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let depths: Vec<u32> = (1024..=1024 + FRAME)
                .map(|slots| {
                    super::on_big_stack(move || {
                        let mut iso = isolate(crate::IsolateConfig {
                            vm_stack_slots: slots,
                            ..super::config(d, stress)
                        });
                        let r = iso
                            .with_cx_checkpoint(|cx| {
                                cx.run_script(make(), "depth.js").map(|v| cx.display(v))
                            })
                            .map_err(|e| e.message);
                        iso.collect_garbage();
                        iso.verify_heap().unwrap();
                        r.unwrap()
                            .strip_prefix("RangeError")
                            .expect("a RangeError")
                            .parse::<u32>()
                            .unwrap()
                    })
                })
                .collect();
            let want: Vec<u32> = (1024..=1024 + FRAME)
                .map(|n| ((n - SCRIPT) / FRAME - 1) as u32)
                .collect();
            assert_eq!(depths, want, "{label}");
            match &first {
                None => first = Some((label, depths)),
                Some((l0, d0)) => assert_eq!(&depths, d0, "{label} differs from {l0}"),
            }
        }
    }
}
