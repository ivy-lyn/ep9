//! Tests of arguments objects (ARGUMENTS, kind 25; §10.4.4; M3-P2 VM-ARGS): the objects that
//! `CreateMappedArguments` and `CreateUnmappedArguments` make, the parameter map (ABI 5
//! mapped-parameter list), and the internal-method hooks of `arguments.rs` through the
//! `property.rs` API. The programs are hand-assembled; the JS-level checks are the test262
//! files under `language/arguments-object`.

use ep9_bytecode::{FunctionCode, VerifiedUnit};
use ep9_bytecode::{FunctionFlags, FunctionId, Register, ScopeId, ScopeKind, VariableMode};

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
        iso.run_script(make(), "test.js").unwrap();
        let a = iso.global("A").expect("A").to_bits();
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
