//! Tests of generators (GENERATOR cells, resume frames, the natives) (owner P1-GEN-VM).
//! BytecodeGen has no generators on this branch, so the generator functions are hand-built
//! in the V8 shape that P1-GEN-CG emits ([`Gen`]: prologue, initial suspend, one resume point
//! per suspend id). Every run goes through every dispatch, with and without GC stress
//! (`run_all`).

use std::hint::black_box;

use ep9_bytecode::{
    FunctionCode, FunctionFlags, FunctionId, FunctionMeta, HandlerKind, Label, Register,
    RegisterList, RuntimeId, ScopeId, ScopeKind, VariableMode, VerifiedUnit,
};

use super::{
    Asm, config, isolate, meta, ok, on_big_stack, run_all, take_output, unit, unit_with_scopes,
};
use crate::builtins::{IntrinsicId, intrinsic};
use crate::frame::{UNDEFINED, int32_of};
use crate::object::{self, cell, genf};
use crate::property;
use crate::runtime::{Atom, Runtime, val};
use crate::{Dispatch, Value};

// ---- hand-built generator functions -------------------------------------------------------

/// Metadata of a (sloppy) generator function `name` with `params` parameters.
fn gen_meta(name: &str, params: u32) -> FunctionMeta {
    let mut m = meta(name, params, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::GENERATOR;
    m
}

/// A generator function body in the V8 shape (contract: scratchpad p1-genvm-contract.md):
///
/// ```text
///   LdaNewTarget ; Star g ; SwitchOnGeneratorState g, [L0 .. Ln]
///   LdaCurrentFunction ; Star t0 ; Ldar <this> ; Star t1
///   CallRuntime CreateGeneratorObject(t0, t1) ; Star g
///   SuspendGenerator g, r0, all, 0          (the caller gets the generator)
/// Lk:
///   ResumeGenerator g, r0, all ; Star v
///   CallRuntime GeneratorGetResumeMode(g) ; SwitchOnSmi [return, throw] (base 1) ; Jump cont
///   return: Ldar v ; Return     throw: Ldar v ; Throw     cont: Ldar v
/// ```
///
/// Every register is allocated before the first suspend, so `r0, all` saves them all.
struct Gen {
    s: Asm,
    g: Register,
    v: Register,
    t: RegisterList,
    /// The registers the body may use (saved at each suspend).
    r: Vec<Register>,
    save: RegisterList,
    resumes: Vec<Label>,
}

impl Gen {
    /// The prologue and the initial suspend (id 0); `suspends` more suspend ids follow.
    fn new(m: FunctionMeta, suspends: u32, user: u32) -> Gen {
        let mut s = Asm::new(m);
        let g = s.b.new_register();
        let v = s.b.new_register();
        let t = s.b.new_register_list(2);
        let r = (0..user).map(|_| s.b.new_register()).collect();
        let save = RegisterList::new(0, s.b.register_count());
        let (table, resumes) = s.b.jump_table(suspends + 1);
        let this = s.b.receiver();
        s.b.lda_new_target().star(g);
        s.b.switch_on_generator_state(g, table);
        s.b.lda_current_function().star(t.get(0));
        s.b.ldar(this).star(t.get(1));
        s.b.call_runtime(RuntimeId::CreateGeneratorObject, t, 2);
        s.b.star(g);
        let mut body = Gen {
            s,
            g,
            v,
            t,
            r,
            save,
            resumes,
        };
        body.suspend(0);
        body
    }

    /// `SuspendGenerator` with id `k` (acc goes to the caller), then resume point `k`: acc
    /// is the sent value after a `next`; a `return` returns it, a `throw` throws it.
    fn suspend(&mut self, k: u32) {
        let (g, v, save) = (self.g, self.v, self.save);
        let b = &mut self.s.b;
        b.suspend_generator(g, save, save.len(), k);
        b.bind(self.resumes[k as usize]);
        b.resume_generator(g, save, save.len());
        b.star(v);
        let gl = RegisterList::new(g.local_index().unwrap(), 1);
        b.call_runtime(RuntimeId::GeneratorGetResumeMode, gl, 1);
        let (modes, l) = b.jump_table(2);
        let cont = b.new_label();
        b.switch_on_smi(modes, 1);
        b.jump(cont);
        b.bind(l[0]);
        b.ldar(v).return_();
        b.bind(l[1]);
        b.ldar(v).throw();
        b.bind(cont);
        b.ldar(v);
    }

    /// `yield acc` with suspend id `k` (`{value: acc, done: false}`, runtime id 3); acc is
    /// the sent value afterwards.
    fn yield_(&mut self, k: u32) {
        let t = self.t;
        let b = &mut self.s.b;
        b.star(t.get(0)).lda_false().star(t.get(1));
        b.call_runtime(RuntimeId::CreateIterResultObject, t, 2);
        self.suspend(k);
    }

    fn finish(self) -> FunctionCode {
        self.s.finish()
    }
}

/// Script helpers: registers for `obj.method(arg)` calls and their results.
struct Calls {
    o: Register,
    m: Register,
    x: Register,
    res: Register,
    ctx: Register,
}

impl Calls {
    fn new(s: &mut Asm) -> Calls {
        Calls {
            o: s.b.new_register(),
            m: s.b.new_register(),
            x: s.b.new_register(),
            res: s.b.new_register(),
            ctx: s.b.new_register(),
        }
    }

    /// `res = <global>.<method>(arg)` (arg undefined when `None`).
    fn call(&self, s: &mut Asm, global: &str, method: &str, arg: Option<f64>) {
        s.lda_global(global);
        s.b.star(self.o);
        s.get_named(self.o, method);
        s.b.star(self.m);
        match arg {
            Some(d) => {
                s.lda_num(d);
            }
            None => {
                s.b.lda_undefined();
            }
        }
        s.b.star(self.x);
        s.b.call_property1(self.m, self.o, self.x, s.call);
        s.b.star(self.res);
    }

    /// `print(res.value); print(res.done)`.
    fn print_result(&self, s: &mut Asm) {
        s.get_named(self.res, "value");
        s.print_acc();
        s.get_named(self.res, "done");
        s.print_acc();
    }

    /// `print(<global>.<method>(arg).value, .done)` on two lines.
    fn step(&self, s: &mut Asm, global: &str, method: &str, arg: Option<f64>) {
        self.call(s, global, method, arg);
        self.print_result(s);
    }

    /// `try { body } catch (e) { print(e.message ?? e) }`: prints the message of an Error, or
    /// the value itself.
    fn try_print(&self, s: &mut Asm, body: impl FnOnce(&mut Asm)) {
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(self.ctx);
        s.b.add_handler(start, end, handler, self.ctx, HandlerKind::Caught);
        s.b.bind(start);
        body(s);
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        let e = self.x;
        s.b.star(e);
        let plain = s.b.new_label();
        let printed = s.b.new_label();
        s.b.ldar(e).jump_if_js_receiver(plain);
        s.b.ldar(e);
        s.print_acc();
        s.b.jump(printed);
        s.b.bind(plain);
        s.get_named(e, "message");
        s.print_acc();
        s.b.bind(printed);
        s.b.bind(done);
    }
}

/// `globalThis[name] = <closure of function id>`.
fn define(s: &mut Asm, id: u32, name: &str) {
    let k = s.b.function_constant(FunctionId(id));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global(name);
}

/// `globalThis[name] = <global f>(args..)` with number arguments.
fn call_global(s: &mut Asm, f: &str, args: &[f64], name: &str) {
    let list = s.a;
    for (i, &d) in args.iter().enumerate() {
        s.lda_num(d);
        s.b.star(list.get(i as u32));
    }
    s.lda_global(f);
    s.b.star(s.f);
    let n = args.len() as u32;
    s.b.call_undefined_receiver(
        s.f,
        RegisterList::new(list.first().local_index().unwrap(), n),
        n,
        s.call,
    );
    s.sta_global(name);
}

// ---- resume protocol ----------------------------------------------------------------------

/// The program of [`next_runs_to_each_yield_and_the_return_completes`].
fn basic_unit() -> VerifiedUnit {
    // function* g(a) { const x = yield a; yield x + 1; return a * 10; }
    let mut g = Gen::new(gen_meta("g", 1), 2, 1);
    let (a, x) = (g.s.b.parameter(0), g.r[0]);
    g.s.b.ldar(a);
    g.yield_(1);
    g.s.b.star(x).ldar(x).add_smi(1, g.s.bin);
    g.yield_(2);
    g.s.b.ldar(a).mul_smi(10, g.s.bin).return_();
    let mut s = Asm::script(0);
    let c = Calls::new(&mut s);
    define(&mut s, 1, "g");
    call_global(&mut s, "g", &[5.0], "it");
    c.step(&mut s, "it", "next", Some(99.0)); // the first sent value is dropped
    c.step(&mut s, "it", "next", Some(7.0));
    c.step(&mut s, "it", "next", None);
    c.step(&mut s, "it", "next", None);
    c.step(&mut s, "it", "return", Some(3.0));
    // A completed generator rethrows what `throw` gets.
    c.try_print(&mut s, |s| c.call(s, "it", "throw", Some(9.0)));
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), g.finish()])
}

#[test]
fn next_runs_to_each_yield_and_the_return_completes() {
    let r = run_all(basic_unit);
    ok(&r);
    assert_eq!(r.output, BASIC_OUT);
}

const BASIC_OUT: [&str; 11] = [
    "5",
    "false",
    "8",
    "false",
    "50",
    "true",
    "undefined",
    "true",
    "3",
    "true",
    "9",
];

#[test]
fn return_and_throw_resume_at_a_suspend_point_and_at_the_start() {
    fn make() -> VerifiedUnit {
        // function* g() { yield 1; yield 2; }
        let mut g = Gen::new(gen_meta("g", 0), 2, 0);
        g.s.b.lda_smi(1);
        g.yield_(1);
        g.s.b.lda_smi(2);
        g.yield_(2);
        g.s.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "g");
        // return(5) at a suspend point, then the generator is done.
        call_global(&mut s, "g", &[], "a");
        c.step(&mut s, "a", "next", None);
        c.step(&mut s, "a", "return", Some(5.0));
        c.step(&mut s, "a", "next", None);
        // return(7) at the start (suspendedStart, §27.5.3.4 step 2).
        call_global(&mut s, "g", &[], "b");
        c.step(&mut s, "b", "return", Some(7.0));
        c.step(&mut s, "b", "next", None);
        // throw(8) at the start: thrown out, then done.
        call_global(&mut s, "g", &[], "c");
        c.try_print(&mut s, |s| c.call(s, "c", "throw", Some(8.0)));
        c.step(&mut s, "c", "next", None);
        // throw(6) at a suspend point with no handler: thrown out, then done.
        call_global(&mut s, "g", &[], "d");
        c.step(&mut s, "d", "next", None);
        c.try_print(&mut s, |s| c.call(s, "d", "throw", Some(6.0)));
        c.step(&mut s, "d", "next", None);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "1",
            "false",
            "5",
            "true",
            "undefined",
            "true",
            "7",
            "true",
            "undefined",
            "true",
            "8",
            "undefined",
            "true",
            "1",
            "false",
            "6",
            "undefined",
            "true"
        ]
    );
}

#[test]
fn throw_is_caught_inside_the_body_and_the_handler_context_survives() {
    fn make() -> VerifiedUnit {
        // function* g() { try { yield 1; } catch (e) { yield e + 1; } return 0; }
        let mut g = Gen::new(gen_meta("g", 0), 2, 2);
        let (ctx, e) = (g.r[0], g.r[1]);
        let (start, end, handler) = (g.s.b.new_label(), g.s.b.new_label(), g.s.b.new_label());
        g.s.b.lda_current_context().star(ctx);
        g.s.b
            .add_handler(start, end, handler, ctx, HandlerKind::Caught);
        g.s.b.bind(start);
        g.s.b.lda_smi(1);
        g.yield_(1);
        g.s.b.bind(end);
        let after = g.s.b.new_label();
        g.s.b.jump(after);
        g.s.b.bind(handler);
        g.s.b.star(e).ldar(e).add_smi(1, g.s.bin);
        g.yield_(2);
        g.s.b.bind(after);
        g.s.b.lda_zero().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "g");
        call_global(&mut s, "g", &[], "it");
        c.step(&mut s, "it", "next", None);
        c.step(&mut s, "it", "throw", Some(41.0));
        c.step(&mut s, "it", "next", None);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["1", "false", "42", "false", "0", "true"]);
}

/// The program of [`a_block_context_and_the_parameters_and_this_survive_a_suspend`].
fn context_unit() -> VerifiedUnit {
    // function* g(a, b) { { let t = a * 5; yield this.x; yield t * 2; } yield b; }
    let mut g = Gen::new(gen_meta("g", 2), 3, 1);
    let save = g.r[0];
    let (a, b) = (g.s.b.parameter(0), g.s.b.parameter(1));
    let this = g.s.b.receiver();
    let kb = g.s.b.scope_constant(ScopeId(0));
    g.s.b.create_block_context(kb).push_context(save);
    g.s.b
        .ldar(a)
        .mul_smi(5, g.s.bin)
        .sta_current_context_slot(0);
    g.get_this_x(this);
    g.yield_(1);
    g.s.b.lda_current_context_slot(0).mul_smi(2, g.s.bin);
    g.yield_(2);
    g.s.b.pop_context(save);
    g.s.b.ldar(b);
    g.yield_(3);
    g.s.b.lda_undefined().return_();
    // o = {x: 3}; o.g = g; it = o.g(4)  (b is missing: padded with undefined)
    let mut s = Asm::script(0);
    let c = Calls::new(&mut s);
    define(&mut s, 1, "g");
    s.b.create_empty_object_literal().star(c.o);
    s.lda_num(3.0);
    s.set_named(c.o, "x");
    s.lda_global("g");
    s.set_named(c.o, "g");
    s.get_named(c.o, "g");
    s.b.star(c.m);
    s.lda_num(4.0);
    s.b.star(c.x);
    s.b.call_property1(c.m, c.o, c.x, s.call);
    s.sta_global("it");
    for _ in 0..4 {
        c.step(&mut s, "it", "next", None);
    }
    s.b.lda_undefined().return_();
    unit_with_scopes(
        "",
        vec![(ScopeKind::Block, None, vec![("t", VariableMode::Let)])],
        vec![s.finish(), g.finish()],
    )
}

#[test]
fn a_block_context_and_the_parameters_and_this_survive_a_suspend() {
    let r = run_all(context_unit);
    ok(&r);
    assert_eq!(r.output, CONTEXT_OUT);
}

const CONTEXT_OUT: [&str; 8] = [
    "3",
    "false",
    "40",
    "false",
    "undefined",
    "false",
    "undefined",
    "true",
];

/// The copy paths (`SuspendGenerator`, `ResumeGenerator`, the resumed frame push with the saved
/// parameters, `this` and context) in one configuration without GC stress: the tests that
/// MIRI(ep9_vm) is meant to run (`cargo miri test -p ep9_vm --lib copy_path`, plan-v3
/// P1-GEN-VM). Miri does not run the conservative stack scan, so GC-stress runs are not valid
/// under it. Today no ep9_vm isolate starts under Miri: its 8 MiB stand-in reservation cannot
/// hold the shape space or meet the cage alignment that `shape::init` asserts.
#[test]
fn copy_path_suspend_resume_and_the_saved_frame() {
    for (make, want) in [
        (basic_unit as fn() -> VerifiedUnit, &BASIC_OUT[..]),
        (context_unit, &CONTEXT_OUT[..]),
    ] {
        let r = super::run_in(config(Dispatch::Match, false), make());
        ok(&r);
        assert_eq!(r.output, want);
    }
}

impl Gen {
    /// `acc = this.x`.
    fn get_this_x(&mut self, this: Register) {
        self.s.get_named(this, "x");
    }
}

// ---- states and errors ----------------------------------------------------------------------

#[test]
fn a_running_generator_cannot_be_resumed_and_a_throw_closes_it() {
    fn make() -> VerifiedUnit {
        // function* g() { it.next(); }   (re-enters itself)
        let mut g = Gen::new(gen_meta("g", 0), 0, 2);
        let (o, m) = (g.r[0], g.r[1]);
        g.s.lda_global("it");
        g.s.b.star(o);
        g.s.get_named(o, "next");
        g.s.b.star(m);
        g.s.b.call_property0(m, o, g.s.call);
        g.s.b.lda_undefined().return_();
        // function* h() { yield 1; throw 7; }
        let mut h = Gen::new(gen_meta("h", 0), 1, 0);
        h.s.b.lda_smi(1);
        h.yield_(1);
        h.s.b.lda_smi(7).throw();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "g");
        define(&mut s, 2, "h");
        call_global(&mut s, "g", &[], "it");
        c.try_print(&mut s, |s| c.call(s, "it", "next", None));
        c.step(&mut s, "it", "next", None);
        call_global(&mut s, "h", &[], "j");
        c.step(&mut s, "j", "next", None);
        c.try_print(&mut s, |s| c.call(s, "j", "next", None));
        c.step(&mut s, "j", "next", None);
        // %GeneratorPrototype%.next/return/throw on a non-generator.
        for m in ["next", "return", "throw"] {
            c.try_print(&mut s, |s| {
                s.lda_global("j");
                s.b.star(c.o);
                s.get_named(c.o, m);
                s.b.star(c.m);
                s.b.create_empty_object_literal().star(c.o);
                s.b.call_property0(c.m, c.o, s.call);
            });
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish(), h.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "Generator is already running",
            "undefined",
            "true",
            "1",
            "false",
            "7",
            "undefined",
            "true",
            "Method [Generator].prototype.next called on incompatible receiver object",
            "Method [Generator].prototype.return called on incompatible receiver object",
            "Method [Generator].prototype.throw called on incompatible receiver object",
        ]
    );
}

/// Runtime ids 19-22 (ABI 3) from code that is not a generator body: 19-21 check their
/// arguments and throw a catchable internal error; 22 is the `yield*` TypeError.
#[test]
fn runtime_ids_19_to_22_check_their_arguments() {
    fn make(id: RuntimeId) -> VerifiedUnit {
        let argc = id.info().arity.unwrap();
        let mut s = Asm::script(0);
        let args = s.b.new_register_list(argc);
        s.b.call_runtime(id, args, argc);
        s.b.lda_smi(1).return_();
        unit("", vec![s.finish()])
    }
    let want = [
        (
            RuntimeId::CreateGeneratorObject,
            "Error: ep9 internal error: CreateGeneratorObject outside its generator function",
        ),
        (
            RuntimeId::GeneratorGetResumeMode,
            "Error: ep9 internal error: GeneratorGetResumeMode of a non-generator",
        ),
        (
            RuntimeId::GeneratorClose,
            "Error: ep9 internal error: GeneratorClose of a non-generator",
        ),
        (
            RuntimeId::ThrowThrowMethodMissing,
            "TypeError: The iterator does not provide a 'throw' method.",
        ),
    ];
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                for (id, msg) in want {
                    let r = super::run_in(config(d, stress), make(id));
                    let e = r.result.unwrap_err();
                    assert_eq!(e.message, msg, "{} stress={stress}", d.name());
                }
            });
        }
    }
}

/// The generator opcodes check the register that should hold the generator: a value that is
/// not this function's generator gives a catchable internal error, never UB.
#[test]
fn the_generator_opcodes_reject_a_register_without_the_generator() {
    fn make() -> VerifiedUnit {
        // f1: SwitchOnGeneratorState on a non-generator object.
        let mut f1 = Asm::new(gen_meta("f1", 0));
        let r = f1.b.new_register();
        let (k, l) = f1.b.jump_table(1);
        f1.b.create_empty_object_literal().star(r);
        f1.b.switch_on_generator_state(r, k);
        f1.b.bind(l[0]);
        f1.b.lda_undefined().return_();
        // f2: SuspendGenerator of undefined.
        let mut f2 = Asm::new(gen_meta("f2", 0));
        let r = f2.b.new_register();
        f2.b.lda_undefined().star(r);
        f2.b.suspend_generator(r, RegisterList::new(0, 1), 1, 0);
        // f3: a generator suspended into the REGS of another generator function (g).
        let mut f3 = Asm::new(gen_meta("f3", 0));
        let r = f3.b.new_register();
        f3.lda_global("other");
        f3.b.star(r);
        f3.b.resume_generator(r, RegisterList::new(0, 1), 1);
        f3.b.return_();
        let mut g = Gen::new(gen_meta("g", 0), 0, 0);
        g.s.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 4, "g");
        call_global(&mut s, "g", &[], "other");
        for (id, name) in [(1, "f1"), (2, "f2"), (3, "f3")] {
            define(&mut s, id, name);
            c.try_print(&mut s, |s| call_global(s, name, &[], "unused"));
        }
        s.b.lda_undefined().return_();
        unit(
            "",
            vec![
                s.finish(),
                f1.finish(),
                f2.finish(),
                f3.finish(),
                g.finish(),
            ],
        )
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "ep9 internal error: SwitchOnGeneratorState of a non-generator",
            "ep9 internal error: the generator register holds no generator",
            "ep9 internal error: the generator belongs to another function",
        ]
    );
}

// ---- objects and prototypes -------------------------------------------------------------------

/// M2-P1-GENFN-PROTO (§27.3.2, §27.7.2): the [[Prototype]] of %GeneratorFunction% and
/// %AsyncFunction% is %Function%; their `prototype` objects inherit from %Function.prototype%.
#[test]
fn generator_and_async_function_constructors_inherit_from_function() {
    let iso = isolate(config(Dispatch::ALL[0], false));
    let rt: &Runtime = iso.cx.rt();
    let function = intrinsic(rt, IntrinsicId::Function);
    let fp = intrinsic(rt, IntrinsicId::FunctionPrototype);
    for (c, p) in [
        (
            IntrinsicId::GeneratorFunction,
            IntrinsicId::GeneratorFunctionPrototype,
        ),
        (
            IntrinsicId::AsyncFunction,
            IntrinsicId::AsyncFunctionPrototype,
        ),
    ] {
        let (c, p) = (intrinsic(rt, c), intrinsic(rt, p));
        assert!(object::is_object(c) && object::is_object(p));
        assert_eq!(object::proto_of(c), function);
        assert_eq!(object::proto_of(p), fp);
    }
}

/// §15.5.3/§15.5.4: a generator function object inherits from %GeneratorFunction.prototype%,
/// its lazy `prototype` {w} from %GeneratorPrototype% (no `constructor`), and a generator
/// object from the function's `prototype` (or %GeneratorPrototype% when that is not an
/// object, §10.1.14). A generator function is no constructor.
#[test]
fn generator_function_and_object_prototypes() {
    fn make() -> VerifiedUnit {
        let mut g = Gen::new(gen_meta("g", 0), 0, 0);
        g.s.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        define(&mut s, 1, "g");
        call_global(&mut s, "g", &[], "it");
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                take_output();
                iso.run_script(make(), "gen.js").unwrap();
                let label = format!("{} stress={stress}", d.name());
                let g = iso.global("g").unwrap().to_bits();
                let it = iso.global("it").unwrap().to_bits();
                let cx = &mut *iso.cx;
                let rt: &Runtime = cx.rt();
                let gfp = intrinsic(rt, IntrinsicId::GeneratorFunctionPrototype);
                let gp = intrinsic(rt, IntrinsicId::GeneratorPrototype);
                assert!(object::is_object(gfp) && object::is_object(gp), "{label}");
                assert_eq!(object::proto_of(g), gfp, "{label}: g's [[Prototype]]");
                assert!(!object::is_constructor(g), "{label}");
                let key = rt.atom(Atom::Prototype);
                let p = property::get(cx, g, key).unwrap();
                assert_eq!(
                    object::proto_of(p),
                    gp,
                    "{label}: g.prototype's [[Prototype]]"
                );
                assert_eq!(
                    object::proto_of(it),
                    p,
                    "{label}: the generator's [[Prototype]]"
                );
                let ctor = rt.atom(Atom::Constructor);
                assert!(
                    object::own_get(p, ctor).is_none(),
                    "{label}: no constructor"
                );
                let (_, a) = object::own_get(g, key).unwrap();
                assert_eq!(a, object::attr::WRITABLE, "{label}: prototype is {{w}}");
                // A new generator after `g.prototype = 1` inherits from %GeneratorPrototype%.
                let one = crate::frame::int32_bits(1);
                property::set(cx, g, key, one, true).unwrap();
                let r = iso.call(val(g), Value::UNDEFINED, &[]).unwrap();
                assert_eq!(object::proto_of(r.to_bits()), gp, "{label}");
                // A generator object is a GENERATOR in state suspended-start (id 0), sync.
                assert!(crate::generator::is_generator(it), "{label}");
                assert_eq!(object::gen_get_i32(it, genf::STATE), 0, "{label}");
                assert_eq!(
                    object::gen_get_i32(it, genf::MODE),
                    genf::MODE_SYNC,
                    "{label}"
                );
                assert_eq!(object::gen_get(it, genf::FUNCTION), g, "{label}");
                // `new g()` is a TypeError (not a constructor).
                let e = crate::call::construct_with(&mut iso.cx, g, 0, |_| UNDEFINED, g);
                assert!(e.is_err(), "{label}");
                iso.rt.take_pending();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// A plain function keeps %Function.prototype% and its ordinary lazy `prototype`
/// (`lazy_prototype_kind`); an arrow function has none.
#[test]
fn ordinary_functions_keep_their_prototypes() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let mut s = Asm::script(0);
    define(&mut s, 1, "f");
    s.b.lda_undefined().return_();
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    f.b.lda_undefined().return_();
    iso.run_script(unit("", vec![s.finish(), f.finish()]), "f.js")
        .unwrap();
    let f = iso.global("f").unwrap().to_bits();
    let cx = &mut *iso.cx;
    let rt = cx.rt();
    assert_eq!(
        object::proto_of(f),
        rt.root(crate::runtime::Root::FunctionPrototype)
    );
    let p = property::get(cx, f, rt.atom(Atom::Prototype)).unwrap();
    assert_eq!(
        object::proto_of(p),
        rt.root(crate::runtime::Root::ObjectPrototype)
    );
    assert_eq!(
        object::own_get(p, rt.atom(Atom::Constructor)).map(|x| x.0),
        Some(f)
    );
}

// ---- GC ---------------------------------------------------------------------------------------

const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frames that make the cells (G16, the
/// pattern of tests/ic.rs and tests/symbols.rs; tests/mod.rs is frozen, so each file has its
/// own copy).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller, so stale words of returned frames cannot
/// keep a cell alive through the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's. Returns its
/// result and the lowest byte of the pad.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// Scrubs the stack and collects; checks that the conservative scan stopped above the pad and
/// kept none of `cells` (masked bits). The isolate must record its conservative roots.
fn collect_after_a_pad(rt: &Runtime, pad_low: usize, cells: &[u64], what: &str) {
    scrub_stack();
    rt.heap.collect();
    let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
    let scan_low = high - 8 * rt.heap.stats().last_stack_words;
    assert!(
        scan_low > pad_low,
        "the conservative scan read the frames below the pad ({scan_low:#x} <= {pad_low:#x})"
    );
    let css = rt.heap.last_conservative_roots();
    for &m in cells {
        let addr = cell(m ^ MASK).addr();
        assert!(
            !css.iter().any(|c| c.addr() == addr),
            "{what}: the conservative scan kept {addr:#x}"
        );
    }
}

/// True if a live cell starts at the masked address `masked` (run it below a pad).
#[inline(never)]
fn is_live(rt: &Runtime, masked: u64) -> bool {
    rt.heap.is_cell(cell(black_box(masked) ^ MASK).addr())
}

/// `function* g() { let o = {}; o.x = 42; yield 1; return o.x; }`: `o` lives only in a
/// register, so while `it` is suspended only its REGS cell holds `o`.
fn holder_unit() -> VerifiedUnit {
    let mut g = Gen::new(gen_meta("g", 0), 1, 1);
    let o = g.r[0];
    g.s.b.create_empty_object_literal().star(o);
    g.s.b.lda_smi(42);
    g.s.set_named(o, "x");
    g.s.b.lda_smi(1);
    g.yield_(1);
    g.s.get_named(o, "x");
    g.s.b.return_();
    let mut s = Asm::script(0);
    define(&mut s, 1, "g");
    call_global(&mut s, "g", &[], "it");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), g.finish()])
}

/// Runs the script and the first `it.next()`, in a frame of its own (run it below a pad).
/// Returns `o` (masked) from the suspended generator's saved registers.
#[inline(never)]
fn made_a_suspended_holder(iso: &mut crate::Isolate) -> u64 {
    iso.run_script(holder_unit(), "holder.js").unwrap();
    let it = iso.global("it").unwrap();
    let next = next_of(iso, it);
    iso.call(next, it, &[]).unwrap();
    let regs = object::gen_regs(it.to_bits()).unwrap();
    // REGS = [input, (no params), r0..]: `o` is the first user register, after the Asm
    // scratch registers f, a0-a2 and the Gen registers g, v, t0, t1.
    let o = regs.get(1 + 8);
    assert!(object::is_object(o));
    black_box(o) ^ MASK
}

fn next_of(iso: &mut crate::Isolate, it: Value) -> Value {
    let cx = &mut *iso.cx;
    let k = crate::atoms::intern_str(cx.rt(), "next");
    val(property::get(cx, it.to_bits(), k).unwrap())
}

/// Resumes `it` to completion in a frame of its own; returns the result's `value` as an int.
#[inline(never)]
fn finish_the_holder(iso: &mut crate::Isolate) -> i32 {
    let it = iso.global("it").unwrap();
    let next = next_of(iso, it);
    let r = iso.call(next, it, &[]).unwrap().to_bits();
    assert!(
        object::gen_regs(it.to_bits()).is_none(),
        "closed: REGS dropped"
    );
    let cx = &mut *iso.cx;
    let v = property::get(cx, r, cx.rt().atom(Atom::ValueKey)).unwrap();
    int32_of(v)
}

/// Plan acceptance (GEN-VM UNIT): a suspended generator is the only holder of an object. A
/// full GC whose conservative scan saw no copy of the object keeps it (the GENERATOR cell's
/// REGS ELEMENTS cell is traced); the resumed body reads it. Once the generator completes, its
/// saved values are dropped and the object dies.
#[test]
fn a_suspended_generator_is_the_only_holder_of_an_object() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let label = format!("{} stress={stress}", d.name());
                let mut c = config(d, stress);
                c.heap.record_conservative_roots = true;
                let mut iso = isolate(c);
                let (o, pad_low) = below_a_pad(|| made_a_suspended_holder(&mut iso));
                collect_after_a_pad(&iso.rt, pad_low, &[o], &format!("{label} suspended"));
                // The unmasked address appears only below a pad, so the next scan cannot see it.
                assert!(
                    below_a_pad(|| is_live(&iso.rt, o)).0,
                    "{label}: the suspended generator keeps it"
                );
                iso.verify_heap().unwrap();
                let (v, pad_low) = below_a_pad(|| finish_the_holder(&mut iso));
                assert_eq!(v, 42, "{label}");
                collect_after_a_pad(&iso.rt, pad_low, &[o], &format!("{label} completed"));
                assert!(
                    !below_a_pad(|| is_live(&iso.rt, o)).0,
                    "{label}: a completed generator keeps nothing"
                );
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// Many generators suspended at once, each with its own values, survive GC stress (a
/// collection before every allocation) and give their values back in order.
#[test]
fn many_suspended_generators_under_gc_stress() {
    fn make() -> VerifiedUnit {
        // function* g(a) { const s = "v" + a; yield s; yield s + "!"; }
        let mut g = Gen::new(gen_meta("g", 1), 2, 1);
        let (a, sr) = (g.s.b.parameter(0), g.r[0]);
        g.s.lda_str("v");
        g.s.b.star(sr).ldar(a).add(sr, g.s.bin).star(sr);
        g.yield_(1);
        g.s.lda_str("!");
        g.s.b.add(sr, g.s.bin);
        g.yield_(2);
        g.s.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "g");
        for i in 0..4 {
            call_global(&mut s, "g", &[f64::from(i)], &format!("it{i}"));
        }
        for round in 0..3 {
            for i in 0..4 {
                c.call(&mut s, &format!("it{i}"), "next", None);
                if round < 2 {
                    s.get_named(c.res, "value");
                } else {
                    s.get_named(c.res, "done");
                }
                s.print_acc();
            }
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "v0", "v1", "v2", "v3", "v0!", "v1!", "v2!", "v3!", "true", "true", "true", "true"
        ]
    );
}

// ---- native stack -----------------------------------------------------------------------------

/// Each resume is one checked native level (`call::run_resumed`): a chain of generators whose
/// bodies resume the next one gives a catchable RangeError, not a crash.
#[test]
fn a_deep_resume_chain_is_a_range_error() {
    fn make() -> VerifiedUnit {
        // function* d(n) { if (n > 0) { const i = d(n - 1); yield i.next().value; } yield 0; }
        let mut g = Gen::new(gen_meta("d", 1), 2, 3);
        let n = g.s.b.parameter(0);
        let (i, m, t) = (g.r[0], g.r[1], g.r[2]);
        let base = g.s.b.new_label();
        g.s.b.lda_zero().test_greater_than(n, g.s.cmp);
        // acc = n > 0
        g.s.b.jump_if_false(base);
        g.s.lda_global("d");
        g.s.b.star(m).ldar(n).sub_smi(1, g.s.bin).star(t);
        g.s.b.call_undefined_receiver1(m, t, g.s.call).star(i);
        g.s.get_named(i, "next");
        g.s.b.star(m).call_property0(m, i, g.s.call).star(t);
        g.s.get_named(t, "value");
        g.yield_(1);
        g.s.b.bind(base);
        g.s.b.lda_zero();
        g.yield_(2);
        g.s.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "d");
        call_global(&mut s, "d", &[3.0], "small");
        c.step(&mut s, "small", "next", None);
        call_global(&mut s, "d", &[1e6], "deep");
        c.try_print(&mut s, |s| c.call(s, "deep", "next", None));
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish()])
    }
    // GC stress is too slow for a chain this deep; the match loop and sibling calls only.
    for &d in Dispatch::ALL {
        let r = on_big_stack(move || super::run_in(config(d, false), make()));
        ok(&r);
        assert_eq!(
            r.output,
            ["0", "false", "Maximum call stack size exceeded"],
            "{}",
            d.name()
        );
    }
}

/// M2-P1-X3-DEFER (closed in M3-P1 ASYNC): `return` and `throw` on a suspended-start
/// generator complete it without a frame (GeneratorResumeAbrupt, §27.5.3.4 step 2), so they
/// work with a nearly full VM stack. `big` has a frame of about 600 slots and `deep` one of
/// about 500, in a VM stack of 1024 slots: a resumed frame of `big` does not fit above `deep`
/// (before the fix both calls threw a RangeError and left the generators unstarted).
#[test]
fn return_and_throw_at_the_start_need_no_frame() {
    fn make() -> VerifiedUnit {
        // function* big() { <600 registers>; yield 1; }
        let mut g = Gen::new(gen_meta("big", 0), 1, 600);
        g.s.b.lda_smi(1);
        g.yield_(1);
        g.s.b.lda_undefined().return_();
        // function deep() { <500 registers>; print(a.return(9)); try { b.throw(8) } catch {} }
        let mut d = Asm::new(meta("deep", 0, 0, 0, 0));
        for _ in 0..500 {
            d.b.new_register();
        }
        let c = Calls::new(&mut d);
        c.step(&mut d, "a", "return", Some(9.0));
        c.try_print(&mut d, |s| c.call(s, "b", "throw", Some(8.0)));
        d.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "big");
        define(&mut s, 2, "deep");
        call_global(&mut s, "big", &[], "a");
        call_global(&mut s, "big", &[], "b");
        call_global(&mut s, "deep", &[], "r");
        // Both are completed: the body never runs.
        c.step(&mut s, "a", "next", None);
        c.step(&mut s, "b", "next", None);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), g.finish(), d.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let cfg = crate::IsolateConfig {
                vm_stack_slots: 1024,
                ..config(d, stress)
            };
            let r = on_big_stack(move || super::run_in(cfg, make()));
            ok(&r);
            assert_eq!(
                r.output,
                ["9", "true", "8", "undefined", "true", "undefined", "true"],
                "{} stress={stress}",
                d.name()
            );
        }
    }
}

/// GV-7 (M2-P2-GEN-ACC, closed in M3-P1 ASYNC): `SwitchOnGeneratorState` does not change the
/// accumulator (`acc: None`), on the first call and on a resume.
#[test]
fn switch_on_generator_state_keeps_the_accumulator() {
    fn make() -> VerifiedUnit {
        // function* g() { <prologue with acc = 7 at the switch>; yield acc; return acc; }: the
        // first call falls through with acc 7 and makes the generator; the second resume
        // jumps with acc 7 to an entry that saves acc in a register the resume does not restore.
        let mut s0 = Asm::new(gen_meta("g", 0));
        let g = s0.b.new_register();
        let t = s0.b.new_register_list(2);
        let k = s0.b.new_register();
        let save = RegisterList::new(0, s0.b.register_count());
        let seen = s0.b.new_register();
        let (table, resumes) = s0.b.jump_table(2);
        s0.b.lda_new_target().star(g);
        s0.b.lda_smi(7);
        s0.b.switch_on_generator_state(g, table);
        // First call: acc is still 7.
        s0.b.star(k);
        s0.b.lda_current_function().star(t.get(0));
        s0.b.mov(Register::RECEIVER, t.get(1));
        s0.b.call_runtime(RuntimeId::CreateGeneratorObject, t, 2);
        s0.b.star(g);
        s0.b.suspend_generator(g, save, save.len(), 0);
        s0.b.bind(resumes[0]);
        s0.b.resume_generator(g, save, save.len());
        // Yield the acc the first call saw at the switch: {value: 7, done: false}.
        s0.b.ldar(k).star(t.get(0)).lda_false().star(t.get(1));
        s0.b.call_runtime(RuntimeId::CreateIterResultObject, t, 2);
        s0.b.suspend_generator(g, save, save.len(), 1);
        s0.b.bind(resumes[1]);
        // Resume: acc is still 7 at the table entry: {value: 7, done: true}.
        s0.b.star(seen);
        s0.b.resume_generator(g, save, save.len());
        s0.b.ldar(seen).return_();
        let mut s = Asm::script(0);
        let c = Calls::new(&mut s);
        define(&mut s, 1, "g");
        call_global(&mut s, "g", &[], "it");
        c.step(&mut s, "it", "next", None);
        c.step(&mut s, "it", "next", None);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), s0.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["7", "false", "7", "true"]);
}
