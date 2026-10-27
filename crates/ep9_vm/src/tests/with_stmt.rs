//! Tests of `with` and dynamic scope lookup (P2-LOOKUP, `interp/slow/lookup.rs`): hand-built
//! units with `CreateWithContext`, `LdaLookupSlot*`, `StaLookupSlot`, `DeleteLookupSlot` (id 8)
//! and `CallRuntimeForPair LoadLookupSlotForCall` (id 26). Every unit runs under every
//! dispatch expansion, with and without GC stress, and all runs must agree. The JS-level
//! tests are in `crates/ep9_builtins/tests/with_stmt.rs`.

use ep9_bytecode::{
    FeedbackSlot, FeedbackSlotKind, FunctionCode, FunctionId, RegisterList, RuntimeId, ScopeFlags,
    ScopeId, ScopeKind, UnitBuilder, UnitString, VariableMode, VerifiedUnit,
};

use super::{
    Asm, Run, config, isolate, meta, ok, on_big_stack, opts, run_all, run_in, take_output,
    unit_with_scopes,
};
use crate::Dispatch;

/// Runs the unit from `make` under every dispatch expansion, with and without GC stress, and
/// checks that all runs agree (like `run_all`, for a capturing `make`).
fn run_every(make: impl Fn() -> VerifiedUnit + Clone + Send + 'static) -> Run {
    let mut first: Option<(String, Run)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let mk = make.clone();
            let r = on_big_stack(move || run_in(config(d, stress), mk()));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// The message of the uncaught exception of a run.
fn err(r: &Run) -> &str {
    match &r.result {
        Ok(v) => panic!("expected an exception, got {v}"),
        Err(e) => &e.message,
    }
}

/// A unit with a script scope 0 and a With scope 1 (child of 0).
fn with_unit(fns: Vec<FunctionCode>) -> VerifiedUnit {
    unit_with_scopes(
        "",
        vec![
            (ScopeKind::Script, None, vec![]),
            (ScopeKind::With, Some(0), vec![]),
        ],
        fns,
    )
}

/// `acc = name` through `LdaLookupSlot`.
fn lookup(s: &mut Asm, name: &str) {
    let k = s.b.string_constant(name);
    s.b.lda_lookup_slot(k);
}

/// A TypeOf feedback slot for [`tof`].
fn typeof_slot(s: &mut Asm) -> FeedbackSlot {
    s.b.feedback_slot(FeedbackSlotKind::TypeOf)
}

/// `acc = typeof name` through `LdaLookupSlotInsideTypeof` (`ts`: a TypeOf slot).
fn tof(s: &mut Asm, ts: FeedbackSlot, name: &str) {
    let k = s.b.string_constant(name);
    s.b.lda_lookup_slot_inside_typeof(k).type_of(ts);
}

/// `name = acc` through `StaLookupSlot` (`strict`: flag bit 0).
fn store(s: &mut Asm, name: &str, strict: bool) {
    let k = s.b.string_constant(name);
    s.b.sta_lookup_slot(k, u8::from(strict));
}

/// `acc = delete name` through `CallRuntime DeleteLookupSlot`. Clobbers `a[0]`.
fn delete(s: &mut Asm, name: &str) {
    s.lda_str(name);
    let a0 = s.a.get(0);
    s.b.star(a0);
    s.b.call_runtime(
        RuntimeId::DeleteLookupSlot,
        RegisterList::new(a0.local_index().unwrap(), 1),
        1,
    );
}

/// `with (o) { ... }` over an object literal: reads, sloppy and strict stores, typeof, a
/// global reached through the with context, a sloppy store that makes a global, delete.
#[test]
fn with_context_is_an_object_environment() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (o, ctx) = (s.b.new_register(), s.b.new_register());
        let ts = typeof_slot(&mut s);
        s.b.lda_smi(7);
        s.sta_global("g");
        s.b.create_empty_object_literal().star(o);
        s.b.lda_smi(1);
        s.set_named(o, "x");
        let kw = s.b.scope_constant(ScopeId(1));
        s.b.create_with_context(o, kw).push_context(ctx);
        // x -> 1 (the object), g -> 7 (the global, through the with context)
        lookup(&mut s, "x");
        s.print_acc();
        lookup(&mut s, "g");
        s.print_acc();
        // x = 2 lands on the object, not on a global
        s.b.lda_smi(2);
        store(&mut s, "x", false);
        s.get_named(o, "x");
        s.print_acc();
        tof(&mut s, ts, "x");
        s.print_acc();
        // a sloppy store of an unresolvable name makes a global property
        s.b.lda_smi(5);
        store(&mut s, "fresh", false);
        s.lda_global("fresh");
        s.print_acc();
        // typeof of an unresolvable name
        tof(&mut s, ts, "nope");
        s.print_acc();
        // delete x deletes the property; then x is unresolvable
        delete(&mut s, "x");
        s.print_acc();
        tof(&mut s, ts, "x");
        s.print_acc();
        // delete of a global var-like property made by a store: configurable, true
        delete(&mut s, "fresh");
        s.print_acc();
        s.b.pop_context(ctx);
        // a strict store of an unresolvable name: ReferenceError
        s.b.lda_smi(3);
        store(&mut s, "undeclared", true);
        s.b.lda_undefined().return_();
        with_unit(vec![s.finish()])
    }
    let r = run_all(make);
    assert_eq!(
        r.output,
        [
            "1",
            "7",
            "2",
            "number",
            "5",
            "undefined",
            "true",
            "undefined",
            "true"
        ]
    );
    assert_eq!(err(&r), "ReferenceError: undeclared is not defined");
}

/// `CallRuntimeForPair LoadLookupSlotForCall`: the function and its receiver, which is the
/// `with` object that holds the name (WithBaseObject), else undefined. Every dispatch, with
/// and without GC stress.
#[test]
fn load_lookup_slot_for_call_gives_the_with_object() {
    fn make() -> VerifiedUnit {
        // f1: function self_() { "use strict"; return this; }
        let mut m = meta("self_", 0, 0, 0, 0);
        m.flags =
            ep9_bytecode::FunctionFlags::STRICT | ep9_bytecode::FunctionFlags::SIMPLE_PARAMETERS;
        let mut f = Asm::new(m);
        f.b.ldar(f.b.receiver()).return_();
        let mut s = Asm::script(0);
        let (o, ctx) = (s.b.new_register(), s.b.new_register());
        let args = s.b.new_register_list(2);
        let out = s.b.new_register_list(2);
        let kf = s.b.function_constant(FunctionId(1));
        s.b.create_empty_object_literal().star(o);
        s.b.create_closure(kf, s.closure, 0);
        s.set_named(o, "m");
        s.b.create_closure(kf, s.closure, 0);
        s.sta_global("gm");
        let kw = s.b.scope_constant(ScopeId(1));
        s.b.create_with_context(o, kw).push_context(ctx);
        let call = |s: &mut Asm, n: &str| {
            s.lda_str(n);
            s.b.star(args.get(0));
            s.b.lda_false().star(args.get(1));
            s.b.call_runtime_for_pair(RuntimeId::LoadLookupSlotForCall, args, 2, out);
            let slot = s.call;
            s.b.call_property0(out.get(0), out.get(1), slot);
        };
        // m() inside with (o): this === o
        call(&mut s, "m");
        s.b.test_equal_strict(o, s.cmp);
        s.print_acc();
        // gm() (a global): this is undefined
        call(&mut s, "gm");
        s.b.test_undefined();
        s.print_acc();
        s.b.pop_context(ctx);
        // outside the with: m is unresolvable
        call(&mut s, "m");
        s.b.lda_undefined().return_();
        with_unit(vec![s.finish(), f.finish()])
    }
    let r = run_all(make);
    assert_eq!(r.output, ["true", "true"]);
    assert_eq!(err(&r), "ReferenceError: m is not defined");
}

/// The scopes of the declarative tests: a function scope with `a` (let, in its TDZ), `c`
/// (const) and `v` (var), and the With scope 1 as its child.
fn decl_unit(fns: Vec<FunctionCode>) -> VerifiedUnit {
    unit_with_scopes(
        "",
        vec![
            (
                ScopeKind::Function,
                None,
                vec![
                    ("a", VariableMode::Let),
                    ("c", VariableMode::Const),
                    ("v", VariableMode::Var),
                ],
            ),
            (ScopeKind::With, Some(0), vec![]),
        ],
        fns,
    )
}

/// The script head of the declarative tests: the function context (a = hole, c = 1, v = 2)
/// and an empty `with` object inside it. Returns the register that saved the function
/// context (for `PopContext`).
fn decl_head(s: &mut Asm) -> ep9_bytecode::Register {
    let (ctx, o, wctx) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    let kf = s.b.scope_constant(ScopeId(0));
    s.b.create_function_context(kf, 3).push_context(ctx);
    s.b.lda_the_hole().sta_current_context_slot(0);
    s.b.lda_smi(1).sta_current_context_slot(1);
    s.b.lda_smi(2).sta_current_context_slot(2);
    s.b.create_empty_object_literal().star(o);
    let kw = s.b.scope_constant(ScopeId(1));
    s.b.create_with_context(o, kw).push_context(wctx);
    wctx
}

/// Declarative contexts are searched by slot name (the unit scope names each slot): a read
/// and a store of a var through the with context, a const read, delete of a slot (false).
#[test]
fn declarative_contexts_bind_by_slot_name() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let saved = decl_head(&mut s);
        lookup(&mut s, "v");
        s.print_acc();
        s.b.lda_smi(3);
        store(&mut s, "v", true);
        lookup(&mut s, "c");
        s.print_acc();
        delete(&mut s, "v");
        s.print_acc();
        s.b.pop_context(saved);
        s.b.lda_current_context_slot(2);
        s.print_acc();
        s.b.lda_undefined().return_();
        decl_unit(vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["2", "1", "false", "3"]);
}

/// The errors of a declarative binding: a TDZ read (also inside `typeof`) and a TDZ store
/// are ReferenceErrors; a store to a const is a TypeError, in sloppy code too.
#[test]
fn declarative_binding_errors() {
    let cases: [(u8, &str); 4] = [
        (0, "ReferenceError: Cannot access 'a' before initialization"),
        (1, "ReferenceError: Cannot access 'a' before initialization"),
        (2, "ReferenceError: Cannot access 'a' before initialization"),
        (3, "TypeError: Assignment to constant variable."),
    ];
    for (case, want) in cases {
        let make = move || {
            let mut s = Asm::script(0);
            decl_head(&mut s);
            let ts = typeof_slot(&mut s);
            match case {
                0 => lookup(&mut s, "a"),
                1 => tof(&mut s, ts, "a"),
                2 => {
                    s.b.lda_smi(9);
                    store(&mut s, "a", false);
                }
                _ => {
                    s.b.lda_smi(9);
                    store(&mut s, "c", false);
                }
            }
            s.b.lda_undefined().return_();
            decl_unit(vec![s.finish()])
        };
        let r = run_every(make);
        assert_eq!(err(&r), want, "case {case}");
    }
}

/// The `.eval_ext` object in slot 0 of a CALLS_SLOPPY_EVAL var scope holds the vars of sloppy
/// direct eval code: a read, a store and a delete reach its own properties, and an inner
/// `with` object still comes first.
#[test]
fn eval_ext_object_is_searched_in_its_scope() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, ext, o, wctx) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let kf = s.b.scope_constant(ScopeId(0));
        s.b.create_function_context(kf, 2).push_context(ctx);
        s.b.create_empty_object_literal().star(ext);
        s.b.lda_smi(9);
        s.set_named(ext, "e");
        s.b.ldar(ext).sta_current_context_slot(0);
        s.b.lda_smi(4).sta_current_context_slot(1);
        // e -> 9; e = 10 lands on the .eval_ext object
        lookup(&mut s, "e");
        s.print_acc();
        s.b.lda_smi(10);
        store(&mut s, "e", false);
        s.get_named(ext, "e");
        s.print_acc();
        // a with object that also has e comes first
        s.b.create_empty_object_literal().star(o);
        s.lda_str("with-e");
        s.set_named(o, "e");
        let kw = s.b.scope_constant(ScopeId(1));
        s.b.create_with_context(o, kw).push_context(wctx);
        lookup(&mut s, "e");
        s.print_acc();
        s.b.pop_context(wctx);
        // the slot binding k is found by name too
        lookup(&mut s, "k");
        s.print_acc();
        // delete e removes the eval var (true); then e is unresolvable
        delete(&mut s, "e");
        s.print_acc();
        let ts = typeof_slot(&mut s);
        tof(&mut s, ts, "e");
        s.print_acc();
        s.b.lda_undefined().return_();
        let mut u = UnitBuilder::new("", opts());
        u.add_scope(
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT | ScopeFlags::CALLS_SLOPPY_EVAL,
            None,
            vec![
                (
                    UnitString::new(ep9_bytecode::EVAL_EXT_BINDING),
                    VariableMode::Var,
                ),
                (UnitString::new("k"), VariableMode::Var),
            ],
        );
        u.add_scope(
            ScopeKind::With,
            ScopeFlags::NEEDS_CONTEXT,
            Some(ScopeId(0)),
            vec![],
        );
        u.set_function(FunctionId(0), s.finish());
        u.finish().unwrap()
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["9", "10", "with-e", "4", "true", "undefined"]);
}

/// The implicit bindings that direct eval code reads by name (`this`, `new.target`,
/// `.home_object`, `.callee`; ep9_bytecode eval_context.rs) are never answered by a `with`
/// object, a `.eval_ext` object or a global property: they come from the context slot that
/// holds them, or the global this for `this`. `arguments` is an ordinary name, so the `with`
/// object answers it.
#[test]
fn implicit_names_skip_object_environments() {
    const NAMES: [&str; 5] = ["this", "new.target", ".home_object", ".callee", "arguments"];
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, ext, o, wctx) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        // Scope 0: the function context, slots [.eval_ext, this, new.target, .home_object,
        // .callee, arguments] = [ext, 1, 2, 3, 4, 5].
        let kf = s.b.scope_constant(ScopeId(0));
        s.b.create_function_context(kf, 6).push_context(ctx);
        s.b.create_empty_object_literal().star(ext);
        s.b.create_empty_object_literal().star(o);
        for (i, n) in NAMES.iter().enumerate() {
            s.b.lda_smi(10 + i as i32);
            s.set_named(ext, n);
            s.b.lda_smi(20 + i as i32);
            s.set_named(o, n);
        }
        s.b.ldar(ext).sta_current_context_slot(0);
        for i in 1..=5 {
            s.b.lda_smi(i).sta_current_context_slot(i as u32);
        }
        // Inside `with (o)`: 1 2 3 4 and the with object's `arguments` (24).
        let kw = s.b.scope_constant(ScopeId(1));
        s.b.create_with_context(o, kw).push_context(wctx);
        for n in NAMES {
            lookup(&mut s, n);
            s.print_acc();
        }
        s.b.pop_context(ctx);
        // A global property named `this` does not answer either; with no context slot,
        // `this` is the global this and `new.target` is undefined.
        s.b.lda_smi(99);
        s.sta_global("this");
        s.b.lda_smi(98);
        s.sta_global("new.target");
        s.b.create_with_context(o, kw).push_context(wctx);
        lookup(&mut s, "this");
        let g = s.b.new_register();
        s.b.star(g);
        s.lda_global("globalThis");
        s.b.test_equal_strict(g, s.cmp);
        s.print_acc();
        lookup(&mut s, "new.target");
        s.print_acc();
        s.b.lda_undefined().return_();
        let mut u = UnitBuilder::new("", opts());
        let mut slots = vec![(
            UnitString::new(ep9_bytecode::EVAL_EXT_BINDING),
            VariableMode::Var,
        )];
        slots.extend(
            NAMES
                .iter()
                .map(|n| (UnitString::new(n), VariableMode::Var)),
        );
        u.add_scope(
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT | ScopeFlags::CALLS_SLOPPY_EVAL,
            None,
            slots,
        );
        u.add_scope(
            ScopeKind::With,
            ScopeFlags::NEEDS_CONTEXT,
            Some(ScopeId(0)),
            vec![],
        );
        u.set_function(FunctionId(0), s.finish());
        u.finish().unwrap()
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["1", "2", "3", "4", "24", "true", "undefined"]);
}

/// The same names in a context with no slot for them: the `.eval_ext` object of a
/// CALLS_SLOPPY_EVAL scope does not answer them, the outer slot does.
#[test]
fn implicit_names_skip_the_eval_ext_object() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (outer, inner, ext) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        let k0 = s.b.scope_constant(ScopeId(0));
        s.b.create_function_context(k0, 1).push_context(outer);
        s.b.lda_smi(1).sta_current_context_slot(0);
        let k1 = s.b.scope_constant(ScopeId(1));
        s.b.create_function_context(k1, 1).push_context(inner);
        s.b.create_empty_object_literal().star(ext);
        s.b.lda_smi(7);
        s.set_named(ext, "this");
        s.b.ldar(ext).sta_current_context_slot(0);
        lookup(&mut s, "this");
        s.print_acc();
        s.b.lda_undefined().return_();
        let mut u = UnitBuilder::new("", opts());
        u.add_scope(
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT,
            None,
            vec![(UnitString::new("this"), VariableMode::Var)],
        );
        u.add_scope(
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT | ScopeFlags::CALLS_SLOPPY_EVAL,
            Some(ScopeId(0)),
            vec![(
                UnitString::new(ep9_bytecode::EVAL_EXT_BINDING),
                VariableMode::Var,
            )],
        );
        u.set_function(FunctionId(0), s.finish());
        u.finish().unwrap()
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["1"]);
}

/// `regs` = [env, name, value, strict]: `env = ResolveLookupSlot(name)` (runtime id 29).
fn resolve_ref(s: &mut Asm, regs: RegisterList, name: &str) {
    s.lda_str(name);
    s.b.star(regs.get(1));
    let first = regs.get(0).local_index().unwrap();
    s.b.call_runtime(
        RuntimeId::ResolveLookupSlot,
        RegisterList::new(first + 1, 1),
        1,
    );
    s.b.star(regs.get(0));
}

/// `acc = LoadLookupSlotResolved(env, name, false)` (runtime id 30, sloppy).
fn load_ref(s: &mut Asm, regs: RegisterList) {
    load_ref_in(s, regs, false);
}

/// `acc = LoadLookupSlotResolved(env, name, strict)` (runtime id 30); `regs[2]` holds the
/// strictness, as BytecodeGen puts it.
fn load_ref_in(s: &mut Asm, regs: RegisterList, strict: bool) {
    if strict {
        s.b.lda_true();
    } else {
        s.b.lda_false();
    }
    s.b.star(regs.get(2));
    let first = regs.get(0).local_index().unwrap();
    s.b.call_runtime(
        RuntimeId::LoadLookupSlotResolved,
        RegisterList::new(first, 3),
        3,
    );
}

/// `StoreLookupSlotResolved(env, name, acc, strict)` (runtime id 31).
fn store_ref(s: &mut Asm, regs: RegisterList, strict: bool) {
    s.b.star(regs.get(2));
    if strict {
        s.b.lda_true();
    } else {
        s.b.lda_false();
    }
    s.b.star(regs.get(3));
    s.b.call_runtime(RuntimeId::StoreLookupSlotResolved, regs, 4);
}

/// Runtime ids 29-31 (ABI 4): a reference resolved once keeps its binding while other code
/// changes the scope: a `with` property deleted in between is set again on the object (sloppy),
/// a declarative slot stays the target when the `with` object gains the name, an
/// unresolvable name stays unresolvable (a sloppy store makes a global), and a global
/// binding stays global.
#[test]
fn a_resolved_reference_keeps_its_binding() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let saved = decl_head(&mut s);
        let o = s.b.new_register();
        let regs = s.b.new_register_list(4);
        // The with object of decl_head is in slot 0 of the current (with) context.
        s.b.lda_current_context_slot(0).star(o);
        s.b.lda_smi(1);
        s.set_named(o, "x");
        // with: resolve x, delete it, store 2 -> o.x = 2 again; read through it -> 2.
        resolve_ref(&mut s, regs, "x");
        delete(&mut s, "x");
        s.b.lda_smi(2);
        store_ref(&mut s, regs, false);
        s.get_named(o, "x");
        s.print_acc();
        load_ref(&mut s, regs);
        s.print_acc();
        // declarative: resolve v (slot 2), then o gains v; the store still goes to the slot.
        resolve_ref(&mut s, regs, "v");
        s.b.lda_smi(7);
        s.set_named(o, "v");
        s.b.lda_smi(3);
        store_ref(&mut s, regs, true);
        load_ref(&mut s, regs);
        s.print_acc();
        s.get_named(o, "v");
        s.print_acc();
        // unresolvable: stays unresolvable; a sloppy store makes a global property.
        resolve_ref(&mut s, regs, "fresh");
        s.b.lda_smi(4);
        s.set_named(o, "fresh");
        s.b.lda_smi(5);
        store_ref(&mut s, regs, false);
        s.lda_global("fresh");
        s.print_acc();
        s.get_named(o, "fresh");
        s.print_acc();
        // global: g resolves to the global environment, then o gains g.
        s.b.lda_smi(8);
        s.sta_global("g");
        resolve_ref(&mut s, regs, "g");
        s.b.lda_smi(9);
        s.set_named(o, "g");
        s.b.lda_smi(10);
        store_ref(&mut s, regs, true);
        s.lda_global("g");
        s.print_acc();
        s.b.pop_context(saved);
        s.b.lda_current_context_slot(2);
        s.print_acc();
        s.b.lda_undefined().return_();
        decl_unit(vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["2", "2", "3", "7", "5", "4", "10", "3"]);
}

/// The errors of a resolved reference: a strict store to a `with` binding that is gone and
/// to a name that was unresolvable (even if a global of that name exists by now) throw a
/// ReferenceError; a read of an unresolvable name throws; a store to a const throws a
/// TypeError (§9.1.1.2.5, §6.2.5.6 step 3.a, §6.2.5.5 step 3).
#[test]
fn resolved_reference_errors() {
    let cases: [(u8, &str); 4] = [
        (0, "ReferenceError: x is not defined"),
        (1, "ReferenceError: late is not defined"),
        (2, "ReferenceError: nope is not defined"),
        (3, "TypeError: Assignment to constant variable."),
    ];
    for (case, want) in cases {
        let make = move || {
            let mut s = Asm::script(0);
            decl_head(&mut s);
            let o = s.b.new_register();
            let regs = s.b.new_register_list(4);
            s.b.lda_current_context_slot(0).star(o);
            match case {
                0 => {
                    s.b.lda_smi(1);
                    s.set_named(o, "x");
                    resolve_ref(&mut s, regs, "x");
                    delete(&mut s, "x");
                    s.b.lda_smi(2);
                    store_ref(&mut s, regs, true);
                }
                1 => {
                    resolve_ref(&mut s, regs, "late");
                    s.b.lda_smi(1);
                    s.sta_global("late");
                    store_ref(&mut s, regs, true);
                }
                2 => {
                    resolve_ref(&mut s, regs, "nope");
                    load_ref(&mut s, regs);
                }
                _ => {
                    resolve_ref(&mut s, regs, "c");
                    s.b.lda_smi(2);
                    store_ref(&mut s, regs, false);
                }
            }
            s.b.lda_undefined().return_();
            decl_unit(vec![s.finish()])
        };
        let r = run_every(make);
        assert_eq!(err(&r), want, "case {case}");
    }
}

/// `LoadLookupSlotResolved` takes the strictness of the read as its third argument (ABI 4,
/// M2-P2 review fix L6), not the function's: in this sloppy script a `with` binding that was
/// resolved and then deleted reads as undefined when the argument is false and throws a
/// ReferenceError when it is true (§9.1.1.2.6 step 2; a class heritage or computed key is
/// strict code in a sloppy function).
#[test]
fn a_resolved_read_takes_its_strictness() {
    for (strict, want) in [
        (false, Ok("undefined")),
        (true, Err("ReferenceError: x is not defined")),
    ] {
        let make = move || {
            let mut s = Asm::script(0);
            decl_head(&mut s);
            let o = s.b.new_register();
            let regs = s.b.new_register_list(4);
            s.b.lda_current_context_slot(0).star(o);
            s.b.lda_smi(1);
            s.set_named(o, "x");
            resolve_ref(&mut s, regs, "x");
            delete(&mut s, "x");
            load_ref_in(&mut s, regs, strict);
            s.b.return_();
            decl_unit(vec![s.finish()])
        };
        let r = run_every(make);
        match want {
            Ok(v) => assert_eq!(ok(&r), v, "strict={strict}"),
            Err(m) => assert_eq!(err(&r), m, "strict={strict}"),
        }
    }
}

/// A var of sloppy direct eval code (the `.eval_ext` object) resolved, then deleted: the
/// store makes it again on the same object, not a global.
#[test]
fn a_resolved_eval_var_is_set_again_on_its_object() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, ext) = (s.b.new_register(), s.b.new_register());
        let regs = s.b.new_register_list(4);
        let kf = s.b.scope_constant(ScopeId(0));
        s.b.create_function_context(kf, 1).push_context(ctx);
        s.b.create_empty_object_literal().star(ext);
        s.b.lda_smi(1);
        s.set_named(ext, "e");
        s.b.ldar(ext).sta_current_context_slot(0);
        resolve_ref(&mut s, regs, "e");
        delete(&mut s, "e");
        s.b.lda_smi(6);
        store_ref(&mut s, regs, false);
        s.get_named(ext, "e");
        s.print_acc();
        let ts = typeof_slot(&mut s);
        s.b.pop_context(ctx);
        tof(&mut s, ts, "e");
        s.print_acc();
        s.b.lda_undefined().return_();
        let mut u = UnitBuilder::new("", opts());
        u.add_scope(
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT | ScopeFlags::CALLS_SLOPPY_EVAL,
            None,
            vec![(
                UnitString::new(ep9_bytecode::EVAL_EXT_BINDING),
                VariableMode::Var,
            )],
        );
        u.set_function(FunctionId(0), s.finish());
        u.finish().unwrap()
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["6", "undefined"]);
}

/// A `VariableMode::FunctionName` slot (a named function expression's own name, ABI 4) is
/// immutable but not strict: a sloppy `StaLookupSlot` or `StoreLookupSlotResolved` leaves it
/// as it is, a strict one throws a TypeError, and `delete` gives false.
#[test]
fn a_function_name_slot_ignores_sloppy_stores() {
    let cases: [(u8, Result<&[&str], &str>); 3] = [
        (0, Ok(&["1", "1", "false", "1"])),
        (1, Err("TypeError: Assignment to constant variable.")),
        (2, Err("TypeError: Assignment to constant variable.")),
    ];
    for (case, want) in cases {
        let make = move || {
            let mut s = Asm::script(0);
            let (ctx, o, wctx) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
            let regs = s.b.new_register_list(4);
            let kb = s.b.scope_constant(ScopeId(0));
            s.b.create_block_context(kb).push_context(ctx);
            s.b.lda_smi(1).sta_current_context_slot(0);
            s.b.create_empty_object_literal().star(o);
            let kw = s.b.scope_constant(ScopeId(1));
            s.b.create_with_context(o, kw).push_context(wctx);
            match case {
                0 => {
                    s.b.lda_smi(2);
                    store(&mut s, "g", false);
                    lookup(&mut s, "g");
                    s.print_acc();
                    resolve_ref(&mut s, regs, "g");
                    s.b.lda_smi(3);
                    store_ref(&mut s, regs, false);
                    load_ref(&mut s, regs);
                    s.print_acc();
                    delete(&mut s, "g");
                    s.print_acc();
                    s.b.pop_context(wctx);
                    s.b.lda_current_context_slot(0);
                    s.print_acc();
                }
                1 => {
                    s.b.lda_smi(2);
                    store(&mut s, "g", true);
                }
                _ => {
                    resolve_ref(&mut s, regs, "g");
                    s.b.lda_smi(3);
                    store_ref(&mut s, regs, true);
                }
            }
            s.b.lda_undefined().return_();
            unit_with_scopes(
                "",
                vec![
                    (
                        ScopeKind::Block,
                        None,
                        vec![("g", VariableMode::FunctionName)],
                    ),
                    (ScopeKind::With, Some(0), vec![]),
                ],
                vec![s.finish()],
            )
        };
        let r = run_every(make);
        match want {
            Ok(lines) => {
                ok(&r);
                assert_eq!(r.output, lines, "case {case}");
            }
            Err(m) => assert_eq!(err(&r), m, "case {case}"),
        }
    }
}

/// `CreateWithContext` converts a primitive with ToObject (a string's `length` is found) and
/// throws a TypeError for undefined; a With scope with another `Create*Context` is a VM
/// internal error, never a context with no object.
#[test]
fn create_with_context_operands() {
    let cases: [(u8, Result<&str, &str>); 3] = [
        (0, Ok("3")),
        (
            1,
            Err("TypeError: Cannot convert undefined or null to object"),
        ),
        (
            2,
            Err("Error: ep9 internal error: CreateWithContext and With scopes go together"),
        ),
    ];
    for (case, want) in cases {
        let make = move || {
            let mut s = Asm::script(0);
            let (o, ctx) = (s.b.new_register(), s.b.new_register());
            let kw = s.b.scope_constant(ScopeId(1));
            match case {
                0 => {
                    s.lda_str("abc");
                    s.b.star(o);
                    s.b.create_with_context(o, kw).push_context(ctx);
                }
                1 => {
                    s.b.lda_undefined().star(o);
                    s.b.create_with_context(o, kw).push_context(ctx);
                }
                _ => {
                    s.b.create_block_context(kw).push_context(ctx);
                }
            }
            lookup(&mut s, "length");
            s.b.return_();
            with_unit(vec![s.finish()])
        };
        let r = run_every(make);
        match want {
            Ok(v) => assert_eq!(ok(&r), v, "case {case}"),
            Err(m) => assert_eq!(err(&r), m, "case {case}"),
        }
    }
}

/// The walk takes one budget unit per context and polls (M2-P2-POLL): with termination
/// requested and the budget nearly spent, a lookup through a chain of contexts stops the run,
/// and the same unit with no lookup (no loop, no call) completes.
#[test]
fn the_context_walk_polls() {
    fn make(look: bool) -> VerifiedUnit {
        let mut s = Asm::script(0);
        let ctxs: Vec<_> = (0..8).map(|_| s.b.new_register()).collect();
        let o = s.b.new_register();
        s.b.create_empty_object_literal().star(o);
        let kw = s.b.scope_constant(ScopeId(1));
        for &c in &ctxs {
            s.b.create_with_context(o, kw).push_context(c);
        }
        if look {
            let ts = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
            let k = s.b.string_constant("nope");
            s.b.lda_lookup_slot_inside_typeof(k).type_of(ts);
        } else {
            s.b.lda_undefined();
        }
        s.b.return_();
        with_unit(vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for look in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(d, false));
                take_output();
                iso.terminate_handle().terminate();
                // The entry takes one unit; the walk takes one per context.
                iso.cx.budget = 4;
                let r =
                    iso.with_cx_checkpoint(|cx| cx.run_script(make(look), "poll.js").map(|_| ()));
                if look {
                    let e = r.unwrap_err();
                    assert!(e.terminated, "{} {e:?}", d.name());
                } else {
                    assert!(r.is_ok(), "{} {r:?}", d.name());
                }
            });
        }
    }
}

/// (M3-P4, CX2-E9-1) A private name is never found by a search from the current context, and
/// a private name slot that holds no private name symbol is never used: a `#x` without the
/// eval code's list (`LoadedUnit::eval_private`, here a script) is a TypeError for
/// `LdaLookupSlot` and `ResolveLookupSlot`, and a brand slot that holds undefined is a
/// TypeError for `LoadLookupSlotResolved`. Before the fix the first two found the class
/// context's `#x` slot (undefined: an ordinary property key) and the third gave undefined.
/// An ordinary name in the same context still resolves.
#[test]
fn a_private_name_never_falls_back_to_a_search_or_a_key() {
    let cases: [(u8, Result<&[&str], &str>); 4] = [
        (
            0,
            Err("TypeError: Private name #x is not declared by a class around this code"),
        ),
        (
            1,
            Err("TypeError: Private name #x is not declared by a class around this code"),
        ),
        (
            2,
            Err("TypeError: Private name #.brand is not declared by a class around this code"),
        ),
        (3, Ok(&["5"])),
    ];
    for (case, want) in cases {
        let make = move || {
            let mut s = Asm::script(0);
            let ctx = s.b.new_register();
            let regs = s.b.new_register_list(4);
            let kc = s.b.scope_constant(ScopeId(1));
            s.b.create_block_context(kc).push_context(ctx);
            s.b.lda_smi(5).sta_current_context_slot(2);
            match case {
                0 => lookup(&mut s, "#x"),
                1 => resolve_ref(&mut s, regs, "#x"),
                2 => {
                    s.b.lda_current_context().star(regs.get(0));
                    s.lda_str("#.brand");
                    s.b.star(regs.get(1));
                    load_ref_in(&mut s, regs, true);
                }
                _ => lookup(&mut s, "y"),
            }
            s.print_acc();
            s.b.lda_undefined().return_();
            unit_with_scopes(
                "",
                vec![
                    (ScopeKind::Script, None, vec![]),
                    (
                        ScopeKind::Class,
                        Some(0),
                        vec![
                            ("#x", VariableMode::Const),
                            ("#.brand", VariableMode::Const),
                            ("y", VariableMode::Let),
                        ],
                    ),
                ],
                vec![s.finish()],
            )
        };
        let r = run_every(make);
        match want {
            Ok(lines) => {
                ok(&r);
                assert_eq!(r.output, lines, "case {case}");
            }
            Err(msg) => {
                assert_eq!(err(&r), msg, "case {case}");
                assert!(r.output.is_empty(), "case {case}: {:?}", r.output);
            }
        }
    }
}
