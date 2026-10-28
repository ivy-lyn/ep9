//! Tests of the megamorphic stub cache (`stub_cache.rs`, M4-P3 IC2) and of the string method
//! loads that `ic.rs` caches from M4-P3 on: megamorphic loads, stores and adds through the
//! cache, the operation in the key (load, 0x24 store, 0x25 define), the validity cell of a
//! prototype case, the flush at every GC, and `KEY_STRING` prototype cases.
//!
//! The harness is `tests::ic` (`T`, one site per function, every dispatch expansion, with and
//! without GC stress, eager feedback).

use super::ic::{
    MASK, SLOT, Site, T, below_a_pad, collect_after_a_pad, each, int, key, object_prototype,
    object_with, own,
};
use crate::frame::UNDEFINED;
use crate::ic::{self, KEY_STRING, State, hk, smi};
use crate::object::{self, attr};
use crate::props;
use crate::runtime::Root;
use crate::shape;
use crate::string;
use crate::stub_cache::{self, Op};
use crate::symbol;

const PROTO_LOAD: i32 = hk::PROTO_LOAD | 0x100;

/// The ShapeId of `o`'s shape (the low half of its bits).
fn id(o: u64) -> u32 {
    shape::shape_of(o) as u32
}

/// The stub cache entry for (`op`, `o`'s shape, `name`).
fn entry(t: &T, op: Op, o: u64, name: &str) -> Option<u64> {
    stub_cache::probe(t.rt(), op, id(o), key(t.rt(), name))
}

/// Five objects with `x` (= 10..14) in five shapes, under `p`.
fn five(t: &T, p: u64) -> [u64; 5] {
    let pads = ["a", "b", "c", "d", "e"];
    core::array::from_fn(|i| {
        object_with(t.rt(), p, &[(pads[i], int(0)), ("x", int(10 + i as i32))])
    })
}

#[test]
fn megamorphic_loads_and_stores_go_through_the_stub_cache() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Set("x", false)]);
        let p = object_prototype(t.rt());
        let objs = five(&t, p);
        for &o in &objs {
            t.ok(0, o, UNDEFINED);
            t.ok(1, o, int(1));
        }
        assert_eq!(
            (t.state(0), t.state(1)),
            (State::Megamorphic, State::Megamorphic)
        );
        // New shapes: the first access misses and fills an entry, the next ones hit it.
        let more: [u64; 3] = core::array::from_fn(|i| {
            object_with(
                t.rt(),
                p,
                &[(["f", "g", "h"][i], int(0)), ("x", int(20 + i as i32))],
            )
        });
        for (i, &o) in more.iter().enumerate() {
            assert_eq!(t.ok(0, o, UNDEFINED), int(20 + i as i32));
            let h = entry(&t, Op::Load, o, "x").expect("a load entry");
            assert_eq!(ic::describe_handler(h).0, smi::FIELD_IN_OBJECT);
            t.ok(1, o, int(30 + i as i32));
            assert!(entry(&t, Op::Store, o, "x").is_some());
            // Hits: the store writes, the load reads it.
            t.ok(1, o, int(40 + i as i32));
            assert_eq!(t.ok(0, o, UNDEFINED), int(40 + i as i32));
            assert_eq!(own(t.rt(), o, "x"), Some(int(40 + i as i32)));
        }
        // An add through the store table: `x` on objects that do not have it.
        let fresh = object_with(t.rt(), p, &[("q", int(0))]);
        let fresh2 = object_with(t.rt(), p, &[("q", int(1))]);
        assert_eq!(id(fresh), id(fresh2));
        t.ok(1, fresh, int(5));
        // The entry is keyed on the shape before the add (`fresh2` still has it).
        assert_eq!(
            entry(&t, Op::Store, fresh2, "x").map(|h| ic::describe_handler(h).0),
            Some(hk::ADD | 0x100)
        );
        t.ok(1, fresh2, int(6));
        assert_eq!(own(t.rt(), fresh2, "x"), Some(int(6)));
        assert_eq!(id(fresh), id(fresh2));
        assert_eq!(t.ok(0, fresh2, UNDEFINED), int(6));
    });
}

/// `ic::mega_load` / `ic::mega_store`, the probes of the miss path (`slow::get_named`,
/// `slow::set_named` call them before the generic op): an answer only for an object at a
/// megamorphic slot with an entry; nothing stored otherwise.
#[test]
fn the_miss_path_probes_answer_from_the_stub_cache() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x"), Site::Set("x", false), Site::Get("y")]);
        let p = object_prototype(t.rt());
        let o = object_with(t.rt(), p, &[("k", int(0)), ("x", int(3))]);
        let x = key(t.rt(), "x");
        // Not megamorphic: no answer.
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        assert_eq!(ic::mega_load(t.rt(), t.vector(0), SLOT, o, x), None);
        let objs = five(&t, p);
        for &q in &objs {
            t.ok(0, q, UNDEFINED);
            t.ok(1, q, int(1));
        }
        let o = object_with(t.rt(), p, &[("m", int(0)), ("x", int(3))]);
        assert_eq!(ic::mega_load(t.rt(), t.vector(0), SLOT, o, x), None);
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        t.ok(1, o, int(3));
        // Entries for `o`'s shape now (unless a GC flushed them, under GC stress).
        let (v0, v1) = (t.vector(0), t.vector(1));
        if entry(&t, Op::Load, o, "x").is_some() {
            assert_eq!(ic::mega_load(t.rt(), v0, SLOT, o, x), Some(int(3)));
            // Not an object: no answer.
            assert_eq!(ic::mega_load(t.rt(), v0, SLOT, int(1), x), None);
        }
        if entry(&t, Op::Store, o, "x").is_some() {
            assert!(ic::mega_store(t.rt(), v1, SLOT, o, x, int(8), false));
            assert_eq!(own(t.rt(), o, "x"), Some(int(8)));
            // A define uses its own entries.
            if entry(&t, Op::Define, o, "x").is_none() {
                assert!(!ic::mega_store(t.rt(), v1, SLOT, o, x, int(9), true));
                assert_eq!(own(t.rt(), o, "x"), Some(int(8)));
            }
        }
        // Another name at a megamorphic slot: no entry, no answer.
        let y = key(t.rt(), "y");
        assert_eq!(ic::mega_load(t.rt(), v0, SLOT, o, y), None);
    });
}

#[test]
fn the_stub_cache_is_empty_after_every_gc() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        let p = object_prototype(t.rt());
        let objs = five(&t, p);
        for &o in &objs {
            t.ok(0, o, UNDEFINED);
        }
        let o = object_with(t.rt(), p, &[("z", int(0)), ("x", int(7))]);
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        assert!(t.rt().stub_cache.len() > 0);
        t.collect();
        assert_eq!(t.rt().stub_cache.len(), 0);
        assert_eq!(entry(&t, Op::Load, o, "x"), None);
        // It fills again.
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        assert!(entry(&t, Op::Load, o, "x").is_some());
    });
}

/// Makes the load site megamorphic, then loads `x` from a new object in a shape no other
/// object has, so that an entry keyed on that shape exists. Returns the masked addresses of
/// the object and its shape.
#[inline(never)]
fn made_an_entry(t: &mut T) -> [usize; 2] {
    let p = object_prototype(t.rt());
    let objs = five(t, p);
    for &o in &objs {
        t.ok(0, o, UNDEFINED);
    }
    let o = object_with(t.rt(), p, &[("lone", int(0)), ("x", int(9))]);
    assert_eq!(t.ok(0, o, UNDEFINED), int(9));
    assert!(entry(t, Op::Load, o, "x").is_some());
    let s = shape::shape_of(o);
    [
        std::hint::black_box(o as usize) ^ MASK,
        std::hint::black_box(s as usize) ^ MASK,
    ]
}

#[test]
fn an_entry_keeps_nothing_alive() {
    each(|c| {
        if c.heap.gc_stress != 0 {
            return; // one collection after `made_an_entry` is the check
        }
        let mut t = T::new(c, &[Site::Get("x")]);
        let (dead, pad_low) = below_a_pad(|| made_an_entry(&mut t));
        // The collection frees the object and its shape: the table did not keep them, and
        // it is empty after the GC.
        collect_after_a_pad(&mut t, pad_low, &dead);
        assert_eq!(t.rt().stub_cache.len(), 0);
    });
}

#[test]
fn a_prototype_case_in_the_stub_cache_checks_its_validity_cell() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m")]);
        let op = object_prototype(t.rt());
        let base = object_with(t.rt(), op, &[("m", int(1))]);
        let mid = object_with(t.rt(), base, &[]);
        let pads = ["a", "b", "c", "d", "e", "f"];
        let objs: [u64; 6] =
            core::array::from_fn(|i| object_with(t.rt(), mid, &[(pads[i], int(0))]));
        for &o in &objs[..5] {
            assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        }
        assert_eq!(t.state(0), State::Megamorphic);
        let o = objs[5];
        assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        let h = entry(&t, Op::Load, o, "m").expect("a prototype entry");
        assert_eq!(ic::describe_handler(h).0, PROTO_LOAD);
        // A value store to the holder: the hit reads the new value.
        assert!(props::own_replace(t.rt(), base, key(t.rt(), "m"), int(2)));
        assert_eq!(t.ok(0, o, UNDEFINED), int(2));
        // A shadowing `m` on `mid`: the cell goes invalid, the generic path finds it.
        props::own_define(t.rt(), mid, key(t.rt(), "m"), int(3), attr::DEFAULT);
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        // The miss put a new case (with a valid cell) in the entry. (Under GC stress the next
        // miss's allocations empty the table, so the probe comes first.)
        let h2 = entry(&t, Op::Load, o, "m").expect("a new entry");
        assert_ne!(h2, h);
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        assert_eq!(t.ok(0, objs[0], UNDEFINED), int(3));
    });
}

#[test]
fn a_load_entry_is_never_used_for_a_store_or_a_define() {
    each(|c| {
        let mut t = T::new(
            c,
            &[
                Site::Get("x"),
                Site::Set("x", true),
                Site::Define("y"),
                Site::Set("y", false),
            ],
        );
        let p = object_prototype(t.rt());
        let objs = five(&t, p);
        for &o in &objs {
            t.ok(0, o, UNDEFINED);
            t.ok(1, o, int(1));
            t.ok(2, o, int(1));
            t.ok(3, o, int(1));
        }
        for i in 0..4 {
            assert_eq!(t.state(i), State::Megamorphic, "f{i}");
        }
        // A read-only `x`: the load entry is a field handler, the strict store throws.
        let ro = object::new_object(t.rt(), p);
        props::own_define(t.rt(), ro, key(t.rt(), "x"), int(8), attr::ENUMERABLE);
        assert_eq!(t.ok(0, ro, UNDEFINED), int(8));
        assert!(entry(&t, Op::Load, ro, "x").is_some());
        t.throws(1, ro, int(9), "TypeError");
        t.throws(1, ro, int(9), "TypeError");
        assert_eq!(own(t.rt(), ro, "x"), Some(int(8)));
        assert_eq!(entry(&t, Op::Store, ro, "x"), None);
        // A read-only `y` on the prototype of `s1` and `s2` (one shape): CreateDataProperty
        // adds `y`; a sloppy `[[Set]]` of `y` must not (it does nothing).
        let rp = object::new_object(t.rt(), p);
        props::own_define(t.rt(), rp, key(t.rt(), "y"), int(0), attr::ENUMERABLE);
        let s1 = object_with(t.rt(), rp, &[("k", int(0))]);
        let s2 = object_with(t.rt(), rp, &[("k", int(0))]);
        assert_eq!(id(s1), id(s2));
        t.ok(2, s1, int(4));
        assert_eq!(own(t.rt(), s1, "y"), Some(int(4)));
        assert!(entry(&t, Op::Define, s2, "y").is_some());
        assert_eq!(entry(&t, Op::Store, s2, "y"), None);
        t.ok(3, s2, int(5));
        assert_eq!(own(t.rt(), s2, "y"), None);
        t.ok(2, s2, int(6));
        assert_eq!(own(t.rt(), s2, "y"), Some(int(6)));
    });
}

#[test]
fn shared_slots_and_strings_at_megamorphic_sites() {
    each(|c| {
        // A slot that `a.x` and `a.y` share is megamorphic from the start: each site probes
        // with its own name.
        let mut t = T::new(c, &[Site::Shared, Site::Get("length")]);
        let p = object_prototype(t.rt());
        let o = object_with(t.rt(), p, &[("x", int(1)), ("y", int(2))]);
        for _ in 0..3 {
            assert_eq!(t.ok(0, o, UNDEFINED), int(2));
        }
        assert!(entry(&t, Op::Load, o, "x").is_some());
        assert!(entry(&t, Op::Load, o, "y").is_some());
        // A string at a megamorphic `length` site: no entry (ShapeId 0); the generic path.
        let lens: [u64; 5] = core::array::from_fn(|i| {
            object_with(
                t.rt(),
                p,
                &[
                    (["a", "b", "c", "d", "e"][i], int(0)),
                    ("length", int(i as i32)),
                ],
            )
        });
        for (i, &l) in lens.iter().enumerate() {
            assert_eq!(t.ok(1, l, UNDEFINED), int(i as i32));
        }
        assert_eq!(t.state(1), State::Megamorphic);
        let s = string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(1, s, UNDEFINED), int(3));
        assert_eq!(t.ok(1, s, UNDEFINED), int(3));
    });
}

// ---- string method loads (ic.rs, M4-P3 IC2) ---------------------------------------------------

#[test]
fn a_string_method_load_is_a_prototype_case_for_strings_only() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("charAt")]);
        // The VM table has no String methods (ep9_builtins adds them): a data property stands
        // for one.
        let sp = t.rt().root(Root::StringPrototype);
        let char_at = int(11);
        props::own_define(
            t.rt(),
            sp,
            key(t.rt(), "charAt"),
            char_at,
            attr::WRITABLE | attr::CONFIGURABLE,
        );
        let s = string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(0, s, UNDEFINED), char_at);
        assert_eq!(t.state(0), State::Monomorphic);
        let (k, cell, holder, _) = t.describe(0, KEY_STRING).expect("a KEY_STRING case");
        assert_eq!(k, PROTO_LOAD);
        assert_eq!(holder, sp);
        assert!(crate::validity::is_valid(cell));
        assert_eq!(t.ok(0, s, UNDEFINED), char_at);
        // A symbol has ShapeId 0 too: it must not take the string case.
        let sym = symbol::new_symbol(t.rt(), UNDEFINED);
        assert_eq!(t.ok(0, sym, UNDEFINED), UNDEFINED);
        // A value store to String.prototype.charAt: the hit reads it.
        assert!(props::own_replace(
            t.rt(),
            sp,
            key(t.rt(), "charAt"),
            int(1)
        ));
        assert_eq!(t.ok(0, s, UNDEFINED), int(1));
        // A shadowing data property on... String.prototype itself changes its shape and its
        // cell: the case misses and is made again.
        props::own_define(t.rt(), sp, key(t.rt(), "pad"), int(0), attr::DEFAULT);
        assert!(!crate::validity::is_valid(cell));
        assert_eq!(t.ok(0, s, UNDEFINED), int(1));
        let (_, cell2, ..) = t.describe(0, KEY_STRING).unwrap();
        assert!(crate::validity::is_valid(cell2));
    });
}

#[test]
fn a_missing_string_method_is_undefined_until_object_prototype_gets_it() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("nope")]);
        let s = string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(0, s, UNDEFINED), UNDEFINED);
        assert_eq!(
            t.describe(0, KEY_STRING).map(|d| d.0),
            Some(hk::MISSING | 0x100)
        );
        assert_eq!(t.ok(0, s, UNDEFINED), UNDEFINED);
        let op = object_prototype(t.rt());
        props::own_define(t.rt(), op, key(t.rt(), "nope"), int(4), attr::DEFAULT);
        assert_eq!(t.ok(0, s, UNDEFINED), int(4));
        assert_eq!(t.ok(0, s, UNDEFINED), int(4));
    });
}
