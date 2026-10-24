//! Tests of module records and namespaces (MODULE, MODULE_NAMESPACE) (owner M3-P2 MODULE):
//! the namespace internal methods (§10.4.6) through the routed hooks, on a namespace made by
//! hand (export names and binding boxes, as `module::get_namespace` makes them). Module
//! graphs (load, Link, Evaluate) are JS-level tests in `ep9_builtins/tests/module.rs`.

use super::{config, isolate, on_big_stack};
use crate::descriptor::Desc;
use crate::exotic::probe;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits};
use crate::object::{self, attr};
use crate::runtime::{Atom, Root, Runtime, val};
use crate::{Dispatch, module, property, props, shape};

/// A non-extensible namespace of a new MODULE record with the exports `names` (sorted) and
/// their values (`HOLE`: a binding in its TDZ). No @@toStringTag. GC point.
fn namespace(rt: &Runtime, exports: &[(&str, u64)]) -> u64 {
    let n = exports.len();
    let m = module::new_module_record(rt);
    let hm = rt.heap.new_global(val(m));
    let list = object::new_list(rt, 2 * n, UNDEFINED);
    let hl = rt.heap.new_global(val(list));
    for (i, (name, v)) in exports.iter().enumerate() {
        let atom = crate::atoms::intern_str(rt, name);
        let list = rt.heap.global(hl).to_bits();
        object::list_set(rt, list, i, atom);
        let b = object::new_list(rt, 1, *v);
        let list = rt.heap.global(hl).to_bits();
        object::list_set(rt, list, n + i, b);
    }
    let (m, list) = (rt.heap.global(hm).to_bits(), rt.heap.global(hl).to_bits());
    let ns = module::new_namespace(rt, m, list);
    property::prevent_extensions(rt, ns);
    rt.heap.drop_global(hm);
    rt.heap.drop_global(hl);
    ns
}

/// Takes the pending exception: its "name: message".
fn take_error(cx: &mut crate::interp::Cx) -> String {
    let rt = cx.rt();
    let e = rt.take_pending();
    assert!(object::is_object(e), "an error is pending");
    let name = crate::atoms::intern_str(rt, "name");
    let n = property::get_from(cx, e, name).unwrap();
    let message = crate::atoms::intern_str(cx.rt(), "message");
    let m = property::get_from(cx, e, message).unwrap();
    format!(
        "{}: {}",
        crate::string::to_rust_string(n),
        crate::string::to_rust_string(m)
    )
}

/// Each internal method of a namespace reaches its module.rs hook and gives the §10.4.6
/// answer: an export is {[[Value]], writable, enumerable, not configurable}; `[[Set]]` fails;
/// `[[DefineOwnProperty]]` accepts only no change; an export cannot be deleted, another
/// string can; `[[OwnPropertyKeys]]` lists the exports in order. The prototype is null.
#[test]
fn namespace_internal_methods_follow_10_4_6() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ns = namespace(rt, &[("a", int32_bits(1)), ("b", int32_bits(2))]);
            let hn = rt.heap.new_global(val(ns));
            let (a, x) = (
                crate::atoms::intern_str(rt, "a"),
                crate::atoms::intern_str(rt, "x"),
            );
            assert_eq!(object::proto_of(ns), NULL);
            probe::take();
            assert_eq!(property::get(cx, ns, a).unwrap(), int32_bits(1));
            assert_eq!(property::get(cx, ns, x).unwrap(), UNDEFINED);
            assert_eq!(probe::take(), ["module get", "module get"]);
            let d = property::get_own_property(cx, ns, a)
                .unwrap()
                .expect("an own a");
            assert_eq!(d.value, Some(int32_bits(1)));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(true), Some(true), Some(false))
            );
            assert!(property::get_own_property(cx, ns, x).unwrap().is_none());
            assert!(property::has_own_property(cx, ns, a));
            assert!(!property::has_own_property(cx, ns, x));
            let hits = probe::take();
            assert!(hits.contains(&"module own_property"), "{hits:?}");
            assert!(hits.contains(&"module own_attrs"), "{hits:?}");
            assert!(!property::set_with_receiver(cx, ns, a, int32_bits(5), ns).unwrap());
            assert!(!property::set_with_receiver(cx, ns, x, int32_bits(5), ns).unwrap());
            assert_eq!(probe::take(), ["module set", "module set"]);
            let same = Desc::data(int32_bits(1), attr::WRITABLE | attr::ENUMERABLE);
            assert!(property::define_own_property(cx, ns, a, same).unwrap());
            let other = Desc::data(int32_bits(9), attr::WRITABLE | attr::ENUMERABLE);
            assert!(!property::define_own_property(cx, ns, a, other).unwrap());
            assert!(!property::define_own_property(cx, ns, x, same).unwrap());
            assert!(!property::delete(cx, ns, a, false).unwrap());
            assert!(property::delete(cx, ns, x, false).unwrap());
            let keys = property::own_property_keys(cx, ns);
            assert_eq!(object::array_len(keys), 2);
            assert_eq!(object::array_get(keys, 0), Some(a));
            let hits = probe::take();
            for hook in [
                "module define_own_property",
                "module delete",
                "module own_property_keys",
            ] {
                assert!(hits.contains(&hook), "{hook} not in {hits:?}");
            }
            assert_eq!(
                property::get(cx, ns, a).unwrap(),
                int32_bits(1),
                "unchanged"
            );
            rt.heap.drop_global(hn);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// CX-VM1, CX-VM2, VF-FORIN with a real namespace: the integer-index paths, the `[[Set]]`
/// walk through a namespace prototype (§10.1.9.2 step 1.b: parent.[[Set]] answers false, so
/// nothing is stored on the receiver) and `for-in` reach the hooks. A namespace that becomes
/// a prototype sets `shape::proto_index_keys`.
#[test]
fn index_paths_set_walk_and_for_in_reach_the_namespace() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt: &Runtime = &iso.rt;
        let ns = namespace(rt, &[("0", int32_bits(7)), ("k", int32_bits(8))]);
        let hn = rt.heap.new_global(val(ns));
        probe::take();
        assert_eq!(property::get_index(cx, ns, 0).unwrap(), int32_bits(7));
        assert!(probe::take().contains(&"module get"));
        assert!(property::has_index(cx, ns, 0));
        assert!(!property::has_index(cx, ns, 1));
        assert!(probe::take().contains(&"module own_attrs"));
        assert!(!property::delete_index(cx, ns, 0, false).unwrap());
        assert!(property::delete_index(cx, ns, 1, false).unwrap());
        assert!(probe::take().contains(&"module delete"));
        assert!(!shape::proto_index_keys(rt));
        let child = object::new_object(rt, ns);
        let hc = rt.heap.new_global(val(child));
        assert!(shape::proto_index_keys(rt), "a namespace as a prototype");
        assert_eq!(property::get_index(cx, child, 0).unwrap(), int32_bits(7));
        let k = crate::atoms::intern_str(rt, "k");
        let y = crate::atoms::intern_str(rt, "y");
        probe::take();
        assert!(!property::set_with_receiver(cx, child, y, int32_bits(1), child).unwrap());
        assert!(probe::take().contains(&"module set"));
        assert!(
            props::own_get(child, y).is_none(),
            "nothing stored on the receiver"
        );
        let keys = property::for_in_keys(cx, ns).unwrap();
        assert_eq!(object::array_len(keys), 2);
        assert_eq!(object::array_get(keys, 1), Some(k));
        assert!(probe::take().contains(&"module own_property_keys"));
        let keys = property::for_in_keys(cx, child).unwrap();
        assert_eq!(object::array_len(keys), 2, "the prototype's keys");
        for h in [hn, hc] {
            rt.heap.drop_global(h);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// VF-TDZ with a real namespace: `[[Get]]` and `[[GetOwnProperty]]` of a binding in its TDZ
/// throw a ReferenceError (§10.4.6.8, §10.4.6.5), and the builtin ABI's HasOwnProperty and
/// `for-in` pass the throw on; `[[HasProperty]]` does not read the binding (§10.4.6.7).
/// `get_data_from` runs no JS and asks no hook.
#[test]
fn a_binding_in_its_tdz_throws_through_every_path() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ns = namespace(rt, &[("x", HOLE)]);
            let hn = rt.heap.new_global(val(ns));
            let x = crate::atoms::intern_str(rt, "x");
            let tdz = "ReferenceError: Cannot access 'x' before initialization";
            assert!(property::get(cx, ns, x).is_err());
            assert_eq!(take_error(cx), tdz, "[[Get]]");
            assert!(property::get_own_property(cx, ns, x).is_err());
            assert_eq!(take_error(cx), tdz, "[[GetOwnProperty]]");
            {
                let mut abi = crate::Cx::new(cx);
                let o: crate::Local<'_, crate::JsObject> = crate::Local::from_bits(ns);
                assert!(abi.has_own_property(o, "x").is_err());
            }
            assert_eq!(take_error(cx), tdz, "HasOwnProperty (builtin ABI)");
            assert!(property::for_in_keys(cx, ns).is_err());
            assert_eq!(take_error(cx), tdz, "for-in");
            assert!(property::has_property(cx, ns, x), "[[HasProperty]]");
            probe::take();
            assert_eq!(property::get_data_from(cx, ns, x), None);
            assert_eq!(rt.pending(), 0, "get_data_from leaves nothing pending");
            assert!(probe::take().is_empty(), "get_data_from asks no hook");
            rt.heap.drop_global(hn);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// X-CODE1 with a real namespace: an error's "Name: message" header does not ask a namespace
/// hook. A namespace on the ReferenceError chain whose `name` is in its TDZ would otherwise
/// make a new ReferenceError whose header asks the hook again: a stack overflow.
#[test]
fn error_headers_skip_namespace_hooks() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ns = namespace(rt, &[("name", HOLE), ("x", HOLE)]);
            let hn = rt.heap.new_global(val(ns));
            let name = rt.atom(Atom::Name);
            let rep = rt.root(Root::ReferenceErrorPrototype);
            assert!(property::delete(cx, rep, name, true).unwrap());
            assert!(property::set_prototype_of(rt, rep, ns));
            probe::take();
            let x = crate::atoms::intern_str(rt, "x");
            assert!(property::get(cx, ns, x).is_err());
            assert_eq!(
                probe::take(),
                ["module get"],
                "only the spec [[Get]] asks the hook"
            );
            let e = rt.take_pending();
            assert!(object::is_object(e));
            let header = crate::error::error_header(cx, e);
            assert!(
                header.ends_with(": Cannot access 'x' before initialization"),
                "{header}"
            );
            assert_eq!(rt.pending(), 0, "error_header leaves nothing pending");
            assert!(probe::take().is_empty(), "the header asks no hook");
            assert!(property::set_prototype_of(
                rt,
                rep,
                rt.root(Root::ErrorPrototype)
            ));
            rt.heap.drop_global(hn);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A MODULE record survives collections with its fields (the trace function visits them).
#[test]
fn a_module_record_keeps_its_fields_across_collections() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt: &Runtime = &iso.rt;
        let ns = namespace(rt, &[("v", int32_bits(3))]);
        let hn = rt.heap.new_global(val(ns));
        for _ in 0..3 {
            rt.heap.collect();
        }
        let ns = rt.heap.global(hn).to_bits();
        let m = object::rd(object::cell(ns), object::nsf::MODULE);
        assert_eq!(
            object::record_get(m, object::modf::EVALUATION_ERROR),
            HOLE,
            "no evaluation error"
        );
        let v = crate::atoms::intern_str(rt, "v");
        assert_eq!(property::get(cx, ns, v).unwrap(), int32_bits(3));
        rt.heap.drop_global(hn);
        iso.verify_heap().unwrap();
    });
}

/// Code that is not module code has no module: `CallRuntime GetImportMeta` (ABI 5 id 42) and
/// the module-variable opcodes throw the v0 "not implemented" errors, under every dispatch.
#[test]
fn script_code_has_no_module() {
    use ep9_bytecode::{RegisterList, RuntimeId, VerifiedUnit};

    use super::{Asm, run_in, unit};
    fn meta() -> VerifiedUnit {
        let mut s = Asm::script(0);
        s.b.call_runtime(RuntimeId::GetImportMeta, RegisterList::EMPTY, 0)
            .return_();
        unit("", vec![s.finish()])
    }
    fn lda() -> VerifiedUnit {
        let mut s = Asm::script(0);
        s.b.lda_module_variable(1, 0).return_();
        unit("", vec![s.finish()])
    }
    for (make, what) in [
        (meta as fn() -> VerifiedUnit, "GetImportMeta"),
        (lda, "module variables"),
    ] {
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let r = on_big_stack(move || run_in(config(d, stress), make()));
                let e = r.result.expect_err("throws");
                assert_eq!(
                    e.to_string(),
                    format!("test.js:1:1: Error: not implemented in ep9 v0: {what}"),
                    "{} stress={stress}",
                    d.name()
                );
            }
        }
    }
}
