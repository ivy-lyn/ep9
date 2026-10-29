//! Tests of the elements kinds (`elements.rs`) (owner M5-P1 ELEM1, then M5-P2 ELEM2; H18):
//! the lattice and its one-way transitions (H18 Rules "Kind transitions are one-way"), the
//! DOUBLE store with every NaN pattern (H18 Rules "A DOUBLE element is a raw f64"), loads
//! that never change a kind (MC-w15-21), the kind-aware readers (`array_get`,
//! `keyed_ic::present_element` through the StoreKeyed hit), shapes of each kind
//! (`shape::with_elements_kind`), elements on objects that are not Arrays (H18-Q3), and the
//! IC_LAYOUT 3 kinds set of the keyed slots. Every isolate test runs with the heap verifier
//! after each collection, without and with GC stress 1 (a collection before every
//! allocation; the elements cells, raw f64 words and new shapes live through each one).
//! Values live in Rust locals (the conservative stack scan keeps them).

use ep9_bytecode::{FeedbackSlotKind, Register, VerifiedUnit};
use ep9_value::Value;

use super::{Asm, config, isolate, on_big_stack, unit};
use crate::atoms;
use crate::descriptor::Desc;
use crate::elements::{self, Storage, kind as ek};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::interp::Cx as VmCx;
use crate::kinds;
use crate::object::{self, attr, delems, obj};
use crate::profile::keyed;
use crate::property;
use crate::props;
use crate::runtime::{Runtime, val};
use crate::shape;
use crate::{Dispatch, IsolateConfig};

/// Runs `f` in a new isolate, without and with GC stress 1; the heap verifier runs after a
/// last collection.
fn each(f: fn(&mut VmCx, &Runtime)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            {
                let iso = &mut iso;
                f(&mut iso.cx, &iso.rt);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

fn int(i: i32) -> u64 {
    int32_bits(i)
}

fn dbl(d: f64) -> u64 {
    Value::double(d).to_bits()
}

fn key(rt: &Runtime, s: &str) -> u64 {
    atoms::intern_str(rt, s)
}

/// The kind of the cell at +16 of `o`, or 0 for none.
fn cell_kind_at_16(o: u64) -> u8 {
    kinds::cell_kind(object::rd(object::cell(o), obj::ELEMENTS))
}

/// Checks the elements invariants of `o` and its shape (`elements::check`, `shape::check`).
fn checked(rt: &Runtime, o: u64) {
    elements::check(o).unwrap();
    shape::check(rt, shape::shape_of(o)).unwrap();
}

/// The texts of the keys in the array `a`.
fn texts(a: u64) -> Vec<String> {
    (0..object::array_len(a))
        .map(|i| crate::string::str_view(object::array_get(a, i).unwrap()).to_string_lossy())
        .collect()
}

// ---- the lattice ---------------------------------------------------------------------------

/// `join` is the least upper bound of the Decision lattice (H18), and a transition is allowed
/// only upward: each pair of kinds 0-5 either allows one direction only or is the same kind.
#[test]
fn the_lattice_is_one_way() {
    let kinds = [
        ek::PACKED_INT32,
        ek::HOLEY_INT32,
        ek::PACKED_DOUBLE,
        ek::HOLEY_DOUBLE,
        ek::PACKED_ELEMENTS,
        ek::HOLEY_ELEMENTS,
    ];
    for &a in &kinds {
        assert_eq!(elements::join(a, a), a);
        assert!(elements::allows(a, elements::holey(a)));
        assert!(elements::allows(a, ek::DICTIONARY));
        for &b in &kinds {
            let j = elements::join(a, b);
            assert_eq!(j, elements::join(b, a), "join is symmetric");
            assert!(elements::allows(a, j) && elements::allows(b, j));
            if a != b {
                assert!(
                    !(elements::allows(a, b) && elements::allows(b, a)),
                    "{a} <-> {b} both ways"
                );
            }
        }
    }
    // The Decision diagram: right (INT32 -> DOUBLE -> ELEMENTS) and down (holey) only.
    assert!(elements::allows(ek::PACKED_INT32, ek::PACKED_DOUBLE));
    assert!(elements::allows(ek::PACKED_DOUBLE, ek::PACKED_ELEMENTS));
    assert!(elements::allows(ek::PACKED_INT32, ek::HOLEY_ELEMENTS));
    assert!(!elements::allows(ek::PACKED_DOUBLE, ek::HOLEY_INT32));
    assert!(!elements::allows(ek::HOLEY_INT32, ek::PACKED_ELEMENTS));
    assert!(!elements::allows(ek::PACKED_ELEMENTS, ek::PACKED_DOUBLE));
    assert_eq!(elements::value_kind(int(7)), ek::PACKED_INT32);
    assert_eq!(elements::value_kind(dbl(0.5)), ek::PACKED_DOUBLE);
    assert_eq!(elements::value_kind(dbl(-0.0)), ek::PACKED_DOUBLE);
    assert_eq!(elements::value_kind(UNDEFINED), ek::PACKED_ELEMENTS);
    // The kinds-set bits: one per kind, all above bit 9.
    assert_eq!(elements::KINDS_SET_MASK, 0x7F << 10);
    assert_eq!(
        elements::KINDS_SET_MASK & 0x3FF,
        0,
        "bits 0-9 keep their meaning"
    );
}

// ---- array transitions ---------------------------------------------------------------------

/// The store side of each transition on an Array: the kind after each store, the storage form
/// of its cell, the values kept through each conversion, and no way back (an int32 store into
/// a DOUBLE or ELEMENTS array, or a store that fills the last hole, keeps the kind).
#[test]
fn array_stores_make_one_way_transitions() {
    each(|_cx, rt| {
        let a = object::new_array(rt, 0);
        assert_eq!(elements::kind_of(a), ek::PACKED_INT32);
        assert_eq!(object::rd(object::cell(a), obj::ELEMENTS), UNDEFINED);
        let step = |i: u32, v: u64, want: u8, cell: u8| {
            assert!(object::array_set(rt, a, i, v));
            assert_eq!(elements::kind_of(a), want, "after a[{i}] = {v:#x}");
            assert_eq!(cell_kind_at_16(a), cell, "the cell after a[{i}]");
            checked(rt, a);
        };
        let (values, doubles) = (kinds::ELEMENTS.get(), kinds::ELEMENTS_DOUBLE.get());
        step(0, int(1), ek::PACKED_INT32, values);
        step(1, int(2), ek::PACKED_INT32, values);
        step(2, dbl(2.5), ek::PACKED_DOUBLE, doubles);
        assert_eq!(
            object::array_get(a, 0),
            Some(int(1)),
            "int32 -> f64 -> int32"
        );
        step(0, int(9), ek::PACKED_DOUBLE, doubles);
        step(3, val_str(rt, "s"), ek::PACKED_ELEMENTS, values);
        assert_eq!(object::array_get(a, 2), Some(dbl(2.5)), "f64 -> Value");
        assert_eq!(object::array_get(a, 0), Some(int(9)));
        step(1, int(5), ek::PACKED_ELEMENTS, values);
        // A gap makes a hole; filling it keeps the holey kind.
        step(5, int(6), ek::HOLEY_ELEMENTS, values);
        step(4, int(7), ek::HOLEY_ELEMENTS, values);
        assert_eq!(object::array_len(a), 6);

        // A hole store (delete) and a `length` that grows make the holey form; a cut keeps it.
        let b = object::new_array(rt, 0);
        assert!(object::array_set(rt, b, 0, dbl(0.5)));
        assert!(object::array_set(rt, b, 1, dbl(1.5)));
        assert_eq!(elements::kind_of(b), ek::PACKED_DOUBLE);
        assert!(object::array_set_length(rt, b, 1));
        assert_eq!(
            elements::kind_of(b),
            ek::PACKED_DOUBLE,
            "a cut keeps PACKED"
        );
        assert_eq!(object::array_get(b, 1), None);
        assert!(object::array_set_length(rt, b, 3));
        assert_eq!(elements::kind_of(b), ek::HOLEY_DOUBLE, "growth makes holes");
        checked(rt, b);
        assert!(object::array_set(rt, b, 1, dbl(3.5)));
        assert!(object::array_set(rt, b, 2, dbl(4.5)));
        assert_eq!(
            elements::kind_of(b),
            ek::HOLEY_DOUBLE,
            "filled: still HOLEY"
        );
        let c = object::new_array(rt, 0);
        assert!(object::array_set(rt, c, 0, int(1)));
        assert!(object::array_set(rt, c, 0, HOLE));
        assert_eq!(elements::kind_of(c), ek::HOLEY_INT32);
        assert_eq!(object::array_get(c, 0), None);
        checked(rt, c);
        // A double into a HOLEY_INT32 array: HOLEY_DOUBLE, and the hole stays a hole.
        assert!(object::array_set(rt, c, 1, dbl(0.25)));
        assert_eq!(elements::kind_of(c), ek::HOLEY_DOUBLE);
        assert_eq!(object::array_get(c, 0), None);
        assert_eq!(object::array_get(c, 1), Some(dbl(0.25)));
        checked(rt, c);
        // ... and an object into it: HOLEY_ELEMENTS, the hole still a hole.
        assert!(object::array_set(rt, c, 2, a));
        assert_eq!(elements::kind_of(c), ek::HOLEY_ELEMENTS);
        assert_eq!(object::array_get(c, 0), None);
        assert_eq!(object::array_get(c, 2), Some(a));
        checked(rt, c);
    });
}

fn val_str(rt: &Runtime, s: &str) -> u64 {
    key(rt, s)
}

/// Every Array kind 0-5 is reached from PACKED_INT32 by stores, and from each one no store
/// sequence goes back to a less general kind.
#[test]
fn every_kind_is_reached_and_never_left_downward() {
    each(|_cx, rt| {
        // (stores that make the kind, the kind)
        let made: [(&[(u32, u64)], u8); 6] = [
            (&[(0, int(1))], ek::PACKED_INT32),
            (&[(1, int(1))], ek::HOLEY_INT32),
            (&[(0, dbl(1.5))], ek::PACKED_DOUBLE),
            (&[(1, dbl(1.5))], ek::HOLEY_DOUBLE),
            (&[(0, UNDEFINED)], ek::PACKED_ELEMENTS),
            (&[(1, NULL_)], ek::HOLEY_ELEMENTS),
        ];
        let probes = [int(3), dbl(-0.0), dbl(f64::NAN), UNDEFINED, HOLE];
        for (stores, want) in made {
            for &p in &probes {
                let a = object::new_array(rt, 0);
                for &(i, v) in stores {
                    assert!(object::array_set(rt, a, i, v));
                }
                assert_eq!(elements::kind_of(a), want);
                assert!(elements::storage(want).is_some());
                assert!(object::array_set(rt, a, 0, p));
                let after = elements::kind_of(a);
                assert!(elements::allows(want, after), "{want} -> {after}");
                checked(rt, a);
            }
        }
    });
}

const NULL_: u64 = crate::frame::NULL;

// ---- DOUBLE elements -----------------------------------------------------------------------

/// H18 Rules: a DOUBLE element is a raw f64 and the hole is one NaN pattern that no store
/// writes. Every NaN pattern (quiet, signalling, negative, the hole pattern itself, the
/// largest payloads) is stored as the canonical NaN, reads back as NaN and is present (not a
/// hole), through `array_set`, the StoreKeyed hit's `double_bits` and a conversion.
#[test]
fn every_nan_pattern_is_stored_canonical_and_never_as_the_hole() {
    let patterns: [u64; 9] = [
        0x7FF8_0000_0000_0000,
        0x7FF0_0000_0000_0001,
        0x7FF4_0000_0000_0000,
        0x7FFF_FFFF_FFFF_FFFF,
        0xFFF8_0000_0000_0000,
        0xFFF0_0000_0000_0001,
        0xFFF7_FFFF_FFF7_FFFF,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFF4_0000_0000_0001,
    ];
    for p in patterns {
        let d = f64::from_bits(p);
        assert!(d.is_nan());
        assert_eq!(object::double_bits(dbl(d)), delems::CANONICAL_NAN, "{p:#x}");
    }
    assert_eq!(object::double_bits(int(0)), 0f64.to_bits());
    assert_eq!(object::double_bits(dbl(-0.0)), (-0f64).to_bits());
    each(|_cx, rt| {
        let patterns = [
            0x7FF0_0000_0000_0001u64,
            0xFFF7_FFFF_FFF7_FFFF,
            0xFFFF_FFFF_FFFF_FFFF,
            0xFFF8_0000_0000_0000,
        ];
        let a = object::new_array(rt, 0);
        assert!(object::array_set(rt, a, 0, dbl(0.5)));
        for (i, &p) in patterns.iter().enumerate() {
            let i = i as u32 + 1;
            assert!(object::array_set(rt, a, i, dbl(f64::from_bits(p))));
            let e = object::cell(object::rd(object::cell(a), obj::ELEMENTS));
            let w = object::raw_get(e, delems::DATA + 8 * i as usize);
            assert_eq!(w, delems::CANONICAL_NAN, "{p:#x} stored as {w:#x}");
            let v = object::array_get(a, i).expect("a NaN is present");
            assert!(val(v).as_number().unwrap().is_nan());
        }
        assert_eq!(elements::kind_of(a), ek::PACKED_DOUBLE);
        checked(rt, a);
        // -0 and the int32 range survive the raw form.
        assert!(object::array_set(rt, a, 0, dbl(-0.0)));
        let z = val(object::array_get(a, 0).unwrap()).as_number().unwrap();
        assert!(z == 0.0 && z.is_sign_negative());
        assert!(object::array_set(rt, a, 0, int(i32::MIN)));
        assert_eq!(object::array_get(a, 0), Some(int(i32::MIN)));
        // A data word that looks like a cell is not traced (the GC never reads raw words).
        let o = object::new_plain_object(rt);
        assert!(object::array_set(rt, a, 1, dbl(f64::from_bits(o))));
        rt.heap.collect();
        checked(rt, a);
    });
}

// ---- loads never change a kind -------------------------------------------------------------

/// MC-w15-21: a load never changes an object's kind or shape: element reads, a hole read, an
/// out-of-bounds read, the key and index paths of `property.rs`, own keys and `for-in`.
#[test]
fn a_load_never_changes_a_kind() {
    each(|cx, rt| {
        // The objects live in `all` (a Rust local, so a root): a Vec's buffer is not scanned.
        let all = object::new_array(rt, 0);
        for n in 0..6 {
            let a = object::new_array(rt, 0);
            let v = match n / 2 {
                0 => int(1),
                1 => dbl(1.5),
                _ => UNDEFINED,
            };
            assert!(object::array_set(rt, a, 0, v));
            if n % 2 == 1 {
                assert!(object::array_set(rt, a, 2, v));
            }
            assert!(object::array_set(rt, all, n, a));
        }
        let plain = object::new_plain_object(rt);
        assert!(object::element_set(rt, plain, 1, dbl(2.5)));
        assert!(object::array_set(rt, all, 6, plain));
        for n in 0..7 {
            let o = object::array_get(all, n).unwrap();
            let (s, k) = (shape::shape_of(o), elements::kind_of(o));
            for i in 0..4u64 {
                let _ = property::get_index(cx, o, i).unwrap();
                let _ = property::has_index(cx, o, i);
                let _ = property::get_keyed(cx, o, int(i as i32)).unwrap();
                let _ = object::element_get(o, i as u32);
                let ki = crate::key::index_key(cx.rt(), i as u32);
                let _ = property::get_own_property(cx, o, ki).unwrap();
            }
            let _ = property::own_property_keys(cx, o);
            let _ = property::for_in_keys(cx, o).unwrap();
            assert_eq!(shape::shape_of(o), s, "a load changed the shape");
            assert_eq!(elements::kind_of(o), k);
        }
    });
}

// ---- shapes of each kind -------------------------------------------------------------------

/// `shape::with_elements_kind` keeps everything but the kind: a root, a tree shape with
/// properties (found again from the root of the new kind), a unique (prototype) shape and a
/// dictionary shape; each later shape change keeps the kind.
#[test]
fn shapes_keep_their_elements_kind() {
    each(|_cx, rt| {
        let o = object::new_plain_object(rt);
        props::own_define(rt, o, key(rt, "a"), int(1), attr::DEFAULT);
        props::own_define(rt, o, key(rt, "b"), int(2), attr::DEFAULT);
        let s = shape::shape_of(o);
        let t = shape::with_elements_kind(rt, s, ek::HOLEY_DOUBLE).unwrap();
        assert_eq!(shape::elements_kind(t), ek::HOLEY_DOUBLE);
        assert_eq!(shape::elements_kind(s), ek::PACKED_INT32);
        assert_eq!(shape::count(t), 2);
        assert_eq!(shape::proto(t), shape::proto(s));
        assert_eq!(shape::flags(t), shape::flags(s));
        assert_eq!(shape::key_at(t, 1), shape::key_at(s, 1));
        shape::check(rt, t).unwrap();
        // The same walk finds the same shape (the tree of that kind).
        assert_eq!(shape::with_elements_kind(rt, s, ek::HOLEY_DOUBLE), Some(t));
        // A child of `t` keeps the kind; so does its dictionary shape.
        shape::install(rt, o, t);
        props::own_define(rt, o, key(rt, "c"), int(3), attr::DEFAULT);
        assert_eq!(elements::kind_of(o), ek::HOLEY_DOUBLE);
        props::to_dictionary(rt, o);
        assert!(shape::is_dictionary(shape::shape_of(o)));
        assert_eq!(elements::kind_of(o), ek::HOLEY_DOUBLE);
        props::prevent_extensions(rt, o);
        assert_eq!(elements::kind_of(o), ek::HOLEY_DOUBLE);
        let d = shape::with_elements_kind(rt, shape::shape_of(o), ek::HOLEY_ELEMENTS).unwrap();
        assert!(shape::is_dictionary(d) && !shape::is_extensible(d));
        assert_eq!(shape::elements_kind(d), ek::HOLEY_ELEMENTS);
        // A prototype's unique shape: a new unique shape of the new kind.
        let p = object::new_array(rt, 0);
        let child = object::new_object(rt, p);
        assert!(shape::is_prototype_object(p));
        assert!(object::array_set(rt, p, 0, dbl(0.5)));
        let ps = shape::shape_of(p);
        assert_eq!(shape::elements_kind(ps), ek::PACKED_DOUBLE);
        assert!(shape::flags(ps) & shape::flag::UNIQUE != 0);
        shape::check(rt, ps).unwrap();
        assert_eq!(property::get_index(_cx, child, 0).unwrap(), dbl(0.5));
        // A new prototype keeps the kind (with_proto).
        let q = object::new_plain_object(rt);
        assert!(object::set_proto(rt, p, q));
        assert_eq!(elements::kind_of(p), ek::PACKED_DOUBLE);
        let arr = object::new_array(rt, 0);
        assert!(object::array_set(rt, arr, 0, dbl(0.5)));
        assert!(object::set_proto(rt, arr, q));
        assert_eq!(elements::kind_of(arr), ek::PACKED_DOUBLE);
        checked(rt, arr);
    });
}

// ---- elements on objects that are not Arrays (H18-Q3) ---------------------------------------

/// An array-index key on a plain object goes to its +16 elements store and keeps a fast
/// shape (no longer an OM-10 trigger); the kind is holey; own keys list the indices first,
/// ascending; non-default attributes, an accessor or an index too far move to (or stay in)
/// the property storage, one place per key; `delete` makes a hole. A string index key
/// defined on an object with no elements cell yet takes the key path (OM-10, as in v0).
#[test]
fn plain_objects_keep_index_keys_in_elements() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        props::own_define(rt, o, key(rt, "b"), int(1), attr::DEFAULT);
        assert!(property::set_index(cx, o, 2, int(2)).unwrap());
        assert!(!shape::is_dictionary(shape::shape_of(o)), "fast");
        assert_eq!(elements::kind_of(o), ek::HOLEY_INT32);
        assert_eq!(cell_kind_at_16(o), kinds::ELEMENTS.get());
        property::define(cx, o, key(rt, "0"), dbl(0.5)).unwrap();
        assert_eq!(elements::kind_of(o), ek::HOLEY_DOUBLE);
        assert_eq!(cell_kind_at_16(o), kinds::ELEMENTS_DOUBLE.get());
        props::own_define(rt, o, key(rt, "a"), int(3), attr::DEFAULT);
        assert!(property::set_index(cx, o, 1, int(4)).unwrap());
        assert_eq!(
            texts(property::own_property_keys(cx, o)),
            ["0", "1", "2", "b", "a"]
        );
        assert_eq!(
            texts(property::for_in_keys(cx, o).unwrap()),
            ["0", "1", "2", "b", "a"]
        );
        assert_eq!(property::get_index(cx, o, 0).unwrap(), dbl(0.5));
        assert_eq!(property::get_keyed(cx, o, int(1)).unwrap(), int(4));
        let k2 = key(rt, "2");
        assert_eq!(property::get_from(cx, o, k2).unwrap(), int(2));
        checked(rt, o);
        // Read-only: the element moves to the property storage (a dictionary), no copy left.
        let ro = Desc {
            writable: Some(false),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, o, k2, ro).unwrap());
        assert_eq!(object::element_get(o, 2), None);
        assert_eq!(
            props::own_get(o, k2),
            Some((int(2), attr::ENUMERABLE | attr::CONFIGURABLE))
        );
        assert!(!property::set_index(cx, o, 2, int(9)).unwrap());
        assert_eq!(
            texts(property::own_property_keys(cx, o)),
            ["0", "1", "2", "b", "a"]
        );
        // A new index key on the dictionary object still goes to the elements store.
        property::define(cx, o, key(rt, "3"), int(5)).unwrap();
        assert_eq!(object::element_get(o, 3), Some(int(5)));
        assert!(props::own_get(o, key(rt, "3")).is_none());
        // `delete` makes a hole.
        assert!(property::delete(cx, o, int(0), true).unwrap());
        assert_eq!(object::element_get(o, 0), None);
        assert!(!property::has_index(cx, o, 0));
        // An index far past the capacity stays a stored property (no dense range).
        let far = object::new_plain_object(rt);
        assert!(property::set_index(cx, far, 100_000, int(1)).unwrap());
        assert_eq!(object::rd(object::cell(far), obj::ELEMENTS), UNDEFINED);
        assert_eq!(property::get_index(cx, far, 100_000).unwrap(), int(1));
        assert!(property::set_index(cx, far, 3, int(2)).unwrap());
        assert_eq!(texts(property::own_property_keys(cx, far)), ["3", "100000"]);
        // No elements cell yet: a string index key is a stored property (a dictionary).
        let n = object::new_plain_object(rt);
        property::define(cx, n, key(rt, "0"), int(1)).unwrap();
        assert!(shape::is_dictionary(shape::shape_of(n)));
        assert_eq!(object::rd(object::cell(n), obj::ELEMENTS), UNDEFINED);
        assert_eq!(
            props::own_get(n, key(rt, "0")),
            Some((int(1), attr::DEFAULT))
        );
        // An accessor element of a plain object is a stored accessor.
        let g = object::new_plain_object(rt);
        assert!(property::set_index(cx, g, 0, int(1)).unwrap());
        assert_eq!(object::element_get(g, 0), Some(int(1)));
        let acc = Desc {
            get: Some(UNDEFINED),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, g, key(rt, "0"), acc).unwrap());
        assert_eq!(object::element_get(g, 0), None);
        let (_, a) = props::own_get(g, key(rt, "0")).unwrap();
        assert!(a & attr::ACCESSOR != 0);
        checked(rt, o);
        checked(rt, far);
        checked(rt, g);
        checked(rt, n);
    });
}

/// Freeze (SetIntegrityLevel) on a plain object with elements: each element moves to the
/// property storage with its new attributes; the object stays correct (no element left).
#[test]
fn freezing_a_plain_object_moves_its_elements() {
    each(|cx, rt| {
        let o = object::new_plain_object(rt);
        for i in 0..3 {
            assert!(property::set_index(cx, o, i, int(i as i32 + 10)).unwrap());
        }
        assert!(property::prevent_extensions(rt, o));
        for i in 0..3u32 {
            let k = crate::key::index_key(cx.rt(), i);
            let frozen = Desc {
                writable: Some(false),
                configurable: Some(false),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, o, k, frozen).unwrap());
        }
        for i in 0..3u32 {
            assert_eq!(object::element_get(o, i), None);
            let k = crate::key::index_key(cx.rt(), i);
            assert_eq!(
                property::get_own_property(cx, o, k).unwrap().unwrap().value,
                Some(int(i as i32 + 10))
            );
        }
        assert!(!property::set_index(cx, o, 0, int(1)).unwrap());
        assert!(
            !property::set_index(cx, o, 7, int(1)).unwrap(),
            "not extensible"
        );
        assert_eq!(texts(property::own_property_keys(cx, o)), ["0", "1", "2"]);
        checked(rt, o);
    });
}

/// An element on a prototype that is not an Array sets the `proto_index` flag and is found by
/// the chain walks; a set on the child lands on the child (as a stored property: with an
/// index key on the chain the set walks the chain, and the child has no elements cell yet).
#[test]
fn a_plain_prototype_with_elements_is_seen_by_the_chain() {
    each(|cx, rt| {
        let p = object::new_plain_object(rt);
        let c = object::new_object(rt, p);
        assert!(shape::is_prototype_object(p));
        assert!(property::set_index(cx, p, 4, val_str(rt, "inh")).unwrap());
        assert!(shape::proto_index_keys(rt));
        assert_eq!(property::get_index(cx, c, 4).unwrap(), val_str(rt, "inh"));
        assert!(property::has_index(cx, c, 4));
        assert!(property::set_index(cx, c, 4, int(1)).unwrap());
        assert_eq!(property::get_index(cx, c, 4).unwrap(), int(1));
        assert_eq!(object::element_get(c, 4), None);
        let k4 = crate::key::index_key(cx.rt(), 4);
        assert_eq!(props::own_get(c, k4), Some((int(1), attr::DEFAULT)));
        assert_eq!(object::element_get(p, 4), Some(val_str(rt, "inh")));
        checked(rt, p);
        checked(rt, c);
    });
}

// ---- the IC_LAYOUT 3 kinds set -------------------------------------------------------------

/// A script with one LoadKeyed slot `get` and one StoreKeyed slot `set`: it makes three arrays
/// with `set` (`[5]`, `[1.5]`, `["x"]` from `[]`), then reads `a[0]` of each and `h[0]` of a
/// holey `h = [, 7]` at `get`. Returns the unit and the two slots.
fn kinds_program() -> (
    VerifiedUnit,
    ep9_bytecode::FeedbackSlot,
    ep9_bytecode::FeedbackSlot,
) {
    let mut s = Asm::script(0);
    let get = s.b.feedback_slot(FeedbackSlotKind::LoadKeyed);
    let set = s.set_keyed;
    let k = s.b.new_register();
    let make = |s: &mut Asm, at: i32, value: &dyn Fn(&mut Asm)| -> Register {
        let r = s.b.new_register();
        let lit = s.lit;
        s.b.create_empty_array_literal(lit).star(r);
        s.b.lda_smi(at).star(k);
        value(s);
        s.b.set_keyed_property(r, k, set);
        r
    };
    let a = make(&mut s, 0, &|s| {
        s.b.lda_smi(5);
    });
    let b = make(&mut s, 0, &|s| {
        s.lda_num(1.5);
    });
    let c = make(&mut s, 0, &|s| {
        s.lda_str("x");
    });
    let h = make(&mut s, 1, &|s| {
        s.b.lda_smi(7);
    });
    for r in [a, b, c, h] {
        s.b.lda_zero().get_keyed_property(r, get);
    }
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), get, set)
}

/// The kinds the LoadKeyed site of [`kinds_program`] sees.
const SEEN: &[u8] = &[
    ek::PACKED_INT32,
    ek::HOLEY_INT32,
    ek::PACKED_DOUBLE,
    ek::PACKED_ELEMENTS,
];

/// The kinds sets: the LoadKeyed site saw PACKED_INT32, PACKED_DOUBLE, PACKED_ELEMENTS (the
/// inline hits, in the sampled window: the script runs right after the budget reset, so
/// `profile::keyed_hit_kind` records them) and HOLEY_INT32 (the hole read, cold); the
/// StoreKeyed site saw only empty PACKED_INT32 arrays (the kind before each store) and a
/// store that makes a hole. The other bits are as before IC_LAYOUT 3.
#[test]
fn keyed_slots_record_the_kinds_they_saw() {
    keyed_kinds_at(None, SEEN);
}

/// A fast keyed hit in the jitter units of a poll (a budget above `BUDGET`,
/// `profile::budget_after_poll`) records no kind, as `profile::record_call` records no
/// target there: the LoadKeyed site keeps only the kind of its cold hole read. The script's
/// own call takes one unit, so a start at `BUDGET + 1` runs at `BUDGET`, the window's top.
#[test]
fn a_keyed_hit_in_the_jitter_units_records_no_kind() {
    use crate::interp::BUDGET;
    for budget in [BUDGET + 2, BUDGET + 100] {
        keyed_kinds_at(Some(budget), &[ek::HOLEY_INT32]);
    }
    for budget in [BUDGET + 1, BUDGET - 1] {
        keyed_kinds_at(Some(budget), SEEN);
    }
}

/// Runs [`kinds_program`] with the budget `budget` (or the isolate's own) and checks that the
/// LoadKeyed site's kinds set is `get_kinds` and the rest of both slots as
/// [`keyed_slots_record_the_kinds_they_saw`] says.
fn keyed_kinds_at(budget: Option<i64>, get_kinds: &'static [u8]) {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: 0,
                    ..config(d, stress)
                });
                if let Some(b) = budget {
                    iso.cx.budget = b;
                }
                let (u, get, set) = kinds_program();
                let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "kinds.js").map(|_| ()));
                assert!(r.is_ok(), "{r:?}");
                let units = iso.rt.units.borrow();
                let fi = &units.last().unwrap().infos[0];
                let v = feedback::vector(fi).unwrap();
                let bits =
                    |s: ep9_bytecode::FeedbackSlot| feedback::value(v, s.index() as u32) as u32;
                let kset = |ks: &[u8]| ks.iter().map(|&k| elements::kinds_set_bit(k)).sum::<u32>();
                let g = bits(get);
                assert_eq!(
                    g & elements::KINDS_SET_MASK,
                    kset(get_kinds),
                    "{} stress={stress} budget={budget:?}: get {g:#x}",
                    d.name()
                );
                assert_eq!(g & 0x3FF, keyed::INDEX | keyed::ARRAY | keyed::HOLE);
                let st = bits(set);
                assert_eq!(st & elements::KINDS_SET_MASK, kset(&[ek::PACKED_INT32]));
                assert_eq!(
                    st & 0x3FF,
                    keyed::INDEX | keyed::ARRAY | keyed::OUT_OF_BOUNDS
                );
            });
        }
    }
}

/// `d = [0.5, 1.5]` and `e = [1, 2]` filled at one StoreKeyed slot, then at a second one
/// (`hit`): `d[0] = 2.5` (cold: the site's first fast case), `d[1] = 3.5` (a hit: a raw f64
/// store), `d[0] = "s"` (no fit: cold, DOUBLE -> ELEMENTS there), `e[0] = 3` (cold: a new
/// receiver shape, megamorphic), `e[1] = 4` (a hit), `e[0] = 0.25` (no fit for INT32: cold,
/// INT32 -> DOUBLE); then prints `d[0]`, `d[1]`, `e[0]`, `e[1]` and keeps `d`, `e` as globals.
fn store_hit_program() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (fill, hit) = (s.set_keyed, s.b.feedback_slot(FeedbackSlotKind::StoreKeyed));
    let get = s.keyed;
    let k = s.b.new_register();
    let (d, e) = (s.b.new_register(), s.b.new_register());
    let store = |s: &mut Asm, r: Register, at: i32, slot, value: &dyn Fn(&mut Asm)| {
        s.b.lda_smi(at).star(k);
        value(s);
        s.b.set_keyed_property(r, k, slot);
    };
    for r in [d, e] {
        let lit = s.lit;
        s.b.create_empty_array_literal(lit).star(r);
    }
    store(&mut s, d, 0, fill, &|s| {
        s.lda_num(0.5);
    });
    store(&mut s, d, 1, fill, &|s| {
        s.lda_num(1.5);
    });
    store(&mut s, e, 0, fill, &|s| {
        s.b.lda_smi(1);
    });
    store(&mut s, e, 1, fill, &|s| {
        s.b.lda_smi(2);
    });
    store(&mut s, d, 0, hit, &|s| {
        s.lda_num(2.5);
    });
    store(&mut s, d, 1, hit, &|s| {
        s.lda_num(3.5);
    });
    store(&mut s, d, 0, hit, &|s| {
        s.lda_str("s");
    });
    store(&mut s, e, 0, hit, &|s| {
        s.b.lda_smi(3);
    });
    store(&mut s, e, 1, hit, &|s| {
        s.b.lda_smi(4);
    });
    store(&mut s, e, 0, hit, &|s| {
        s.lda_num(0.25);
    });
    for r in [d, e] {
        for i in 0..2 {
            s.b.lda_smi(i).get_keyed_property(r, get);
            s.print_acc();
        }
    }
    s.b.ldar(d);
    s.sta_global("d");
    s.b.ldar(e);
    s.sta_global("e");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// The StoreKeyed hit stores in the storage form of the cell and never makes a transition: a
/// value that does not fit the kind goes cold, where the transition happens.
#[test]
fn the_store_hit_keeps_the_kind() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: 0,
                    ..config(d, stress)
                });
                super::take_output();
                let r = iso.with_cx_checkpoint(|cx| {
                    cx.run_script(store_hit_program(), "hit.js").map(|_| ())
                });
                assert!(r.is_ok(), "{r:?}");
                assert_eq!(
                    super::take_output(),
                    ["s", "3.5", "0.25", "4"],
                    "{}",
                    d.name()
                );
                let da = iso.global_raw("d").unwrap().to_bits();
                let ea = iso.global_raw("e").unwrap().to_bits();
                assert_eq!(elements::kind_of(da), ek::PACKED_ELEMENTS);
                assert_eq!(elements::kind_of(ea), ek::PACKED_DOUBLE);
                assert_eq!(
                    elements::storage(elements::kind_of(ea)),
                    Some(Storage::Doubles)
                );
                elements::check(da).unwrap();
                elements::check(ea).unwrap();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// MC-w18-4: the one map between the VM kinds and `ep9_facts::ElementsKind`, over every value
/// the 4 shape bits hold: 0-6 map to the fact kind of the same meaning (packed or holey, and
/// the storage of the name) and back; 7-15 map to none; every fact kind maps back to itself.
#[test]
fn the_fact_kind_map_is_one_to_one() {
    use ep9_facts::ElementsKind as F;
    let want = [
        (ek::PACKED_INT32, F::PackedInt32),
        (ek::HOLEY_INT32, F::HoleyInt32),
        (ek::PACKED_DOUBLE, F::PackedDouble),
        (ek::HOLEY_DOUBLE, F::HoleyDouble),
        (ek::PACKED_ELEMENTS, F::PackedElements),
        (ek::HOLEY_ELEMENTS, F::HoleyElements),
        (ek::DICTIONARY, F::Dictionary),
    ];
    for k in 0..=ek::MAX {
        let got = elements::to_fact(k);
        match want.iter().find(|&&(v, _)| v == k) {
            Some(&(_, f)) => {
                assert_eq!(got, Some(f), "VM kind {k}");
                assert_eq!(elements::from_fact(f), k, "{f:?}");
                // The names agree on holes and storage.
                assert_eq!(
                    f.name().starts_with("holey"),
                    k != ek::DICTIONARY && elements::is_holey(k),
                    "{k} {f:?}"
                );
            }
            None => assert_eq!(got, None, "reserved VM kind {k}"),
        }
    }
    for b in 0..=u8::MAX {
        if let Some(f) = F::from_u8(b) {
            assert_eq!(elements::to_fact(elements::from_fact(f)), Some(f), "{f:?}");
        }
    }
}

// ---- DICTIONARY elements (M5-P2 ELEM2; H18-Q2, Q4) -------------------------------------------

/// The `length` key.
fn length_key(rt: &Runtime) -> u64 {
    key(rt, "length")
}

/// A descriptor with only a value.
fn vdesc(v: u64) -> Desc {
    Desc {
        value: Some(v),
        ..Desc::default()
    }
}

/// The capacity of the elements cell of `a` (a dense one).
fn capacity(a: u64) -> u64 {
    let e = object::cell(object::rd(object::cell(a), obj::ELEMENTS));
    object::rd_i32(e, object::elems::CAPACITY) as u64
}

/// A data descriptor with all four fields.
fn data_desc(v: u64, w: bool, e: bool, c: bool) -> Desc {
    Desc {
        value: Some(v),
        writable: Some(w),
        enumerable: Some(e),
        configurable: Some(c),
        ..Desc::default()
    }
}

/// H18-Q2: an index 1,024 or more past the capacity moves the array to DICTIONARY (one-way)
/// with its present elements as {w, e, c} entries; holes stay absent; `length` follows the
/// highest index; reads, `in`, own keys, delete and later stores (near or far) go through the
/// dictionary cell; the array never goes back to a dense kind.
#[test]
fn a_far_index_makes_a_dictionary_array() {
    each(|cx, rt| {
        let a = object::new_array(rt, 0);
        for i in 0..3u64 {
            assert!(property::set_index(cx, a, i, int(i as i32 + 1)).unwrap());
        }
        assert!(property::set_index(cx, a, 5, dbl(2.5)).unwrap());
        assert_eq!(elements::kind_of(a), ek::HOLEY_DOUBLE);
        let cap = capacity(a);
        // One short of the gap: still dense.
        let near = cap + u64::from(elements::ARRAY_GAP) - 1;
        assert!(property::set_index(cx, a, near, int(7)).unwrap());
        assert_eq!(elements::kind_of(a), ek::HOLEY_DOUBLE, "inside the gap");
        let cap = capacity(a);
        let far = cap + u64::from(elements::ARRAY_GAP);
        assert!(property::set_index(cx, a, far, val_str(rt, "far")).unwrap());
        assert_eq!(elements::kind_of(a), ek::DICTIONARY, "a store at the gap");
        assert_eq!(cell_kind_at_16(a), kinds::ELEMENTS_DICT.get());
        checked(rt, a);
        assert_eq!(u64::from(object::array_len(a)), far + 1);
        // The elements kept their values; a hole is still absent.
        for (i, want) in [(0, int(1)), (1, int(2)), (2, int(3)), (5, dbl(2.5))] {
            assert_eq!(property::get_index(cx, a, i).unwrap(), want, "a[{i}]");
        }
        assert_eq!(property::get_index(cx, a, near).unwrap(), int(7));
        assert_eq!(property::get_index(cx, a, far).unwrap(), val_str(rt, "far"));
        assert_eq!(property::get_index(cx, a, 3).unwrap(), UNDEFINED, "a hole");
        assert!(!property::has_index(cx, a, 4));
        assert!(property::has_index(cx, a, far));
        // The dense readers find nothing in the dictionary cell (its capacity is 0).
        assert_eq!(object::array_get(a, 0), None);
        let keys = texts(property::own_property_keys(cx, a));
        assert_eq!(
            keys,
            [
                "0".to_string(),
                "1".into(),
                "2".into(),
                "5".into(),
                near.to_string(),
                far.to_string(),
                "length".into()
            ]
        );
        // Stores near the start stay in the dictionary (one-way); a delete removes an entry.
        assert!(property::set_index(cx, a, 3, int(4)).unwrap());
        assert_eq!(elements::kind_of(a), ek::DICTIONARY, "one-way");
        assert_eq!(property::get_index(cx, a, 3).unwrap(), int(4));
        let k1 = crate::key::index_key(rt, 1);
        assert!(property::internal_delete(cx, a, k1).unwrap());
        assert!(!property::has_index(cx, a, 1));
        // A `length` cut drops the entries at or past it.
        let lk = length_key(rt);
        assert!(property::define_own_property(cx, a, lk, vdesc(int(4))).unwrap());
        assert_eq!(object::array_len(a), 4);
        assert!(!property::has_index(cx, a, 5));
        assert!(!property::has_index(cx, a, far));
        assert_eq!(
            texts(property::own_property_keys(cx, a)),
            ["0", "2", "3", "length"]
        );
        checked(rt, a);
    });
}

/// H18-Q4: 2^26 stays the dense limit, but an index or a `length` past it works: up to
/// 2^32 - 2 for an index and 2^32 - 1 for `length` (ECMA-262), as DICTIONARY. Index
/// 2^32 - 1 is not an array index: a property that leaves `length` alone.
#[test]
fn an_index_or_a_length_past_the_dense_limit_works() {
    each(|cx, rt| {
        let max_index = u64::from(u32::MAX) - 1;
        let a = object::new_array(rt, 0);
        assert!(property::set_index(cx, a, 0, int(1)).unwrap());
        assert!(property::set_index(cx, a, max_index, int(2)).unwrap());
        assert_eq!(elements::kind_of(a), ek::DICTIONARY);
        assert_eq!(object::array_len(a), u32::MAX);
        let lk = length_key(rt);
        let len = property::get_own_property(cx, a, lk).unwrap().unwrap();
        assert_eq!(
            len.value,
            Some(dbl(4294967295.0)),
            "length 2^32 - 1, a double"
        );
        // The `length` IC reads +24 as a Value: the double there is the right Number.
        assert_eq!(
            object::rd(object::cell(a), object::arr::LENGTH),
            dbl(4294967295.0)
        );
        // Index 2^32 - 1: a property, `length` stays.
        assert!(property::set_index(cx, a, u64::from(u32::MAX), int(3)).unwrap());
        assert_eq!(object::array_len(a), u32::MAX);
        assert_eq!(
            property::get_index(cx, a, u64::from(u32::MAX)).unwrap(),
            int(3)
        );
        checked(rt, a);

        // A `length` past the dense limit on a dense array: DICTIONARY, the elements kept.
        let b = object::new_array(rt, 0);
        assert!(property::set_index(cx, b, 1, dbl(0.5)).unwrap());
        let big = num_bits(f64::from(object::arr::MAX_LENGTH) + 1.0);
        assert!(property::define_own_property(cx, b, lk, vdesc(big)).unwrap());
        assert_eq!(elements::kind_of(b), ek::DICTIONARY);
        assert_eq!(object::array_len(b), object::arr::MAX_LENGTH + 1);
        assert_eq!(property::get_index(cx, b, 1).unwrap(), dbl(0.5));
        // And down again: the entries stay, `length` is an int32 again.
        assert!(property::define_own_property(cx, b, lk, vdesc(int(2))).unwrap());
        assert_eq!(object::array_len(b), 2);
        assert_eq!(object::rd(object::cell(b), object::arr::LENGTH), int(2));
        assert_eq!(elements::kind_of(b), ek::DICTIONARY, "one-way");
        checked(rt, b);
    });
}

fn num_bits(d: f64) -> u64 {
    crate::conv::num(d)
}

/// H18-Q2 / H17 "Element accessors": an accessor element, an element with attributes other
/// than {w, e, c} and a non-writable `length` are DICTIONARY entries and flags (v0 threw
/// "not implemented"). A non-configurable entry stops a `length` cut (§10.4.2.4 step 17); a
/// non-writable `length` refuses a new index at or past it and any other value.
#[test]
fn attributes_accessors_and_a_read_only_length_live_in_the_dictionary() {
    each(|cx, rt| {
        let a = object::new_array(rt, 0);
        for i in 0..4u64 {
            assert!(property::set_index(cx, a, i, int(i as i32)).unwrap());
        }
        // A read-only, non-configurable element 1.
        let k1 = crate::key::index_key(rt, 1);
        assert!(
            property::define_own_property(cx, a, k1, data_desc(int(9), false, true, false))
                .unwrap()
        );
        assert_eq!(elements::kind_of(a), ek::DICTIONARY);
        let d = property::get_own_property(cx, a, k1).unwrap().unwrap();
        assert_eq!(
            (d.value, d.writable, d.enumerable, d.configurable),
            (Some(int(9)), Some(false), Some(true), Some(false))
        );
        assert!(!property::set_index(cx, a, 1, int(5)).unwrap(), "read-only");
        assert_eq!(property::get_index(cx, a, 1).unwrap(), int(9));
        assert!(
            !property::internal_delete(cx, a, k1).unwrap(),
            "non-configurable"
        );
        // An accessor element 2 (a getter of undefined).
        let k2 = crate::key::index_key(rt, 2);
        let acc = Desc {
            get: Some(UNDEFINED),
            enumerable: Some(false),
            configurable: Some(true),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, a, k2, acc).unwrap());
        let d = property::get_own_property(cx, a, k2).unwrap().unwrap();
        assert!(d.is_accessor());
        assert_eq!(property::get_index(cx, a, 2).unwrap(), UNDEFINED);
        // Not enumerable: own keys keep it, for-in does not.
        assert_eq!(
            texts(property::own_property_keys(cx, a)),
            ["0", "1", "2", "3", "length"]
        );
        // A cut stops at the non-configurable element: `length` = 2, the result false.
        let lk = length_key(rt);
        assert!(!property::define_own_property(cx, a, lk, vdesc(int(0))).unwrap());
        assert_eq!(object::array_len(a), 2);
        assert!(property::has_index(cx, a, 1));
        assert!(!property::has_index(cx, a, 2));
        // A non-writable `length`: the flag in the dictionary cell.
        let ro = Desc {
            writable: Some(false),
            ..Desc::default()
        };
        assert!(property::define_own_property(cx, a, lk, ro).unwrap());
        let d = property::get_own_property(cx, a, lk).unwrap().unwrap();
        assert_eq!((d.value, d.writable), (Some(int(2)), Some(false)));
        assert!(!elements::length_writable(a));
        assert!(
            !property::set_index(cx, a, 2, int(1)).unwrap(),
            "past a read-only length"
        );
        assert!(!property::has_index(cx, a, 2));
        assert!(
            property::set_index(cx, a, 0, int(8)).unwrap(),
            "below it: fine"
        );
        assert!(!property::define_own_property(cx, a, lk, vdesc(int(5))).unwrap());
        assert!(
            property::define_own_property(cx, a, lk, vdesc(int(2))).unwrap(),
            "the same value"
        );
        let rw = Desc {
            writable: Some(true),
            ..Desc::default()
        };
        assert!(
            !property::define_own_property(cx, a, lk, rw).unwrap(),
            "never writable again"
        );
        checked(rt, a);

        // A read-only `length` on a dense array moves it to DICTIONARY too.
        let b = object::new_array(rt, 0);
        assert!(property::set_index(cx, b, 0, dbl(1.5)).unwrap());
        assert!(property::define_own_property(cx, b, lk, ro).unwrap());
        assert_eq!(elements::kind_of(b), ek::DICTIONARY);
        assert_eq!(property::get_index(cx, b, 0).unwrap(), dbl(1.5));
        assert!(!property::set_index(cx, b, 1, int(1)).unwrap());
        assert_eq!(object::array_len(b), 1);
        checked(rt, b);
    });
}

/// H18 Rules "A store far past the end goes to dictionary elements, not to a dense
/// allocation": one high index keeps the memory flat. `a[2**32 - 2] = 1` and a `length` of
/// 2^32 - 1 each allocate a few hundred bytes (v0: `a[2**26 - 1] = 1` allocated 2^26 slots,
/// 540,606,464 B peak RSS, H18 v0 notes). The heap's allocated-bytes counter is the measure:
/// it counts the heap's pages as it takes them (256 KiB steps), so the bound is 1 MiB, 1/512
/// of the v0 figure; the dictionary cell itself has the fewest buckets.
#[test]
fn one_high_index_keeps_rss_flat() {
    const FLAT: u64 = 1 << 20;
    each(|cx, rt| {
        let a = object::new_array(rt, 0);
        assert!(property::set_index(cx, a, 0, int(1)).unwrap());
        let before = rt.heap.stats().allocated_bytes;
        assert!(property::set_index(cx, a, u64::from(u32::MAX) - 1, int(1)).unwrap());
        let grew = rt.heap.stats().allocated_bytes - before;
        assert!(grew < FLAT, "a[2**32 - 2] = 1 allocated {grew} B");
        assert_eq!(elements::kind_of(a), ek::DICTIONARY);
        let d = object::cell(object::rd(object::cell(a), obj::ELEMENTS));
        assert_eq!(
            object::rd_i32(d, elements::dict::BUCKETS) as usize,
            elements::dict::MIN_BUCKETS
        );

        let b = object::new_array(rt, 0);
        let before = rt.heap.stats().allocated_bytes;
        let lk = length_key(rt);
        assert!(property::define_own_property(cx, b, lk, vdesc(dbl(4294967295.0))).unwrap());
        let grew = rt.heap.stats().allocated_bytes - before;
        assert!(grew < FLAT, "length = 2**32 - 1 allocated {grew} B");

        // Under the dense limit but far past the capacity: the gap rule, also flat.
        let c = object::new_array(rt, 0);
        let before = rt.heap.stats().allocated_bytes;
        assert!(
            property::set_index(cx, c, u64::from(object::arr::MAX_LENGTH) - 1, int(1)).unwrap()
        );
        let grew = rt.heap.stats().allocated_bytes - before;
        assert!(grew < FLAT, "a[2**26 - 1] = 1 allocated {grew} B");
        checked(rt, a);
        checked(rt, b);
        checked(rt, c);
    });
}

/// A store or define on an array (the storage-form test).
type Op = dyn Fn(&mut VmCx, u64);

/// H18 Rules "The storage form follows the kind" (M5-P1-ORCH-11: the shape guard is the kind
/// source of the tiers, so the cell at +16 must be the form the shape's kind names): every
/// kind 0-6 that stores, defines and `length` writes reach has the form `storage_form_ok`
/// names (none or ELEMENTS for 0, 1, 4, 5; none or ELEMENTS_DOUBLE for 2, 3; ELEMENTS_DICT
/// for 6), the shape holds the kind, and the check refuses a wrong form.
#[test]
fn the_storage_form_follows_the_shape_kind() {
    each(|cx, rt| {
        let mut seen = [false; 7];
        let lk = length_key(rt);
        let mut note = |a: u64| {
            let k = elements::kind_of(a);
            assert_eq!(
                k,
                shape::elements_kind(shape::shape_of(a)),
                "the shape holds the kind"
            );
            assert!(
                elements::storage_form_ok(a),
                "kind {k}, cell {:#x}",
                cell_kind_at_16(a)
            );
            checked(rt, a);
            seen[k as usize] = true;
        };
        // Each may fail (a read-only element 0 refuses a later store): the kinds still hold.
        let ops: [&Op; 8] = [
            &|cx, a| {
                let _ = property::set_index(cx, a, 0, int(1)).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 2, int(1)).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 0, dbl(0.5)).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 3, dbl(0.5)).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 0, UNDEFINED).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 4, NULL_).unwrap();
            },
            &|cx, a| {
                let _ = property::set_index(cx, a, 5000, int(1)).unwrap();
            },
            &|cx, a| {
                let k = crate::key::index_key(cx.rt(), 0);
                let d = data_desc(int(1), false, true, true);
                let _ = property::define_own_property(cx, a, k, d).unwrap();
            },
        ];
        // Every pair of operations, from an empty array and from a 2-element one.
        for (n, first) in ops.iter().enumerate() {
            for second in ops.iter() {
                for start in [0, 2] {
                    let a = object::new_array(rt, 0);
                    for i in 0..start {
                        assert!(property::set_index(cx, a, i, int(1)).unwrap());
                    }
                    note(a);
                    first(cx, a);
                    note(a);
                    second(cx, a);
                    note(a);
                    if n % 2 == 0 {
                        let _ = property::define_own_property(cx, a, lk, vdesc(int(1))).unwrap();
                        note(a);
                    }
                }
            }
        }
        assert_eq!(seen, [true; 7], "every kind 0-6 was reached");
        // The check refuses a wrong form: a DOUBLE array whose +16 is a Values cell.
        let a = object::new_array(rt, 0);
        assert!(property::set_index(cx, a, 0, dbl(0.5)).unwrap());
        let e = object::rd(object::cell(a), obj::ELEMENTS);
        let values = rt.alloc(kinds::ELEMENTS, object::elems::DATA + 8);
        object::init(values, object::elems::CAPACITY, int(1));
        object::init(values, object::elems::DATA, int(1));
        object::wr(
            &rt.heap,
            object::cell(a),
            obj::ELEMENTS,
            values.to_value().to_bits(),
        );
        assert!(!elements::storage_form_ok(a));
        assert!(elements::check(a).is_err());
        object::wr(&rt.heap, object::cell(a), obj::ELEMENTS, e);
        assert!(elements::storage_form_ok(a));
    });
}
