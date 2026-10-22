//! Objects with internal slots on the builtin ABI (H20; M2-P3 ABI-SLOTS): the append-only
//! ObjectClass list against its golden, `new_object_with_slots`, `slot`, `set_slot` and
//! `object_class`, the TypeError of a wrong class, and slot values under GC stress.

use std::path::PathBuf;

use super::{config, isolate};
use crate::abi::ObjectClass;
use crate::runtime::Runtime;
use crate::{Cx, Dispatch, Isolate, JsObject, Local};

/// Runs `f` with a builtin-ABI context of a fresh isolate, then collects and verifies the
/// heap. `stress`: a collection before every allocation, then the heap verifier.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let out = {
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

/// Takes the pending exception: "Name: message".
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// The string in slot `i` of `o` (class `class`) as Rust text.
fn slot_text<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, class: ObjectClass, i: usize) -> String {
    let v = cx.slot(o, class, i).unwrap();
    cx.to_rust_string(v).unwrap()
}

// ---- the append-only list -------------------------------------------------------------------

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/object-classes.txt")
}

/// The golden text of `classes` (one line per class, in id order).
fn text(classes: &[ObjectClass]) -> String {
    let mut s = String::from(
        "# ep9 ObjectClass ids (H20, M2-P3 ABI-SLOTS). Append-only: never reorder, remove or\n\
         # reuse a line; `EP9_BLESS=1` rewrites this file after a review.\n\
         # id  name\n",
    );
    for (i, c) in classes.iter().enumerate() {
        s.push_str(&format!("{i:>4}  {}\n", c.name()));
    }
    s
}

fn read_golden() -> String {
    std::fs::read_to_string(golden_path())
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", golden_path().display()))
        .replace("\r\n", "\n")
}

#[test]
fn object_classes_match_the_golden() {
    let actual = text(ObjectClass::ALL);
    if std::env::var_os("EP9_BLESS").is_some() {
        std::fs::write(golden_path(), &actual).unwrap();
        return;
    }
    let want = read_golden();
    assert!(
        actual == want,
        "the ObjectClass ids changed (ids are append-only).\n--- golden\n{want}\n--- now\n{actual}\n\
         If only lines were appended, run with EP9_BLESS=1 and review the diff."
    );
}

#[test]
fn swapping_two_object_classes_fails_the_golden() {
    let n = ObjectClass::ALL.len();
    assert!(n >= 4);
    for (a, b) in [(0, 1), (2, 3), (n - 2, n - 1)] {
        let mut classes = ObjectClass::ALL.to_vec();
        classes.swap(a, b);
        assert_ne!(text(&classes), read_golden(), "swap {a} and {b}");
    }
}

#[test]
fn a_class_id_is_its_position_in_the_list() {
    assert_eq!(ObjectClass::ALL.len(), 11);
    for (i, &c) in ObjectClass::ALL.iter().enumerate() {
        assert_eq!(usize::from(c.id()), i, "{}", c.name());
        assert_eq!(ObjectClass::from_id(c.id()), Some(c));
    }
    assert_eq!(ObjectClass::from_id(11), None);
    assert_eq!(ObjectClass::from_id(u16::MAX), None);
    assert_eq!(
        (ObjectClass::Promise.id(), ObjectClass::RegExp.id()),
        (0, 6)
    );
}

// ---- new_object_with_slots, slot, set_slot, object_class ----------------------------------------

#[test]
fn new_slots_are_undefined_and_keep_what_set_slot_stores() {
    with_cx(false, |cx, _| {
        let proto = cx.new_object();
        let m = cx.new_object_with_slots(proto, ObjectClass::Map, 3);
        assert_eq!(cx.object_class(m), Some(ObjectClass::Map));
        assert!(cx.get_prototype_of(m).unwrap().bits() == proto.bits());
        for i in 0..3 {
            assert!(cx.slot(m, ObjectClass::Map, i).unwrap().is_undefined());
        }
        let s = cx.new_string("data").unwrap();
        cx.set_slot(m, ObjectClass::Map, 1, s).unwrap();
        cx.set_slot(m, ObjectClass::Map, 2, Local::number(2.5))
            .unwrap();
        assert_eq!(slot_text(cx, m, ObjectClass::Map, 1), "data");
        assert_eq!(
            cx.slot(m, ObjectClass::Map, 2).unwrap().as_number(),
            Some(2.5)
        );
        assert!(cx.slot(m, ObjectClass::Map, 0).unwrap().is_undefined());
        // A null prototype (a PromiseReaction record) and a class with no slots.
        let r = cx.new_object_with_slots(Local::null(), ObjectClass::PromiseReaction, 3);
        assert!(cx.get_prototype_of(r).unwrap().is_null());
        let d = cx.new_object_with_slots(proto, ObjectClass::Date, 0);
        assert_eq!(cx.object_class(d), Some(ObjectClass::Date));
    });
}

#[test]
fn object_class_is_none_for_everything_else() {
    with_cx(false, |cx, _| {
        let o = cx.new_object();
        let a = cx.new_array();
        let s = cx.new_string("x").unwrap();
        for v in [
            o.value(),
            a.value(),
            s.value(),
            Local::undefined(),
            Local::null(),
            Local::number(1.0),
            Local::boolean(true),
        ] {
            assert_eq!(cx.object_class(v), None);
        }
    });
}

/// RequireInternalSlot: an object of another class, an ordinary object and a primitive each
/// give a catchable TypeError (the pending exception), for `slot` and for `set_slot`.
#[test]
fn a_wrong_class_throws_a_type_error() {
    with_cx(false, |cx, rt| {
        let p = cx.new_object_with_slots(Local::null(), ObjectClass::Promise, 5);
        let o = cx.new_object();
        let s = cx.new_string("m").unwrap();
        let cases: [(Local<'_>, &str); 5] = [
            (p.value(), "object"),
            (o.value(), "object"),
            (s.value(), "\"m\""),
            (Local::undefined(), "undefined"),
            (Local::number(3.0), "3"),
        ];
        for (v, shown) in cases {
            assert!(cx.slot(v, ObjectClass::Map, 0).is_err());
            assert_eq!(
                take_error(cx, rt),
                format!("TypeError: {shown} is not a Map object")
            );
            assert!(cx.set_slot(v, ObjectClass::Set, 0, Local::null()).is_err());
            assert_eq!(
                take_error(cx, rt),
                format!("TypeError: {shown} is not a Set object")
            );
        }
        // The right class still reads after the errors.
        assert!(cx.slot(p, ObjectClass::Promise, 4).unwrap().is_undefined());
    });
}

#[test]
#[should_panic(expected = "internal slot 2 out of range (the object has 2)")]
fn a_slot_index_past_the_count_panics() {
    with_cx(false, |cx, _| {
        let m = cx.new_object_with_slots(Local::null(), ObjectClass::Map, 2);
        let _ = cx.slot(m, ObjectClass::Map, 2);
    });
}

#[test]
#[should_panic(expected = "internal slot 0 out of range (the object has 0)")]
fn a_class_with_no_slots_has_no_slot_zero() {
    with_cx(false, |cx, _| {
        let m = cx.new_object_with_slots(Local::null(), ObjectClass::Date, 0);
        let _ = cx.set_slot(m, ObjectClass::Date, 0, Local::null());
    });
}

#[test]
fn the_slot_count_is_bounded() {
    with_cx(false, |cx, _| {
        let n = ObjectClass::MAX_SLOTS;
        let m = cx.new_object_with_slots(Local::null(), ObjectClass::ArrayBuffer, n);
        cx.set_slot(m, ObjectClass::ArrayBuffer, n - 1, Local::number(1.0))
            .unwrap();
        assert_eq!(
            cx.slot(m, ObjectClass::ArrayBuffer, n - 1)
                .unwrap()
                .as_number(),
            Some(1.0)
        );
    });
    let r = std::panic::catch_unwind(|| {
        with_cx(false, |cx, _| {
            cx.new_object_with_slots(Local::null(), ObjectClass::Map, ObjectClass::MAX_SLOTS + 1);
        })
    });
    assert!(
        r.is_err(),
        "{} slots must panic",
        ObjectClass::MAX_SLOTS + 1
    );
}

/// Property code never touches the internal slots: named properties, deletes (a dictionary
/// move), a new prototype and PreventExtensions leave the class and every slot as they were.
#[test]
fn property_changes_leave_the_slots_alone() {
    with_cx(true, |cx, _| {
        let m = cx.new_object_with_slots(Local::null(), ObjectClass::Set, 2);
        let s = cx.new_string("kept").unwrap();
        cx.set_slot(m, ObjectClass::Set, 0, s).unwrap();
        cx.set_slot(m, ObjectClass::Set, 1, Local::number(7.0))
            .unwrap();
        for i in 0..12 {
            let k = format!("p{i}");
            cx.set(m, k.as_str(), Local::number(f64::from(i)), true)
                .unwrap();
        }
        assert!(cx.delete(m, "p3").unwrap());
        assert!(cx.delete(m, "p0").unwrap());
        let proto = cx.new_object();
        assert!(cx.set_prototype_of(m, proto).unwrap());
        assert!(cx.prevent_extensions(m).unwrap());
        assert_eq!(cx.object_class(m), Some(ObjectClass::Set));
        assert_eq!(slot_text(cx, m, ObjectClass::Set, 0), "kept");
        assert_eq!(
            cx.slot(m, ObjectClass::Set, 1).unwrap().as_number(),
            Some(7.0)
        );
        let p11 = cx.get(m, "p11").unwrap();
        assert_eq!(p11.as_number(), Some(11.0));
        // The slots are not properties.
        let keys = cx.own_property_keys(m).unwrap();
        let n = cx.get(keys, "length").unwrap();
        assert_eq!(n.as_number(), Some(10.0));
    });
}

/// Objects and strings stored with `set_slot` survive a collection before every allocation
/// (GC stress, heap verifier on): the only path to them is array -> slot object -> slots cell.
#[test]
fn slot_values_survive_gc_stress() {
    with_cx(true, |cx, _| {
        let all = cx.new_array();
        let proto = cx.new_object();
        for i in 0..40u64 {
            let class = ObjectClass::ALL[i as usize % ObjectClass::ALL.len()];
            let o = cx.new_object_with_slots(proto, class, 3);
            let s = cx.new_string(&format!("s{i}")).unwrap();
            cx.set_slot(o, class, 0, s).unwrap();
            let inner = cx.new_object();
            let t = cx.new_string(&format!("t{i}")).unwrap();
            cx.set(inner, "t", t, true).unwrap();
            cx.set_slot(o, class, 1, inner).unwrap();
            // Slot 2 holds the previous slot object: a chain through the slots cells.
            if i > 0 {
                let prev = cx.get_index(all, i - 1).unwrap();
                cx.set_slot(o, class, 2, prev).unwrap();
            }
            cx.set_index(all, i, o, true).unwrap();
        }
        cx.collect_garbage();
        for i in 0..40u64 {
            let class = ObjectClass::ALL[i as usize % ObjectClass::ALL.len()];
            let o = cx.get_index(all, i).unwrap().as_object().unwrap();
            assert_eq!(cx.object_class(o), Some(class));
            assert_eq!(slot_text(cx, o, class, 0), format!("s{i}"));
            let inner = cx.slot(o, class, 1).unwrap().as_object().unwrap();
            let t = cx.get(inner, "t").unwrap();
            assert_eq!(cx.to_rust_string(t).unwrap(), format!("t{i}"));
            let prev = cx.slot(o, class, 2).unwrap();
            if i > 0 {
                assert!(prev.bits() == cx.get_index(all, i - 1).unwrap().bits());
            } else {
                assert!(prev.is_undefined());
            }
        }
    });
}

/// A slot value whose only owner is the slot object survives while that object lives in a
/// Rust local, and a value stored over it is released (the heap verifier checks the cell).
#[test]
fn a_slot_holds_its_value_alone() {
    with_cx(true, |cx, _| {
        let m = cx.new_object_with_slots(Local::null(), ObjectClass::Map, 1);
        {
            let s = cx.new_string("only in the slot").unwrap();
            cx.set_slot(m, ObjectClass::Map, 0, s).unwrap();
        }
        for i in 0..20 {
            cx.new_string(&format!("garbage {i}")).unwrap();
            cx.new_object();
        }
        cx.collect_garbage();
        assert_eq!(slot_text(cx, m, ObjectClass::Map, 0), "only in the slot");
        cx.set_slot(m, ObjectClass::Map, 0, Local::null()).unwrap();
        cx.collect_garbage();
        assert!(cx.slot(m, ObjectClass::Map, 0).unwrap().is_null());
    });
}
