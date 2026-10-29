//! Tests of field representation and const bits (`field_rep.rs`, M5-P2 CONSTREP; H17 rep/const
//! row, C-02): the one-way order, the const bit that the second store clears, a widening that
//! reaches every shape of the field's family (branch copies too), the kind-3 fuse that a
//! widening pops (its dependents invalidated, its word one-way), the representation check of
//! the store IC hits (field and add cases) and the global store case, and the join of an
//! elements-kind transition. Each isolate test runs with and without GC stress (a collection
//! before every allocation, then the heap verifier). Objects and shapes live in Rust locals
//! (the conservative stack scan keeps them); no test asserts that anything dies.

use ep9_value::Value;

use super::ic::{Site, T, each as each_ic, int, key, object_prototype, object_with};
use super::{config, isolate, on_big_stack};
use crate::frame::NULL;
use crate::fuse::{CodeHandle, Descriptor, FuseRef, Refusal, descriptor};
use crate::ic::{self, smi};
use crate::object::{self, attr};
use crate::props;
use crate::runtime::Runtime;
use crate::shape::{self, rep};
use crate::{Dispatch, Isolate};

/// Runs `test` on a fresh isolate, with and without GC stress, on a big stack; collects and
/// verifies the heap after it.
fn each(test: fn(&mut Isolate)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            test(&mut iso);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

fn dbl(d: f64) -> u64 {
    Value::double(d).to_bits()
}

/// A string value (an atom).
fn text(rt: &Runtime, s: &str) -> u64 {
    key(rt, s)
}

/// (representation, const) of property `i` of the shape of `o`.
fn bits(o: u64, i: usize) -> (i32, bool) {
    let d = shape::details_at(shape::shape_of(o), i);
    (d.rep(), d.is_const())
}

/// `o.name = v` on an own writable data property (a second store).
fn store(rt: &Runtime, o: u64, name: &str, v: u64) {
    assert!(props::own_replace(rt, o, key(rt, name), v));
}

#[test]
fn the_tag_order_and_the_join() {
    use crate::field_rep::{fits, join, rep_of};
    assert_eq!(rep_of(int(-7)), rep::INT32);
    assert_eq!(rep_of(dbl(1.5)), rep::DOUBLE);
    assert_eq!(rep_of(dbl(f64::NAN)), rep::DOUBLE);
    for v in [
        Value::UNDEFINED.to_bits(),
        Value::NULL.to_bits(),
        Value::TRUE.to_bits(),
        crate::frame::HOLE,
    ] {
        assert_eq!(rep_of(v), rep::HEAP, "{v:#x}");
    }
    // One-way: NONE -> INT32 -> DOUBLE -> ANY, NONE -> HEAP -> ANY.
    assert_eq!(join(rep::NONE, rep::INT32), rep::INT32);
    assert_eq!(join(rep::INT32, rep::DOUBLE), rep::DOUBLE);
    assert_eq!(join(rep::DOUBLE, rep::INT32), rep::DOUBLE);
    assert_eq!(join(rep::INT32, rep::HEAP), rep::ANY);
    assert_eq!(join(rep::DOUBLE, rep::HEAP), rep::ANY);
    assert_eq!(join(rep::HEAP, rep::HEAP), rep::HEAP);
    assert_eq!(join(rep::ANY, rep::NONE), rep::ANY);
    for r in [rep::NONE, rep::INT32, rep::DOUBLE, rep::HEAP, rep::ANY] {
        assert_eq!(join(r, rep::ANY), rep::ANY);
        assert_eq!(join(r, r), r);
    }
    // A DOUBLE field holds every number; NONE holds nothing.
    assert!(fits(rep::DOUBLE, int(3)) && fits(rep::DOUBLE, dbl(0.5)));
    assert!(!fits(rep::DOUBLE, Value::NULL.to_bits()));
    assert!(!fits(rep::INT32, dbl(0.5)) && !fits(rep::HEAP, int(0)));
    assert!(!fits(rep::NONE, int(0)) && fits(rep::ANY, int(0)));
}

#[test]
fn a_field_widens_one_way_and_the_second_store_clears_const() {
    each(|iso| {
        let rt = &iso.rt;
        let op = object_prototype(rt);
        let o = object_with(rt, op, &[("a", int(1)), ("b", text(rt, "s"))]);
        assert_eq!(bits(o, 0), (rep::INT32, true), "the first store");
        assert_eq!(bits(o, 1), (rep::HEAP, true));
        // A second object takes the same transitions: its first stores keep const.
        let p = object_with(rt, op, &[("a", int(2)), ("b", Value::NULL.to_bits())]);
        assert_eq!(shape::shape_of(p), shape::shape_of(o));
        assert_eq!(bits(p, 0), (rep::INT32, true));
        assert_eq!(bits(p, 1), (rep::HEAP, true));
        // The second store clears const; an int32 keeps INT32.
        store(rt, o, "a", int(5));
        assert_eq!(bits(o, 0), (rep::INT32, false));
        store(rt, o, "a", dbl(2.5));
        assert_eq!(bits(o, 0), (rep::DOUBLE, false));
        store(rt, p, "a", int(9));
        assert_eq!(bits(o, 0), (rep::DOUBLE, false), "never narrows");
        store(rt, o, "a", text(rt, "x"));
        assert_eq!(bits(p, 0), (rep::ANY, false));
        store(rt, o, "a", int(0));
        assert_eq!(bits(o, 0), (rep::ANY, false));
        // HEAP -> ANY.
        store(rt, p, "b", int(1));
        assert_eq!(bits(o, 1), (rep::ANY, false));
        // A first store that does not fit a transition other objects took widens it, and
        // keeps const (no object replaced a value).
        let q = object_with(rt, op, &[("c", int(1))]);
        let r = object_with(rt, op, &[("c", dbl(0.25))]);
        assert_eq!(shape::shape_of(q), shape::shape_of(r));
        assert_eq!(bits(q, 0), (rep::DOUBLE, true));
        // An accessor pair is a cell.
        let g = object::new_object(rt, op);
        let pair =
            crate::accessor::new_pair(rt, Value::UNDEFINED.to_bits(), Value::UNDEFINED.to_bits());
        props::own_define(
            rt,
            g,
            key(rt, "g"),
            pair,
            attr::CONFIGURABLE | shape::ACCESSOR,
        );
        assert_eq!(bits(g, 0).0, rep::HEAP);
    });
}

/// A widening writes every DESCRIPTORS cell of the family: the owner's (shared with its
/// descendants by the prefix rule) and a branch's copy; the ancestors do not have the field.
#[test]
fn a_widening_reaches_every_shape_of_the_family() {
    each(|iso| {
        let rt = &iso.rt;
        let op = object_prototype(rt);
        let o1 = object_with(rt, op, &[("fa", int(1))]);
        let o2 = object_with(rt, op, &[("fa", int(1)), ("fb", int(1))]);
        // A branch: {fa, fx} copies the prefix [fa].
        let o3 = object_with(rt, op, &[("fa", int(1)), ("fx", int(1))]);
        let (s1, s2, s3) = (
            shape::shape_of(o1),
            shape::shape_of(o2),
            shape::shape_of(o3),
        );
        assert_ne!(
            shape::descriptors(s2),
            shape::descriptors(s3),
            "a branch copy"
        );
        assert_eq!(shape::field_owner(s2, 0), s1);
        assert_eq!(shape::field_owner(s3, 0), s1);
        assert_eq!(shape::field_owner(s3, 1), s3);
        // A store through the deepest shape widens the whole family.
        store(rt, o3, "fa", dbl(0.5));
        for o in [o1, o2, o3] {
            assert_eq!(bits(o, 0), (rep::DOUBLE, false));
        }
        // Siblings of the field are untouched.
        assert_eq!(bits(o2, 1), (rep::INT32, true));
        assert_eq!(bits(o3, 1), (rep::INT32, true));
        // Another family of the same key (another prototype) is its own.
        let other = object_with(rt, NULL, &[("fa", int(1))]);
        assert_eq!(bits(other, 0), (rep::INT32, true));
        // A transition made after the widening copies the wide bits.
        let o4 = object_with(rt, op, &[("fa", int(2)), ("fy", int(1))]);
        assert_eq!(bits(o4, 0), (rep::DOUBLE, false));
        // A prototype-mode object (a unique shape) shares or copies the entry, and widens.
        let p = object_with(rt, op, &[("fa", int(1)), ("fb", int(1))]);
        let _child = object::new_object(rt, p);
        assert!(shape::is_prototype_object(p));
        store(rt, p, "fb", text(rt, "s"));
        assert_eq!(bits(p, 1), (rep::ANY, false));
        assert!(
            matches!(bits(o2, 1), (rep::ANY, false)),
            "the shared entry widens too"
        );
    });
}

/// Kind 3 (C-02): a widening of the field pops its fuse; the dependents are invalidated; the
/// word never reads intact again; another family's fuse stays.
#[test]
fn a_widening_pops_the_field_fuse_and_invalidates_its_dependents() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let op = object_prototype(rt);
        let o1 = object_with(rt, op, &[("ka", int(1))]);
        let o2 = object_with(rt, op, &[("ka", int(1)), ("kb", int(1))]);
        let other = object_with(rt, NULL, &[("ka", int(1))]);
        let d = Descriptor::Field { index: 0 };
        assert_eq!(d.kind(), descriptor::FIELD);
        assert!(descriptor::known(descriptor::FIELD));
        let f = FuseRef::Dynamic(d);
        let (s1, s2, so) = (
            shape::shape_of(o1),
            shape::shape_of(o2),
            shape::shape_of(other),
        );
        // Any shape of the family names the owner's fuse.
        let w = fuses.arm(rt, d, s2).unwrap();
        let w1 = fuses.arm(rt, d, s1).unwrap();
        assert!(std::rc::Rc::ptr_eq(&w, &w1));
        assert_eq!(fuses.subject_count(d), 1);
        let wo = fuses.arm(rt, d, so).unwrap();
        assert!(!std::rc::Rc::ptr_eq(&w, &wo));
        let code: Vec<CodeHandle> = (0..3).map(|_| CodeHandle::new()).collect();
        for c in &code {
            fuses.depend(rt, f, s1, c).unwrap();
        }
        let oc = CodeHandle::new();
        fuses.depend(rt, f, so, &oc).unwrap();
        // A first store that fits and a store into another field do not pop.
        let _o5 = object_with(rt, op, &[("ka", int(7))]);
        store(rt, o2, "kb", dbl(1.5));
        assert!(w.intact() && fuses.intact(rt, f, s2));
        // The second store clears const: that is a change of the field's bits.
        store(rt, o2, "ka", int(2));
        assert!(!w.intact() && code.iter().all(CodeHandle::is_invalidated));
        assert!(!fuses.intact(rt, f, s1));
        // One-way, for this subject.
        assert_eq!(fuses.arm(rt, d, s1).err(), Some(Refusal::Popped));
        assert_eq!(fuses.guard(rt, f, s2).err(), Some(Refusal::Popped));
        assert_eq!(
            fuses.depend(rt, f, s1, &CodeHandle::new()),
            Err(Refusal::Popped)
        );
        // Another family's field: its own subject, its own dependents.
        assert!(wo.intact() && !oc.is_invalidated() && fuses.intact(rt, f, so));
        store(rt, other, "ka", text(rt, "s"));
        assert!(!wo.intact() && oc.is_invalidated());
        // Field 1 of the first family: armed after its own widening, pops on the next.
        let d1 = Descriptor::Field { index: 1 };
        let w2 = fuses.arm(rt, d1, s2).unwrap();
        store(rt, o2, "kb", int(3));
        assert!(w2.intact(), "DOUBLE holds an int32; const is clear already");
        store(rt, o2, "kb", Value::NULL.to_bits());
        assert!(!w2.intact());
        // Names: a dictionary shape, an index past the shape, an accessor, a non-shape.
        let dict = object_with(rt, op, &[("x", int(1))]);
        props::to_dictionary(rt, dict);
        let ds = shape::shape_of(dict);
        assert_eq!(fuses.arm(rt, d, ds).err(), Some(Refusal::NotNamed));
        let far = Descriptor::Field { index: 9 };
        assert_eq!(fuses.arm(rt, far, s2).err(), Some(Refusal::NotNamed));
        assert_eq!(fuses.arm(rt, d, o1).err(), Some(Refusal::NotNamed));
        assert_eq!(fuses.arm(rt, d, NULL).err(), Some(Refusal::NotNamed));
        let g = object::new_object(rt, op);
        let pair =
            crate::accessor::new_pair(rt, Value::UNDEFINED.to_bits(), Value::UNDEFINED.to_bits());
        props::own_define(
            rt,
            g,
            key(rt, "g"),
            pair,
            attr::CONFIGURABLE | shape::ACCESSOR,
        );
        let gs = shape::shape_of(g);
        assert_eq!(fuses.arm(rt, d, gs).err(), Some(Refusal::NotNamed));
        // An unused field fuse goes at the sweep.
        let o6 = object_with(rt, op, &[("kz", int(1))]);
        let s6 = shape::shape_of(o6);
        let w6 = fuses.arm(rt, d, s6).unwrap();
        drop(w6);
        fuses.sweep(rt);
        assert_eq!(fuses.guard(rt, f, s6).err(), Some(Refusal::NotArmed));
        std::hint::black_box((o1, o6));
    });
}

/// The store IC (0x24) records a representation-checked field case after the miss store,
/// and a hit stores only a value that fits: a double into an INT32 field misses, so the miss
/// path widens it (with no check in the hit the field would stay INT32 while it holds a
/// double).
#[test]
fn a_store_hit_checks_the_value_tag() {
    each_ic(|c| {
        let mut t = T::new(c, &[Site::Set("x", false), Site::Set("y", false)]);
        let rt_op = object_prototype(t.rt());
        let o = object_with(t.rt(), rt_op, &[("x", int(1))]);
        let s = shape::shape_of(o);
        let raw = |t: &T, i: usize, s: u64| {
            ic::find_raw(t.vector(i), super::ic::SLOT, s).map(crate::frame::int32_of)
        };
        // The miss: the second store clears const; the case names INT32.
        t.ok(0, o, int(2));
        assert_eq!(bits(o, 0), (rep::INT32, false));
        let h = raw(&t, 0, s).expect("a case");
        assert_eq!(h & smi::KIND_MASK, smi::FIELD_INT32_IN_OBJECT);
        assert_eq!(
            t.kind(0, s),
            Some(smi::FIELD_IN_OBJECT),
            "find gives the plain case"
        );
        // A hit that fits.
        t.ok(0, o, int(3));
        assert_eq!(props::own_get(o, key(t.rt(), "x")).unwrap().0, int(3));
        // A double: the hit refuses, the miss widens and records the NUMBER case.
        t.ok(0, o, dbl(0.5));
        assert_eq!(props::own_get(o, key(t.rt(), "x")).unwrap().0, dbl(0.5));
        assert_eq!(bits(o, 0), (rep::DOUBLE, false));
        assert_eq!(
            raw(&t, 0, s).map(|h| h & smi::KIND_MASK),
            Some(smi::FIELD_NUMBER_IN_OBJECT)
        );
        t.ok(0, o, int(4));
        assert_eq!(bits(o, 0), (rep::DOUBLE, false));
        // A string: ANY, the plain case (no check).
        t.ok(0, o, Value::NULL.to_bits());
        assert_eq!(bits(o, 0), (rep::ANY, false));
        assert_eq!(
            raw(&t, 0, s).map(|h| h & smi::KIND_MASK),
            Some(smi::FIELD_IN_OBJECT)
        );

        // The add case (ADD): the target's new field holds the first value's representation;
        // a hit with a value that does not fit misses, and the add widens the transition.
        let a = object_with(t.rt(), rt_op, &[("x", int(1))]);
        let b = object_with(t.rt(), rt_op, &[("x", int(1))]);
        let e = object_with(t.rt(), rt_op, &[("x", int(1))]);
        t.ok(1, a, int(1));
        let ta = shape::shape_of(a);
        assert_eq!(bits(a, 1), (rep::INT32, true));
        t.ok(1, b, int(2));
        assert_eq!(shape::shape_of(b), ta);
        assert_eq!(
            bits(b, 1),
            (rep::INT32, true),
            "a fitting add hit keeps the bits"
        );
        t.ok(1, e, text(t.rt(), "s"));
        assert_eq!(shape::shape_of(e), ta);
        assert_eq!(
            bits(e, 1),
            (rep::ANY, true),
            "the add widened; still no second store"
        );
        std::hint::black_box((o, a, b, e));
    });
}

/// A global store case (0x12): the hit does not check the value (its slot is shared with the
/// loads of the name), so the case makes the field ANY, not const, before it is recorded.
#[test]
fn a_global_store_case_widens_the_binding() {
    each_ic(|c| {
        let mut t = T::new(c, &[Site::StoreGlobal("gfr")]);
        t.ok(0, int(1), int(0));
        t.ok(0, int(2), int(0));
        let g = t.rt().global();
        let gs = shape::shape_of(g);
        let f = shape::lookup(t.rt(), gs, key(t.rt(), "gfr")).expect("the global");
        let d = f.details;
        assert_eq!((d.rep(), d.is_const()), (rep::ANY, false));
        t.ok(0, text(t.rt(), "s"), int(0));
        t.ok(0, dbl(0.5), int(0));
        let d = shape::details_at(shape::shape_of(g), f.index);
        assert_eq!(d.rep(), rep::ANY);
    });
}

/// An object that changes its elements kind moves to the matching shape of the other kind's
/// tree with its values: a transition found there widens to hold them; a new one copies them.
#[test]
fn an_elements_kind_change_joins_the_field_bits() {
    each(|iso| {
        let rt = &iso.rt;
        let op = object_prototype(rt);
        let k = crate::elements::kind::PACKED_DOUBLE;
        // The other tree has `ea` as INT32 (const).
        let p = object::new_object(rt, op);
        let ps = shape::with_elements_kind(rt, shape::shape_of(p), k).unwrap();
        shape::install(rt, p, ps);
        props::own_define(rt, p, key(rt, "ea"), int(1), attr::DEFAULT);
        assert_eq!(bits(p, 0), (rep::INT32, true));
        // `o` has `ea` as HEAP and `eb` as DOUBLE (not const).
        let o = object_with(rt, op, &[("ea", text(rt, "s")), ("eb", int(1))]);
        store(rt, o, "eb", dbl(0.5));
        let to = shape::with_elements_kind(rt, shape::shape_of(o), k).unwrap();
        assert_eq!(
            shape::field_owner(to, 0),
            shape::shape_of(p),
            "the found transition"
        );
        let d0 = shape::details_at(to, 0);
        assert_eq!((d0.rep(), d0.is_const()), (rep::ANY, true));
        let d1 = shape::details_at(to, 1);
        assert_eq!(
            (d1.rep(), d1.is_const()),
            (rep::DOUBLE, false),
            "the new one copies"
        );
        assert_eq!(bits(p, 0), (rep::ANY, true));
        std::hint::black_box(o);
    });
}

/// Shapes made with no value known (`shape::add`; the cached function shapes) hold ANY: their
/// fields are written with no hook (`object::new_function`).
#[test]
fn shapes_with_no_known_value_are_any() {
    each(|iso| {
        let rt = &iso.rt;
        let fs = shape::function_shape(rt);
        for i in 0..shape::count(fs) {
            let d = shape::details_at(fs, i);
            assert_eq!((d.rep(), d.is_const()), (rep::ANY, false));
        }
        let r = shape::initial_shape(rt, NULL, crate::kinds::OBJECT, 4);
        let shape::Added::Shape(c) = shape::add(rt, r, key(rt, "plain"), attr::DEFAULT) else {
            panic!("a child");
        };
        assert_eq!(shape::details_at(c, 0).rep(), rep::ANY);
    });
}
