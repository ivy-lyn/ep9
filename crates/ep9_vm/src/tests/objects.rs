//! Object layout tests (H17 v0): word0 shape ids of real objects, in-object capacity, the
//! dictionary-mode triggers (OM-10), prototype mode and the epoch, function `length`/`name`,
//! key order, and CreateDataPropertyOrThrow on the define opcodes. Every test runs with the
//! heap verifier after each collection, and most also under GC stress 1 (a collection
//! before every allocation). Values live in Rust locals (the conservative stack scan keeps
//! them) or in the global object.

use ep9_bytecode::{FeedbackSlotKind, RuntimeId, VerifiedUnit};

use super::{Asm, config, isolate, on_big_stack, unit};
use crate::atoms;
use crate::descriptor::Desc;
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::interp::Cx as VmCx;
use crate::isolate::IsolateConfig;
use crate::kinds;
use crate::object::{self, attr, obj};
use crate::property;
use crate::props;
use crate::runtime::{Root, Runtime};
use crate::shape::{self, MAX_FAST_PROPERTIES, MAX_TRANSITIONS, flag};
use crate::{Cx, Dispatch, Isolate, JsObject, Local, PropertyDescriptor};
use crate::{string, wrapper};

// ---- CreateDataPropertyOrThrow on the bytecode define paths (P1 review: non-extensible and
// non-configurable targets). Compiled JS only defines on fresh literals today; these units
// are hand-built so the opcodes see any object. ------------------------------------------------

/// Globals: `o` = a non-extensible `{}`; `fz` = `{k: 1}` with k {w: false, e: false,
/// c: false}; `arr` = a non-extensible `[0]`; `src` = `{p: 1}`.
fn define_setup(iso: &mut Isolate) {
    let mut cx = Cx::new(&mut iso.cx);
    let cx = &mut cx;
    let g = cx.global();
    let o = cx.new_object();
    cx.prevent_extensions(o).unwrap();
    cx.create_data_property_or_throw(g, "o", o).unwrap();
    let fz = cx.new_object();
    let frozen = PropertyDescriptor::data(Local::int32(1), false, false, false);
    cx.define_property_or_throw(fz, "k", frozen).unwrap();
    cx.create_data_property_or_throw(g, "fz", fz).unwrap();
    let arr = cx.new_array();
    cx.create_data_property_or_throw(arr, "0", Local::int32(0))
        .unwrap();
    cx.prevent_extensions(arr).unwrap();
    cx.create_data_property_or_throw(g, "arr", arr).unwrap();
    let src = cx.new_object();
    cx.create_data_property_or_throw(src, "p", Local::int32(1))
        .unwrap();
    cx.create_data_property_or_throw(g, "src", src).unwrap();
}

fn global_obj<'s>(cx: &mut Cx<'s>, name: &str) -> Local<'s, JsObject> {
    let g = cx.global();
    cx.get(g, name).unwrap().as_object().unwrap()
}

/// Runs `make` after `define_setup` (with and without GC stress) and expects `Ok` (`throws`
/// false) or an uncaught TypeError; then `check` looks at the globals.
fn run_define(make: fn() -> VerifiedUnit, throws: bool, check: fn(&mut Cx<'_>)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            define_setup(&mut iso);
            match iso.with_cx_checkpoint(|cx| cx.run_script(make(), "define.js").map(|_| ())) {
                Ok(_) => assert!(!throws, "stress={stress}: expected a TypeError"),
                Err(e) => assert!(
                    throws && e.constructor.as_deref() == Some("TypeError"),
                    "stress={stress}: {}",
                    e.message
                ),
            }
            {
                let mut cx = Cx::new(&mut iso.cx);
                check(&mut cx);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// `DefineNamedOwnProperty(<global target>, key)` with the value 7.
fn define_named(target: &str, key: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let r = s.b.new_register();
    s.lda_global(target);
    s.b.star(r);
    let slot = s.b.feedback_slot(FeedbackSlotKind::DefineNamedOwn);
    let k = s.b.string_constant(key);
    s.b.lda_smi(7).define_named_own_property(r, k, slot);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// The key register of a keyed define.
#[derive(Clone, Copy)]
enum Key {
    Str(&'static str),
    Index(i32),
}

/// `DefineKeyedOwnProperty`, `DefineKeyedOwnPropertyInLiteral` or `StaInArrayLiteral` on a
/// global target with the value 7.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyedOp {
    Define,
    DefineInLiteral,
    ArrayLiteral,
}

fn define_keyed(op: KeyedOp, target: &str, key: Key) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (r, k) = (s.b.new_register(), s.b.new_register());
    s.lda_global(target);
    s.b.star(r);
    match key {
        Key::Str(t) => {
            s.lda_str(t);
        }
        Key::Index(i) => {
            s.b.lda_smi(i);
        }
    }
    s.b.star(k);
    s.b.lda_smi(7);
    match op {
        KeyedOp::Define => {
            let slot = s.b.feedback_slot(FeedbackSlotKind::DefineKeyedOwn);
            s.b.define_keyed_own_property(r, k, 0, slot);
        }
        KeyedOp::DefineInLiteral => {
            let slot = s.b.feedback_slot(FeedbackSlotKind::DefineKeyedOwn);
            s.b.define_keyed_own_property_in_literal(r, k, 0, slot);
        }
        KeyedOp::ArrayLiteral => {
            let slot = s.b.feedback_slot(FeedbackSlotKind::StoreInArrayLiteral);
            s.b.sta_in_array_literal(r, k, slot);
        }
    }
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

fn no_own(cx: &mut Cx<'_>, target: &str, key: &str) {
    let o = global_obj(cx, target);
    assert!(
        cx.get_own_property(o, key).unwrap().is_none(),
        "{target}.{key} must not exist"
    );
    assert!(!cx.is_extensible(o).unwrap());
}

fn fz_k_unchanged(cx: &mut Cx<'_>) {
    let fz = global_obj(cx, "fz");
    let d = cx.get_own_property(fz, "k").unwrap().unwrap();
    assert_eq!(d.value.unwrap().as_int32(), Some(1));
    assert_eq!(
        (d.writable, d.enumerable, d.configurable),
        (Some(false), Some(false), Some(false))
    );
}

#[test]
fn define_named_own_property_on_a_non_extensible_object_throws() {
    run_define(|| define_named("o", "x"), true, |cx| no_own(cx, "o", "x"));
}

#[test]
fn define_named_own_property_over_a_frozen_property_throws() {
    run_define(|| define_named("fz", "k"), true, fz_k_unchanged);
}

#[test]
fn define_named_own_property_on_an_extensible_object_still_works() {
    run_define(
        || define_named("src", "q"),
        false,
        |cx| {
            let src = global_obj(cx, "src");
            let d = cx.get_own_property(src, "q").unwrap().unwrap();
            assert_eq!(d.value.unwrap().as_int32(), Some(7));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(true), Some(true), Some(true))
            );
        },
    );
}

#[test]
fn keyed_defines_on_a_non_extensible_object_throw() {
    run_define(
        || define_keyed(KeyedOp::Define, "o", Key::Str("y")),
        true,
        |cx| no_own(cx, "o", "y"),
    );
    run_define(
        || define_keyed(KeyedOp::DefineInLiteral, "o", Key::Str("z")),
        true,
        |cx| no_own(cx, "o", "z"),
    );
    run_define(
        || define_keyed(KeyedOp::Define, "fz", Key::Str("k")),
        true,
        fz_k_unchanged,
    );
}

#[test]
fn array_defines_of_a_new_index_on_a_non_extensible_array_throw() {
    for op in [KeyedOp::ArrayLiteral, KeyedOp::Define] {
        let make: fn() -> VerifiedUnit = if op == KeyedOp::ArrayLiteral {
            || define_keyed(KeyedOp::ArrayLiteral, "arr", Key::Index(5))
        } else {
            || define_keyed(KeyedOp::Define, "arr", Key::Index(3))
        };
        run_define(make, true, |cx| {
            let arr = global_obj(cx, "arr");
            assert!(cx.get_own_property(arr, "3").unwrap().is_none());
            assert!(cx.get_own_property(arr, "5").unwrap().is_none());
            let len = cx.get(arr, "length").unwrap();
            assert_eq!(len.as_number(), Some(1.0), "length stays 1");
        });
    }
    // An existing element stays definable ({w, e, c} data property).
    run_define(
        || define_keyed(KeyedOp::ArrayLiteral, "arr", Key::Index(0)),
        false,
        |cx| {
            let arr = global_obj(cx, "arr");
            let v = cx.get(arr, "0").unwrap();
            assert_eq!(v.as_int32(), Some(7));
        },
    );
}

#[test]
fn copy_data_properties_into_a_non_extensible_target_throws() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let a = s.b.new_register_list(2);
        s.lda_global("o");
        s.b.star(a.get(0));
        s.lda_global("src");
        s.b.star(a.get(1));
        s.b.call_runtime(RuntimeId::CopyDataProperties, a, 2);
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    run_define(make, true, |cx| no_own(cx, "o", "p"));
}

// ---- helpers for the layout tests --------------------------------------------------------------

/// The isolate config of the layout tests: GC stress 1 when `stress`, the heap verifier after
/// every collection either way.
fn verified(stress: bool) -> IsolateConfig {
    let mut c = config(Dispatch::DEFAULT, stress);
    c.heap.verify_after_gc = true;
    c
}

/// Runs `f` with the VM context and runtime of a new isolate, without and with GC stress, on
/// a big stack; collects and verifies the heap after `f`.
fn each(f: fn(&mut VmCx, &Runtime)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(verified(stress));
            {
                let iso = &mut iso;
                f(&mut iso.cx, &iso.rt);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// The property key `name` (an atom).
fn k(rt: &Runtime, name: &str) -> u64 {
    atoms::intern_str(rt, name)
}

fn int(i: i32) -> u64 {
    int32_bits(i)
}

/// The shape id in word0 of the object `o`.
fn word0_shape_id(o: u64) -> u32 {
    // SAFETY: `o` is a live object cell (the callers keep it in a Rust local or a global).
    unsafe { object::cell(o).header() }.shape_id().get()
}

/// Defines (or redefines) the own data property `name` = `v` with `attrs` (raw storage).
fn put(rt: &Runtime, o: u64, name: &str, v: u64, attrs: i32) {
    props::own_define(rt, o, k(rt, name), v, attrs);
}

/// The value of the own property `name` of `o`, if it has one (raw storage).
fn own(rt: &Runtime, o: u64, name: &str) -> Option<u64> {
    props::own_get(o, k(rt, name)).map(|(v, _)| v)
}

/// The texts of the keys in the array `a`.
fn key_texts(a: u64) -> Vec<String> {
    (0..object::array_len(a))
        .map(|i| string::str_view(object::array_get(a, i).unwrap()).to_string_lossy())
        .collect()
}

/// `[[OwnPropertyKeys]]` of `o` as texts (no symbols exist yet).
fn own_keys(cx: &mut VmCx, o: u64) -> Vec<String> {
    key_texts(property::own_property_keys(cx, o))
}

/// The keys a `for-in` over `o` visits, as texts.
fn for_in(cx: &mut VmCx, o: u64) -> Vec<String> {
    key_texts(property::for_in_keys(cx, o).unwrap())
}

/// A plain object with prototype `proto` whose root shape no other test object uses: the
/// prototype is new, so the transition tree under it is empty.
fn fresh_object(rt: &Runtime) -> (u64, u64) {
    let proto = object::new_plain_object(rt);
    let o = object::new_object(rt, proto);
    (o, proto)
}

// ---- word0 shape ids, capacity ------------------------------------------------------------------

/// Globals `x`, `y` = `{a: 1, b: 2}` (two literal sites), `z` = `{b: 2, a: 1}`, `e` = `{}`.
fn literals() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (ka, kb) = (s.b.string_constant("a"), s.b.string_constant("b"));
    let (k1, k2) = (s.b.number_constant(1.0), s.b.number_constant(2.0));
    for (name, props) in [
        ("x", [(ka, k1), (kb, k2)]),
        ("y", [(ka, k1), (kb, k2)]),
        ("z", [(kb, k2), (ka, k1)]),
    ] {
        let bp =
            s.b.object_boilerplate(props.iter().map(|&(k, v)| (k, Some(v))).collect());
        s.b.create_object_literal(bp, s.lit, 0);
        s.sta_global(name);
    }
    s.b.create_empty_object_literal();
    s.sta_global("e");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

#[test]
fn two_literals_with_the_same_keys_share_a_shape_id() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(verified(stress));
            iso.with_cx_checkpoint(|cx| cx.run_script(literals(), "literals.js").map(|_| ()))
                .unwrap();
            iso.collect_garbage();
            let rt = &*iso.rt;
            let g = |name: &str| own(rt, rt.global(), name).unwrap();
            let (x, y, z, e) = (g("x"), g("y"), g("z"), g("e"));
            assert_ne!(word0_shape_id(x), 0);
            assert_eq!(word0_shape_id(x), word0_shape_id(y), "{{a, b}} twice");
            assert_ne!(word0_shape_id(x), word0_shape_id(z), "{{b, a}} differs");
            let (sx, sz, se) = (shape::shape_of(x), shape::shape_of(z), shape::shape_of(e));
            // A literal gets one in-object slot per named key; `{}` gets 4 (H17 v0).
            assert_eq!((shape::count(sx), shape::capacity(sx)), (2, 2));
            assert_eq!((shape::count(sz), shape::capacity(sz)), (2, 2));
            assert_eq!((shape::count(se), shape::capacity(se)), (0, 4));
            assert_eq!(shape::proto(sx), rt.root(Root::ObjectPrototype));
            assert!(!shape::is_dictionary(sx) && shape::is_extensible(sx));
            assert_eq!(own(rt, x, "b"), Some(int(2)));
            assert_eq!(own(rt, z, "a"), Some(int(1)));
            for s in [sx, sz, se] {
                shape::check(rt, s).unwrap();
            }
            iso.verify_heap().unwrap();
        });
    }
}

#[test]
fn new_objects_get_their_in_object_capacity() {
    each(|_, rt| {
        // `{}`: 4 slots; the fifth property goes to the out-of-object SLOTS store.
        let o = object::new_plain_object(rt);
        assert_eq!(shape::capacity(shape::shape_of(o)), 4);
        for (i, name) in ["a", "b", "c", "d"].into_iter().enumerate() {
            put(rt, o, name, int(i as i32), attr::DEFAULT);
        }
        assert_eq!(object::rd(object::cell(o), obj::SLOTS), UNDEFINED);
        put(rt, o, "e", int(4), attr::DEFAULT);
        assert_ne!(object::rd(object::cell(o), obj::SLOTS), UNDEFINED);
        assert_eq!(shape::count(shape::shape_of(o)), 5);
        for (i, name) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            assert_eq!(own(rt, o, name), Some(int(i as i32)));
        }
        // `new F` (an object whose prototype is not %Object.prototype%): 8 slots.
        let (n, _) = fresh_object(rt);
        assert_eq!(shape::capacity(shape::shape_of(n)), 8);
        // Functions: 2 slots, holding `length` and `name`.
        let f = crate::realm::new_host_function(rt, super::print, "f", 1, false);
        let sf = shape::shape_of(f);
        assert_eq!((shape::capacity(sf), shape::count(sf)), (2, 2));
        assert_eq!(object::rd(object::cell(f), obj::SLOTS), UNDEFINED);
        // Arrays and errors.
        assert_eq!(
            shape::capacity(shape::shape_of(object::new_array(rt, 0))),
            0
        );
        let err = object::new_error_object(rt, rt.root(Root::ErrorPrototype));
        assert_eq!(shape::capacity(shape::shape_of(err)), 2);
    });
}

#[test]
fn deleting_the_last_property_goes_back_to_the_parent_shape() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        put(rt, o, "a", int(1), attr::DEFAULT);
        let sa = shape::shape_of(o);
        put(rt, o, "b", int(2), attr::DEFAULT);
        let sab = shape::shape_of(o);
        assert_eq!(shape::parent(sab), sa);
        assert!(property::delete(cx, o, k(rt, "b"), true).unwrap());
        assert_eq!(shape::shape_of(o), sa, "back to the parent");
        assert!(!shape::is_dictionary(shape::shape_of(o)));
        assert_eq!(own(rt, o, "b"), None);
        assert_eq!(own(rt, o, "a"), Some(int(1)));
        // Adding `b` again takes the same transition.
        put(rt, o, "b", int(3), attr::DEFAULT);
        assert_eq!(shape::shape_of(o), sab);
        assert_eq!(own(rt, o, "b"), Some(int(3)));
        // Back to the root, then empty.
        assert!(property::delete(cx, o, k(rt, "b"), true).unwrap());
        assert!(property::delete(cx, o, k(rt, "a"), true).unwrap());
        assert_eq!(shape::count(shape::shape_of(o)), 0);
        assert_eq!(shape::parent(shape::shape_of(o)), UNDEFINED);
        assert_eq!(own_keys(cx, o), Vec::<String>::new());
    });
}

// ---- OM-10: one test per dictionary-mode trigger ------------------------------------------------

#[test]
fn om10_a_delete_that_is_not_the_last_makes_a_dictionary() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
            put(rt, o, name, int(i as i32), attr::DEFAULT);
        }
        let before = shape::shape_of(o);
        assert!(property::delete(cx, o, k(rt, "b"), true).unwrap());
        let s = shape::shape_of(o);
        assert_ne!(s, before);
        assert!(shape::is_dictionary(s) && shape::is_extensible(s));
        assert_eq!(shape::proto(s), rt.root(Root::ObjectPrototype));
        assert_eq!(own_keys(cx, o), ["a", "c"]);
        assert_eq!(
            (own(rt, o, "a"), own(rt, o, "c")),
            (Some(int(0)), Some(int(2)))
        );
        // Dictionary objects take new properties at the end of the order.
        put(rt, o, "b", int(9), attr::DEFAULT);
        assert_eq!(own_keys(cx, o), ["a", "c", "b"]);
        assert_eq!(for_in(cx, o), ["a", "c", "b"]);
        shape::check(rt, s).unwrap();
    });
}

#[test]
fn om10_an_attribute_change_makes_a_dictionary() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        put(rt, o, "a", int(1), attr::DEFAULT);
        put(rt, o, "b", int(2), attr::DEFAULT);
        // Same attributes: a value store, the shape stays.
        let s = shape::shape_of(o);
        put(rt, o, "a", int(5), attr::DEFAULT);
        assert_eq!(shape::shape_of(o), s);
        // {writable: false} through [[DefineOwnProperty]].
        let desc = Desc {
            writable: Some(false),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, o, k(rt, "a"), desc).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(o)));
        let d = property::get_own_property(cx, o, k(rt, "a"))
            .unwrap()
            .unwrap();
        assert_eq!(d.value, Some(int(5)));
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(true), Some(true))
        );
        assert_eq!(own_keys(cx, o), ["a", "b"]);
    });
}

#[test]
fn om10_a_data_to_accessor_switch_makes_a_dictionary() {
    // Accessor values come with P3-ACC; the switch itself is a storage attribute change.
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        put(rt, o, "a", int(1), attr::DEFAULT);
        put(rt, o, "b", int(2), attr::DEFAULT);
        put(rt, o, "a", UNDEFINED, attr::DEFAULT | attr::ACCESSOR);
        assert!(shape::is_dictionary(shape::shape_of(o)));
        assert_eq!(
            props::own_get(o, k(rt, "a")),
            Some((UNDEFINED, attr::DEFAULT | attr::ACCESSOR))
        );
        // And back to data.
        put(rt, o, "a", int(3), attr::DEFAULT);
        assert_eq!(props::own_get(o, k(rt, "a")), Some((int(3), attr::DEFAULT)));
        assert_eq!(own_keys(cx, o), ["a", "b"]);
    });
}

#[test]
fn om10_an_index_key_on_a_non_array_makes_a_dictionary() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        put(rt, o, "b", int(1), attr::DEFAULT);
        property::define(cx, o, k(rt, "2"), int(2)).unwrap();
        assert!(shape::is_dictionary(shape::shape_of(o)));
        put(rt, o, "a", int(3), attr::DEFAULT);
        property::define(cx, o, k(rt, "1"), int(4)).unwrap();
        // Indices ascending, then strings in creation order.
        assert_eq!(own_keys(cx, o), ["1", "2", "b", "a"]);
        assert_eq!(for_in(cx, o), ["1", "2", "b", "a"]);
        // A non-index numeric-looking key stays fast.
        let p = object::new_plain_object(rt);
        put(rt, p, "01", int(1), attr::DEFAULT);
        put(rt, p, "4294967295", int(1), attr::DEFAULT);
        assert!(!shape::is_dictionary(shape::shape_of(p)));
    });
}

#[test]
fn om10_set_prototype_of_with_own_properties_makes_a_dictionary() {
    each(|cx, rt| {
        let other = object::new_plain_object(rt);
        // No own properties: a fast root shape of the new prototype.
        let e = object::new_plain_object(rt);
        assert!(property::set_prototype_of(rt, e, other));
        let se = shape::shape_of(e);
        assert!(!shape::is_dictionary(se));
        assert_eq!((shape::proto(se), shape::count(se)), (other, 0));
        assert_eq!(object::proto_of(e), other);
        // Own properties: dictionary mode.
        let o = object::new_plain_object(rt);
        put(rt, o, "a", int(1), attr::DEFAULT);
        put(rt, o, "b", int(2), attr::DEFAULT);
        assert!(property::set_prototype_of(rt, o, other));
        let so = shape::shape_of(o);
        assert!(shape::is_dictionary(so) && shape::is_extensible(so));
        assert_eq!(shape::proto(so), other);
        assert_eq!(own_keys(cx, o), ["a", "b"]);
        // To null and back.
        assert!(property::set_prototype_of(rt, o, NULL));
        assert_eq!(object::proto_of(o), NULL);
        assert!(property::set_prototype_of(rt, o, other));
        assert_eq!(object::proto_of(o), other);
        // A cycle is refused; %Object.prototype% keeps its null prototype.
        assert!(!property::set_prototype_of(rt, other, o));
        let op = rt.root(Root::ObjectPrototype);
        assert!(!property::set_prototype_of(rt, op, other));
        assert_eq!(object::proto_of(op), NULL);
    });
}

#[test]
fn om10_prevent_extensions_makes_a_non_extensible_dictionary() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        put(rt, o, "a", int(1), attr::DEFAULT);
        let before = shape::shape_of(o);
        assert!(property::prevent_extensions(rt, o));
        let s = shape::shape_of(o);
        assert_ne!(s, before);
        assert!(shape::is_dictionary(s) && !shape::is_extensible(s));
        assert!(!property::is_extensible(o));
        assert_eq!(own(rt, o, "a"), Some(int(1)));
        // Existing properties stay writable and deletable; new ones are refused.
        property::set(cx, o, k(rt, "a"), int(2), true).unwrap();
        assert_eq!(own(rt, o, "a"), Some(int(2)));
        assert!(property::set(cx, o, k(rt, "b"), int(3), true).is_err());
        rt.take_pending();
        assert_eq!(own(rt, o, "b"), None);
        assert!(property::delete(cx, o, k(rt, "a"), true).unwrap());
        assert_eq!(own_keys(cx, o), Vec::<String>::new());
        assert!(
            property::prevent_extensions(rt, o),
            "a second time is a no-op"
        );
        assert_eq!(shape::shape_of(o), s);
    });
}

#[test]
fn prevent_extensions_through_the_abi_changes_the_word0_shape() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(verified(stress));
            {
                let mut cx = Cx::new(&mut iso.cx);
                let cx = &mut cx;
                for with_property in [false, true] {
                    let o = cx.new_object();
                    if with_property {
                        cx.create_data_property_or_throw(o, "a", Local::int32(1))
                            .unwrap();
                    }
                    let id = word0_shape_id(o.bits());
                    assert!(cx.is_extensible(o).unwrap());
                    assert!(cx.prevent_extensions(o).unwrap());
                    assert_ne!(word0_shape_id(o.bits()), id, "a new shape");
                    assert!(!cx.is_extensible(o).unwrap());
                    assert!(!shape::is_extensible(shape::shape_of(o.bits())));
                    assert!(!cx.create_data_property(o, "b", Local::int32(2)).unwrap());
                }
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

#[test]
fn om10_more_than_1020_properties_make_a_dictionary() {
    each(|cx, rt| {
        let (o, _) = fresh_object(rt);
        for i in 0..MAX_FAST_PROPERTIES {
            put(rt, o, &format!("p{i}"), int(i as i32), attr::DEFAULT);
        }
        let s = shape::shape_of(o);
        assert!(!shape::is_dictionary(s));
        assert_eq!(shape::count(s), MAX_FAST_PROPERTIES);
        put(rt, o, "last", int(-1), attr::DEFAULT);
        assert!(shape::is_dictionary(shape::shape_of(o)));
        let keys = own_keys(cx, o);
        assert_eq!(keys.len(), MAX_FAST_PROPERTIES + 1);
        assert_eq!((keys[0].as_str(), keys[1019].as_str()), ("p0", "p1019"));
        assert_eq!(keys[MAX_FAST_PROPERTIES], "last");
        assert_eq!(own(rt, o, "p777"), Some(int(777)));
    });
}

#[test]
fn om10_more_than_64_transitions_make_a_dictionary() {
    each(|_, rt| {
        let proto = object::new_plain_object(rt);
        // The objects stay alive in `all` (a Rust local), so their shapes (weak transitions)
        // stay too.
        let all = object::new_array(rt, 0);
        for i in 0..=MAX_TRANSITIONS {
            let o = object::new_object(rt, proto);
            object::array_set(rt, all, i as u32, o);
            put(rt, o, &format!("t{i}"), int(i as i32), attr::DEFAULT);
        }
        let first = object::array_get(all, 0).unwrap();
        let root = shape::parent(shape::shape_of(first));
        assert_eq!(shape::transition_count(root), MAX_TRANSITIONS);
        for i in 0..MAX_TRANSITIONS {
            let o = object::array_get(all, i as u32).unwrap();
            let s = shape::shape_of(o);
            assert!(!shape::is_dictionary(s), "object {i}");
            assert_eq!(shape::parent(s), root);
        }
        let last = object::array_get(all, MAX_TRANSITIONS as u32).unwrap();
        assert!(shape::is_dictionary(shape::shape_of(last)));
        assert_eq!(object::proto_of(last), proto);
        assert_eq!(own(rt, last, "t64"), Some(int(64)));
    });
}

// ---- key order ----------------------------------------------------------------------------------

#[test]
fn object_keys_and_for_in_order_survive_layout_changes() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        for name in ["b", "x", "a", "y", "c"] {
            property::define(cx, o, k(rt, name), int(1)).unwrap();
        }
        // Hidden (non-enumerable) `y`: listed by [[OwnPropertyKeys]], skipped by for-in.
        let hidden = Desc {
            enumerable: Some(false),
            ..Desc::default()
        };
        let want = ["b", "x", "a", "y", "c"];
        assert_eq!(own_keys(cx, o), want);
        assert_eq!(for_in(cx, o), want);
        assert!(property::define_own_property(cx, o, k(rt, "y"), hidden).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(o)));
        assert_eq!(own_keys(cx, o), want);
        assert_eq!(for_in(cx, o), ["b", "x", "a", "c"]);
        // Inherited enumerable keys come after own ones; shadowed ones are skipped.
        let child = object::new_object(rt, o);
        property::define(cx, child, k(rt, "a"), int(2)).unwrap();
        property::define(cx, child, k(rt, "z"), int(2)).unwrap();
        assert_eq!(for_in(cx, child), ["a", "z", "b", "x", "c"]);
        // A fast object with the same keys in another order lists them in its own order.
        let p = object::new_plain_object(rt);
        for name in ["c", "a", "b"] {
            property::define(cx, p, k(rt, name), int(1)).unwrap();
        }
        assert!(!shape::is_dictionary(shape::shape_of(p)));
        assert_eq!(own_keys(cx, p), ["c", "a", "b"]);
        assert_eq!(for_in(cx, p), ["c", "a", "b"]);
    });
}

#[test]
fn define_paths_store_atom_keys() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        // A key string that is not an atom (as from a computed key) is interned on define.
        let key = string::new_string(rt, "computed");
        assert!(!string::is_atom(key));
        property::define(cx, o, key, int(1)).unwrap();
        let (stored, v, a) = object::own_entry(o, 0);
        assert!(string::is_atom(stored));
        assert_eq!((stored, v, a), (k(rt, "computed"), int(1), attr::DEFAULT));
        assert_eq!(own(rt, o, "computed"), Some(int(1)));
        // Lookups by the atom compare by bits.
        assert_eq!(property::get(cx, o, k(rt, "computed")).unwrap(), int(1));
    });
}

// ---- functions: `length` and `name`, the lazy `prototype` ------------------------------------

/// Checks `{value, writable: false, enumerable: false, configurable: true}`.
fn assert_fn_prop(cx: &mut VmCx, f: u64, name: &str, value: u64) {
    let key = k(cx.rt(), name);
    let d = property::get_own_property(cx, f, key).unwrap().unwrap();
    assert_eq!(d.value, Some(value), "{name}");
    assert_eq!(
        (d.writable, d.enumerable, d.configurable),
        (Some(false), Some(false), Some(true)),
        "{name}"
    );
}

#[test]
fn function_length_and_name_are_own_data_properties() {
    each(|cx, rt| {
        let f = crate::realm::new_host_function(rt, super::print, "probe", 3, true);
        assert_fn_prop(cx, f, "length", int(3));
        assert_fn_prop(cx, f, "name", k(rt, "probe"));
        // Two functions share the function shape.
        let g = crate::realm::new_host_function(rt, super::print, "other", 0, false);
        assert_eq!(word0_shape_id(f), word0_shape_id(g));
        assert_fn_prop(cx, g, "length", int(0));
        // `length` and `name` first, then the lazy `prototype` (constructors).
        assert_eq!(own_keys(cx, f), ["length", "name", "prototype"]);
        assert_eq!(own_keys(cx, g), ["length", "name"]);
        // Not writable: a store fails (strict: TypeError).
        assert!(property::set(cx, f, k(rt, "length"), int(9), true).is_err());
        rt.take_pending();
        assert_fn_prop(cx, f, "length", int(3));
        // Configurable: defineProperty may change it (dictionary mode).
        let desc = Desc {
            value: Some(int(7)),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, f, k(rt, "length"), desc).unwrap());
        assert_fn_prop(cx, f, "length", int(7));
    });
}

#[test]
fn deleting_function_name_removes_the_own_property() {
    each(|cx, rt| {
        let f = crate::realm::new_host_function(rt, super::print, "probe", 1, false);
        let name = k(rt, "name");
        assert!(property::delete(cx, f, name, true).unwrap());
        assert!(property::get_own_property(cx, f, name).unwrap().is_none());
        // `name` is now inherited (%Function.prototype%.name is "" in the spec; v0 may have
        // none), and re-adding it puts it last.
        assert_eq!(own_keys(cx, f), ["length"]);
        property::define(cx, f, name, k(rt, "again")).unwrap();
        assert_eq!(own_keys(cx, f), ["length", "name"]);
        let d = property::get_own_property(cx, f, name).unwrap().unwrap();
        assert_eq!(d.enumerable, Some(true), "a new data property");
        // `length` delete too.
        assert!(property::delete(cx, f, k(rt, "length"), true).unwrap());
        assert!(
            property::get_own_property(cx, f, k(rt, "length"))
                .unwrap()
                .is_none()
        );
    });
}

#[test]
fn a_lazy_prototype_keeps_its_place_in_the_key_order() {
    each(|cx, rt| {
        // Made by a get after another key was added: still right after `length`, `name`.
        let f = crate::realm::new_host_function(rt, super::print, "F", 0, true);
        property::define(cx, f, k(rt, "z"), int(1)).unwrap();
        assert_eq!(own_keys(cx, f), ["length", "name", "prototype", "z"]);
        let p = property::get(cx, f, k(rt, "prototype")).unwrap();
        assert!(kinds::is_object(p));
        assert_eq!(property::get(cx, p, k(rt, "constructor")).unwrap(), f);
        assert_eq!(own_keys(cx, f), ["length", "name", "prototype", "z"]);
        // Not made yet, `name` deleted: `prototype` still comes after `length`.
        let g = crate::realm::new_host_function(rt, super::print, "G", 0, true);
        assert!(property::delete(cx, g, k(rt, "name"), true).unwrap());
        assert_eq!(own_keys(cx, g), ["length", "prototype"]);
        property::define(cx, g, k(rt, "name"), int(1)).unwrap();
        assert_eq!(own_keys(cx, g), ["length", "prototype", "name"]);
        // {w: true, e: false, c: false}.
        let d = property::get_own_property(cx, g, k(rt, "prototype"))
            .unwrap()
            .unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(true), Some(false), Some(false))
        );
    });
}

// ---- prototype mode and the epoch (OM-8) ----------------------------------------------------

#[test]
fn an_object_used_as_a_prototype_gets_a_unique_shape_and_bumps_the_epoch() {
    each(|cx, rt| {
        let epoch = || rt.shapes.proto_epoch.get();
        let p = object::new_plain_object(rt);
        put(rt, p, "m", int(1), attr::DEFAULT);
        let q = object::new_plain_object(rt);
        put(rt, q, "m", int(1), attr::DEFAULT);
        assert_eq!(shape::shape_of(p), shape::shape_of(q), "shared before");
        assert!(!shape::is_prototype_object(p));
        // `p` becomes a prototype: a unique shape with the same keys.
        let o = object::new_object(rt, p);
        assert!(shape::is_prototype_object(p));
        let sp = shape::shape_of(p);
        assert_ne!(sp, shape::shape_of(q));
        assert_eq!(shape::flags(sp) & flag::UNIQUE, flag::UNIQUE);
        assert_eq!(shape::count(sp), 1);
        assert_eq!(own(rt, p, "m"), Some(int(1)));
        assert_eq!(object::proto_of(o), p);
        assert_eq!(property::get(cx, o, k(rt, "m")).unwrap(), int(1));
        // A second object with prototype `p` shares the root shape of `p`.
        let o2 = object::new_object(rt, p);
        assert_eq!(shape::shape_of(o), shape::shape_of(o2));
        // Changes to `p` bump the epoch; changes to `q` (not a prototype) do not.
        let e0 = epoch();
        put(rt, q, "n", int(2), attr::DEFAULT);
        assert_eq!(epoch(), e0);
        put(rt, p, "n", int(2), attr::DEFAULT);
        assert!(epoch() != e0);
        let sp2 = shape::shape_of(p);
        assert_ne!(sp2, sp);
        assert_eq!(shape::flags(sp2) & flag::UNIQUE, flag::UNIQUE);
        let e1 = epoch();
        assert!(property::delete(cx, p, k(rt, "n"), true).unwrap());
        assert!(epoch() != e1, "a delete bumps it");
        assert_eq!(shape::count(shape::shape_of(p)), 1);
        let e2 = epoch();
        put(rt, p, "m", int(3), attr::DEFAULT);
        assert_eq!(epoch(), e2, "a value store does not");
        assert_eq!(property::get(cx, o2, k(rt, "m")).unwrap(), int(3));
        let e3 = epoch();
        assert!(property::set_prototype_of(rt, p, NULL));
        assert!(epoch() != e3, "SetPrototypeOf bumps it");
        assert_eq!(property::get(cx, o2, k(rt, "m")).unwrap(), int(3));
        let e4 = epoch();
        assert!(property::prevent_extensions(rt, p));
        assert!(epoch() != e4, "PreventExtensions bumps it");
        assert!(!property::set_prototype_of(rt, p, q), "not extensible");
        // Two prototypes with the same keys keep different shapes.
        let p2 = object::new_plain_object(rt);
        put(rt, p2, "m", int(1), attr::DEFAULT);
        let _ = object::new_object(rt, p2);
        let p3 = object::new_plain_object(rt);
        put(rt, p3, "m", int(1), attr::DEFAULT);
        let _ = object::new_object(rt, p3);
        assert_ne!(shape::shape_of(p2), shape::shape_of(p3));
        shape::check(rt, shape::shape_of(p2)).unwrap();
    });
}

// ---- the epoch rule on every mutation path (OM-8; P2 review fixes) ---------------------------
//
// A structural change of a prototype (an own property appears or goes away, attributes,
// `[[Prototype]]`, extensibility, dictionary mode) bumps the epoch; a value store does not.
// P3-IC guards prototype-chain loads with the epoch, so both halves matter.

/// Records each step whose epoch effect breaks the rule, so one run lists every broken path.
struct EpochLog<'r> {
    rt: &'r Runtime,
    /// The case of the steps that follow (a prefix of each message).
    case: String,
    wrong: Vec<String>,
}

impl<'r> EpochLog<'r> {
    fn new(rt: &'r Runtime) -> Self {
        EpochLog {
            rt,
            case: String::new(),
            wrong: Vec::new(),
        }
    }

    /// Runs `f`, which must bump the epoch (`bumps`) or leave it as it is.
    fn step<T>(&mut self, what: &str, bumps: bool, f: impl FnOnce() -> T) -> T {
        let before = self.rt.shapes.proto_epoch.get();
        let r = f();
        let after = self.rt.shapes.proto_epoch.get();
        if (after != before) != bumps {
            let want = if bumps { "a bump" } else { "no bump" };
            let case = &self.case;
            self.wrong.push(format!(
                "{case}: {what}: want {want}, epoch {before} -> {after}"
            ));
        }
        r
    }

    fn check(&mut self, what: &str, ok: bool) {
        if !ok {
            self.wrong.push(format!("{}: {what}", self.case));
        }
    }

    fn done(self) {
        assert!(
            self.wrong.is_empty(),
            "the epoch rule is broken:\n  {}",
            self.wrong.join("\n  ")
        );
    }
}

/// True if every `[[Prototype]]` on the chain of `o` is in prototype mode (H17: a shape's
/// proto is null or a prototype-mode object, so a change to it bumps the epoch).
fn chain_in_prototype_mode(o: u64) -> bool {
    let mut p = object::proto_of(o);
    while p != NULL {
        if !shape::is_prototype_object(p) {
            return false;
        }
        p = object::proto_of(p);
    }
    true
}

/// A plain object with the own property `name` = 1, in fast mode, or (`dict`) in dictionary
/// mode by a delete that is not the last (OM-10).
fn plain_with(cx: &mut VmCx, rt: &Runtime, name: &str, dict: bool) -> u64 {
    let o = object::new_plain_object(rt);
    put(rt, o, "pad", int(0), attr::DEFAULT);
    put(rt, o, name, int(1), attr::DEFAULT);
    if dict {
        assert!(property::delete(cx, o, k(rt, "pad"), true).unwrap());
    }
    assert_eq!(shape::is_dictionary(shape::shape_of(o)), dict);
    o
}

#[test]
fn set_prototype_of_a_prototype_puts_the_new_prototype_in_prototype_mode() {
    each(|cx, rt| {
        let mut log = EpochLog::new(rt);
        for (p_dict, q_dict) in [(false, false), (false, true), (true, false), (true, true)] {
            log.case = format!("p dictionary {p_dict}, q dictionary {q_dict}");
            // `p` is a prototype (unique shape); `q` is a new object, not one yet.
            let p = plain_with(cx, rt, "m", p_dict);
            let child = object::new_object(rt, p);
            assert!(shape::is_prototype_object(p));
            let q = plain_with(cx, rt, "n", q_dict);
            assert!(!shape::is_prototype_object(q));
            log.step("SetPrototypeOf(p, q)", true, || {
                assert!(property::set_prototype_of(rt, p, q));
            });
            log.check("q is not in prototype mode", shape::is_prototype_object(q));
            log.check(
                "a proto on the chain is outside prototype mode",
                chain_in_prototype_mode(child),
            );
            let y = k(rt, "y");
            assert_eq!(property::get(cx, child, y).unwrap(), UNDEFINED);
            log.step("add q.y", true, || {
                property::set(cx, q, y, int(42), true).unwrap();
            });
            assert_eq!(property::get(cx, child, y).unwrap(), int(42));
            log.step("store q.y", false, || {
                property::set(cx, q, y, int(43), true).unwrap();
            });
            assert_eq!(property::get(cx, child, y).unwrap(), int(43));
            log.step("delete q.y", true, || {
                assert!(property::delete(cx, q, y, true).unwrap());
            });
            assert_eq!(property::get(cx, child, y).unwrap(), UNDEFINED);
            // The same through a prototype whose `[[Prototype]]` is set again: `r` is new.
            let r = plain_with(cx, rt, "o", q_dict);
            log.step("SetPrototypeOf(q, r)", true, || {
                assert!(property::set_prototype_of(rt, q, r));
            });
            log.check("r is not in prototype mode", chain_in_prototype_mode(child));
            log.step("add r.y", true, || {
                put(rt, r, "y", int(7), attr::DEFAULT);
            });
            assert_eq!(property::get(cx, child, y).unwrap(), int(7));
            for o in [p, q, r] {
                shape::check(rt, shape::shape_of(o)).unwrap();
            }
        }
        log.done();
    });
}

/// An array `[1]` with the own named properties `a` (and `pad` unless `dict`), in fast mode
/// or (`dict`) in dictionary mode (OM-10).
fn array_with(cx: &mut VmCx, rt: &Runtime, dict: bool) -> u64 {
    let a = object::new_array(rt, 0);
    property::set_keyed(cx, a, int(0), int(1), true).unwrap();
    put(rt, a, "pad", int(0), attr::DEFAULT);
    put(rt, a, "a", int(0), attr::DEFAULT);
    if dict {
        assert!(property::delete(cx, a, k(rt, "pad"), true).unwrap());
    }
    assert_eq!(shape::is_dictionary(shape::shape_of(a)), dict);
    a
}

#[test]
fn dense_element_changes_of_an_array_prototype_bump_the_epoch() {
    each(|cx, rt| {
        let mut log = EpochLog::new(rt);
        for dict in [false, true] {
            log.case = format!("dictionary {dict}");
            let a = array_with(cx, rt, dict);
            // Not a prototype yet: no element change bumps the epoch.
            log.step("append to a non-prototype", false, || {
                assert!(object::array_set(rt, a, 1, int(5)));
            });
            log.step("truncate a non-prototype", false, || {
                assert!(object::array_set_length(rt, a, 1));
            });
            let child = object::new_object(rt, a);
            assert!(shape::is_prototype_object(a));
            let len = k(rt, "length");
            let at = |cx: &mut VmCx, i: i32| property::get_keyed(cx, child, int(i)).unwrap();
            assert_eq!(at(cx, 1), UNDEFINED);
            // Adds.
            log.step("[[Set]] adds element 1", true, || {
                property::set_keyed(cx, a, int(1), int(42), true).unwrap();
            });
            assert_eq!(at(cx, 1), int(42));
            log.step("CreateDataProperty adds element 2", true, || {
                property::define_keyed(cx, a, int(2), int(2)).unwrap();
            });
            log.step("DefineOwnProperty adds element 3", true, || {
                let d = Desc::data(int(3), attr::DEFAULT);
                assert!(property::define_own_property(cx, a, k(rt, "3"), d).unwrap());
            });
            log.step("a raw append (push, literals)", true, || {
                assert!(object::array_set(rt, a, 4, int(4)));
            });
            assert_eq!(at(cx, 4), int(4));
            // Value stores.
            log.step("[[Set]] stores element 1", false, || {
                property::set_keyed(cx, a, int(1), int(43), true).unwrap();
            });
            log.step("CreateDataProperty stores element 1", false, || {
                property::define_keyed(cx, a, int(1), int(44)).unwrap();
            });
            log.step("DefineOwnProperty stores element 3", false, || {
                let d = Desc {
                    value: Some(int(33)),
                    ..Desc::default()
                };
                assert!(property::define_own_property(cx, a, k(rt, "3"), d).unwrap());
            });
            assert_eq!((at(cx, 1), at(cx, 3)), (int(44), int(33)));
            // Deletes.
            log.step("delete element 2", true, || {
                assert!(property::delete(cx, a, int(2), true).unwrap());
            });
            assert_eq!(at(cx, 2), UNDEFINED);
            log.step("delete a hole", false, || {
                assert!(property::delete(cx, a, int(2), true).unwrap());
            });
            log.step("a hole stored over a hole", false, || {
                assert!(object::array_set(rt, a, 2, crate::frame::HOLE));
            });
            // `length`: growth and a cut over holes only change its value.
            log.step("length grows", false, || {
                property::set(cx, a, len, int(10), true).unwrap();
            });
            log.step("length cut over holes", false, || {
                property::set(cx, a, len, int(6), true).unwrap();
            });
            log.step("length cut removes elements", true, || {
                property::set(cx, a, len, int(1), true).unwrap();
            });
            assert_eq!(
                (at(cx, 0), at(cx, 1), at(cx, 4)),
                (int(1), UNDEFINED, UNDEFINED)
            );
            log.step("DefineOwnProperty length cut", true, || {
                let d = Desc {
                    value: Some(int(0)),
                    ..Desc::default()
                };
                assert!(property::define_own_property(cx, a, len, d).unwrap());
            });
            assert_eq!(at(cx, 0), UNDEFINED);
            log.step("a raw cut (no element left)", false, || {
                assert!(object::array_set_length(rt, a, 0));
            });
            // Named properties of the array prototype follow the same rule.
            log.step("add a.b", true, || {
                put(rt, a, "b", int(1), attr::DEFAULT);
            });
            log.step("store a.b", false, || {
                put(rt, a, "b", int(2), attr::DEFAULT);
            });
            shape::check(rt, shape::shape_of(a)).unwrap();
        }
        log.done();
    });
}

// ---- [[Set]] of a missing array element and index keys on prototypes -----------------------
//
// `shape::proto_index_keys` turns on the first time a prototype-mode object has an
// array-index own property. Until then `[[Set]]` adds a missing array element without the
// walk; after it, the walk finds an inherited setter, getter or read-only index (§10.1.9.2).

#[test]
fn proto_index_keys_stays_off_without_index_keys_on_prototypes() {
    each(|cx, rt| {
        assert!(!shape::proto_index_keys(rt), "after realm init");
        let a = object::new_array(rt, 0);
        property::set_keyed(cx, a, int(0), int(1), true).unwrap();
        let o = object::new_plain_object(rt);
        property::define(cx, o, k(rt, "0"), int(1)).unwrap();
        assert!(!shape::proto_index_keys(rt), "index keys of non-prototypes");
        let p = object::new_plain_object(rt);
        let _ = object::new_object(rt, p);
        put(rt, p, "x", int(1), attr::DEFAULT);
        let e = object::new_array(rt, 0);
        let _ = object::new_object(rt, e);
        assert!(!shape::proto_index_keys(rt), "a named key; an empty array");
    });
}

#[test]
fn an_index_key_added_to_a_prototype_sets_proto_index_keys() {
    each(|_, rt| {
        let p = object::new_plain_object(rt);
        let _ = object::new_object(rt, p);
        put(rt, p, "3", int(1), attr::DEFAULT);
        assert!(shape::proto_index_keys(rt), "a stored index key");
    });
    each(|cx, rt| {
        let p = object::new_array(rt, 0);
        let _ = object::new_object(rt, p);
        property::set_keyed(cx, p, int(0), int(1), true).unwrap();
        assert!(shape::proto_index_keys(rt), "an element");
    });
}

/// `arr[i] = 7` in sloppy and strict code; true if the element was added. A strict failure
/// must be a TypeError.
fn set_adds(cx: &mut VmCx, rt: &Runtime, arr: u64, i: i32) -> bool {
    property::set_keyed(cx, arr, int(i), int(7), false).unwrap();
    let added = object::array_get(arr, i as u32) == Some(int(7));
    if !added {
        assert!(property::set_keyed(cx, arr, int(i), int(7), true).is_err());
        rt.take_pending();
    }
    added
}

#[test]
fn an_object_with_index_keys_that_becomes_a_prototype_blocks_array_appends() {
    // A read-only index 0, stored before the object is a prototype.
    each(|cx, rt| {
        let p = object::new_plain_object(rt);
        let ro = Desc::data(int(1), 0);
        assert!(property::define_own_property(cx, p, k(rt, "0"), ro).unwrap());
        assert!(!shape::proto_index_keys(rt));
        let arr = object::new_array(rt, 0);
        assert!(property::set_prototype_of(rt, arr, p));
        assert!(shape::proto_index_keys(rt));
        assert!(!set_adds(cx, rt, arr, 0), "§10.1.9.2 step 2.a");
        assert_eq!(object::array_len(arr), 0);
        assert!(set_adds(cx, rt, arr, 1), "another index");
    });
    // A String wrapper's indices are read-only (§10.4.3).
    each(|cx, rt| {
        let s = object::new_string(rt, "ab");
        let w = wrapper::new_wrapper(rt, rt.root(Root::StringPrototype), s);
        let arr = object::new_array(rt, 0);
        assert!(property::set_prototype_of(rt, arr, w));
        assert!(shape::proto_index_keys(rt));
        assert!(!set_adds(cx, rt, arr, 1));
        assert!(set_adds(cx, rt, arr, 2), "past the string");
    });
}

// ---- OBJECT_LAYOUT 2 (M2-P0): new kinds, the ITERATOR kind field, private names ------------

#[test]
fn generator_and_slot_object_cells_are_registered_object_kinds() {
    each(|_cx, rt| {
        let proto = rt.root(Root::ObjectPrototype);
        let g = object::new_object_with_fields(rt, kinds::GENERATOR, proto, object::genf::FIXED);
        let s =
            object::new_object_with_fields(rt, kinds::SLOT_OBJECT, proto, object::slot_obj::FIXED);
        // GC points with both cells in Rust locals: the heap verifier walks their fields.
        let t = object::new_plain_object(rt);
        object::fixed_set(rt, g, 2, t);
        object::fixed_set(rt, s, 0, int(3));
        put(rt, g, "p", int(1), attr::DEFAULT);
        assert!(kinds::is_object(g) && kinds::is_object(s));
        assert_eq!(object::fixed_count(kinds::GENERATOR.get()), 8);
        assert_eq!(object::fixed_count(kinds::SLOT_OBJECT.get()), 2);
        assert_eq!(object::fixed_get(g, 2), t);
        assert_eq!(object::fixed_get(s, 0), int(3));
        assert_eq!(own(rt, g, "p"), Some(int(1)));
    });
}

/// SLOT_OBJECT keeps its class id and its internal-slots cell in fixed fields, which property
/// code never touches: named properties, a dictionary move (it clears the in-object slots)
/// and PreventExtensions leave both fields as they were. This test builds the cell by hand
/// with `new_object_with_fields` and puts a plain object where the ELEMENTS cell of the
/// internal slots goes. The real allocator is `object::new_slot_object` (P3-ABI-SLOTS); the
/// same check on its objects is `tests::slots::property_changes_leave_the_slots_alone`.
#[test]
fn slot_object_fixed_fields_survive_property_changes() {
    each(|_cx, rt| {
        assert_eq!(
            (
                object::slot_obj::CLASS,
                object::slot_obj::SLOTS,
                object::slot_obj::FIXED
            ),
            (24, 32, 2)
        );
        assert_eq!(object::fixed_count(kinds::SLOT_OBJECT.get()), 2);
        let proto = rt.root(Root::ObjectPrototype);
        let s =
            object::new_object_with_fields(rt, kinds::SLOT_OBJECT, proto, object::slot_obj::FIXED);
        let slots = object::new_plain_object(rt);
        object::fixed_set(rt, s, 0, int(7));
        object::fixed_set(rt, s, 1, slots);
        for i in 0..6 {
            put(rt, s, &format!("p{i}"), int(i), attr::DEFAULT);
        }
        assert!(!shape::is_dictionary(shape::shape_of(s)));
        props::to_dictionary(rt, s);
        assert!(shape::is_dictionary(shape::shape_of(s)));
        assert!(property::prevent_extensions(rt, s));
        put(rt, s, "p0", int(10), attr::DEFAULT);
        assert_eq!(object::fixed_get(s, 0), int(7), "class id");
        assert_eq!(object::fixed_get(s, 1), slots, "internal slots cell");
        assert_eq!(own(rt, s, "p0"), Some(int(10)));
        assert_eq!(own(rt, s, "p5"), Some(int(5)));
    });
}

#[test]
fn an_iterator_cell_has_a_kind_field() {
    each(|_cx, rt| {
        let a = object::new_array(rt, 0);
        let it = object::new_iterator(rt, rt.root(Root::ArrayIteratorPrototype), a);
        assert_eq!(object::fixed_count(kinds::ITERATOR.get()), 3);
        assert_eq!(object::iterator_kind(it), object::iter_kind::VALUES);
        assert_eq!(object::iterator_state(it), (a, 0));
        let e = object::new_iterator_of_kind(
            rt,
            rt.root(Root::ArrayIteratorPrototype),
            a,
            object::iter_kind::ENTRIES,
        );
        assert_eq!(object::iterator_kind(e), object::iter_kind::ENTRIES);
        assert_eq!(object::iterator_kind(it), object::iter_kind::VALUES);
    });
}

#[test]
fn a_private_name_is_not_listed_by_own_property_keys_or_for_in() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        let private = crate::symbol::new_private(rt, k(rt, "#x"));
        let public = crate::symbol::new_symbol(rt, k(rt, "s"));
        assert!(crate::symbol::is_private(private));
        assert!(!crate::symbol::is_private(public));
        assert!(crate::symbol::is_private_key(private));
        assert!(!crate::symbol::is_private_key(k(rt, "#x")));
        put(rt, o, "a", int(1), attr::DEFAULT);
        props::own_define(rt, o, private, int(2), attr::DEFAULT);
        props::own_define(rt, o, public, int(3), attr::DEFAULT);
        put(rt, o, "b", int(4), attr::DEFAULT);
        let keys = property::own_property_keys(cx, o);
        let got: Vec<u64> = (0..object::array_len(keys))
            .map(|i| object::array_get(keys, i).unwrap())
            .collect();
        assert_eq!(got, [k(rt, "a"), k(rt, "b"), public]);
        assert_eq!(for_in(cx, o), ["a", "b"]);
        // The private key is still stored; only the listings skip it.
        assert_eq!(props::own_get(o, private).map(|(v, _)| v), Some(int(2)));
        // `[[Get]]` still finds it: the P0 stubs of slow/private.rs forward to this path.
        assert_eq!(property::get_keyed(cx, o, private).unwrap(), int(2));
    });
}

/// H13 native-word rule, FUNCTION +24 (A1): what each kind of FunctionInfo a function cell
/// can point at offers a snapshot as an index form today.
/// - bytecode info: (unit, function id), inside its unit;
/// - table builtin: a host info inside its module's dense `infos`, one per BuiltinId, so the
///   pointer gives back the BuiltinId (the info does not store it);
/// - embedder host function (`realm::new_host_function`): unit null, function id u32::MAX,
///   the same pair for every such function, so no index form until the external-reference
///   table (H13-Q5, A1 in M4).
#[test]
fn function_cell_infos_have_an_index_form() {
    on_big_stack(|| {
        // f0: `g = function g() {}`; f1: g.
        let mut s = Asm::script(0);
        let k0 = s.b.function_constant(ep9_bytecode::FunctionId(1));
        s.b.create_closure(k0, s.closure, 0);
        s.sta_global("g");
        s.b.lda_undefined().return_();
        let mut g = Asm::new(super::meta("g", 0, 0, 0, 0));
        g.b.lda_undefined().return_();
        let mut iso = isolate(verified(false));
        iso.with_cx_checkpoint(|cx| {
            cx.run_script(unit("", vec![s.finish(), g.finish()]), "t.js")
                .map(|_| ())
        })
        .unwrap();
        let rt = &*iso.rt;
        // Bytecode: (unit, function id) names the info.
        let gv = iso.global_raw("g").unwrap().to_bits();
        let fi = object::function_info(gv);
        let lu = fi.loaded_unit().expect("a bytecode info has its unit");
        assert!(fi.is_bytecode() && fi.host.is_none());
        assert!((fi.function_id as usize) < lu.infos.len());
        assert!(core::ptr::eq(&lu.infos[fi.function_id as usize], fi));
        // Table builtins: host infos, one distinct info per BuiltinId.
        let mut seen = std::collections::HashMap::new();
        for m in crate::natives::VM_TABLE.modules {
            for i in 0..m.defs.len() {
                let id = crate::builtins::BuiltinId::new(m.block, i as u16);
                let f = crate::builtins::function_object(rt, id);
                let info = object::function_info(f);
                assert!(info.host.is_some() && info.unit.is_null(), "{id}");
                assert_eq!(info.function_id, u32::MAX, "{id}");
                let p = core::ptr::from_ref(info).addr();
                assert_eq!(seen.insert(p, id), None, "{id}: shares an info");
            }
        }
        assert!(!seen.is_empty());
        // Embedder host functions: the (unit, function id) pair is the same for all of them
        // (H13-Q5).
        let pair = |f: u64| {
            let info = object::function_info(f);
            assert!(info.host.is_some());
            (info.unit, info.function_id)
        };
        let a = crate::realm::new_host_function(rt, super::print, "a", 0, false);
        let b = crate::realm::new_host_function(rt, super::print, "b", 0, false);
        assert_eq!(pair(a), (core::ptr::null(), u32::MAX));
        assert_eq!(pair(a), pair(b));
        assert!(!seen.contains_key(&core::ptr::from_ref(object::function_info(a)).addr()));
    });
}
