//! Calls, recursion, argument padding, `this`, construct, classes, closures, host functions.

use ep9_bytecode::{
    FeedbackSlotKind, FunctionFlags, FunctionId, FunctionKind, Register, RegisterList, RuntimeId,
    ScopeId, ScopeKind, VariableMode, VerifiedUnit,
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
/// `call::fill_missing_args` and `call::write_call_frame`) is the frame `call::push_frame`
/// writes, slot for slot, and both refuse the same sizes: no missing argument, missing
/// arguments, no register, more arguments than parameters, 0 to 4 arguments (the list forms
/// take the copy loop from 3), a frame one slot too big, one that just fits and one with room
/// left.
#[test]
fn the_fast_frame_write_equals_push_frame() {
    use crate::call::{call_frame_fp, fill_missing_args, push_frame, write_call_frame};
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
        for argc in 0..=4u32 {
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
                        write_call_frame(
                            fp, &fi, callee, this, argc, arg, caller_fp, return_pc, context_of,
                        )
                    })
                };
                let case = format!("params {params} regs {regs} argc {argc} room {room}");
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
