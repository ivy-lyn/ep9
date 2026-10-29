//! Tests of arguments objects (ARGUMENTS, kind 25; §10.4.4; M3-P2 VM-ARGS): the objects that
//! `CreateMappedArguments` and `CreateUnmappedArguments` make, the parameter map (ABI 5
//! mapped-parameter list), and the internal-method hooks of `arguments.rs` through the
//! `property.rs` API. The programs are hand-assembled; the JS-level checks are the test262
//! files under `language/arguments-object`.

use ep9_bytecode::{FunctionCode, VerifiedUnit};
use ep9_bytecode::{
    FunctionFlags, FunctionId, Register, RegisterList, ScopeId, ScopeKind, VariableMode,
};

use super::{Asm, config, isolate, meta, ok, on_big_stack, run_all, unit_with_scopes};
use crate::builtins::{self, BuiltinId};
use crate::descriptor::Desc;
use crate::frame::{UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::key::{array_index_of, index_key};
use crate::object::{self, attr};
use crate::runtime::{Atom, Runtime};
use crate::symbol::{self, WellKnownSymbol};
use crate::{Dispatch, kinds, natives, property};

/// `function f(a, b) { ... }` (sloppy, simple parameters, function 1, scope 0): its context
/// holds `a` and `b` (slots 0 and 1) and `slots` is its mapped-parameter list. The body
/// leaves the arguments object in the register it returns.
fn mapped_f(slots: Vec<Option<u32>>, body: impl FnOnce(&mut Asm, Register)) -> FunctionCode {
    let mut f = Asm::new(meta("f", 2, 0, 0, 0));
    let ctx = f.b.new_register();
    let args = f.b.new_register();
    let ks = f.b.scope_constant(ScopeId(0));
    f.b.create_function_context(ks, 2).push_context(ctx);
    for i in 0..2 {
        f.b.ldar(Register::parameter(i)).sta_current_context_slot(i);
    }
    f.b.set_mapped_parameters(ScopeId(0), slots);
    f.b.create_mapped_arguments().star(args);
    body(&mut f, args);
    f.b.ldar(args).return_();
    f.finish()
}

/// A unit whose script calls `f(1, 2, 3)` (function 1) and stores the result in the global
/// `A`, then returns it.
fn call_f(f: FunctionCode) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(s.f);
    let args = s.a;
    for i in 0..3 {
        s.b.lda_smi(i as i32 + 1).star(args.get(i));
    }
    s.b.call_undefined_receiver(s.f, args, 3, s.call);
    s.sta_global("A");
    s.b.return_();
    unit_with_scopes(
        "",
        vec![(
            ScopeKind::Function,
            None,
            vec![("a", VariableMode::Var), ("b", VariableMode::Var)],
        )],
        vec![s.finish(), f],
    )
}

/// §10.4.4.3-4: a mapped index and its parameter alias both ways; an extra argument is not
/// mapped. Every dispatch, with and without GC stress.
#[test]
fn mapped_indices_alias_the_parameters() {
    fn make() -> VerifiedUnit {
        call_f(mapped_f(vec![Some(0), Some(1)], |f, args| {
            // a = 10; print(arguments[0])
            f.b.lda_smi(10).sta_current_context_slot(0);
            f.b.lda_zero().get_keyed_property(args, f.keyed);
            f.print_acc();
            // arguments[1] = 20; print(b)
            let k = f.b.new_register();
            f.b.lda_smi(1).star(k).lda_smi(20);
            f.b.set_keyed_property(args, k, f.set_keyed);
            f.b.lda_current_context_slot(1);
            f.print_acc();
            // print(arguments[2]); print(arguments.length)
            f.b.lda_smi(2).get_keyed_property(args, f.keyed);
            f.print_acc();
            f.get_named(args, "length");
            f.print_acc();
        }))
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(r.output, ["10", "20", "3", "3"]);
}

/// Runs `make` in a new isolate (GC stress `stress`) and calls `check` with the global `A`.
fn with_a(make: fn() -> VerifiedUnit, stress: bool, check: fn(&mut Cx, u64)) {
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        iso.with_cx_checkpoint(|cx| cx.run_script(make(), "test.js").map(|_| ()))
            .unwrap();
        let a = iso.global_raw("A").expect("A").to_bits();
        let h = iso.rt.heap.new_global(crate::runtime::val(a));
        check(&mut iso.cx, a);
        iso.rt.heap.drop_global(h);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

fn key(rt: &Runtime, i: u32) -> u64 {
    index_key(rt, i)
}

fn get(cx: &mut Cx, a: u64, i: u32) -> u64 {
    let k = key(cx.rt(), i);
    property::get(cx, a, k).unwrap()
}

/// The parameter in context slot `s` of the arguments object `a`'s parameter map.
pub(super) fn param(a: u64, s: u32) -> u64 {
    object::context_get(object::fixed_get(a, 0), s).unwrap()
}

/// Sets the parameter in context slot `s` of the arguments object `a`'s parameter map.
pub(super) fn set_param(cx: &Cx, a: u64, s: u32, v: u64) {
    assert!(object::context_set(cx.rt(), object::fixed_get(a, 0), s, v));
}

/// The own keys of `a` as text (symbols as `@@iterator`).
pub(super) fn keys(cx: &mut Cx, a: u64) -> Vec<String> {
    let ks = property::own_property_keys(cx, a);
    let it = symbol::well_known(cx.rt(), WellKnownSymbol::Iterator);
    (0..object::array_len(ks))
        .map(|j| object::array_get(ks, j).unwrap())
        .map(|k| {
            if k == it {
                "@@iterator".to_string()
            } else {
                crate::string::to_rust_string(k)
            }
        })
        .collect()
}

/// A sloppy `function f(a, b) { return arguments; }` (function 1, `a` and `b` mapped),
/// called as `f(1, 2, 3)`; the global `A` holds the result.
pub(super) fn mapped_a() -> VerifiedUnit {
    call_f(mapped_f(vec![Some(0), Some(1)], |_, _| {}))
}

/// CreateMappedArgumentsObject (§10.4.4.7): kind ARGUMENTS, %Object.prototype%, the keys in
/// order (indices, `length`, `callee`, @@iterator), `length` and `callee` {w, c},
/// @@iterator %Array.prototype.values%, elements {w, e, c}.
#[test]
fn a_mapped_arguments_object_has_the_spec_properties() {
    for stress in [false, true] {
        with_a(mapped_a, stress, |cx, a| {
            let rt = cx.rt();
            assert_eq!(kinds::cell_kind(a), kinds::ARGUMENTS.get());
            assert_eq!(
                object::proto_of(a),
                rt.root(crate::runtime::Root::ObjectPrototype)
            );
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"]
            );
            let rt = cx.rt();
            let length = rt.atom(Atom::Length);
            let d = property::get_own_property(cx, a, length).unwrap().unwrap();
            assert_eq!(d, Desc::data(int32_bits(3), attr::HIDDEN));
            let callee = crate::atoms::intern_str(cx.rt(), "callee");
            let d = property::get_own_property(cx, a, callee).unwrap().unwrap();
            assert!(kinds::is_callable(d.value.unwrap()));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(true), Some(false), Some(true))
            );
            let it = symbol::well_known(cx.rt(), WellKnownSymbol::Iterator);
            let d = property::get_own_property(cx, a, it).unwrap().unwrap();
            let values = BuiltinId::new(natives::VM_BLOCK, natives::vm::ARRAY_VALUES);
            assert_eq!(d.value, builtins::existing_function(cx.rt(), values));
            assert_eq!(d.enumerable, Some(false));
            for i in 0..3 {
                let k = key(cx.rt(), i);
                let d = property::get_own_property(cx, a, k).unwrap().unwrap();
                assert_eq!(d, Desc::data(int32_bits(i as i32 + 1), attr::DEFAULT));
            }
        });
    }
}

/// §10.4.4.1-5 through the `property.rs` API: reads and writes alias while mapped;
/// `{writable: false}` keeps the parameter's value and unmaps (§10.4.4.2 step 3); a
/// non-configurable mapped index stays mapped and cannot be deleted; an accessor moves to the
/// ordinary storage and unmaps; delete unmaps; a deleted index comes back as an ordinary
/// property in key order.
#[test]
fn the_hooks_follow_the_parameter_map() {
    for stress in [false, true] {
        with_a(mapped_a, stress, |cx, a| {
            // [[Get]] and [[Set]] alias.
            set_param(cx, a, 0, int32_bits(7));
            assert_eq!(get(cx, a, 0), int32_bits(7));
            let k0 = key(cx.rt(), 0);
            property::set(cx, a, k0, int32_bits(8), true).unwrap();
            assert_eq!(param(a, 0), int32_bits(8));
            // [[GetOwnProperty]] reads the parameter.
            set_param(cx, a, 0, int32_bits(9));
            let d = property::get_own_property(cx, a, k0).unwrap().unwrap();
            assert_eq!(d.value, Some(int32_bits(9)));
            // {writable: false}: the value stays 9, then index 0 is unmapped.
            let ro = Desc {
                writable: Some(false),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k0, ro).unwrap());
            set_param(cx, a, 0, int32_bits(10));
            assert_eq!(get(cx, a, 0), int32_bits(9));
            assert!(!property::set_with_receiver(cx, a, k0, int32_bits(11), a).unwrap());
            assert_eq!(param(a, 0), int32_bits(10));
            // {configurable: false} keeps index 1 mapped; delete fails.
            let k1 = key(cx.rt(), 1);
            let nc = Desc {
                configurable: Some(false),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k1, nc).unwrap());
            set_param(cx, a, 1, int32_bits(5));
            assert_eq!(get(cx, a, 1), int32_bits(5));
            assert!(!property::delete(cx, a, k1, false).unwrap());
            let v = Desc {
                value: Some(int32_bits(6)),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k1, v).unwrap());
            assert_eq!(param(a, 1), int32_bits(6));
            // Index 2 (an extra argument, not mapped) becomes an accessor.
            let k2 = key(cx.rt(), 2);
            let acc = Desc {
                get: Some(UNDEFINED),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k2, acc).unwrap());
            let d = property::get_own_property(cx, a, k2).unwrap().unwrap();
            assert!(d.is_accessor() && d.enumerable == Some(true));
            assert_eq!(get(cx, a, 2), UNDEFINED);
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"]
            );
            // Delete index 0 (configurable), then add it again: an ordinary property.
            assert!(property::delete(cx, a, k0, true).unwrap());
            assert!(!property::has_own_property(cx, a, k0));
            assert_eq!(keys(cx, a), ["1", "2", "length", "callee", "@@iterator"]);
            property::define(cx, a, k0, int32_bits(12)).unwrap();
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"]
            );
            assert_eq!(get(cx, a, 0), int32_bits(12));
            assert_eq!(param(a, 0), int32_bits(10));
            // The accessor index goes through OrdinaryDelete.
            assert!(property::delete(cx, a, k2, true).unwrap());
            let ks = keys(cx, a);
            assert!(
                ks.iter()
                    .all(|k| array_index_of(crate::atoms::intern_str(cx.rt(), k)) != Some(2))
            );
        });
    }
}

/// CreateMappedArgumentsObject maps only the last of two parameters with the same name: a
/// list `[-, 0]` maps index 1, not index 0.
#[test]
fn a_shadowed_parameter_is_not_mapped() {
    fn make() -> VerifiedUnit {
        call_f(mapped_f(vec![None, Some(0)], |_, _| {}))
    }
    for stress in [false, true] {
        with_a(make, stress, |cx, a| {
            // Index 1 reads slot 0; index 0 keeps its own value.
            set_param(cx, a, 0, int32_bits(30));
            assert_eq!(get(cx, a, 0), int32_bits(1));
            assert_eq!(get(cx, a, 1), int32_bits(30));
        });
    }
}

/// CreateUnmappedArgumentsObject (§10.4.4.6) for a strict function: the elements do not
/// alias the parameters, and the keys are the indices, `length`, `callee` (the
/// %ThrowTypeError% accessor; the VM table makes it since VM-ARGS step 2) and @@iterator.
#[test]
fn an_unmapped_arguments_object_does_not_alias() {
    fn make() -> VerifiedUnit {
        let mut m = meta("g", 1, 0, 0, 0);
        m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
        let mut g = Asm::new(m);
        let args = g.b.new_register();
        g.b.create_unmapped_arguments().star(args);
        g.b.lda_smi(42).star(Register::parameter(0));
        g.b.ldar(args).return_();
        call_f(g.finish())
    }
    for stress in [false, true] {
        with_a(make, stress, |cx, a| {
            assert_eq!(get(cx, a, 0), int32_bits(1));
            assert_eq!(object::fixed_get(a, 0), UNDEFINED);
            assert!(kinds::is_callable(builtins::intrinsic(
                cx.rt(),
                builtins::IntrinsicId::ThrowTypeError,
            )));
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"]
            );
        });
    }
}

/// CreateUnmappedArgumentsObject step 9 and %ThrowTypeError% (§10.2.4.1; the VM table makes
/// it, VM-ARGS step 2): `callee` is {get, set: %ThrowTypeError%, e: false,
/// c: false}; %ThrowTypeError% is not extensible, its `length` (0) and `name` ("") are
/// {w: false, e: false, c: false}, and a call throws a TypeError.
#[test]
fn the_strict_callee_is_throw_type_error() {
    fn make() -> VerifiedUnit {
        let mut m = meta("g", 1, 0, 0, 0);
        m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
        let mut g = Asm::new(m);
        let args = g.b.new_register();
        g.b.create_unmapped_arguments().star(args);
        g.b.ldar(args).return_();
        call_f(g.finish())
    }
    for stress in [false, true] {
        with_a(make, stress, |cx, a| {
            let t = builtins::intrinsic(cx.rt(), builtins::IntrinsicId::ThrowTypeError);
            assert!(kinds::is_callable(t));
            let callee = crate::atoms::intern_str(cx.rt(), "callee");
            let d = property::get_own_property(cx, a, callee).unwrap().unwrap();
            assert_eq!((d.get, d.set), (Some(t), Some(t)));
            assert_eq!((d.enumerable, d.configurable), (Some(false), Some(false)));
            assert!(!property::is_extensible(t));
            for (atom, v) in [(Atom::Length, Some(int32_bits(0))), (Atom::Name, None)] {
                let key = cx.rt().atom(atom);
                let d = property::get_own_property(cx, t, key).unwrap().unwrap();
                assert_eq!(
                    (d.writable, d.enumerable, d.configurable),
                    (Some(false), Some(false), Some(false))
                );
                if let Some(v) = v {
                    assert_eq!(d.value, Some(v));
                } else {
                    assert_eq!(crate::string::to_rust_string(d.value.unwrap()), "");
                }
            }
            assert!(property::get(cx, a, callee).is_err());
            let e = cx.rt().take_pending();
            let name = crate::atoms::intern_str(cx.rt(), "name");
            let n = property::get_from(cx, e, name).unwrap();
            assert_eq!(crate::string::to_rust_string(n), "TypeError");
        });
    }
}

/// A strict `function g(a) { return arguments; }` (function 1), called as `f(1, 2, 3)`.
pub(super) fn unmapped_a() -> VerifiedUnit {
    let mut m = meta("g", 1, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut g = Asm::new(m);
    let args = g.b.new_register();
    g.b.create_unmapped_arguments().star(args);
    g.b.ldar(args).return_();
    call_f(g.finish())
}

/// Object.prototype.toString (§20.1.3.6 step 5): an arguments object has a [[ParameterMap]]
/// slot (undefined for an unmapped one, §10.4.4.6 step 3), so its builtinTag is "Arguments",
/// mapped or not. With no @@toStringTag the result is "[object Arguments]".
#[test]
fn the_builtin_tag_of_an_arguments_object_is_arguments() {
    fn check(cx: &mut Cx, a: u64) {
        assert_eq!(kinds::cell_kind(a), kinds::ARGUMENTS.get());
        let id = BuiltinId::new(natives::VM_BLOCK, natives::vm::OBJECT_TO_STRING);
        let to_string = builtins::function_object(cx.rt(), id);
        let s = crate::call::call_function(cx, to_string, a, &[]).unwrap();
        assert_eq!(crate::string::to_rust_string(s), "[object Arguments]");
    }
    for stress in [false, true] {
        with_a(mapped_a, stress, check);
        with_a(unmapped_a, stress, check);
    }
}

// ---- the ARGS fast path (V10-D11, M5-P2 VMIC) ------------------------------------------------

/// A unit whose script calls `f` (function 1) four times and stores each result in a global:
/// `B = f(1, 2, 3)` (the realm's first object of its kind: the spec path, which caches the
/// shape), then `A = f(1, 2, 3)`, `C = f(1, 2)` and `D = f()` (the fast path).
fn call_f_four_times(f: FunctionCode) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(s.f);
    let args = s.a;
    let two = s.b.new_register_list(2);
    for i in 0..3 {
        s.b.lda_smi(i as i32 + 1).star(args.get(i));
        if i < 2 {
            s.b.star(two.get(i));
        }
    }
    for (list, name) in [
        (args, "B"),
        (args, "A"),
        (two, "C"),
        (RegisterList::EMPTY, "D"),
    ] {
        s.b.call_undefined_receiver(s.f, list, list.len(), s.call);
        s.sta_global(name);
    }
    s.b.return_();
    unit_with_scopes(
        "",
        vec![(
            ScopeKind::Function,
            None,
            vec![("a", VariableMode::Var), ("b", VariableMode::Var)],
        )],
        vec![s.finish(), f],
    )
}

/// The mapped form of [`call_f_four_times`]: `function f(a, b) { return arguments; }`.
fn mapped_four() -> VerifiedUnit {
    call_f_four_times(mapped_f(vec![Some(0), Some(1)], |_, _| {}))
}

/// The unmapped form of [`call_f_four_times`]: a strict `function g(a) { return arguments; }`.
fn unmapped_four() -> VerifiedUnit {
    let mut m = meta("g", 1, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut g = Asm::new(m);
    let args = g.b.new_register();
    g.b.create_unmapped_arguments().star(args);
    g.b.ldar(args).return_();
    call_f_four_times(g.finish())
}

/// Runs `make` in a new isolate (GC stress `stress`) and calls `check` with the globals `B`,
/// `A`, `C` and `D`, each kept by a global handle meanwhile.
fn with_four(make: fn() -> VerifiedUnit, stress: bool, check: fn(&mut Cx, [u64; 4])) {
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        iso.with_cx_checkpoint(|cx| cx.run_script(make(), "test.js").map(|_| ()))
            .unwrap();
        let v = ["B", "A", "C", "D"].map(|n| iso.global_raw(n).expect(n).to_bits());
        let hs = v.map(|a| iso.rt.heap.new_global(crate::runtime::val(a)));
        check(&mut iso.cx, v);
        for h in hs {
            iso.rt.heap.drop_global(h);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// The REALM record field that caches the arguments shape for `mapped`.
fn cached(rt: &Runtime, mapped: bool) -> u64 {
    let realm = rt.root(crate::runtime::Root::Realm);
    let f = if mapped {
        object::realmf::MAPPED_ARGUMENTS_SHAPE
    } else {
        object::realmf::UNMAPPED_ARGUMENTS_SHAPE
    };
    object::record_get(realm, f)
}

/// Every own property of `a` as (key text, descriptor), in key order.
fn props_of(cx: &mut Cx, a: u64) -> Vec<(String, Desc)> {
    let ks = property::own_property_keys(cx, a);
    let names = keys(cx, a);
    let mut out = Vec::new();
    for (j, name) in names.into_iter().enumerate() {
        let k = object::array_get(ks, j as u32).unwrap();
        let d = property::get_own_property(cx, a, k).unwrap().unwrap();
        out.push((name, d));
    }
    out
}

/// V10-D11: the realm's first arguments object of a kind (made the spec way) leaves its
/// shape in the REALM record; every later one (the fast path) has that shape and the same
/// own properties with the same attributes and values: 3, 2 and 0 arguments (no elements
/// list), mapped and unmapped, with and without GC stress.
#[test]
fn the_fast_path_makes_the_spec_object() {
    fn check(cx: &mut Cx, [b, a, c, d]: [u64; 4], mapped: bool) {
        let s = crate::shape::shape_of(b);
        assert_eq!(
            cached(cx.rt(), mapped),
            s,
            "the first object's shape is cached"
        );
        assert_eq!(
            cached(cx.rt(), !mapped),
            UNDEFINED,
            "the other kind is not made"
        );
        for o in [a, c, d] {
            assert_eq!(kinds::cell_kind(o), kinds::ARGUMENTS.get());
            assert_eq!(
                crate::shape::shape_of(o),
                s,
                "the fast path takes the cached shape"
            );
            assert_eq!(object::proto_of(o), object::proto_of(b));
        }
        // A (fast) and B (spec) have equal properties; the unmapped `callee` accessor pairs
        // are two cells with the same halves.
        assert_eq!(props_of(cx, a), props_of(cx, b));
        assert_eq!(
            keys(cx, c),
            ["0", "1", "length", "callee", "@@iterator"],
            "f(1, 2)"
        );
        assert_eq!(keys(cx, d), ["length", "callee", "@@iterator"], "f()");
        let length = cx.rt().atom(Atom::Length);
        for (o, n) in [(a, 3), (c, 2), (d, 0)] {
            assert_eq!(property::get(cx, o, length).unwrap(), int32_bits(n));
            for i in 0..n as u32 {
                assert_eq!(get(cx, o, i), int32_bits(i as i32 + 1));
            }
        }
        assert_eq!(
            object::rd(object::cell(d), crate::object::obj::ELEMENTS),
            UNDEFINED
        );
        if mapped {
            // The parameter map of a fast object: index 0 and 1 alias `a` and `b`.
            set_param(cx, a, 0, int32_bits(70));
            assert_eq!(get(cx, a, 0), int32_bits(70));
            let k1 = key(cx.rt(), 1);
            property::set(cx, a, k1, int32_bits(80), true).unwrap();
            assert_eq!(param(a, 1), int32_bits(80));
            assert_eq!(
                object::fixed_get(d, 0),
                UNDEFINED,
                "f(): no mapped parameter"
            );
        } else {
            assert_eq!(object::fixed_get(a, 0), UNDEFINED, "no parameter map");
        }
    }
    for stress in [false, true] {
        with_four(mapped_four, stress, |cx, v| check(cx, v, true));
        with_four(unmapped_four, stress, |cx, v| check(cx, v, false));
    }
}

/// The spec tests of this file, on objects of the fast path (the second call).
#[test]
fn the_spec_tests_hold_on_the_fast_path() {
    for stress in [false, true] {
        with_four(mapped_four, stress, |cx, [_, a, _, _]| {
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"]
            );
            let callee = cx.rt().atom(Atom::Callee);
            let d = property::get_own_property(cx, a, callee).unwrap().unwrap();
            assert!(kinds::is_callable(d.value.unwrap()));
            // Delete index 0 and redefine `length`: the hooks and the ordinary storage.
            let k0 = key(cx.rt(), 0);
            assert!(property::delete(cx, a, k0, true).unwrap());
            assert_eq!(keys(cx, a), ["1", "2", "length", "callee", "@@iterator"]);
            let length = cx.rt().atom(Atom::Length);
            property::set(cx, a, length, int32_bits(9), true).unwrap();
            assert_eq!(property::get(cx, a, length).unwrap(), int32_bits(9));
        });
        with_four(unmapped_four, stress, |cx, [_, a, _, _]| {
            let t = builtins::intrinsic(cx.rt(), builtins::IntrinsicId::ThrowTypeError);
            let callee = cx.rt().atom(Atom::Callee);
            let d = property::get_own_property(cx, a, callee).unwrap().unwrap();
            assert_eq!((d.get, d.set), (Some(t), Some(t)));
            assert_eq!((d.enumerable, d.configurable), (Some(false), Some(false)));
            assert!(property::get(cx, a, callee).is_err());
            cx.rt().take_pending();
        });
    }
}

/// `arguments::unmodified_length`, the `length` read of the CreateListFromArrayLike fast
/// path: the int32 in field 0 while the object keeps the realm's cached shape (a value store
/// keeps it); `None` for a negative or non-int32 `length` and after any shape change (a new
/// property, a redefined or deleted `length`). CreateListFromArrayLike reads the same list
/// on both paths: a changed `length` past the elements reads the chain.
#[test]
fn the_list_fast_path_reads_an_unmodified_arguments_object() {
    use crate::abi::{Cx as AbiCx, JsValue, Local};
    use crate::arguments::unmodified_length;
    fn list(cx: &mut Cx, a: u64) -> Vec<u64> {
        let mut acx = AbiCx::new(cx);
        let l = acx
            .create_list_from_array_like(Local::<JsValue>::from_bits(a))
            .unwrap();
        (0..l.len()).map(|i| l.get(i).bits()).collect()
    }
    for stress in [false, true] {
        with_four(mapped_four, stress, |cx, [b, a, c, d]| {
            let rt = cx.rt();
            assert_eq!(unmodified_length(rt, b), Some(3));
            assert_eq!(unmodified_length(rt, c), Some(2));
            assert_eq!(unmodified_length(rt, d), Some(0));
            let i = int32_bits;
            assert_eq!(list(cx, a), [i(1), i(2), i(3)]);
            assert_eq!(list(cx, d), Vec::<u64>::new());
            // A mapped element reads its parameter.
            set_param(cx, a, 1, i(20));
            assert_eq!(list(cx, a), [i(1), i(20), i(3)]);
            // length = 5 (a value store: the same shape): indices 3 and 4 read the chain,
            // undefined, then %Object.prototype%[4] once it has one.
            let length = cx.rt().atom(Atom::Length);
            property::set(cx, a, length, i(5), true).unwrap();
            assert_eq!(unmodified_length(cx.rt(), a), Some(5));
            assert_eq!(list(cx, a), [i(1), i(20), i(3), UNDEFINED, UNDEFINED]);
            let proto = object::proto_of(a);
            property::set_keyed(cx, proto, i(4), i(44), true).unwrap();
            assert_eq!(list(cx, a), [i(1), i(20), i(3), UNDEFINED, i(44)]);
            // A deleted element reads the chain too (undefined at index 0).
            let k0 = key(cx.rt(), 0);
            assert!(property::delete(cx, a, k0, true).unwrap());
            assert_eq!(list(cx, a), [UNDEFINED, i(20), i(3), UNDEFINED, i(44)]);
            // A negative or a double `length`: the generic path (ToLength).
            property::set(cx, a, length, i(-1), true).unwrap();
            assert_eq!(unmodified_length(cx.rt(), a), None);
            assert_eq!(list(cx, a), Vec::<u64>::new());
            property::set(cx, a, length, crate::conv::num(2.5), true).unwrap();
            assert_eq!(unmodified_length(cx.rt(), a), None);
            assert_eq!(list(cx, a), [UNDEFINED, i(20)]);
            // A new own property changes the shape: the generic path.
            let x = crate::atoms::intern_str(cx.rt(), "x");
            property::set(cx, c, x, i(1), true).unwrap();
            assert_eq!(unmodified_length(cx.rt(), c), None);
            assert_eq!(list(cx, c), [i(1), i(2)]);
            // A deleted `length`: the generic path reads undefined, so an empty list.
            assert!(property::delete(cx, b, length, true).unwrap());
            assert_eq!(unmodified_length(cx.rt(), b), None);
            assert_eq!(list(cx, b), Vec::<u64>::new());
        });
    }
}

/// Each realm caches its own arguments shapes: a second realm's first object is made the spec
/// way with that realm's %Object.prototype%, its shape goes into the second REALM record, and
/// the first realm's cache is not used there (nor its shape taken as unmodified there).
#[test]
fn each_realm_caches_its_own_arguments_shape() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        iso.with_cx_checkpoint(|cx| cx.run_script(mapped_four(), "test.js").map(|_| ()))
            .unwrap();
        let a1 = iso.global_raw("A").unwrap().to_bits();
        let h1 = iso.rt.heap.new_global(crate::runtime::val(a1));
        let s1 = cached(&iso.rt, true);
        assert_eq!(crate::shape::shape_of(a1), s1);
        let r = iso.create_realm(|cx, g| {
            cx.run_script(mapped_four(), "realm2.js")?;
            let rt: &Runtime = cx.inner().rt();
            let s2 = cached(rt, true);
            assert!(crate::shape::is_shape(s2));
            assert_ne!(s2, s1, "the second realm's own shape");
            let op2 = rt.root(crate::runtime::Root::ObjectPrototype);
            assert_eq!(crate::shape::proto(s2), op2);
            for n in ["B", "A", "C", "D"] {
                let k = crate::atoms::intern_str(rt, n);
                let o = property::get(cx.inner(), g.bits(), k).unwrap();
                assert_eq!(crate::shape::shape_of(o), s2);
                assert_eq!(object::proto_of(o), op2);
            }
            assert_eq!(crate::arguments::unmodified_length(rt, a1), None);
            Ok(())
        });
        assert_eq!(r, Ok(()));
        assert_eq!(cached(&iso.rt, true), s1, "the first realm's cache stays");
        assert_eq!(crate::arguments::unmodified_length(&iso.rt, a1), Some(3));
        iso.rt.heap.drop_global(h1);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}
