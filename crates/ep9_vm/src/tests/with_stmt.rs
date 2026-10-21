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
        let (o, ctx, name) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
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
            s.b.star(name);
            s.b.call_runtime_for_pair(
                RuntimeId::LoadLookupSlotForCall,
                RegisterList::new(name.local_index().unwrap(), 1),
                1,
                out,
            );
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
                let r = iso.run_script(make(look), "poll.js");
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
